# The authoring eval

How well does a model write µNorman, given only the reference and a task?
The design, and what the numbers are for, is
[`../design/14-authoring-eval.md`](../design/14-authoring-eval.md).

**Built so far: Stage 0, the corpus.** The harness that asks a model to
solve these (`run.mjs`, Stage 1) doesn't exist yet.

## Layout

Each directory in [`tasks/`](tasks/) is one task:

| File | Shown to the model? | Contents |
|---|---|---|
| `task.md` | yes | the contract: what to define, how it behaves, the site names to use, one example |
| `interface.nrm` | yes, appended to `task.md` | grants and datatypes the solution uses and must not redefine |
| `tests.nrm` | no | the hidden tests; it `use`s `interface.nrm` and `solution.nrm` itself |
| `reference.nrm` | no | a solution written by a person; passes every hidden test |
| `naive.nrm` | no | a plausible wrong solution; type-checks, fails at least one hidden test |

Tiers: **A** pure code, **B** one `ask`, **C** loops and tools, **D** budgets
and failures, **E** concurrency, **F** `consult`.

## Grading a solution by hand

Put the solution beside the task's interface and tests as `solution.nrm`, and
run the tests:

```
mkdir /tmp/grade && cp eval/tasks/B01-triage/{interface,tests}.nrm /tmp/grade/
cp my-triage.nrm /tmp/grade/solution.nrm
cargo run -- /tmp/grade/tests.nrm
```

The line starting `tests.nrm:` is the score. `cargo test --test eval_corpus`
does this for every task's `reference.nrm` and `naive.nrm`.

## Adding a task

Follow the rules in `14` §1. In short: the contract in `task.md` is
complete, the hidden tests include a failure path, the naive solution makes
exactly the mistake the task is about, and a cost or time bound leaves room
for any reasonable prompt. Then run `cargo test --test eval_corpus`.
