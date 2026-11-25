# C03 · Summarize in chunks

Define `(summarize model chunks)`.

`chunks` is a list of `Text`. Keep a running summary, starting from the empty
text `""`. For each chunk, in order, ask `model` for a new summary of type
`(Text 500)`, with a context holding the current summary and the chunk; the
reply becomes the current summary. Answer with the summary after the last
chunk. With no chunks, answer `""` without asking anything.

If an ask fails with `Invalid`, keep the current summary unchanged and go on
to the next chunk. Any other failure propagates.

**Sites:** every `ask` uses the site `'fold`.

**Example:** with two chunks, if the model replies `"Alice left."` and then
`"Alice left; Bob stayed."`, the answer is `"Alice left; Bob stayed."`.
