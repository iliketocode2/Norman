# The live oracle: `ask` against a real model

Until now, µNorman models answered only from scripts. This document specifies
the **live oracle**: each `ask` becomes one call to the Anthropic Messages API
(`POST /v1/messages`), and the provider's reported token usage replaces the test
tokenizer.

The semantics of `04`/`07` don't change. What changes is *who answers*
M-ASK-ISSUE's request and *what clock* M-TIME reads.

Sources: the Claude API documentation (structured outputs, token counting,
errors), and decisions A–E below, made in session.

---

## 0. Decisions

| # | Question | Decision |
|---|---|---|
| A | Thinking tokens count toward `max_tokens` and are billed as output. How do we budget them? | **Each model grant carries a thinking allowance**, `[think n]` (default 2000). It's added to both `max_tokens` and the reservation, so Theorem 1 stays sound on every model. |
| B | What is a refusal (`stop_reason: "refusal"`)? | **A new failure, `Refused (category Sym)`.** `retry` doesn't retry it, since asking again usually gets refused again. |
| C | Do we retry 429 / 5xx inside the HTTP client? | **No hidden retries.** They become `ToolError`, so `retry` handles them and the trace shows them. This keeps D3: `ask` asks once. |
| D | How is an ask's input size known before the call? | **Call `count_tokens` with the same request first.** It's free and has its own rate limit. The docs call the count an *estimate* that "might differ by a small amount," so the reservation adds a safety margin (§3). |
| E | Which model by default? | **`claude-sonnet-5`, as a default only.** Every field of a model grant can be overridden, and the default lives in one place ([`src/defaults.rs`](../src/defaults.rs)) so it's easy to change later. |

### The default model, stated plainly

A model grant with no fields, `(grant m (model))`, means:

| Field | Default | Meaning |
|---|---|---|
| `[id "claude-sonnet-5"]` | Claude Sonnet 5 | the API model ID |
| `[in 2]` | $2 per million input tokens | µ$ per input token |
| `[out 10]` | $10 per million output tokens | µ$ per output token |
| `[ceiling 128000]` | 128K | the model's maximum output tokens |
| `[think 2000]` | 2000 | thinking allowance, in tokens (decision A) |

Any field can be given explicitly: `(grant m (model [id "claude-opus-5"] [in 5] [out 25]))`.
**Prices are part of the grant, not looked up.** The API doesn't report
prices, so a grant is where the program states what it believes a token
costs. If the prices are wrong, the budget arithmetic is wrong. Check current
pricing when changing models.

---

## 1. One `ask`, one request

| µNorman | Messages API |
|---|---|
| `(ask m τ ctx 's)` | `POST /v1/messages` with `model = m.id` |
| `System` messages | the top-level `system` field (joined, in order) |
| `User` messages | `{"role": "user"}` turns |
| `Tool` messages | user turns whose text is marked as tool output |
| `Assistant` messages | `{"role": "assistant"}` turns |
| adjacent messages of the same role | merged into one turn |
| context ending with an assistant turn | **checked run-time error**. Current models reject assistant prefill with a 400, and a prefill isn't a question anyway. |
| `τ` | `output_config.format = {"type": "json_schema", "schema": S(τ)}` (§2) |
| `max_out(m, τ)` | `max_tokens = min(ceiling, bound(τ) + think)` |
| thinking | left at the model's default (adaptive); `think` budgets for it |

The answer is JSON in the response's `text` block. It goes through the existing
`validate_Θ(j, τ)`, so a live reply and a scripted reply are checked by the
same code.

---

## 2. Types to JSON Schema: `S(τ)`

The API constrains decoding to a JSON Schema with some limitations. The ones
that matter here:

- the root must be an object
- every object needs `additionalProperties: false`
- recursive schemas aren't supported
- `maxLength` isn't supported

| µNorman `τ` | `S(τ)` |
|---|---|
| `Text`, `(Text n)` | `{"type": "string"}`; the bound `n` is enforced by `validate`, not the API |
| `Sym` | `{"type": "string"}` |
| `Num` | `{"type": "integer"}` |
| `Bool` | `{"type": "boolean"}` |
| `(List τ)`, `(List τ n)` | `{"type": "array", "items": S(τ)}`; `n` enforced by `validate` |
| record `R` | `{"type": "object", "properties": {f: S(τ_f)…}, "required": [all fields], "additionalProperties": false}` |
| datatype `D` | `{"anyOf": [one object per constructor, with "tag": {"const": "K"} plus its fields]}` |

**Root wrapping.** If `S(τ)` isn't an object schema (for example `(Text 20)`, or
a datatype's `anyOf`), the request asks for `{"value": S(τ)}` and the reply is
unwrapped before `validate`. `bound(τ)` accounts for the wrapper's 10 bytes (`{"value":` and `}`).

**Recursive datatypes** can't be asked of a live model. That's a checked
run-time error naming the type. Scripts can still produce them.

Where the API can't enforce part of a type (string and list bounds),
`validate` does. So **the guarantee a program sees is the same in scripted and
live mode**: a returned value has type `τ`, including its bounds, or the `ask`
failed with `Invalid`.

---

## 3. Reservation and charging

`07`'s M-ASK-ISSUE needs the input size *before* the call. In live mode it
comes from `count_tokens`, called with the same model, system, messages and
`output_config`. That also counts the system prompt the API injects for the
schema.

```
n_in   = count_tokens(request)                      ; free; separate rate limit
n_in*  = ⌈n_in × 1.05⌉ + 32                          ; safety margin: the count is an estimate
ĉ      = n_in* × in + max_tokens × out               ; reservation
charge = usage.input_tokens × in + usage.output_tokens × out
```

**Theorem 1 in live mode.** The charge is at most `ĉ` whenever the billed
input is within the margin of the count, and billed output never exceeds
`max_tokens`. The docs add that "You are not billed for system-added tokens,"
so billed input is usually *at or below* the count. The margin (5% + 32
tokens) is a constant in `src/defaults.rs`. If a charge ever exceeds its
reservation, the interpreter still records the true charge. A trace entry flagged
`over-reservation` then shows exactly where the estimate failed. The
guarantee is honest about its one assumption rather than silently wrong.

**The count is a new event, M-ASK-COUNT.** In live mode, counting takes real
time, so `07`'s issue step splits in two:

1. **M-ASK-COUNT.** Expired-deadline check, then `count_tokens`. No money is
   reserved yet.
2. **M-ASK-ISSUE.** The reservation check with the counted size. Then either
   refuse (`OverBudget`, no charge; counting was free) or send the
   request.

In scripted mode, counting is instantaneous: the test tokenizer, as before. So
the `07` machine is unchanged there, and every existing test still means what
it did.

---

## 4. Outcomes

| API outcome | µNorman result | Charge |
|---|---|---|
| `stop_reason: "end_turn"`, valid JSON | the value | `usage` |
| `stop_reason: "end_turn"`, JSON fails `validate` (e.g. a `(Text n)` bound) | `fail (Invalid raw)` | `usage` |
| `stop_reason: "max_tokens"` | `fail (Invalid raw)`: truncated | `usage` |
| `stop_reason: "refusal"` | `fail (Refused 'category)` (decision B) | `usage` |
| HTTP 429, 500, 529; network failure | `fail (ToolError msg)` (decision C) | none |
| HTTP 400, 401, 403, 404, 413 | **checked run-time error**: a bad key, model ID or request, i.e. a bug in the program or its grants | none |
| deadline reached before the reply | `fail PastDeadline` (M-TIME's cut); the HTTP call is abandoned | the reservation (`07` §5) |

`Refused` joins `Failure`:

```scheme
(datatype Failure [Invalid (raw Text)] [ToolError (message Text)] [Refused (category Sym)]
                  [OverBudget] [PastDeadline] [Raised (reason Sym)])
```

`retry` and `attempt` are unchanged. They recover only `Invalid` and
`ToolError`, so a refusal propagates, and a refusal inside `best-of` cancels
its siblings. That's deliberate: every attempt sends the same request.
Scripts can inject refusals with `(refusal "category")`, which makes the path
testable without a real model.

---

## 5. The clock, and concurrency

In scripted mode, M-TIME jumps a **virtual** clock to the next scripted
completion. In live mode:

- **Each request runs on its own worker thread.** So concurrent workflow nodes
  really are concurrent, and `par` really saves time.
- **The clock is real:** milliseconds since the run started.
- **M-TIME becomes "block until the next reply arrives, or until the earliest
  deadline among in-flight requests, whichever is first."** A reply is
  processed as a batch of everything that arrived by then, in issue order. At
  a deadline, the requests due are cut, exactly as `07` §4.2 says.

Both modes sit behind one interface (§6), so the machine code doesn't branch on
the mode.

---

## 6. Implementation plan

1. **Refactor the oracle into a trait.** `Machine` stops pulling from the
   script directly. It asks an `Oracle` to *count*, *issue*, *cancel*, and
   deliver the *next batch* of completions up to a time limit. Add
   `Refused`, `(refusal …)` script entries, and the model-grant fields
   `id`/`think` with the defaults of §0. **Every existing test must still
   pass unchanged.**
2. **Pure translations,** each with unit tests and no network:
   `S(τ)`, context → request body, response body → outcome.
3. **The live client.** Raw HTTP (there's no official Rust SDK), worker
   threads, a real clock. It's tested against a **local stub server** that
   replays canned API responses, so 429s, truncation and refusals are tested
   without spending money.
4. **One opt-in real call.** `cargo test -- --ignored` with
   `ANTHROPIC_API_KEY` set runs the analyst example against Sonnet 5. It costs
   well under $0.01.

---

## 7. What this doesn't do yet

- **No prompt caching.** `cache_control` isn't sent, and cache-read discounts
  aren't modelled in charges.
- **No streaming.** Requests whose `max_tokens` is large (unbounded `Text`
  answers) should stream to avoid HTTP timeouts. That's part of step 3.
- **No effort control.** A future `[effort low]` grant field would map to
  `output_config.effort`.
- **Prices are not fetched.** They're stated in the grant (§0).
