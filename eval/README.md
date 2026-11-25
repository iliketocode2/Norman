# The authoring eval

How well does a model write µNorman, given only the reference and a task?
The design, and what the numbers are for, is
[`../design/14-authoring-eval.md`](../design/14-authoring-eval.md).

**Built:** the corpus (Stage 0) and the harness (Stage 1). **The baseline**
(2026-11-25) is in [`results/`](results/README.md). Both Sonnet 5.5 and
Haiku 5.5 pass 93–100% of tasks on the first program and all of them after
repair, so the next work is harder tasks (design/14 §6, Stage 1b).

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

| File | What it is |
|---|---|
| [`run.mjs`](run.mjs) | the harness: asks a model, checks and grades each program, repairs, writes `summary.md` |
| [`test.mjs`](test.mjs) | the harness's own tests, with a mock model (run in CI) |
| `runs/` | one directory per run (gitignored): `trials.jsonl`, `meta.json`, `summary.md`, and every program in `work/` |
| `results/` | the summaries worth keeping, with a hand-written reading of each (design/14 §4) |

## Running the baseline

You need Node 22 or later, a built `norman`, and an API key.

```
cargo build --release
npm ci --prefix eval
export ANTHROPIC_API_KEY=...          # or: ant auth login

for model in claude-sonnet-5-5 claude-haiku-5-5; do
  for condition in ref ref+examples; do
    node eval/run.mjs --model $model --condition $condition --norman target/release/norman
  done
done
```

On Windows with `norman` built in WSL, pass the command that runs it, for
example `--norman "wsl -e /home/me/.cache/norman-target/release/norman"`.

Each run prints one line per trial and ends with the path of its
`summary.md`. To rebuild a summary from a finished run:
`node eval/run.mjs --summarize eval/runs/<dir>`.

**Cost.** Each run is capped by `--max-spend` (default $10): before every
call, the harness reserves the call's worst case and makes no call that
doesn't fit. The baseline cost $0.43 (Sonnet, `ref`), $1.04 (Sonnet,
`ref+examples`), $0.06 and $0.10 (Haiku). The reference is cached, so most input is read from the cache.
The estimate printed is computed from reported usage and the prices in
`run.mjs`; the bill itself is in the Console.

**Checking the harness for free:** `--replay reference` and `--replay naive`
answer every task with its own reference or naive solution instead of
asking a model. `node eval/test.mjs --norman CMD` runs the harness's tests.

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
