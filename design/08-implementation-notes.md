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
cargo test                                    # Steps 5, 6 and the must-fail suite
```

Current results:

| Suite | Result |
|---|---|
| Step 5 example results | 35 of 35 pass |
| Step 6 law instances | 19 of 19 pass |
| Must-fail suite ([`../tests/must-fail.nrm`](../tests/must-fail.nrm)) | 8 of 8 fail, as they must |

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
   convention `05` fixes for scripted mode; a live model would report real
   counts.
4. **A scripted reply longer than `max_out`** is treated as truncated. It
   fails `Invalid` and is charged at `max_out`, so the charge never exceeds the
   reservation (Theorem 1′).
5. **Hosts are scripted only.** `cost_κ(op) = 0` for every host. There's no
   real Python kernel or filesystem yet. Kernels implement `fork` natively.
6. **There is no live oracle yet.** Models answer only from scripts.
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

This is the arc Ramsey's process promises. Laws and tests written *before* the
code found five design mistakes (`06` §7) and one flawed ownership rule (`05`).
Running them found one more. Once the code was written, it agreed with the
specification on every other test.

---

## What's next

- **A live oracle.** Connect `ask` to a real model API. The provider reports
  token usage, which replaces the test tokenizer; `max_tokens` enforces
  `max_out`. Cancelled calls can then be refunded down to reported usage
  (`07` §5).
- **Real hosts.** A Python kernel and a filesystem capability, with `fork`.
- **A read-eval-print loop,** for interactive use.
- **The type and effect system** (Lesson 5). It makes askability, ownership
  and the `(remaining)` side condition of law W1 into compile-time checks.
- **More examples.** P3 (the committee) and P5 (ProofFlow) from `01`/`02` as
  runnable programs.
