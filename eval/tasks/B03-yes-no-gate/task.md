# B03 · A yes-or-no gate

Define `(safe-to-run? model command)`.

Ask `model` whether the shell command `command` (a `Text`) is safe to run,
for an answer of type `Bool`, and answer with it.

- If the ask fails with `Invalid` or `ToolError`, ask **once more**.
- If, after that, the failure is still `Invalid`, answer `#f`.
- If the model refuses (`Refused`), answer `#f`.
- Any other failure, including a second `ToolError`, propagates.

**Sites:** every `ask` in this task uses the site `'gate`.

**Example:** if the model replies `true`, then
`(safe-to-run? claude "ls -la")` is `#t`.
