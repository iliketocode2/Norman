# E01 · Compare two documents

Define `(compare model left right)`.

`left` and `right` are documents (`Text`). Ask `model` to summarize each one,
for an answer of type `(Text 300)`. The two summaries don't depend on each
other, so ask for them **at the same time**: the whole of `compare` should
take only as long as the slower summary, plus the final ask. Then ask
`model` which document is better, for an answer of type `Pick` (defined for
you), with a context holding both summaries. Answer with the `Pick`.

Every failure propagates.

**Sites:** the summary of `left` uses the site `'left`, the summary of
`right` uses `'right`, and the final ask uses `'judge`.

**Example:** if the summaries take 3s and 1s and the final ask takes 2s,
`compare` takes 5s, not 6s.
