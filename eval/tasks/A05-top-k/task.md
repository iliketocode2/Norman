# A05 · The k largest

Define `(top-k k xs)`.

`xs` is a list of `Num`. Answer with the `k` largest numbers in `xs`, largest
first. Equal numbers are kept: `(top-k 2 (list 5 5 1))` is `(list 5 5)`. If
`xs` has fewer than `k` numbers, answer with all of them, largest first. If
`k` is zero or negative, answer `'()`.

**Sites:** none. This task makes no `ask`, `call` or `consult`.

**Example:**

```
(top-k 3 (list 4 9 1 7 3))
;; is
(list 9 7 4)
```
