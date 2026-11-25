# A01 · Word frequencies

Define `(word-freq words)`.

`words` is a list of `Text`. Answer with a list holding one
`(Pair [fst w] [snd n])` for each distinct word `w`, where `n` is the number
of times `w` occurs in `words`. The pairs are in the order in which each word
**first** appears. Words are compared exactly, so `"A"` and `"a"` are
different words.

**Sites:** none. This task makes no `ask`, `call` or `consult`.

**Example:**

```
(word-freq (list "to" "be" "or" "not" "to" "be"))
;; is
(list (Pair [fst "to"] [snd 2]) (Pair [fst "be"] [snd 2])
      (Pair [fst "or"] [snd 1]) (Pair [fst "not"] [snd 1]))
```
