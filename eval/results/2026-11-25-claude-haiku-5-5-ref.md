# Authoring eval: claude-haiku-5-5, ref

## Reading

Every trial passes in the end; 56 of 60 on the first program. The first-round
failures:

- **Redefining the interface** (3): `B01` redefines the grant `claude`,
  `C02` the grant `web`, and `C04` both the record `Quote` and the grant
  `prices`, although the task lists them under "Already defined for you
  (don't redefine these)". The checker reports the grants. It says nothing
  about `Quote`, which is the datatype gap in design/14 §3.
- **The site passed as an argument** (1, `E01`), as in the Sonnet run.
- **A test naming a script that doesn't exist** (1, `C04`): `(under ([script
  none] ...))`. It isn't caught until the test runs ("unknown script none").
  The hidden tests still passed, so it counts in pass@1 but not check@1.

One repair round made a program whose own test expected the wrong order; the
next round fixed the test.

The run directory was `eval/runs/2026-11-25T15-02-54-claude-haiku-5-5-ref/`; the full baseline reading is in
[`README.md`](README.md).

Run 2026-11-25T15-02-54. Commit efebd26 (with uncommitted changes); norman 0.4.0; reference sha256 33481c07dc0e. 3 trials, up to 3 repair rounds, effort medium. Estimated spend $0.06 of a $10 cap.

## Overall and by tier

| | trials | check@1 | pass@1 | pass@R | mean repair rounds to a clean check |
|---|---|---|---|---|---|
| **all** | 60 | 92% | 93% | 100% | 0.10 |
| A | 15 | 100% | 100% | 100% | 0.00 |
| B | 12 | 92% | 92% | 100% | 0.08 |
| C | 12 | 75% | 83% | 100% | 0.33 |
| D | 9 | 100% | 100% | 100% | 0.00 |
| E | 6 | 83% | 83% | 100% | 0.17 |
| F | 6 | 100% | 100% | 100% | 0.00 |

## By task

| task | pass@1 | pass@R | reference lines | model lines (final, median) | final categories |
|---|---|---|---|---|---|
| A01-word-freq | 3/3 | 3/3 | 12 | 19 | pass, pass, pass |
| A02-run-length | 3/3 | 3/3 | 11 | 18 | pass, pass, pass |
| A03-kv-parse | 3/3 | 3/3 | 22 | 39 | pass, pass, pass |
| A04-money-sum | 3/3 | 3/3 | 7 | 17 | pass, pass, pass |
| A05-top-k | 3/3 | 3/3 | 15 | 25 | pass, pass, pass |
| B01-triage | 2/3 | 3/3 | 6 | 16 | pass, pass, pass |
| B02-extract-invoice | 3/3 | 3/3 | 19 | 14 | pass, pass, pass |
| B03-yes-no-gate | 3/3 | 3/3 | 9 | 18 | pass, pass, pass |
| B04-route | 3/3 | 3/3 | 8 | 10 | pass, pass, pass |
| C01-lookup-loop | 3/3 | 3/3 | 20 | 23 | pass, pass, pass |
| C02-flaky-tool | 2/3 | 3/3 | 11 | 21 | pass, pass, pass |
| C03-chunked-summary | 3/3 | 3/3 | 14 | 16 | pass, pass, pass |
| C04-typed-tool | 2/3 | 3/3 | 9 | 13 | pass, pass, pass |
| D01-cheap-then-strong | 3/3 | 3/3 | 9 | 8 | pass, pass, pass |
| D02-best-so-far | 3/3 | 3/3 | 12 | 17 | pass, pass, pass |
| D03-split-budget | 3/3 | 3/3 | 12 | 12 | pass, pass, pass |
| E01-fan-out | 2/3 | 3/3 | 11 | 17 | pass, pass, pass |
| E02-best-of-3 | 3/3 | 3/3 | 7 | 24 | pass, pass, pass |
| F01-approve-plan | 3/3 | 3/3 | 16 | 17 | pass, pass, pass |
| F02-revise-on-decline | 3/3 | 3/3 | 17 | 21 | pass, pass, pass |

## What went wrong

Primary category of every failing round (design/14 §3).

| category | first rounds | all rounds |
|---|---|---|
| redefinition | 3 | 3 |
| own tests fail | 0 | 1 |
| other | 1 | 1 |
| syntax | 1 | 1 |

Unrecognized first errors (category "other"):

- (1) `solution.nrm:17:3: unknown script none`

## Self-tests

36% of programs had tests of their own, and 92% of those passed them.
