# C04 · The cheapest quote

Define `(cheapest prices sku)`.

Call `prices`'s `quotes` operation on `sku` (a `Text`). Its result is a list
of `Quote` records (defined for you); the grant declares the type, so the
call answers with the list itself. Answer with `(Some q)`, where `q` is the
quote with the lowest `cents`, or `None` if there are no quotes. If two quotes
tie for the lowest price, answer with the one that comes **first**.

If the call fails, with `Invalid` (a malformed result) or `ToolError`, call
it **once more**. If that fails too, the failure propagates.

**Sites:** calls are keyed `prices/quotes`.

**Example:** if the result is
`[{"vendor":"A","cents":500},{"vendor":"B","cents":300}]`, the answer is
`(Some (Quote [vendor "B"] [cents 300]))`.
