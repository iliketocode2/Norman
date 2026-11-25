# D01 · Cheap model first

Define `(answer fast strong question)`.

Ask the cheap model `fast` to answer `question` (a `Text`), for an answer of
type `(Text 300)`. If that ask fails with `Invalid` or `Refused`, ask the
expensive model `strong` instead, with the same context, and answer with its
reply. If `fast` succeeds, `strong` is never asked.

Every other failure propagates without asking `strong`: in particular
`ToolError` (the provider is down, and the other model is no more likely to
work) and `OverBudget` (there's no money for a dearer model). A failure from
`strong` propagates too.

**Sites:** the ask of `fast` uses the site `'fast`, and the ask of `strong`
uses `'strong`.

**Example:** if `fast` replies `"Paris"`, then
`(answer fast strong "What is the capital of France?")` is `"Paris"`.
