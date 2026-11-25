# Authoring eval: claude-haiku-5-5, ref+examples

## Reading

Every trial passes in the end; 57 of 60 on the first program. Six first rounds
failed, three of them only on the model's own tests (so check@1 is lower than
pass@1):

- `A03`: an unbalanced parenthesis in a 45-line program, and in another
  trial a `catch` whose handler returns a symbol where the body returns a list.
- `C02`: a test whose script runs out of results for the tool call
  `small/get` (a run-time "script exhausted").
- `D02`: script replies written as bare text, `(reply "v1")`, so the
  program's `(Text 1000)` ask fails with `Invalid "v1"`. A reply is read as
  JSON, so it has to be `(reply "\"v1\"")`. The reference's script table
  says "as JSON" for `answer` but not for `reply`.
- `E01`: once the site rule, once redefining the grant `claude`.

The examples cut the interface redefinitions from 3 to 1, but the site
rule still caught one trial, as it did without them.

The run directory was `eval/runs/2026-11-25T15-02-55-claude-haiku-5-5-ref+examples/`; the full baseline reading is in
[`README.md`](README.md).

Run 2026-11-25T15-02-55. Commit efebd26 (with uncommitted changes); norman 0.4.0; reference sha256 33481c07dc0e. 3 trials, up to 3 repair rounds, effort medium. Estimated spend $0.10 of a $10 cap.

## Overall and by tier

| | trials | check@1 | pass@1 | pass@R | mean repair rounds to a clean check |
|---|---|---|---|---|---|
| **all** | 60 | 90% | 95% | 100% | 0.10 |
| A | 15 | 87% | 93% | 100% | 0.13 |
| B | 12 | 100% | 100% | 100% | 0.00 |
| C | 12 | 92% | 100% | 100% | 0.08 |
| D | 9 | 89% | 100% | 100% | 0.11 |
| E | 6 | 67% | 67% | 100% | 0.33 |
| F | 6 | 100% | 100% | 100% | 0.00 |

## By task

| task | pass@1 | pass@R | reference lines | model lines (final, median) | final categories |
|---|---|---|---|---|---|
| A01-word-freq | 3/3 | 3/3 | 12 | 21 | pass, pass, pass |
| A02-run-length | 3/3 | 3/3 | 11 | 21 | pass, pass, pass |
| A03-kv-parse | 2/3 | 3/3 | 22 | 45 | pass, pass, pass |
| A04-money-sum | 3/3 | 3/3 | 7 | 24 | pass, pass, pass |
| A05-top-k | 3/3 | 3/3 | 15 | 28 | pass, pass, pass |
| B01-triage | 3/3 | 3/3 | 6 | 16 | pass, pass, pass |
| B02-extract-invoice | 3/3 | 3/3 | 19 | 35 | pass, pass, pass |
| B03-yes-no-gate | 3/3 | 3/3 | 9 | 48 | pass, pass, pass |
| B04-route | 3/3 | 3/3 | 8 | 11 | pass, pass, pass |
| C01-lookup-loop | 3/3 | 3/3 | 20 | 21 | pass, pass, pass |
| C02-flaky-tool | 3/3 | 3/3 | 11 | 31 | pass, pass, pass |
| C03-chunked-summary | 3/3 | 3/3 | 14 | 23 | pass, pass, pass |
| C04-typed-tool | 3/3 | 3/3 | 9 | 12 | pass, pass, pass |
| D01-cheap-then-strong | 3/3 | 3/3 | 9 | 18 | pass, pass, pass |
| D02-best-so-far | 3/3 | 3/3 | 12 | 27 | pass, pass, pass |
| D03-split-budget | 3/3 | 3/3 | 12 | 10 | pass, pass, pass |
| E01-fan-out | 1/3 | 3/3 | 11 | 19 | pass, pass, pass |
| E02-best-of-3 | 3/3 | 3/3 | 7 | 8 | pass, pass, pass |
| F01-approve-plan | 3/3 | 3/3 | 16 | 18 | pass, pass, pass |
| F02-revise-on-decline | 3/3 | 3/3 | 17 | 26 | pass, pass, pass |

## What went wrong

Primary category of every failing round (design/14 §3).

| category | first rounds | all rounds |
|---|---|---|
| syntax | 2 | 2 |
| type mismatch | 1 | 1 |
| run-time error | 1 | 1 |
| own tests fail | 1 | 1 |
| redefinition | 1 | 1 |

## Self-tests

61% of programs had tests of their own, and 93% of those passed them.
