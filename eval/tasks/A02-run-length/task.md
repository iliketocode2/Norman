# A02 · Run-length encoding

Define `(run-length xs)`.

`xs` is a list of `Num`. Answer with a list of `(Pair [fst x] [snd n])`, one
for each **run** of equal adjacent numbers, where `x` is the number and `n`
is the length of the run, in order. Equal numbers that aren't adjacent are
separate runs.

**Sites:** none. This task makes no `ask`, `call` or `consult`.

**Example:**

```
(run-length (list 7 7 2 7))
;; is
(list (Pair [fst 7] [snd 2]) (Pair [fst 2] [snd 1]) (Pair [fst 7] [snd 1]))
```
