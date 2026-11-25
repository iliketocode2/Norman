# A03 · Parse key-value pairs

Define `(parse-pairs text)`.

`text` is ASCII text of the form `key=value;key=value;…`. Answer with a list
of `(Pair [fst key] [snd value])`, both `Text`, in the order they appear.

- The empty text has no pairs: the answer is `'()`.
- A pair splits at its **first** `=`, so `"a=b=c"` is the key `"a"` with the
  value `"b=c"`.
- A value may be empty: `"a="` is the key `"a"` with the value `""`.
- Nothing is trimmed: spaces are part of keys and values.
- A piece with no `=` at all is malformed, and the whole call fails with
  `(fail 'malformed)`.

**Sites:** none. This task makes no `ask`, `call` or `consult`.

**Example:**

```
(parse-pairs "host=example.org;port=8080")
;; is
(list (Pair [fst "host"] [snd "example.org"]) (Pair [fst "port"] [snd "8080"]))
```
