# B04 · Route a question

Define `(route model web calc question)`.

Ask `model` once which way to handle `question` (a `Text`), for an answer of
type `Route` (defined for you). Then:

- `(Search query)`: answer with the result of `web`'s `search` operation on
  `query`.
- `(Calculate expression)`: answer with the result of `calc`'s `eval`
  operation on `expression`, written as text with `show`.
- `(Answer text)`: answer with `text`.

The answer is always `Text`. Every failure propagates.

**Sites:** the `ask` uses the site `'route`. The tool calls are keyed by
their grant and operation, `web/search` and `calc/eval`.

**Example:** if the model replies
`{"tag":"Calculate","expression":"6*7"}` and `calc` answers `42`, then the
answer is `"42"`.
