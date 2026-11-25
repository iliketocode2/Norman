# C02 · Fetch from a flaky service

Define `(fetch-all web urls)`.

`urls` is a list of `Text`. Fetch each one with `web`'s `get` operation, one
at a time, in order. A fetch that fails with `ToolError` is tried again, up
to **two more times** (three tries in all). Answer with a list of
`(Option Text)`, one per URL in the same order: `(Some body)` for a fetch
that succeeded, or `None` for one that failed all three tries. A URL that
fails doesn't stop the others. Any failure other than `ToolError`
propagates.

**Sites:** fetches are keyed `web/get`.

**Example:** if `"a.example"` answers `"A"` and `"b.example"` fails three
times, then `(fetch-all web (list "a.example" "b.example"))` is
`(list (Some "A") None)`.
