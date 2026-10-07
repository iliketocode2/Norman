# Steps 7–9: the interpreter, and what running it taught us

The Rust interpreter in [`../src/`](../src/) is a **definitional interpreter**
in Ramsey's sense: "a language implementation that is intended to implement the
language's theory directly." This document maps the theory to the code, lists
every place the code narrows or departs from `04`/`07`, and records what
running the tests (Step 9) found.

## Running it

```
cargo build
cargo run -- examples/step5-examples.nrm      # or: target/debug/norman FILE.nrm …
cargo test                                    # Steps 5, 6, 9, the laws, and the must-fail suite
```

Current results:

| Suite | Result |
|---|---|
| Step 5 example results | 35 of 35 pass |
| Step 6 law instances and non-law counterexamples | 26 of 26 pass |
| Step 9 tests of the initial basis ([`../examples/step9-revisit.nrm`](../examples/step9-revisit.nrm)) | 39 of 39 pass |
| Scripted-oracle tests ([`../examples/oracle-scripted.nrm`](../examples/oracle-scripted.nrm)) | 11 of 11 pass |
| SAGA's bi-level loop ([`../examples/saga-loop.nrm`](../examples/saga-loop.nrm)) | 7 of 7 pass |
| Random tests of 13 laws ([`../tests/properties.rs`](../tests/properties.rs)) | 150 cases each, all pass |
| The same harness on 3 false laws | finds a counterexample to each, as it must |
| Must-fail suite ([`../tests/must-fail.nrm`](../tests/must-fail.nrm)) | 8 of 8 fail, as they must |
| One real API call ([`../tests/live_real.rs`](../tests/live_real.rs)) | 3 tests, `#[ignore]`d; they spend money, and `cargo test` skips them |

Every scripted run also checks Theorem 1′ (the budget invariant) in every
state it reaches. See "The budget invariant is checked" below.

The must-fail suite holds deliberately wrong versions of Step 5 tests: 39 s
instead of 40 s, a success instead of `OverBudget`, and so on. It checks that
the suite can tell right from wrong, which a suite that passes proves nothing
about by itself.

---

## Step 7: the case analysis is the rule table

The machine ([`src/machine.rs`](../src/machine.rs)) is a CEK-style
implementation of the `07` machine. It has a **control** (evaluate an
expression, return a value, or raise a failure), an **environment**, and a
**stack of continuation frames**. The frames are exactly `07`'s evaluation
contexts: one frame per hole in `E`.

Continuations are *data*, so a thread can stop in the middle of an expression
when it issues an `ask` and resume when the reply arrives. That's what makes
the event-by-event semantics implementable exactly.

| Semantics | Code |
|---|---|
| judgment `⟨e, Θ, ρ, W⟩ ⇓ ⟨r, W′⟩` | `machine::run`, returning `Outcome { result, spent, elapsed, trace }` |
| LITERAL, VAR, CON, RECORD, FIELD, IF, LET, LAMBDA, APPLY, CASE | one arm each in `Machine::eval`, and one frame each in `Machine::ret` |
| PROPAGATE, CATCHOK, CATCHFAIL | `Machine::raise` unwinds to the nearest `Catch` frame |
| M-BUDGET, SCOPE(σ, e) | `enter_scope` pushes `Frame::Scope`; `current_scope` finds the innermost one |
| `available(σ)`, `deadline*(σ)` | `Machine::available`, `Machine::deadline_star` over `Machine::chain` |
| M-ASK-EXPIRED, M-ASK-REFUSED, M-ASK-ISSUE | `issue_ask`, in that order |
| M-CALL-… | `issue_call` (`fork` on a kernel is native) |
| M-TIME | `advance_time`, then `complete` for each request, in issue order |
| M-WF-SHARED, M-WF-START | `start_workflow` (the ownership check comes first) |
| M-NODE-DONE, M-WF-DONE, M-NODE-FAIL | `finish` |
| cancellation, charged the reservation (07 §5) | `cancel` |
| scheduler policy (least path; run until blocked) | `run_main` |
| `reach(v)` via free variables | `Machine::reach`, using each closure's precomputed `fv` |
| `validate_Θ`, `bound`, `askable` | [`src/types.rs`](../src/types.rs) |
| sugar of 04 §7.1 | [`src/parser.rs`](../src/parser.rs): `let*`, `begin`, `par`, `and`, `or`, `(fail 'sym)`, the default `ask` site |
| initial basis of 04 §7.2 | [`src/prelude.nrm`](../src/prelude.nrm), written in µNorman |

---

## Step 8: where the code narrows or departs from the spec

Each item is deliberate and listed here so it isn't mistaken for a bug.

1. **Askability is checked when an `ask` is evaluated,** not at parse time as
   `04` §3.1 rule 5 says. A function may `ask` for a datatype that's defined
   later in the file, so the check needs the Θ in force at run time. Failing
   it is a checked run-time error. Once the type checker exists, it moves to
   compile time.
2. **Top-level names are late-bound,** as in µScheme's global store, rather
   than through `04`'s fixed-point environment. The two differ only if a
   function is **redefined**. In that case, earlier functions see the new
   definition. Ramsey's own interpreters behave the same way.
3. **The test tokenizer.** A context's size is the sum, over its messages, of
   role name + content + 4 bytes. Tokens are ⌈bytes ÷ 4⌉. That's the
   convention `05` fixes for scripted mode; a live model reports real counts.
   **It understates a live ask by about an order of magnitude**, because it
   counts only the messages, and the JSON schema is sent as input too: for the
   analyst ask, 39 predicted against 538 billed (`09` §6a). This is a
   *fidelity* gap, not a soundness one — Theorem 1 holds in live mode because
   the reservation comes from `count_tokens` on the real request, not from this
   tokenizer — but every cost in the scripted examples is far below what the
   same program would really pay.
4. **A scripted reply longer than `max_out`** is treated as truncated. It
   fails `Invalid` and is charged at `max_out`, so the charge never exceeds the
   reservation (Theorem 1′).
5. **Hosts are scripted only.** `cost_κ(op) = 0` for every host. There's no
   real Python kernel or filesystem yet. Kernels implement `fork` natively.
6. **The live oracle has made real calls** (`09` §6a, step 4 ✅). Steps 1–4 of
   [`09`](09-live-oracle.md) are done. The machine talks to an `Oracle` trait
   ([`src/host.rs`](../src/host.rs)), which is implemented by scripts and by
   the live client ([`src/live.rs`](../src/live.rs), run with `--live`). The
   live client is tested against a local stub server. Step 4, one real call,
   is next. Refusals, the default model grant (Claude Sonnet 5, set in
   [`src/defaults.rs`](../src/defaults.rs)) and thinking allowances are in
   place and tested in
   [`examples/oracle-scripted.nrm`](../examples/oracle-scripted.nrm). Live mode
   still has no tool hosts, so `call` is a checked run-time error there.
7. **`check-equiv 'exact`** compares results, money, time, and the trace by
   (site, kind, cost, start, end, outcome). It ignores workflow paths, because
   the same computation can legitimately run at a different path (for
   example, `best-of 1` vs. `attempt` in `06`). It doesn't compare host state.
8. **`check-expect` runs each side in its own fresh world,** like
   `check-equiv`, so evaluating the expected value can't consume the tested
   expression's script.
9. **`use` runs the used file's tests,** following Ramsey's convention. So
   loading `step6-laws.nrm` also reports `step5-examples.nrm`.
10. **Pure computation is bounded by a step limit** (20 million steps), which
    is a checked run-time error (`07` §7).

---

## Step 9: what running the tests found

**One real defect in the specification, on the first run.** 32 of 35 Step 5
tests passed. The other three failed because `04` listed `cost` and `time` as
reserved words, while the predefined `Resources` record uses them as field
names. So `(. (remaining) cost)` was a syntax error in the language's own
basis. They're no longer reserved (`04` §2 now explains why). After that, all
tests passed.

**The numbers match the hand calculations** in `05`/`06`:

| Test | Measured |
|---|---|
| analyst ask | $0.000375 = 25 input tokens × 3 µ$ + 20 output tokens × 15 µ$ |
| dataflow `n-graph` vs. fork-join `n-graph-par` | 40 s vs. 1 min, at identical cost |
| unbounded `LooseVerdict` under $0.05 | refused before the oracle, at $0 and 0 s |
| `retry 5` with 1 µ$ | `OverBudget` immediately, $0 spent |
| fail-fast | stops at 5 s, not 30 s |
| nested retries | `(retry 1 (λ () (retry 1 f)))` is exactly `(retry 3 f)` |

**Revisiting the tests found three more problems in the laws** (`06` §7,
#6–#8). Step 9 asks, "Do they test every form of input?" They didn't:
`window`, `drop`, `filter` and `last-n` had no tests, and `best-of` was
tested only at widths 0 and 1, which skips the concurrent case it exists for.
[`step9-revisit.nrm`](../examples/step9-revisit.nrm) adds them, including a
true and a false result for every predicate.

- Running one instance of the `window` property showed it was false.
- Random testing ([`tests/properties.rs`](../tests/properties.rs), Lesson 2's
  "random, automated, property-based testing") showed that Theorem 4 and law
  W1 fail when a limit binds. The law had been tested on one hand-written
  script with generous limits. Each case is generated from a fixed seed and
  printed as a complete `.nrm` file when it fails, so any failure can be
  reproduced exactly.
- The nested-retry law had lost its side condition when fix #1 made `retry`
  accept negative counts.

**The first real call found a tenth problem** (`06` §7, #9), and it is the one
no amount of scripted testing could have found. The ask billed 538 input tokens
for a 175-character context, because **the schema is sent with every ask** and
accounted for 478 of them. Law A1 had priced an ask as `price_m(|ctx|,
max_out(m, τ))`, leaving the type out of the input entirely, which made law A3
false. The scripted tokenizer had hidden this by counting only messages. See
`09` §6a.

**The budget invariant is checked** (Lesson 6: representation invariants
"can be coded, typechecked, and tested"). Only `reserve` and `settle` move
money, so after each one the machine checks `spent + reserved ≤ limit` on the
scopes it touched (`budget_invariant` in
[`src/machine.rs`](../src/machine.rs), with unit tests). A violation is a
run-time error, so every scripted test checks Theorem 1′ in every state it
reaches. On a live oracle, the check stops after an `over-reservation`,
because the theorem's one assumption has failed (`07` §6).

This is the arc Ramsey's process promises. Laws and tests written *before* the
code found five design mistakes (`06` §7) and one flawed ownership rule (`05`).
Running them found one more. Once the code was written, it agreed with the
specification on every other test.

---

## What's next

- **Prompt caching** (`09` §7). Newly urgent: the schema is 89% of an ask's
  input and is re-sent every iteration of a loop (`09` §6a).
- **A scripted tokenizer that includes the schema,** so scripted costs predict
  live ones. Today they are about an order of magnitude low (deviation 3).
- **Refunding cancelled calls** down to reported usage (`07` §5).
- **Real hosts.** A Python kernel and a filesystem capability, with `fork`.
- **A read-eval-print loop,** for interactive use.
- **The type and effect system** (Lesson 5). It makes askability, ownership
  and the `(remaining)` side condition of law W1 into compile-time checks.
- **Tests of the inherited forms** (LITERAL, VAR, IF, LET, LAMBDA, APPLY,
  CON, RECORD, FIELD, CASE) on their own, which `05` promised for Step 9.
  They're still tested only indirectly.
- **More examples.** P3 (the committee) and P5 (ProofFlow) from `01`/`02` as
  runnable programs.
- **The three changes SAGA argues for** ([`10`](10-saga.md)): a result type on
  `call`, declared at the grant; a `consult` effect for the decisions a human
  should approve; and `show`, so values can be rendered back into a prompt.
