# Authoring eval: claude-sonnet-5-5, ref

## Reading

Every trial passes in the end, and 57 of 60 pass on the first program. The
three that don't are all `E01-fan-out`, and all three fail the same way: the
model factors the two summaries into a helper `(summarize model doc site)`
and passes the site in as an argument. µNorman wants the site of an `ask` to
be a quoted symbol written at the `ask`, so the parser rejects it ("the site
of an ask is a quoted symbol"), and the rejected definition then shows up a
second time as "unbound name summarize". With the error in hand, the model
inlines both `ask`s inside a `workflow` and passes on round 2. Nothing in the
reference says a site has to be written literally. This is the only failure
cause in the run.

The run directory was `eval/runs/2026-11-25T15-02-54-claude-sonnet-5-5-ref/`; the full baseline reading is in
[`README.md`](README.md).

Run 2026-11-25T15-02-54. Commit efebd26 (with uncommitted changes); norman 0.4.0; reference sha256 33481c07dc0e. 3 trials, up to 3 repair rounds, effort medium. Estimated spend $0.43 of a $10 cap.

## Overall and by tier

| | trials | check@1 | pass@1 | pass@R | mean repair rounds to a clean check |
|---|---|---|---|---|---|
| **all** | 60 | 95% | 95% | 100% | 0.05 |
| A | 15 | 100% | 100% | 100% | 0.00 |
| B | 12 | 100% | 100% | 100% | 0.00 |
| C | 12 | 100% | 100% | 100% | 0.00 |
| D | 9 | 100% | 100% | 100% | 0.00 |
| E | 6 | 50% | 50% | 100% | 0.50 |
| F | 6 | 100% | 100% | 100% | 0.00 |

## By task

| task | pass@1 | pass@R | reference lines | model lines (final, median) | final categories |
|---|---|---|---|---|---|
| A01-word-freq | 3/3 | 3/3 | 12 | 23 | pass, pass, pass |
| A02-run-length | 3/3 | 3/3 | 11 | 15 | pass, pass, pass |
| A03-kv-parse | 3/3 | 3/3 | 22 | 32 | pass, pass, pass |
| A04-money-sum | 3/3 | 3/3 | 7 | 16 | pass, pass, pass |
| A05-top-k | 3/3 | 3/3 | 15 | 24 | pass, pass, pass |
| B01-triage | 3/3 | 3/3 | 6 | 17 | pass, pass, pass |
| B02-extract-invoice | 3/3 | 3/3 | 19 | 19 | pass, pass, pass |
| B03-yes-no-gate | 3/3 | 3/3 | 9 | 13 | pass, pass, pass |
| B04-route | 3/3 | 3/3 | 8 | 7 | pass, pass, pass |
| C01-lookup-loop | 3/3 | 3/3 | 20 | 23 | pass, pass, pass |
| C02-flaky-tool | 3/3 | 3/3 | 11 | 20 | pass, pass, pass |
| C03-chunked-summary | 3/3 | 3/3 | 14 | 20 | pass, pass, pass |
| C04-typed-tool | 3/3 | 3/3 | 9 | 13 | pass, pass, pass |
| D01-cheap-then-strong | 3/3 | 3/3 | 9 | 12 | pass, pass, pass |
| D02-best-so-far | 3/3 | 3/3 | 12 | 16 | pass, pass, pass |
| D03-split-budget | 3/3 | 3/3 | 12 | 10 | pass, pass, pass |
| E01-fan-out | 0/3 | 3/3 | 11 | 14 | pass, pass, pass |
| E02-best-of-3 | 3/3 | 3/3 | 7 | 25 | pass, pass, pass |
| F01-approve-plan | 3/3 | 3/3 | 16 | 16 | pass, pass, pass |
| F02-revise-on-decline | 3/3 | 3/3 | 17 | 19 | pass, pass, pass |

## What went wrong

Primary category of every failing round (design/14 §3).

| category | first rounds | all rounds |
|---|---|---|
| syntax | 3 | 3 |

## Self-tests

33% of programs had tests of their own, and 100% of those passed them.
