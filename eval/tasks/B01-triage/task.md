# B01 · Triage a support ticket

Define `(triage model ticket)`.

Ask `model` once to classify the support ticket `ticket` (a `Text`) as a
`Triage` (defined for you), and answer with the classification. Write the
context yourself: a `System` message saying what to do, and a `User` message
holding the ticket.

If the reply doesn't fit the type (`Invalid`), ask **exactly once more**,
with the bad reply and a correction added to the context. If that reply
doesn't fit either, the `Invalid` failure propagates. Any other failure
propagates at once.

**Sites:** every `ask` in this task uses the site `'triage`.

**Example:** if the model replies `{"tag":"Bug","summary":"Login crashes."}`,
then `(triage claude "The app crashes when I log in.")` is
`(Bug "Login crashes.")`.
