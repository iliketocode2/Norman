# Authoring eval: claude-sonnet-5-5, ref+examples

## Reading

60 of 60 pass on the first program. With the examples in the prompt, the
model passes the E01 sites literally (as `examples/` does) and never meets
the site rule. The examples triple the input, so the run costs about 2.4 times
as much as `ref` for 5 more points of pass@1.

The run directory was `eval/runs/2026-11-25T15-02-54-claude-sonnet-5-5-ref+examples/`; the full baseline reading is in
[`README.md`](README.md).

Run 2026-11-25T15-02-54. Commit efebd26 (with uncommitted changes); norman 0.4.0; reference sha256 33481c07dc0e. 3 trials, up to 3 repair rounds, effort medium. Estimated spend $1.04 of a $10 cap.

## Overall and by tier

| | trials | check@1 | pass@1 | pass@R | mean repair rounds to a clean check |
|---|---|---|---|---|---|
| **all** | 60 | 100% | 100% | 100% | 0.00 |
| A | 15 | 100% | 100% | 100% | 0.00 |
| B | 12 | 100% | 100% | 100% | 0.00 |
| C | 12 | 100% | 100% | 100% | 0.00 |
| D | 9 | 100% | 100% | 100% | 0.00 |
| E | 6 | 100% | 100% | 100% | 0.00 |
| F | 6 | 100% | 100% | 100% | 0.00 |

## By task

| task | pass@1 | pass@R | reference lines | model lines (final, median) | final categories |
|---|---|---|---|---|---|
| A01-word-freq | 3/3 | 3/3 | 12 | 23 | pass, pass, pass |
| A02-run-length | 3/3 | 3/3 | 11 | 16 | pass, pass, pass |
| A03-kv-parse | 3/3 | 3/3 | 22 | 32 | pass, pass, pass |
| A04-money-sum | 3/3 | 3/3 | 7 | 17 | pass, pass, pass |
| A05-top-k | 3/3 | 3/3 | 15 | 24 | pass, pass, pass |
| B01-triage | 3/3 | 3/3 | 6 | 17 | pass, pass, pass |
| B02-extract-invoice | 3/3 | 3/3 | 19 | 20 | pass, pass, pass |
| B03-yes-no-gate | 3/3 | 3/3 | 9 | 29 | pass, pass, pass |
| B04-route | 3/3 | 3/3 | 8 | 12 | pass, pass, pass |
| C01-lookup-loop | 3/3 | 3/3 | 20 | 21 | pass, pass, pass |
| C02-flaky-tool | 3/3 | 3/3 | 11 | 20 | pass, pass, pass |
| C03-chunked-summary | 3/3 | 3/3 | 14 | 18 | pass, pass, pass |
| C04-typed-tool | 3/3 | 3/3 | 9 | 30 | pass, pass, pass |
| D01-cheap-then-strong | 3/3 | 3/3 | 9 | 8 | pass, pass, pass |
| D02-best-so-far | 3/3 | 3/3 | 12 | 17 | pass, pass, pass |
| D03-split-budget | 3/3 | 3/3 | 12 | 15 | pass, pass, pass |
| E01-fan-out | 3/3 | 3/3 | 11 | 11 | pass, pass, pass |
| E02-best-of-3 | 3/3 | 3/3 | 7 | 8 | pass, pass, pass |
| F01-approve-plan | 3/3 | 3/3 | 16 | 17 | pass, pass, pass |
| F02-revise-on-decline | 3/3 | 3/3 | 17 | 19 | pass, pass, pass |

## What went wrong

Primary category of every failing round (design/14 §3).

| category | first rounds | all rounds |
|---|---|---|

## Self-tests

45% of programs had tests of their own, and 100% of those passed them.
