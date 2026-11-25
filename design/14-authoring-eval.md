# Measuring how well a model writes µNorman

> **Status: design, not built.** This is step 2 of the order in
> [`13`](13-agents-limiting-themselves.md) §"Order", and every later step
> uses it: each one is judged by what it does to the numbers here. Decisions
> marked **(open)** in §0 need review before Stage 1 starts.

[`13`](13-agents-limiting-themselves.md) rests on one bet: **the author of a
µNorman program is a model**, handed the reference and a task. Nothing in the
repository measures whether that works. Every test today is a program a
person wrote.

A few minutes of trying the obvious first moves suggests it doesn't yet work
well:

- `map` and `car` are unbound. They're the first names a model reaches for.
- A nested pattern, `[(Some (Some x)) x]`, is a syntax error. It's reported
  as `type error: syntax error: …`.
- `(+ x "a")` inside `f` is reported at `1:1`, the start of the definition,
  not at the `+`. The call site then reports a second error caused by the
  first.
- An `ask` without the site name the test script expects fails as `script
  exhausted: no reply left for ask site 'solution.nrm:1:31'`. The message
  doesn't say that the script has entries, just under other names.

Each of these is a guess about what matters. This document replaces guesses
with a number: **give a model the reference and a task, and count how often
it writes a program that checks and passes hidden tests, and how many rounds
of error messages it needs to get there.**

---

## 0. Decisions

| # | Question | Decision |
|---|---|---|
| E1 | What is measured? | **Authoring, not agent behaviour.** The model writes a program. The interpreter grades it in scripted mode, so grading is free, exact and repeatable. Only the model calls that write the programs cost money. |
| E2 | How is a program graded? | **Hidden tests in µNorman,** run against the model's program by the same `norman` binary an agent would use. Each task has a reference solution written by a person, and CI checks that it passes, so the corpus can't silently rot. |
| E3 | What does the model get to see? | **The reference, the task, and an interface file.** The interface defines the grants and datatypes the program may use. It stands in for the operator, who will supply grants under `13` Phase 2. The hidden tests are never shown. |
| E4 | Is there a repair loop? | **Yes, up to 3 rounds.** After each round the harness runs the model's program and its own tests, with the interface but without the hidden tests, and sends the output back verbatim. That is the loop a coding agent runs. The hidden tests are run after every round and recorded, but never fed back. |
| E5 | Where does the harness live? | **A Node script, `eval/run.mjs`,** that drives the `norman` binary as a subprocess. It tests the interface agents will actually use, keeps model-API client code out of the interpreter crate, and follows `docs/verify.mjs`, which CI already runs with Node 22. |
| E6 | Which models? | **(open)** Proposal: the current Sonnet-class model as the main subject, and the current Haiku-class model to see whether a cheap model can write the language too. Model ids are flags, never constants in the harness. |
| E7 | What's committed? | The tasks, their tests and reference solutions, the harness, and **a summary table per run** in `eval/results/`. Raw transcripts go in `eval/runs/`, which is gitignored. |
| E8 | How is spending capped? | `--max-spend` (default $10). It counts the usage the API reports and stops the run, cleanly, before the next call would cross the cap. |

---

## 1. A task

Each task is a directory, `eval/tasks/<tier><nn>-<name>/`:

| File | Shown to the model? | Contents |
|---|---|---|
| `task.md` | yes | the goal, the function to define and its contract, required site names, one example |
| `interface.nrm` | yes, verbatim in `task.md` | grants and datatypes the program uses and **must not redefine** |
| `tests.nrm` | no | the hidden tests: scripts and `check-*` forms |
| `reference.nrm` | no | a solution written by a person |

The harness writes the model's program to `solution.nrm` beside them, and
two driver files:

```
;; check.nrm: what the model's repair loop sees
(use interface.nrm)
(use solution.nrm)

;; grade.nrm: what decides the score
(use interface.nrm)
(use solution.nrm)
(use tests.nrm)
```

This relies on two properties of `use` (`08`, deviation 9): definitions join
one namespace, so `solution.nrm` can refer to the interface's grants, and a
used file's tests run. Both were checked against 0.4.0 while writing this
document.

### Example: `B01-triage`

`task.md` (abridged):

> Define `(triage model ticket)`. It asks `model` once, at site `'triage`,
> to classify the support ticket as a `Triage`. If the reply doesn't fit the
> type, ask exactly once more, with the bad reply and a correction added to
> the context. Any other failure propagates.
>
> Already defined for you (don't redefine these):
>
> ```
> (grant claude (model))
> (datatype Triage
>   [Bug     (summary (Text 200))]
>   [Feature (summary (Text 200))]
>   [Spam])
> ```

`reference.nrm`:

```
(define triage-ctx (ticket)
  (list (Message [role System]
                 [content "Classify the support ticket as a Bug, a Feature request or Spam, with a one-sentence summary."])
        (Message [role User] [content ticket])))

(define triage (model ticket)
  (repair 1 (lambda (ctx) (ask model Triage ctx 'triage)) (triage-ctx ticket)))
```

`tests.nrm`:

```
(script bug      [triage (reply "{\"tag\":\"Bug\",\"summary\":\"Login crashes.\"}") (out 12) (latency 1s)])
(script spam     [triage (reply "{\"tag\":\"Spam\"}") (out 4) (latency 1s)])
(script fixable  [triage (reply "Bug: login crashes") (out 8) (latency 1s)
                         (reply "{\"tag\":\"Bug\",\"summary\":\"Login crashes.\"}") (out 12) (latency 1s)])
(script hopeless [triage (reply "Bug") (out 2) (latency 1s)
                         (reply "Bug") (out 2) (latency 1s)])

(under ([script bug] [cost $1.00] [time 1min])
  (check-expect (triage claude "The app crashes when I log in.") (Bug "Login crashes.")))
(under ([script spam] [cost $1.00] [time 1min])
  (check-expect (triage claude "Buy cheap watches!") Spam))
(under ([script fixable] [cost $1.00] [time 1min])
  ;; One bad reply is repaired, and only once: two asks, two seconds.
  (check-expect (triage claude "The app crashes when I log in.") (Bug "Login crashes."))
  (check-within (triage claude "The app crashes when I log in.") ([time 2s])))
(under ([script hopeless] [cost $1.00] [time 1min])
  (check-fail (triage claude "The app crashes when I log in.") (Invalid _)))
```

The reference passes all five tests. A one-line solution that asks without
a site name or a repair fails all five, with the `script exhausted` message
quoted above.

### Rules for writing a task

1. **The contract is in `task.md`, completely.** If a hidden test checks it,
   the task states it. The tests check behaviour the task promises, not
   style.
2. **Site names are part of the contract** until stable default sites exist
   (`13` §1.3). A missing or wrong site is scored as its own category (§3),
   because it measures exactly the cost that §1.3 removes.
3. **No task copies an example from the reference or `examples/`.** The model
   has the reference in context, so a copied task measures recall.
4. **Hidden tests include at least one failure path:** an `Invalid` reply, a
   `ToolError`, a budget that runs out, or a person who declines. Agents are
   mostly about what happens when things go wrong.
5. **The reference solution is the shortest one a person would write today.**
   Its length is recorded. A task the reference finds painful is data too:
   it shows what the prelude is missing (`13` Phase 3).

---

## 2. The corpus

Twenty tasks in six tiers, ordered by how much of the language they need.
Names are provisional; the features each tier exercises are the point.

| Tier | Tasks | What it exercises |
|---|---|---|
| **A**: pure | `A01-word-freq`, `A02-run-length`, `A03-kv-parse`, `A04-money-sum`, `A05-dedupe` | syntax, recursion over lists, strings, `Money` arithmetic, the prelude's gaps. No effects. |
| **B**: one ask | `B01-triage`, `B02-extract-invoice`, `B03-yes-no-gate`, `B04-route` | askable types, bounds, records vs. datatypes, `repair`/`retry`, `Refused` |
| **C**: loops and tools | `C01-react-calc`, `C02-flaky-tool`, `C03-chunked-summary`, `C04-typed-tool` | `call`, a fuel-bounded loop, `ToolError` as an observation, tool result types |
| **D**: budgets and failures | `D01-cheap-then-strong`, `D02-best-so-far`, `D03-spend-cap` | `budget`, `catch` on `OverBudget` and `PastDeadline`, nested limits |
| **E**: concurrency | `E01-fan-out`, `E02-best-of-3` | `par`, `workflow`, the ownership rule |
| **F**: people | `F01-approve-plan`, `F02-revise-on-decline` | `consult`, `[propose …]`, `Declined` |

Twenty is enough to see which tiers fail and why, and small enough that a
person can read every failure of a baseline run. The corpus grows as later
steps add features: `13` Phase 1 adds tasks that define `main`, and Phase 3
adds tasks that use `par-map` and `json-parse`.

---

## 3. A run

```
node eval/run.mjs --model <id> [--trials 3] [--rounds 3] \
                  [--condition ref|ref+examples] [--tasks 'B*'] \
                  [--norman target/release/norman] [--max-spend 10]
```

For each task and each trial:

1. **Generate.** System prompt: a short preamble and the whole reference
   (`docs/munorman-reference.md`, about 7k tokens), marked for prompt
   caching, since it's the same for every call. Under `ref+examples` it's
   followed by every file in `examples/`. User turn: `task.md`. The model
   replies with one fenced code block. A reply with no code block counts as
   a failed round, not as a crash of the harness.
2. **Check.** Run `norman check.nrm` in a fresh temporary directory. Record
   the exit code and output.
3. **Grade.** Run `norman grade.nrm`. The hidden tests pass when the line for
   `tests.nrm` in the output reports every test passed and nothing else went
   wrong. The model's own tests are scored separately (§4), so a wrong test
   the model wrote doesn't hide a correct program, or the other way round.
4. **Repair.** If step 2 wasn't clean and rounds remain, send its output back
   verbatim as the next user turn, and go to 1 with the conversation so far.
   A clean step 2 ends the trial: an agent whose program checks and whose own
   tests pass stops there.

Every round's program, outputs and API usage go to
`eval/runs/<timestamp>/<task>/<trial>.jsonl`. Each run records the model id,
the condition, the git commit, a hash of the `norman` binary and a hash of
the reference, so two runs can be compared knowing exactly what differed.
(`norman` has no `--version` yet; Stage 1 adds one, and records it too.)

### Classifying what went wrong

Each failing round gets one primary category, from the first error in the
output:

| Category | Recognized by (0.4.0's messages) |
|---|---|
| no code | the reply had no code block |
| syntax | `syntax error`, `unclosed` |
| unbound name | `unbound name X` (and X is recorded) |
| redefinition | `is already defined`, `can't be redefined` |
| type mismatch | `expected … but found …` |
| unaskable / ownership / other check | any other checker error |
| wrong site | `no reply left for ask site '…'` naming a site the task didn't specify |
| run-time error | `run-time error` otherwise |
| hidden tests fail | the program checks and runs, but `tests.nrm` reports failures |

**Matching on message text is brittle on purpose.** It's acceptable for one
baseline, and it's the argument for JSON diagnostics (`13` §1.2). When those
land, the harness reads `--format json` instead, and since raw outputs are
kept, old runs can be reclassified.

---

## 4. What is reported

Per model and condition, overall and per tier, with all trials pooled:

| Metric | Meaning |
|---|---|
| **check@1** | share of first programs that pass the checker |
| **pass@1** | share of first programs that pass the hidden tests |
| **pass@R** | share of final programs (after up to R repair rounds) that pass the hidden tests |
| rounds | mean rounds to a clean check, among trials that got there |
| errors | count by category (§3), and the 10 most common unbound names |
| self-tests | share of programs with at least one test form, and the share of those whose tests pass |
| ref length | the reference solution's length beside the model's, per task |
| cost | API spend for the run, from reported usage |

`eval/results/<date>-<model>-<condition>.md` holds these tables and a short
paragraph saying what changed since the previous run. It's written by hand,
because the reading of a run is the point.

---

## 5. How the numbers steer `13`

The baseline is taken before any other step in `13`'s order lands. After
that, **every step that changes the language, the prelude, the messages or
the reference re-runs the eval**, and its pull request quotes the before and
after numbers. A step that doesn't move the numbers it was meant to move gets
looked at again before more is built on it.

The baseline decides some of the order directly. The thresholds are a
starting point, to be revised once a baseline exists:

| If the baseline shows… | then… |
|---|---|
| unbound names are over a quarter of first-round errors | the prelude (step 4) moves ahead of `run` (step 5) |
| wrong-site failures appear in more than 1 trial in 10 | stable site names (`13` §1.3) move into step 3 |
| syntax and nested-pattern failures are common | nested patterns (`13` Phase 6) move up to step 4 |
| pass@R on tiers A and B is below 60% for the Sonnet-class model | language and message work (steps 3 and 4, Phase 6) comes before Phase 2. Nobody adopts a policy runtime for a language their agent can't write. |
| `ref+examples` beats `ref` by more than 15 points | the reference needs more examples (`13` Phase 0.3) before it's called complete |
| Haiku-class pass@R is close to Sonnet-class | the reference is doing its job; prefer cheap models in examples and defaults |

---

## 6. Stages

**Stage 0: the corpus, with no model involved.**
Write the twenty tasks. Add `tests/eval_corpus.rs`, which for every task
(a) grades `reference.nrm` and requires a pass, and (b) grades an empty
solution and requires every hidden test to fail, so no test passes
vacuously. Add `eval/**` to the CI `paths` filter. This runs in `cargo test`,
costs nothing, and is worth having even if Stage 1 never runs: twenty more
programs exercising the language.

**Stage 1: the harness and a baseline.**
Write `eval/run.mjs`. Run it on both models (E6), under both conditions,
with 3 trials and 3 rounds. Commit the summaries. Read every failure of the
Sonnet-class `ref` run by hand and write up the five most common causes. That
write-up, not the numbers alone, is the input to `13` steps 3 and 4.

**Stage 2: use it.** Re-run after each step, as §5 says.

**Stage 3 (open): a comparison.** The harder question from the review: is a
*language* what makes the limits work, or would a runtime that takes a JSON
task spec give most of the value? A comparison condition would give the
model the same tasks in Python, against a small shim library (`ask`,
`budget`, `call`, scripted replies) with equivalent hidden tests. It would
compare pass rates and, more importantly, **how often each kind of program
breaks its own limits**: a µNorman program can't, and a Python one can, but
whether it does in practice is what makes the guarantee worth a language.
This is real work, about as much as Stage 0 again. Decide after the
baseline, when it's clear whether µNorman's pass rates are even in range.

---

## 7. What this doesn't measure

- **Live behaviour.** Every grade is scripted. Whether a program that passes
  makes good use of a real model is a different evaluation.
- **Whether budgets are well chosen.** The tasks give budgets or don't need
  them. Choosing a budget is `13` Phase 5's problem, and its "done when"
  (an agent picks a budget that runs `cp-agent` on the first try) becomes a
  task here once `estimate` exists.
- **Adversarial authors.** The model is trying to succeed. Whether a program
  can escape its limits is the theorems' job, and Phase 2's tests.

Once `13` Phase 2 gives µNorman a `command` host, the harness itself is a
good program to rewrite in µNorman: a loop of asks and calls under a hard
budget, with the operator's policy saying which binary it may run.
