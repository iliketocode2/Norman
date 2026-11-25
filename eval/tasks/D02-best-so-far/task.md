# D02 · Polish until time runs out

Define `(polish model draft limit)`.

Improve `draft` (a `Text`) by asking `model` for a better version, of type
`(Text 1000)`, with a context holding the current draft. The reply becomes the
current draft, and you ask again. Stop when either:

- the model replies with **exactly** the current draft (it has nothing to
  improve): answer with that draft; or
- the time limit `limit` (a `Dur`, for the whole of `polish`) passes: answer
  with the **last complete** draft. An ask cut off by the deadline doesn't
  count, so if the first ask is cut off, the answer is the original `draft`.

Every other failure propagates.

**Sites:** every `ask` uses the site `'polish`.

**Example:** if the model replies `"v1"` and then `"v1"` again, then
`(polish claude "v0" 1min)` is `"v1"`.
