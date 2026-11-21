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
cargo run -- --live --python python3 FILE.nrm # a real model and a real (unsandboxed) Python
```

`norman` exits 0 only if every unit test passed **and** nothing else went
wrong: a syntax error, a top-level form that failed or got stuck (such as a
live consult with nobody there), or a file that couldn't be used. Problems in
a `use`d file count against the file that uses it. (Until this was fixed, a
top-level error was printed but the command still exited 0.) The playground
reports the same verdict.

Current results:

| Suite | Result |
|---|---|
| Step 5 example results | 35 of 35 pass |
| Step 6 law instances and non-law counterexamples | 26 of 26 pass |
| Step 9 tests of the initial basis ([`../examples/step9-revisit.nrm`](../examples/step9-revisit.nrm)) | 50 of 50 pass |
| Scripted-oracle tests ([`../examples/oracle-scripted.nrm`](../examples/oracle-scripted.nrm)) | 11 of 11 pass |
| SAGA's bi-level loop, at all three autonomy levels ([`../examples/saga-loop.nrm`](../examples/saga-loop.nrm)) | 15 of 15 pass |
| Strings and `show` ([`../examples/strings.nrm`](../examples/strings.nrm)) | 45 of 45 pass |
| Typed grants ([`../examples/typed-tools.nrm`](../examples/typed-tools.nrm)) | 18 of 18 pass |
| CP-Agent, scripted ([`../examples/cp-agent.nrm`](../examples/cp-agent.nrm)) | 6 of 6 pass |
| `consult`, scripted ([`../examples/consult.nrm`](../examples/consult.nrm)) | 52 of 52 pass |
| Type parameters ([`../examples/type-parameters.nrm`](../examples/type-parameters.nrm)) | 19 of 19 pass |
| The type checker, steps 1–2 of `12` ([`../tests/check.rs`](../tests/check.rs)): the prelude's inferred types, effects, rejections, load order, polymorphism, overloading | 16 of 16 pass; not yet run on programs (`12` §9, step 5) |
| `consult`'s trace record, the terminal host driven by a simulated person, and the start-up check ([`../tests/consult.rs`](../tests/consult.rs)) | 18 of 18 pass |
| Live client against a local stub server ([`../tests/live_stub.rs`](../tests/live_stub.rs)) | 9 of 9 pass |
| Live `call` against real hosts, including Python ([`../tests/live_tools.rs`](../tests/live_tools.rs)) | 12 of 12 pass |
| Random tests of 17 laws ([`../tests/properties.rs`](../tests/properties.rs)) | 150 cases each, all pass |
| The same harness on 3 false laws | finds a counterexample to each, as it must |
| Must-fail suite ([`../tests/must-fail.nrm`](../tests/must-fail.nrm)) | 11 of 11 fail, as they must |
| Real API calls ([`../tests/live_real.rs`](../tests/live_real.rs), [`../tests/live_cp_agent.rs`](../tests/live_cp_agent.rs)) | 6 tests, `#[ignore]`d; they spend money, and `cargo test` skips them |

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
| CONSULT's premises; M-CONSULT-EXPIRED, -REFUSED, -ISSUE | frames `ConsultHuman`, `ConsultPrompt`, `ConsultProposal`, then `issue_consult` (askability and `v ⊨ τ` first, then the deadline, then the price) |
| the human oracle `H ⊢ ⟨η, …⟩ ⇝ …` | `Oracle::issue_consult`; the scripted oracle answers from `answer`, `approve` and `decline` entries |
| an uncharged cut or cancelled consult | `Machine::abandon_charge`, used by `cut` and `cancel` |
| a consult's audit record (`04` §5) | `TraceEntry::consult`, a `ConsultRecord` |
| the live person (`11` §4.1) | `LiveOracle::issue_consult`, `show_next`, `on_line` and `withdraw` in [`src/live.rs`](../src/live.rs); typed lines arrive through `next` like any reply |
| H3's start-up check | `Interp::check_people`, called by `main` before anything runs |
| M-TIME | `advance_time`, then `complete` for each request, in issue order |
| M-WF-SHARED, M-WF-START | `start_workflow` (the ownership check comes first) |
| M-NODE-DONE, M-WF-DONE, M-NODE-FAIL | `finish` |
| cancellation, charged the reservation (07 §5), except for a consult | `cancel` |
| scheduler policy (least path; run until blocked) | `run_main` |
| `reach(v)` via free variables | `Machine::reach`, using each closure's precomputed `fv` |
| `validate_Θ`, `bound`, `askable` | [`src/types.rs`](../src/types.rs); a parameterized type goes through `TypeEnv::instance`, which checks the arity and substitutes the arguments |
| sugar of 04 §7.1 | [`src/parser.rs`](../src/parser.rs): `let*`, `begin`, `par`, `and`, `or`, `(fail 'sym)`, the default `ask` site |
| initial basis of 04 §7.2 | [`src/prelude.nrm`](../src/prelude.nrm), written in µNorman |

---

## Step 8: where the code narrows or departs from the spec

Each item is deliberate and listed here so it isn't mistaken for a bug.

1. **Askability is checked when an `ask` or `consult` is evaluated,** not at
   parse time as `04` §3.1 rule 5 says. A function may `ask` for a datatype that's defined
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
5. **One real host, and no money cost for tools.** `cost_κ(op) = 0` for every
   host: a `call` takes time but never money. Live mode has one real host, a
   Python kernel ([`src/tools.rs`](../src/tools.rs)), enabled with
   `--python`; there's no filesystem or HTTP host. It is **not sandboxed**.
   Scripted kernels implement `fork` natively. A live kernel can't copy its
   state, so `fork` there is a `ToolError` (`Oracle::fork`): forking in the
   machine alone would give a second capability id to the *same* process,
   and the ownership check, which compares ids, would let two workflow nodes
   share it. When a deadline or a cancellation cuts a live call, the host is
   interrupted (the Python process is killed), so later calls on that
   capability are tool errors.
6. **The live oracle has made real calls** (`09` §6a, step 4 ✅). Steps 1–4 of
   [`09`](09-live-oracle.md) are done. The machine talks to an `Oracle` trait
   ([`src/host.rs`](../src/host.rs)), which is implemented by scripts and by
   the live client ([`src/live.rs`](../src/live.rs), run with `--live`). The
   live client is tested against a local stub server, and CP-Agent has run
   end to end against Claude Sonnet 5 and a real Python kernel
   ([`../tests/live_cp_agent.rs`](../tests/live_cp_agent.rs)). Refusals, the
   default model grant (Claude Sonnet 5, set in
   [`src/defaults.rs`](../src/defaults.rs)) and thinking allowances are in
   place and tested in
   [`examples/oracle-scripted.nrm`](../examples/oracle-scripted.nrm). Without
   `--python`, live mode has no tool hosts, and `call` is a checked run-time
   error.
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
11. **The live person is at the terminal, answering one line at a time**
    (`11` §4.1). `--human NAME=tty` shows each consult on stderr and reads
    the answer from stdin: `a` approves the proposal, `d REASON` declines,
    and anything else is the answer, as JSON or, for a `Text` answer, as
    plain text. A reply that doesn't fit the type is asked for again, which
    is form validation, not a second consult. Four rules the design left
    open are fixed here:
    - **One question at a time.** Concurrent consults are shown in issue
      order; each waits for the one before it, and the wait is its latency.
    - **A line typed before a question appeared never answers it.** Each
      line is timestamped when it's read. Without this, a reply meant for
      one question could land on the next, and a pipe that types ahead could
      approve something nobody saw.
    - **A closed terminal is no person at all.** If stdin ends while a
      question is waiting, the run stops with a checked run-time error,
      rather than hanging forever or reporting a decline.
    - **A withdrawn question can't be answered late.** When a deadline or
      fail-fast withdraws a question, the console says so and moves on; a
      line typed afterwards answers the next question or nothing. So the
      design's `discarded` trace entry (`11` §2.3), meant for hosts that
      can't withdraw a question, never arises here.

    The start-up check runs before anything is evaluated: it reads the
    files on the command line, and every file they `use`, for `human`
    grants, and refuses the run (exit 2) if one is unbound or if `--human`
    names a grant that doesn't exist, which is almost always a typo. A
    consult that reaches the oracle with no binding anyway, as through the
    library API, is still a checked run-time error.
12. **A human's price is a money literal,** `[price $2.00]`, not an
    expression. Grants are read before anything is evaluated, as a model
    grant's prices are.

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

**Building `consult` found two more** (`06` §7, #10 and #11), and one of them
was in the tests rather than the code. Every new test passed on the first
run, so the implementation was deliberately broken to see whether they would
notice. Charging cancelled consults was caught at once. Removing the equality
half of the proposal check was not: the tests for checked errors ran under an
empty script, and an exhausted script is itself a checked error (`05`,
convention 5), so they passed whether or not the check existed. Moved under a
script in which the person *would* answer, with a control test beside each,
they catch it, and the case the gap was hiding (a `Sym` proposed as `Text`,
which validates but doesn't read back as itself) is now tested. The other,
Theorem 4's premise about humans (premise (iv) of law W1), was found by reviewing the design before any code
was written.

**Reviewing the type design found a bug in the interpreter.** Comparing
against an unlimited budget, `(> (. (remaining) cost) $0.10)`, was a
run-time error ("can't compare ∞"), so `02`'s own budget-aware idiom crashed
whenever no limit was set, which is the case for every top-level form. ∞ now
compares above every amount of money or time and equal only to itself;
arithmetic on it is still an error. The tests are in
[`step9-revisit.nrm`](../examples/step9-revisit.nrm).

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
- **More hosts.** A filesystem capability; a sandboxed Python kernel (today's
  runs with the user's permissions); and a kernel that can `fork`.
- **A read-eval-print loop,** for interactive use.
- **The type and effect system** (Lesson 5). It makes askability, ownership,
  and law W1's side conditions about `(remaining)` and consulting the same
  human into compile-time checks, and it can warn when a consult's answer is
  never used (`11` §8).
- **Tests of the inherited forms** (LITERAL, VAR, IF, LET, LAMBDA, APPLY,
  CON, RECORD, FIELD, CASE) on their own, which `05` promised for Step 9.
  They're still tested only indirectly.
- **More examples.** P3 (the committee) and P5 (ProofFlow) from `01`/`02` as
  runnable programs.
- **Other hosts for a person,** such as a web form or a chat message, behind
  the same interface as the terminal (`11` §4.1); and a person who can step
  away, which is `11`'s open question Q-G.
