# Steps 3–5: names, contracts, example results

The "function" being designed is the language's evaluator. Steps 1–2 (forms of
data and example inputs) are in `01`–`04`. This document does Steps 3–5. The
example results themselves are real unit tests, in
[`examples/step5-examples.nrm`](../examples/step5-examples.nrm), written before
the evaluator exists. That's what Ramsey does in `triangle.imp`: "A unit test
can appear before the function it tests. This trick can be a great way to plan
a function."

---

## Step 3. Names

Ramsey: "Use a noun, verb, or property." We keep his names wherever the
function plays the same role as in Impcore, so readers of his book can find
their way around.

| Function | Implements | Why this name |
|---|---|---|
| `eval` | `⟨e, Θ, ρ, W⟩ ⇓ ⟨r, W′⟩` | Ramsey's name for the expression judgment |
| `evaldef` | `⟨d, Θ, ρ, W⟩ → ⟨Θ′, ρ′, W′⟩` | Ramsey's name for the definition judgment |
| `wellformed` | the parse-time checks of `04` §3.1 | a property: "is this program well formed?" (compare `check_def_duplicates`) |
| `validate` | `validate_Θ(j, τ)` | a verb: validate a reply against a type |
| `bound` | `bound(τ)` for `max_out` | a noun: the output bound of a type |
| `reach` | ownership: `reach(v)` | a verb, as in the semantics |

---

## Step 4. Contracts

Ramsey: "in a simple, clear sentence, what should the function return, as
determined by its argument(s)?" Each contract below is the sentence first,
then the judgment, then the guarantees that go with it. The guarantees are the
theorems of `04` §8, promised to callers.

**`eval(e, Θ, ρ, W)`** returns the result of evaluating `e`, together with the
world as it is after the evaluation.

- *Judgment:* returns `⟨r, W′⟩` such that `⟨e, Θ, ρ, W⟩ ⇓ ⟨r, W′⟩`.
- *Result:* `r` is a value, or `fail φ` with `φ : Failure`. Stuck states are
  **checked run-time errors**, reported with `runerror`, never silently
  wrong. Examples: an unbound name, an `if` on a non-Boolean, a `case` with no
  matching pattern, or an exhausted script in scripted mode.
- *Guarantees:*
  1. It never spends more than `W.pool` (Theorem 1).
  2. It never accepts a reply that arrives after `W.deadline` (Theorem 3).
  3. It calls only capabilities reachable from `ρ|fv(e)` (Theorem 2).
  4. It asks the oracle at most once per `ASK` it evaluates (D3).
  5. It appends one trace entry per `ask` or `call` it evaluates.

**`evaldef(d, Θ, ρ, W)`** returns the basis and world after evaluating
definition `d`.

- *Judgment:* returns `⟨Θ′, ρ′, W′⟩` such that `⟨d, Θ, ρ, W⟩ → ⟨Θ′, ρ′, W′⟩`.
- If `d` is a `val` or an expression whose evaluation fails, it reports the
  failure and returns `ρ′ = ρ` (no binding). The world keeps what was spent.
- Extended definitions are handled by the read-eval-print loop, as in Ramsey.

**`wellformed(d)`** returns `true` if `d` satisfies the five checks of `04`
§3.1. Otherwise it reports a syntax error with a source location.

**`validate(j, τ, Θ)`** returns the µNorman value that JSON text `j` encodes if
it conforms to askable type `τ`, and is undefined otherwise. The encoding:

| µNorman | JSON |
|---|---|
| `Text`, `(Text n)` | string, at most `n` bytes when bounded |
| `Num` | integer |
| `Bool` | `true` / `false` |
| `Sym` | string |
| `(List τ)`, `(List τ n)` | array (at most `n` elements) |
| record `R` | object with exactly R's fields |
| constructor `K` with fields `f₁…` | `{"tag": "K", "f₁": …}`; a nullary `K` is `{"tag": "K"}` |

**`bound(τ)`** returns an upper bound, in bytes, on the JSON encoding of any
value of askable type `τ`, or `∞`. Bytes bound tokens (`03` Q1).

**`reach(v)`** returns the set of stateful capabilities reachable from `v`
through the free variables of closures (`04` §6.2).

---

## Step 5. Example results

### Conventions for scripted mode

These are fixed here so the tests in `examples/step5-examples.nrm` have exactly
one meaning.

1. **Every test runs in a fresh world** built from its `under` configuration:
   a fresh pool, a deadline of `now + time`, fresh capability states, the
   script rewound to its start, and an empty trace. So tests are independent,
   as Ramsey's are.
2. **Scripts are keyed by site.** `ask` sites are quoted symbols (or source
   locations). `call` sites are `capability/op`, such as `py/exec`. Each
   site's entries are consumed in order.
3. **Concurrent requests to the same site** are served in virtual-time order,
   with ties broken by workflow binding order (`03` Q3).
4. **Input tokens are computed,** by a fixed test tokenizer: ⌈bytes ÷ 4⌉ of the
   serialized context. Scripts give only replies, output tokens and latency.
5. **An exhausted script is a checked run-time error,** tested with
   `check-error`, not a failure. A test that needs more replies than the
   script provides is a bug in the test.
6. **A forked kernel draws from its parent's script key,** and the scripted
   kernel host implements `fork` itself.
7. **`check-within e ([cost c] [time t])`** passes if evaluating `e` spends at
   most `c` and takes at most `t` of virtual time, *whatever its result*. That
   lets it measure failures as well as successes.

### Every rule gets an example

Ramsey's Step 2 says "every form of data needs an example," and Step 9 asks
"do they test every form of input?" For an evaluator, the forms are the
**rules**. This table shows which test group exercises which rule of `04`.

| Rule | Group | What the example shows |
|---|---|---|
| ASKOK | 1, 3, 4, 7, 9 | a typed value comes back, charged within bounds |
| ASKINVALID | 2, 4 | bad model output becomes `fail (Invalid _)`, not a crash |
| ASKUNAFFORDABLE | 3, 5 | **the reservation comes from the type:** unbounded `LooseVerdict` is refused at $0.05, while bounded `Verdict` fits; refusal costs $0 and 0 s |
| ASKLATE | 6 | a 10-minute reply under a 1-minute scope fails at the deadline; a nested 1 s budget does the same |
| ASKPROVIDERERROR | 10 | a 503 becomes `ToolError`, which `retry` handles |
| CALLOK | 8, 9 | kernel execution; `fork` gives a second owner |
| CALLERROR | 10 | a Python `NameError` becomes `ToolError`, then an observation in `observe` |
| FAIL | 9 | `react-fuel` runs out of fuel |
| CATCHOK / CATCHFAIL | 2, 3, 6, 8 | the fallback verdict; `catch … e e` returns the failure *as a value* for `check-expect` |
| BUDGET | 6, 9 | a nested deadline is the smaller one; `cp-agent`'s scope is the smaller of the $0.50/10 min request and the test's 5 min |
| WORKFLOW | 7, 8 | **40 s vs. 60 s:** dataflow finds the critical path, fork-join doesn't; work is identical |
| WORKFLOWSHARED | 8 | two nodes on one kernel are rejected before running, including through a closure (`run-x`) |
| WORKFLOWFAIL | 11 | node `a` fails at 5 s; sibling `b` (30 s) is cancelled, so the workflow takes 5 s |
| CASE | 9 | **the ReAct loop is case analysis on the oracle's answer** (`Exec` / `Done`) |
| `reach` via free variables | 8 | `analyst`'s environment contains `py`, but its body doesn't mention it, so two analyst nodes run concurrently |
| predefined `retry` | 4, 5, 10 | `retry 0` fails; `retry 1` recovers; `retry 5` never retries `OverBudget` and spends $0 |
| predefined `repair` | 4 | the failed reply goes into the context; the second attempt succeeds |
| sugar `par` | 7 | `par` returns a `Pair`; the fork-join version is the 60 s one |

Rules for the inherited forms (LITERAL, VAR, LET, IF, APPLY, CON, RECORD,
FIELD) are exercised throughout. The predefined functions got their own
tests at Step 9, Ramsey's "revisit tests" step, in
[`examples/step9-revisit.nrm`](../examples/step9-revisit.nrm). The inherited
forms are still tested only indirectly, through the programs above.

### What writing the examples changed

Step 5's rationale: "If something is going to be wrong, misunderstood, or
confusing, we want to identify it early—for example, before we start coding
the wrong function." Writing these tests found three problems:

1. **`reach` was wrong** (fixed in `04` §6.2). Following whole closure
   environments would make every top-level function appear to own every
   granted kernel, so *every* workflow would be rejected. It has to follow only
   free variables.
2. **A test can consume a script twice.** Group 7's last test runs both
   versions of the DAG, so each site needs two entries. Hence convention 5:
   exhaustion is a checked error, so the mistake shows up loudly.
3. **Measuring time and money needs no new syntax.** `elapsed` and `spent`
   are six-line µNorman functions built on the `(remaining)` observer. That's
   good evidence the observer was the right primitive.

---

## Next: Step 6, algebraic laws

The laws will be written for the forms the tests just exercised, and each law
will point to the rule(s) of `04` that justify it:

- **Budget:** nesting (`min`), `(budget ∞ e) == e`, and "refusal costs nothing."
- **Workflow:** sequentialization (Theorem 4), work = Σ, span = critical path,
  `par` commutativity, and the `par`/`workflow` translation.
- **Retry and repair:** Peano laws, plus "never retries `OverBudget` or
  `PastDeadline`."
- **Catch:** `(catch (fail φ) x e) == e[x ↦ φ]`, `(catch v x e) == v`, and the
  fallback idiom.
- **Properties** (Lesson 2's non-algorithmic laws) for testing and refactoring,
  for example: *moving an `ask` into its own workflow node never changes the
  value, only the time.*

Then Steps 7–8 (case analysis and code) are the Rust interpreter, written one
`match` arm per rule, following Lesson 5's translation procedure.
