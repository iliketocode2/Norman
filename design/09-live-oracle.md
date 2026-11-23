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
| `max_out(m, τ)` | `max_tokens = min(ceiling, bound(τ) + wrapper + think)`, where wrapper is 10 when the answer is wrapped (§2) and 0 otherwise |
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
| `(D τ₁ …)`, `(R τ₁ …)` | the schema of `D` or `R` with each type parameter replaced by its argument's schema ([`12`](12-types-and-effects.md), T5). Root wrapping depends only on the definition's shape, so `(Option Num)` is wrapped and `(Pair τ₁ τ₂)` isn't |

**Root wrapping.** If `S(τ)` isn't an object schema (for example `(Text 20)`, or
a datatype's `anyOf`), the request asks for `{"value": S(τ)}` and the reply is
unwrapped before `validate`. `bound(τ)` accounts for the wrapper's 10 bytes (`{"value":` and `}`).

**Recursive datatypes** can't be asked of a live model. That's a checked
run-time error naming the type. Scripts can still produce them. A type that
merely *nests* itself at different arguments, such as
`(Option (Option Num))`, isn't recursive and is asked for normally.

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

1. ✅ **Refactor the oracle into a trait.** `Machine` stops pulling from the
   script directly. It asks an `Oracle` to *count*, *issue*, *cancel*, and
   deliver the *next batch* of completions up to a time limit. Add
   `Refused`, `(refusal …)` script entries, and the model-grant fields
   `id`/`think` with the defaults of §0. **Every existing test must still
   pass unchanged.**
2. ✅ **Pure translations,** each with unit tests and no network:
   `S(τ)`, context → request body, response body → outcome. `S(τ)` and the
   root-wrapping rule are in [`src/types.rs`](../src/types.rs), next to
   `bound` and `validate`. The request and response formats are in
   [`src/anthropic.rs`](../src/anthropic.rs), with 14 unit tests. The 10-byte
   wrapper is counted in `max_tokens` in *both* modes, so scripted and live
   reservations use the same arithmetic.
3. ✅ **The live client.** Raw HTTP (there's no official Rust SDK), worker
   threads, a real clock. It's tested against a **local stub server** that
   replays canned API responses, so 429s, truncation and refusals are tested
   without spending money. The client is
   [`src/live.rs`](../src/live.rs), using `ureq` 2. The stub-server tests are
   [`tests/live_stub.rs`](../tests/live_stub.rs), 9 tests covering: an answer
   charged from reported usage, and the exact request sent; a 429 retried and
   shown in the trace; a refusal not retried; `max_tokens` truncation; 401s
   as run-time errors; a transient count failure; the exact reservation
   boundary; a deadline cut; and two 600 ms asks finishing together. Run
   with `norman --live [--trace] FILE`.

   **What step 3 narrows or defers:**
   - **Counting is synchronous.** M-ASK-COUNT blocks the machine for one
     round trip, so the counts of concurrent asks happen one after another.
     The asks themselves are concurrent.
   - **No streaming.** A request with a very large `max_tokens` (an unbounded
     `Text` answer) can hit the 10-minute HTTP timeout.
   - **No tool hosts in live mode** at step 3. *Since done:* `--python` backs
     every `kernel` grant with a real, unsandboxed Python process
     ([`../src/tools.rs`](../src/tools.rs), `08` deviation 5). Scripted tools
     still can't be mixed with a live model.
   - **Abandoned asks keep running.** When a deadline cuts an ask, or
     fail-fast cancels it, its worker thread finishes the HTTP call and the
     reply is discarded. (An abandoned *call* is different: its host is
     interrupted, which kills a Python kernel.) The provider may bill for it, and µNorman charges the
     reservation, which is the upper bound (`07` §5).
   - **Credentials** come from `ANTHROPIC_API_KEY`, or `ANTHROPIC_AUTH_TOKEN` as
     a bearer token. `ANTHROPIC_BASE_URL` overrides the endpoint (that's how
     the stub is reached).
4. ✅ **One opt-in real call.**
   `cargo test --test live_real -- --ignored --nocapture` with
   `ANTHROPIC_API_KEY` set asks Claude Sonnet 5 for a `Verdict`.
   [`tests/live_real.rs`](../tests/live_real.rs); three tests, two of which
   spend money, together well under a cent.
   [`LIVE-SETUP.md`](../LIVE-SETUP.md) is the operator's companion: how to get
   a credential and how to cap the spend.

---

## 6a. What the first real call found

Run on 2026-10-06, against `claude-sonnet-5`.

```
result:  (Sell "A significant revenue decline combined with a steep 17-point
          gross margin contraction signals deteriorating pricing power…")
spent:   $0.001846
  ask analyst  in 538 tok, out 77 tok, cost $0.001846, 177..2625 ms, ok
```

**Four things that had only ever been checked against our own stub now hold
against the real API.**

1. **The schema is accepted.** A sum type compiles to an `anyOf` of tagged
   objects, wrapped in `{"value": …}` because the root isn't an object (§2).
   The API took it and the model obeyed it.
2. **The answer validated.** `validate_Θ` turned the reply into a real
   `Verdict`. The reason came back at 175 bytes, inside the type's 400 — even
   though the schema never states that bound, which is enforced afterwards. One
   sample, so this is encouraging rather than conclusive.
3. **The reservation bounded the bill,** with no `over-reservation` in the
   trace. The thinking allowance of 2000 tokens was ample: the model spent 77
   output tokens in total on this task.
4. **An unaffordable ask is refused for free** on the live path too: $0 spent,
   no trace entry, the model never called.

**And one surprise, which is the reason to make the call.**

The context was about 175 characters, but the ask billed **538 input tokens**.
Asking `count_tokens` (which is free) for the same request with and without
`output_config` says where they went:

| | tokens |
|---|---|
| messages alone | 60 |
| messages + schema | 538 |
| **the schema** | **478** (from 521 bytes of JSON) |
| what the test tokenizer predicts (`05`, convention 4) | 39 |

**The schema is 89% of the input, and it is re-sent on every ask.** Three
consequences:

- **Decision D is what makes Theorem 1 hold in live mode.** Counting the real
  request, rather than estimating from the context, is not a refinement: an
  estimate from the context alone would have been thirteen times too small, and
  every reservation would have been wrong.
- **A type costs money on input, not just on output.** `04`'s cost model and
  law A1 of `06` both priced an ask as `price_m(|ctx|, max_out(m, τ))`. The
  type appears in *both* arguments: `price_m(|ctx| + |schema(τ)|, max_out(m,
  τ))`. Law A3 was false as a result (`06` §7, #9).
- **Prompt caching stops being a nicety** (§7). In a loop like CP-Agent the
  same schema is re-sent every iteration. Caching it is worth real money.

---

## 7a. Prompt caching, and a tokenizer that counts the schema

§6a found that the answer type's schema is most of an ask's input and is
re-sent every time. This section is how µNorman stops paying full price for
it, and how scripted runs learned to count it. Three decisions were made with
the user:

| # | Question | Decision |
|---|---|---|
| P1 | When does an ask use the cache? | **Opt-in, per model grant:** `(model [cache 5min])` or `[cache 1h]`. A cache write costs 1.25× the input price (2× for the hour), so a one-off ask on a caching grant pays more, not less. Loops like CP-Agent, `retry` and `repair` are where it pays, and their author opts in. A grant without `[cache]` behaves exactly as before. |
| P2 | Do scripted runs simulate the cache? | **Yes,** deterministically, so a scripted run is charged what a live one would be, and `check-within` on cost means the same thing in both. |
| P3 | How is the read price stated? | **`[cache-read p]`, in dollars per million tokens,** like `[in]` and `[out]`, which now also accept up to three decimals (`[in 0.1]`). It defaults to a tenth of `[in]`, the rate for most models; Claude Opus 5.5 and Sonnet 5.5 read at a twentieth, and Claude Fable 5.1 and 5.2 at a fortieth, so a grant for one of those states it. Writes need no field: they are always 1.25× or 2× `[in]`. |

**The request.** A caching grant sends two breakpoints, both with the
grant's lifetime: an explicit `cache_control` on the system prompt, and a
top-level `cache_control`, which is automatic caching. The API puts the
automatic breakpoint on the last block and moves it forward as a
conversation grows, so an agent loop reads everything the previous turn
wrote. The explicit one lets asks that share a system prompt and an answer
type but differ after it share that prefix. `count_tokens` is sent without
either, since caching doesn't change what is counted.

**Charging.** The response's `usage` splits input three ways, and the
machine prices them separately: `input_tokens` (the uncached remainder) at
`[in]`, `cache_creation_input_tokens` at 1.25× or 2× `[in]` (broken down by
lifetime when the API gives `usage.cache_creation`, and otherwise at the
grant's), and `cache_read_input_tokens` at `[cache-read]`. Prices are kept in
nano-dollars per token and the charge is summed in quarter nano-dollars,
where all of these are whole, then rounded up to a micro-dollar once per ask.
The trace records how many input tokens were read and written; `--trace`
prints them.

**The reservation (Theorem 1′).** Before a cached ask is sent, nobody knows
how much of it will be read and how much written. Every input token might be
a write, so a caching grant reserves input at the write price: 1.25× (or 2×)
the counted input, plus output as before. Any mix of reads and writes is then
within the reservation, which `host.rs` tests exhaustively over the mixes. The
cost is that a caching grant's reservations are a quarter larger, so a budget
that just affords an uncached ask can refuse the same ask cached
(`examples/caching.nrm` tests both sides).

**The scripted tokenizer counts the schema.** `05` convention 4 counted only
the messages, which understated a real ask about thirteen-fold (§6a). An
ask's count now adds the schema at 478 tokens per 521 bytes of JSON, the
first real call's measurement, which `src/defaults.rs` holds as
`SCHEMA_TOKENS` / `SCHEMA_BYTES`. The rate includes the instructions the API
wraps around a schema, which is why it is nearly a token per byte. It rests
on one data point; `the_schema_is_most_of_the_input` in `tests/live_real.rs`
prints the real count beside the estimate for anyone with a key to refine
it. A recursive type has no schema (it can't be asked live), so it adds
nothing.

**The simulated cache** keeps, per model, the prefixes the breakpoints
wrote, each with an expiry on the scripted clock. Its rules are the API's:

- A prompt shorter than the model's minimum (`min_cacheable` in
  `src/defaults.rs`: 1 024 tokens for Claude Sonnet 5, 512 for the newest
  models) isn't cached at all, and costs what it would without `[cache]`.
- Otherwise a request reads the longest live cached prefix and writes
  everything after it.
- An entry lasts its lifetime from the *start* of the request that wrote or
  last read it; a read refreshes it.
- An entry can be read only by a request that starts after the one that wrote
  it, since it exists only once that one's answer begins. So two asks in a
  `par` both write: concurrent requests can't share a cache.

Two simplifications, both of which make scripted costs err high rather than
low: the API's 20-block lookback isn't modelled (a µNorman context is a
handful of messages), and neither is an entry becoming readable partway
through the request that writes it. One assumption is about the API rather
than the simulation: that the schema is cached as part of the prefix, with
the system prompt. `a_cached_grant_reads_its_cache_on_the_second_ask` checks
it live and prints the share of input read from the cache.

**What caching does to the laws (`06`).** With an uncached grant, nothing
changes. With a caching grant, an ask's cost depends on what was asked
before it in the same world, so two programs that make the same asks in a
different order, or concurrently instead of in sequence, can differ in cost.
The laws stated `≡v` (same value) are unaffected; those stated up to
resources hold for caching grants only when both sides make the same asks in
the same order. W1 already relates a workflow to its sequential form only up
to value, and caching is one more reason why: the sequential form can read
what the first ask wrote, and the workflow can't.

## 7. What this doesn't do yet

- **No cache pre-warming or keep-alive.** A long pause between asks lets a
  5-minute entry expire; the fix is the hour's lifetime, or (not yet) a
  `max_tokens: 0` request that refreshes the entry.
- **No streaming.** Requests whose `max_tokens` is large (unbounded `Text`
  answers) should stream to avoid HTTP timeouts. That's part of step 3.
- **No effort control.** A future `[effort low]` grant field would map to
  `output_config.effort`.
- **Prices are not fetched.** They're stated in the grant (§0).
- **A person is consulted only at the terminal.** With `--live`, each
  `human` grant must be bound with `--human NAME=tty` or
  `--human NAME=unattended`, or the run refuses to start
  ([`11`](11-consult.md) §4.1; `08`, deviation 11). There is no web or chat
  host yet.
