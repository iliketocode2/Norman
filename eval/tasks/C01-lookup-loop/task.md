# C01 · A lookup loop

Define `(investigate model kb question fuel)`.

Answer `question` (a `Text`) by letting `model` look things up in the
knowledge base `kb` until it gives a final answer. The context starts with a
`System` message saying what to do and a `User` message holding the question.
Then, repeatedly, ask `model` for a `Move` (defined for you):

- `(Lookup key)`: call `kb`'s `lookup` operation on `key`. Add two messages to
  the end of the context: an `Assistant` message whose content is the move
  written with `show`, then a `Tool` message whose content is the lookup's
  result. If the lookup fails with `ToolError`, the `Tool` message's content
  is `"error: "` followed by the failure's message, and the loop goes on.
  Then ask again.
- `(Final answer)`: answer with `answer`.

Each ask uses one unit of `fuel`. If the fuel is zero or less when an ask is
due, fail with `(fail 'out-of-fuel)` instead of asking. Every failure other
than a lookup's `ToolError` propagates.

**Sites:** every `ask` uses the site `'move`. Lookups are keyed `kb/lookup`.

**Example:** if the model replies `{"tag":"Lookup","key":"boiling point of water"}`,
the lookup answers `"100 C at sea level"`, and the model then replies
`{"tag":"Final","answer":"100 C"}`, the answer is `"100 C"`.
