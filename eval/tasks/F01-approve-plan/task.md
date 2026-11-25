# F01 · Run only what was approved

Define `(plan-and-run model reviewer sh goal)`.

1. Ask `model` for a plan to reach `goal` (a `Text`): a list of shell
   commands, of type `(List (Text 80) 5)`.
2. Consult the person `reviewer` about the plan, for an answer of the same
   type, **proposing** the model's plan. The person may approve it, edit it,
   or decline.
3. Run each command of the plan **the person gave back**, in order, with
   `sh`'s `run` operation, one at a time. Answer with the list of their
   outputs, in the same order.

If the person declines, the `Declined` failure propagates and nothing is run.
Every other failure propagates too.

**Sites:** the plan uses the site `'plan`, and the consult uses `'review`.
Commands are run keyed `sh/run`.

**Example:** if the model proposes `["ls","wc -l notes.txt"]`, the person
approves, and the two commands print `"notes.txt"` and `"3 notes.txt"`, the
answer is `(list "notes.txt" "3 notes.txt")`.
