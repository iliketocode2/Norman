<!-- Generated from docs/docs.html by scripts/docs-assets.mjs. Do not edit by hand. -->

# µNorman language reference

> This is the reference for µNorman, a small programming language for AI agents, from <https://norman-docs.vercel.app/docs>, converted to Markdown so it can be given to an agent as context. Examples are µNorman source (`.nrm` files). In the grammar blocks, lower-case words such as `exp`, `name` and `ty` are placeholders (the site shows them in italics); everything else is literal.

Everything in the language, in the order you would meet it. This page follows the [formal definition](https://github.com/iliketocode2/Norman/blob/main/design/04-formal-definition.md), which is the authority where the two disagree.

> **How to read the grammars**
>
> `{ x }` is zero or more, `[ x ]` is optional, and `|` separates alternatives. Words in **this colour** are literal; *this one* is a placeholder. Round and square brackets are interchangeable as long as they match — square brackets are used by convention where a form takes a list of clauses.

## Lexical structure

A **semicolon** begins a comment that runs to the end of the line. Each bracket is a token on its own. A **string** is `"…"` with the escapes `\"`, `\\` and `\n`. Every other character joins a token that extends as far as it can, stopping only at a bracket, a semicolon, whitespace or a quote.

| Token | Form | Examples | Meaning |
| --- | --- | --- | --- |
| numeral | digits, optional sign | `42` `-7` | a 64-bit integer |
| money | `$` digits, optionally `.` and up to six more | `$0.50` `$20` | integer micro-dollars; `$0.50` is 500 000 µ$ |
| duration | digits then `ms`, `s`, `min` or `h` | `250ms` `30s` `10min` | integer milliseconds |
| Boolean | `#t` or `#f` |  |  |
| symbol | `'` then a name | `'declined` | a symbol value |
| empty list | `'()` |  |  |
| name | starts lowercase or with a symbol character | `react` `best-of` `+` | a variable or function |
| Name | starts uppercase | `Step` `Exec` `Text` | a constructor, record or type |

> **Money and time are not numbers**
>
> They are distinct kinds of value. You may add money to money, and a duration to a duration, but never money to seconds. That rules out an entire class of budget bug at the level of values rather than by convention.

## Program structure

A file is a sequence of **definitions** and **extended definitions**. Definitions are the language proper. Extended definitions arrange the world around it: loading another file, receiving authority from the host, and testing. As in Ramsey's interpreters, a file's unit tests run after the whole file has loaded, so a test may mention anything the file defines, in any order.

```
def  ::= (val name exp)
     | exp
     | (define name (formals) exp)
     | (datatype Type {[Con {(field ty)}]})
     | (record Record {(field ty)})
     | xdef

xdef ::= (use file)
     | (grant name host-spec)
     | (script Name {site-script})
     | (under ({config}) {unit-test})
     | unit-test
```

## Definitions

- `(define name (formals) body)`

  A function. The formals must be distinct. Top-level names are mutually recursive.

- `(val name exp)`

  Evaluate once and bind the result. If it fails, nothing is bound and the failure is reported.

- `(datatype Type [Con (field ty)…]…)`

  A sum type: a value is *one of* its constructors. Constructor names must be distinct, and they share one namespace across the program.

- `(record Record (field ty)…)`

  A product type: a value has *all* of its fields.

The difference matters when you write code over them. A datatype is consumed with `case`, one branch per constructor. A record is consumed with `(. r field)`. It also shows up in how you build them:

```
(Sell "Margins are compressing.")             ; constructor: positional
(Message [role User] [content "FY2025 10-K"]) ; record: named, in any order
```

## Expressions

### Core forms

- `(if test then else)`

  The test must evaluate to a Boolean; anything else is a checked run-time error.

- `(let* ([x e]…) body)`

  Sequential binding: each right-hand side sees the bindings before it. There is no simultaneous `let`.

- `(lambda (formals) body)`

  An anonymous function, closing over its environment.

- `(f arg…)`

  Application. Arguments are evaluated left to right.

- `(begin e…)`

  Evaluate in order, answering with the last. Empty `begin` is `#f`. Sugar for nested `let*`.

- `(and e₁ e₂)` · `(or e₁ e₂)`

  Short-circuiting, and sugar for `if`.

- `(. e field)`

  Select a field from a record.

### Case analysis

`case` is how a program consumes a datatype, a list, or a model's answer. If no branch matches, that is a checked run-time error, not a failure.

```
pattern ::= Con                        ; a constructor with no fields
          | (Con {name | _})          ; its fields, bound or ignored
          | '()                        ; the empty list
          | (cons name name)           ; head and tail
          | _                          ; anything
```

**Patterns are flat.** A constructor applies to variables or `_`, never to another pattern, and no variable may repeat within one pattern. This keeps matching a single step, which keeps every proof about `case` to a single step too. Nested patterns could be added later as sugar.

### Building values

```
(Con {exp})                 ; constructor application
Con                         ; a nullary constructor is just its name
(Record {[field exp]})     ; every field, named
(list {exp})                ; a list
(cons exp exp)              ; prepend
```

## The six agentic forms

These are what the language adds. Everything a program can do to the outside world happens through `ask` and `call`; there is no other door, which is why a budget, a deadline, the trace and the ownership rules apply to *everything*.

### ask

```
(ask model ty context ['site])
```

Consult a model **once**, for an answer of type *ty*. The context is an ordinary list of `Message`s that you build and pass: nothing accumulates behind your back. The site is a name for this place in the program; scripts and traces are keyed by it, and it defaults to the source location.

The answer is validated against *ty* before it is bound. If it doesn't fit, the ask produces `Invalid` rather than a value. Asking again is a separate, visible decision — see [`retry`](https://norman-docs.vercel.app/docs#basis-functions).

Before the call, `ask` **reserves** the worst case: everything it is about to send, plus the largest answer *ty* allows. If that doesn't fit the budget, the model is never called and nothing is charged.

### call

```
(call capability op {exp})
```

Use a tool once. The capability comes from [`grant`](https://norman-docs.vercel.app/docs#hosts) and can only come from there. What the operation answers with is declared by the grant, not written at the call site — see [Hosts and capabilities](https://norman-docs.vercel.app/docs#hosts).

### fail and catch

```
(fail exp)
(catch exp name handler)
```

A failure is a *value*, not a crash. `catch` binds it to *name* and runs the handler. `(fail 'sym)` is sugar for `(fail (Raised 'sym))`. A failure propagates left to right: everything before it still ran and still spent.

### budget

```
(budget ([cost exp] [time exp]) body)
```

Everything inside gets at most that much money and time. Either limit may be omitted, which means unlimited. Nesting takes the smaller of the two, and whatever the inside spends is charged to the outside. A budget scope has one pool and **one absolute deadline** shared by every branch running inside it.

### workflow and par

```
(workflow ([name exp]…) body)
(par e₁ e₂)
```

A set of named steps. **Dependencies come from the names each step mentions**: a step whose inputs are ready starts immediately, alongside the others. The graph must be acyclic, and the names must be distinct — both are checked before the program runs.

Steps may not share a stateful tool, because the outcome would depend on timing. Such a workflow is a type error, so the program never starts. To work in parallel on a kernel, fork it first, before the workflow. (A scripted kernel forks freely; a live Python kernel can’t copy its state, so there `fork` fails with `ToolError`.) `par` is sugar for a two-step workflow and answers with a `Pair`.

## Types

```
ty ::= Text | (Text numeral)        ; optionally bounded, in bytes
     | Num | Bool | Sym | Money | Dur
     | (List ty) | (List ty numeral)
     | Type | Record              ; anything you defined
     | (Type ty …)                   ; one with type parameters, applied
```

A datatype or record can take **type parameters**, and is then given that many arguments wherever it’s used:

```
(datatype (Outcome e a) [Got (value a)] [Unsure (why e)])
(ask m (Outcome (Text 40) Num) ctx)    ; a (Got 7), or an (Unsure "…")
```

A type variable may appear only inside the definition that declares it, and inside its own definition a type is applied to exactly its own parameters: `(Tree a)`, never `(Tree (List a))`.

### Which types can be asked for

A type is **askable** when it is built only from `Text`, `Num`, `Bool`, `Sym`, `List`, and datatypes or records whose fields are all askable, at the arguments given: `(Option Num)` is askable and `(Option Dur)` isn’t. `Money` and `Dur` are not askable; JSON has no amounts of money or time.

That one rule is what keeps authority unforgeable: a capability is not an askable type, so no model answer can ever contain a tool. Authority comes only from the host.

### Bounds, and why a type costs money twice

A bound caps the answer's size, which caps the reservation. A `Verdict` with 400-byte reasons reserves far less than the same type with unbounded text, so it fits budgets the unbounded one is refused by.

> **Measured, not assumed**
>
> The type is also *sent* with every ask, as a JSON schema, and billed as input. The project's first real API call spent 538 input tokens on a 175-character prompt, 478 of them on the schema. So bounding an *answer* shrinks the reservation, while adding a constructor or a field makes every ask dearer on input for as long as the program runs. No bound on the text inside those fields helps, because what is sent is the schema, not the answer.

### How a type maps to JSON

| µNorman | JSON |
| --- | --- |
| `Text`, `(Text n)` | a string, at most *n* bytes when bounded |
| `Num` | an integer |
| `Bool` | `true` / `false` |
| `Sym` | a string |
| `(List τ)`, `(List τ n)` | an array, at most *n* elements |
| a record `R` | an object with exactly R's fields |
| constructor `K` with fields | `{"tag": "K", "f₁": …}`; a nullary `K` is `{"tag": "K"}` |

## Failures

Every failure is a value of the predefined `Failure` datatype, so you can `case` on it like anything else.

| Failure | Means |
| --- | --- |
| `(Invalid raw)` | the reply did not fit the type; `raw` is what came back |
| `(ToolError message)` | a tool or the provider failed — a 503, a Python `NameError` |
| `(Refused category)` | the model declined |
| `(Declined reason)` | a person consulted with `consult` said no |
| `OverBudget` | the reservation did not fit the remaining money |
| `PastDeadline` | the scope's deadline passed |
| `(Raised reason)` | the program's own `(fail 'reason)` |

A **checked run-time error** is a different thing and cannot be caught: an unbound name, an `if` on a non-Boolean, a `case` with no matching branch, an exhausted script. Those mean the program is wrong, not that the world misbehaved.

## The initial basis

### Predefined types

```
(datatype Role    [System] [User] [Assistant] [Tool])
(record   Message (role Role) (content Text))
(datatype Failure [Invalid (raw Text)] [ToolError (message Text)]
                  [Refused (category Sym)] [Declined (reason Text)]
                  [OverBudget] [PastDeadline] [Raised (reason Sym)])
(datatype (Option a) [None] [Some (value a)])
(record   (Pair a b) (fst a) (snd b))
(record   Resources (cost Money) (time Dur))
```

### Primitives

| Primitive | Notes |
| --- | --- |
| `+ - * /` | on `Num`, and on money or durations of the same kind. Scaling takes a `Num`. |
| `= < >` | comparison; `<` and `>` also order strings |
| `cons` · `list` | `list` is variadic |
| `string-append` · `string-length` | length is in bytes |
| `show` | write a value as text that reads back — see [below](https://norman-docs.vercel.app/docs#show) |
| `substring` · `string-truncate` · `string-contains?` | byte offsets, as everything else is. `substring` is a checked error out of range or mid-character; `string-truncate` is the safe cut. |
| `println` | print a value |
| `(remaining)` | a `Resources` record for the current scope: `(. (remaining) cost)` and `… time` |

#### show

`(show v)` writes a value as text that **reads back**. For every askable type it produces exactly the JSON that `ask` validates, which is what lets you put a value into a prompt and read the answer back out:

```
validate (show v) τ  =  v          for askable τ and v : τ
```

So `(show (Sell "Margins fell."))` is `{"reason":"Margins fell.","tag":"Sell"}`. Keys come out sorted, which makes `show` canonical: equal values always produce identical text.

Three things sit outside that law, each on purpose:

- **Money, durations and ∞ are not askable**, and JSON has none of them. They are written in µNorman's own literal syntax — `$0.50`, `40s`, `∞` — bare at the top level so a prompt reads naturally, and as strings when nested.
- **Capabilities, models and functions cannot be shown at all.** That is a checked run-time error: authority must never reach a prompt, and a function has no written form.
- **The key `tag` is reserved.** A constructor with a field of that name would overwrite its own discriminator, so `show` refuses rather than quietly producing JSON that means something else.

### Predefined functions

These are written in µNorman itself, and each is defined by algebraic laws.

- `(not b)` · `(<= x y)` · `(>= x y)`

  The obvious things.

- `(append xs ys)` · `(length xs)` · `(filter p xs)` · `(drop k xs)`

  Lists. `drop` treats a count of zero or less as zero.

- `(retry n f)`

  Call the thunk up to *n*+1 times, stopping at the first success. Retries `Invalid` and `ToolError` only — **never** `OverBudget` or `PastDeadline`, since asking again cannot help and would spend what little is left.

- `(repair n f ctx)`

  Like `retry`, but the bad reply and a correction are appended to the context. Costs more per attempt, because the context grows — a trade-off explicit context makes visible.

- `(best-of k f check)`

  *k* attempts, concurrently, keeping one that *check* accepts. Built from a recursive `par`, so the cost adds up and the time is the longest attempt.

- `(window n ctx)`

  A context policy: every `System` message, then the last *n* others. Note that it moves system messages to the front.

## Hosts and capabilities

```
(grant name (model [id "…"] [in p] [out p] [cache 5min|1h] [cache-read p] [ceiling n] [think n]))
(grant name (kernel {[op ty]}))        ; stateful
(grant name (filesystem {[op ty]}))    ; stateful
(grant name (http {[op ty]}))          ; stateless
```

### What a tool answers with

A grant declares each operation's result type, so a tool's answer is typed the same way a model's is. The declaration lives at the grant rather than the call site because the grant is already the host–program contract, and because a capability should carry its own interface.

```
(grant py (kernel [exec (Text 4000)] [count Num] [solve Outcome]))
(call py count "wc -l")        ; a Num, not the text "7"
```

1. **A grant with no declarations is open:** any operation, result `Text`.
2. **A grant with declarations is closed:** only those operations, plus `fork` on a kernel. Calling another is a checked run-time error — the program is wrong, not the world.
3. **A declared `Text` takes the result verbatim,** and only its bound is checked.
4. **Any other type reads the result as JSON** and validates it, exactly as a model's reply is validated. Failure is `Invalid`, so `retry` handles a malformed tool result for free.

> **Why 3 and 4 differ**
>
> An `ask` that wants `Text` receives a JSON string; a `call` that wants `Text` receives raw bytes. The channels really are different — a model emits JSON because a schema told it to, while a tool emits bytes, and the grant is what says what those bytes mean.

Every model field is optional and falls back to a default. Prices are dollars per million tokens, with up to three decimals: `[in 3]` is $3 per million input tokens, `[in 0.1]` is ten cents. **Prices are stated here, not looked up** — the API does not report them, so a grant is where the program says what it believes a token costs. If they are wrong, the budget arithmetic is wrong.

**`[cache 5min]` or `[cache 1h]` turns on prompt caching.** An answer type's schema is sent as input with every ask and is usually most of it, so a loop that asks again and again pays for it again and again. With caching, a repeated prefix is read back at `[cache-read]`, a tenth of `[in]` unless the grant says otherwise. Writing the cache costs a quarter more than plain input (twice as much for the hour), so a one-off ask costs more with caching, not less, and a caching grant reserves input at the write price. Use it where asks repeat: an agent loop, `retry`, `repair`. Scripted runs simulate the cache, so a test's costs are what a live run would pay.

## Scripts and tests

Tests run against **scripts**: canned replies keyed by site, with a simulated clock. Every test runs in a fresh world — a fresh pool, a fresh deadline, the script rewound, an empty trace — so tests are independent and exactly repeatable.

```
(script Name
  [site (reply "…") (out n) (latency d)
        (reply "…") (out n) (latency d)]   ; consumed in order
  [cap/op (result "…") (latency d)])          ; a tool, keyed capability/operation
```

Faults are injected the same way, which is what makes failure paths testable at all:

|  |  |
| --- | --- |
| `(reply "…")` | what the model says; pair with `(out n)` for output tokens |
| `(refusal "category")` | the model declines |
| `(provider-error "503 …")` | the provider fails |
| `(result "…")` | a tool succeeds |
| `(error "…")` | a tool fails |
| `(latency 2s)` | how long it takes on the simulated clock |

### Test forms

```
(under ([script Name] [cost $1.00] [time 1min]) {unit-test})
```

- `(check-expect e expected)`

  Both sides run in their own fresh world, so evaluating the expected value cannot consume the script. Two equal failures agree too, so `(check-expect (ask-v) (fail (Refused 'cyber)))` says exactly how it fails.

- `(check-assert e)`

  The expression is true.

- `(check-fail e pattern)`

  It fails, with a failure matching the pattern: `(check-fail (ask-v) (Invalid _))`.

- `(check-error e)`

  It is a checked run-time error — the program is wrong, not the world.

- `(check-type-error e)`

  It is ill-typed, so the checker rejects it before it runs. Every other test fails if its expression is ill-typed.

- `(check-within e ([cost c] [time t]))`

  It spends at most *c* and takes at most *t*, *whatever its result*. That lets you measure failures as well as successes.

- `(check-equiv e₁ e₂ 'grade)`

  Two expressions agree, at `'exact` (result and the whole world, trace included), `'resource` (result, spend, time, host state), or `'value`.

A complete, runnable example of all of it together:

Example, `reference.nrm`:

```nrm
(grant claude (model [in 3] [out 15] [ceiling 8000]))

(datatype Verdict
  [Buy  (reason (Text 400))]
  [Sell (reason (Text 400))])

(define ask-v ()
  (ask claude Verdict (list (Message [role User] [content "FY2025 10-K"])) 'analyst))

(script flaky
  [analyst (provider-error "503 Service Unavailable") (latency 1s)
           (reply "{\"tag\":\"Sell\",\"reason\":\"Margins fell.\"}") (out 12) (latency 2s)])

(under ([script flaky] [cost $1.00] [time 1min])
  ;; A provider outage is a ToolError, which retry handles.
  (check-fail   (ask-v) (ToolError _))
  (check-expect (retry 1 ask-v) (Sell "Margins fell."))
  (check-within (retry 1 ask-v) ([cost $0.01] [time 3s]))
  ;; Failures are values: a test can name the exact one it expects.
  (check-expect (ask-v) (fail (ToolError "503 Service Unavailable"))))
```

## Syntactic sugar

Sugar is defined by translation, so it inherits every theorem already proved about the core forms, with no new cases to check.

```
(let* () e)                 =  e
(let* ([x e₁] rest…) e)   =  LETX(x, e₁, (let* (rest…) e))
(begin)                      =  #f
(begin e₁ … eₙ)             =  (let* ([_₁ e₁] …) eₙ)
(par e₁ e₂)                  =  (workflow ([a e₁] [b e₂]) (Pair [fst a] [snd b]))
(and e₁ e₂)                  =  (if e₁ e₂ #f)
(or e₁ e₂)                   =  (if e₁ #t e₂)
(fail 'sym)                  =  (fail (Raised 'sym))
(ask m τ c)                  =  (ask m τ c 'file:line:col)
(budget ([cost e]) body)    =  BUDGET(e, ∞, body)
```

## What is checked before a program runs

Each of these is decidable from the syntax alone, so the parser rejects it outright.

1. The formals of a `lambda` or `define` are distinct; so are a datatype's constructors and a record's fields.
2. A `workflow`'s binding names are distinct.
3. A `workflow`'s dependency graph is **acyclic**.
4. Patterns are flat, and no variable repeats within one pattern.
5. Forms are applied with the right number of parts.

Askability is checked when the `ask` runs rather than at parse time, because a function may ask for a datatype defined later in the file. Everything else about types is a run-time check too. Moving those to compile time is what the planned type and effect system is for — see [Status](https://norman-docs.vercel.app/#status).

## Reserved words

`val` `define` `datatype` `record` `use` `grant` `script` `under` `check-expect` `check-assert` `check-error` `check-fail` `check-within` `check-equiv` `check-type-error` `if` `let*` `lambda` `case` `ask` `call` `consult` `fail` `catch` `budget` `workflow` `begin` `par` `and` `or` `_`

`cost` and `time` are **not** reserved. They appear only in fixed bracket positions, and they are the field names of `Resources`. An earlier draft did reserve them, which made `(. (remaining) cost)` a syntax error in the language's own basis. Running the tests caught it.

## Going deeper

- [The formal definition](https://github.com/iliketocode2/Norman/blob/main/design/04-formal-definition.md) — abstract syntax, big-step semantics, every rule, and the theorems.
- [The machine](https://github.com/iliketocode2/Norman/blob/main/design/07-small-step-semantics.md) — the event-by-event semantics behind concurrency and shared budgets.
- [The laws](https://github.com/iliketocode2/Norman/blob/main/design/06-algebraic-laws.md) — which refactors are safe, which are not, and the nine problems writing them found.
