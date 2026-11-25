# E02 · Best of three

Define `(short-slogan model product)`.

Ask `model` for a slogan for `product` (a `Text`), for an answer of type
`(Text 200)`, **three times at the same time**. Accept a slogan only if it is
shorter than 30 bytes (`string-length` less than 30). Answer with an
accepted slogan. An attempt that fails with `Invalid` or `ToolError` is
simply not accepted. If no attempt is accepted, fail with
`(fail 'none-accepted)`. Any other failure propagates.

All three attempts run concurrently, so `short-slogan` takes as long as the
slowest attempt, not the sum.

**Sites:** every `ask` uses the site `'slogan`.

**Example:** if the three replies are a 45-byte slogan, `"Brew bold."` and a
60-byte slogan, the answer is `"Brew bold."`.
