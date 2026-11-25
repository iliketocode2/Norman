# D03 · Split a budget between planning and doing

Define `(plan-then-do model task total)`.

Do `task` (a `Text`) in two asks of `model`, spending at most `total` (a
`Money`) on the two together:

1. **Plan:** ask for a plan of type `(List (Text 200) 10)`. Planning may spend
   at most **a third of `total`**, so that it can never leave too little for
   the work.
2. **Do:** ask for the result, of type `(Text 200)`, with a context holding
   the task and the plan (written with `show`). This step may use whatever
   is left of `total`.

Answer with the result. Every failure propagates, including `OverBudget` when
an ask doesn't fit its share.

**Sites:** the plan uses the site `'plan`, and the result uses `'do`.

**Example:** if the plan is `["list the files","count them"]` and the result
is `"12 files"`, then `(plan-then-do claude "Count the files." $1.00)` is
`"12 files"`.
