# Steps 1–2: forms of data and example inputs

Working name for the core language: **µNorman** (a placeholder; naming is Step 3).

## What function are we designing?

Following Lesson 5, a language is a set of judgment forms, and each judgment
form is implemented by one function. The first function is the **evaluator**.
Its input is a *program*, so its forms of data are the language's **abstract
syntax**. The second function, the type/effect checker, comes later and
consumes the same syntax.

Steps 1 and 2 therefore ask:

1. What forms can an agent program take? (abstract syntax, values, world)
2. What's an example of each form?

Design rule, taken from Lesson 2 and the CP-Agent ablation: **a construct is
a core form only if it can't be derived by an algebraic law from other forms.**
Anything derivable goes in the library. Library code can be swapped and
ablated; core forms can't.

---

## Step 1a. Expressions

An expression `e` is one of the following.

### Inherited from µScheme / ML (no new ideas)

| Form | Concrete syntax | Parts |
|---|---|---|
| `LITERAL v` | `42`, `"hi"`, `#t`, `'sym` | a value |
| `VAR x` | `task` | a name |
| `IF (e1, e2, e3)` | `(if e1 e2 e3)` | three expressions |
| `LET ([(x, e)…], e)` | `(let* ([x e] …) e)` | bindings, body |
| `LAMBDA (xs, e)` | `(lambda (x …) e)` | formals, body |
| `APPLY (e, es)` | `(e e …)` | function, arguments |
| `RECORD (R, [(f, e)…])` | `(R [f e] …)` | record type, fields |
| `FIELD (e, f)` | `(. e f)` | record, field name |
| `CON (K, es)` | `(K e …)` | constructor, arguments |
| `CASE (e, [(K xs, e)…])` | `(case e [(K x …) e] …)` | scrutinee, branches |

Sum types (`CON`/`CASE`) are **not optional**. Turn has only products (structs),
but an agent's decision is a *choice*: run code, answer, or give up. The model's
output is a form of data, and the program must be able to ask it, "how were you
formed?" (Step 7). The ReAct loop turns out to be case analysis on the oracle's
answer.

### New: the agentic forms

| Form | Concrete syntax | Parts | Why it's primitive |
|---|---|---|---|
| `ASK (e_m, τ, e_ctx)` | `(ask m τ ctx)` | a model, a *type*, a context | The only way to consult the oracle. Nondeterministic, so it can't be defined by laws in terms of other forms. |
| `CALL (e_c, op, es)` | `(call c op e …)` | a capability, an operation name, arguments | The only way to cause an effect in the world. Authority comes from holding `c`. |
| `FAIL e` | `(fail e)` | a reason | Invalid model output, tool errors and budget exhaustion all have to be *values the program can handle*, not crashes. |
| `CATCH (e1, x, e2)` | `(catch e1 x e2)` | body, name, handler | The eliminator for `FAIL`. |
| `BUDGET (e_b, e)` | `(budget b e)` | an amount, a body | Agent loops aren't structurally recursive. A budget is the measure that "gets smaller" (Lesson 2's hallmark of an algorithmic law). |
| `PAR (e1, e2)` | `(par e1 e2)` | two expressions | Concurrency can't be derived from sequential forms. Fork-join is the minimal version. |

That's **six new forms**. Everything below is deliberately left out of the core.

### Deliberately *not* core forms

| Construct | Status | Reason |
|---|---|---|
| `retry`, `refine`, `react` loops | **Library**, defined by laws | Derivable from `ASK`, `CATCH` and recursion (see Step 6 preview). |
| Context tiers (Turn's P0/P1/P2) | **Library** | A context is an ordinary value: a list of `Message` records. Truncation and summarization are functions over lists. Nothing is ambient. |
| `confidence` | **Rejected as a primitive** | Token logprobs aren't calibrated semantic confidence (see reading notes). If you want a self-report, request it in the schema: `(ask m (Scored Verdict) ctx)`. |
| Actors (`spawn`/`send`/`receive`) | **Deferred** | `PAR` covers fork-join. Long-lived agents with mailboxes are a later lesson (Lesson 7: objects and messages). |
| Durable suspend/resume | **Implementation** | A property of how the evaluator is run (replayable traces), not something the syntax needs to express. |
| Human in the loop | **A capability** | `(call human confirm "…")`. No new form. |
| Todo lists and plans | **Library** | The CP-Agent ablation shows imposed plan structure can hurt, so it has to stay optional. |

---

## Step 1b. Values

A value `v` is one of:

- a number, string, Boolean or symbol
- a list of values
- a record `R {f = v, …}`
- a constructed value `K v …` (a sum)
- a closure
- a **capability** `cap(κ)`: opaque. All you can do with a capability is
  `CALL` it. It can't be printed, compared, serialized or placed in a context.
- a **model** `model(μ)`: opaque. All you can do with a model is `ASK` it.
  This is Lesson 3 carried over: *you can't break a model down by cases.*

The last two are the abstract types of Lesson 6. Outside the runtime they have
no forms of data.

**Context is ordinary data:**

```
(datatype Role [System] [User] [Assistant] [Tool])
(record Message (role Role) (content Text))
;; A Context is a list of Message.
```

Because a context is a list, every Lesson 2 technique (laws over `'()` and
`(cons m ms)`) applies to context management directly.

---

## Step 1c. Types, needed now because `ASK` takes one

```
τ ::= Text | Num | Bool | Sym | (List τ)
    | R                        ; record type
    | D                        ; datatype (sum)
    | (τ … -> τ ! ε)           ; function with effect set ε
    | (Cap κ) | Model
```

**Askable types** are the first-order subset: Text, Num, Bool, Sym, lists,
records and datatypes whose fields are themselves askable. That rules out
functions, capabilities and models.

This restriction does real work: **a model can't manufacture a capability,**
because `Cap κ` is never askable. Turn gets unforgeability from runtime
checks; here it follows from a typing side-condition.

---

## Step 1d. The world

`ASK` and `CALL` act on something outside the expression. From CP-Agent's
persistent kernel, tool state survives between calls. From its T=0 variance,
the oracle isn't a function. So an evaluation must mention:

- **ρ**: environment (names to values)
- **W**: the world: the state behind each capability, the remaining budget,
  and the oracle
- **t**: a trace of every `ASK` and `CALL`, with their results. This covers the
  survey's replay, audit and undo requirements.

Pinning down exactly how these appear in the judgment is **Step 4**.

---

## Distinguishing forms (the Table 1.2 / 2.2 analogue)

These are the forms a program *consumes* and must tell apart at run time:

| Data | Form | Test for form | Parts |
|---|---|---|---|
| Result of any `e` | normal value `v` | evaluation returns | `v` |
|  | failure `fail r` | evaluation raises | `r` (caught by `CATCH`) |
| Result of `ASK m D ctx`, `D` a datatype | `K₁ v…` / `K₂ v…` / … | `case` on the constructor | the fields, already validated against `D` |
|  | invalid output | validation fails, becoming `fail (Invalid raw)` | raw text, for a retry prompt |
| Budget | remaining `n + 1` | `ASK`/`CALL` may proceed | `n` |
|  | exhausted `0` | next `ASK`/`CALL` fails | none |
| Context | `'()` | `(null? ctx)` | none |
|  | `(cons m ms)` | otherwise | `m`, `ms` |

Note the budget row: it has the **Peano forms** from Lesson 1.

---

# Step 2. Example inputs

"Every form of data needs an example." Since `ASK` consults an oracle, an
example input for `ASK` has to include **an example oracle**. We use a
*scripted* oracle, a list of canned replies consumed in order. This is what
makes Step 5's example results deterministic.

## One example per form

```scheme
;; LITERAL, VAR
42      "hello"      task

;; IF, LET
(if (< n 3) 'small 'big)
(let* ([x 1] [y (+ x 1)]) y)

;; LAMBDA, APPLY
((lambda (x) (+ x 1)) 41)

;; RECORD, FIELD
(Message [role (User)] [content "Solve SEND+MORE=MONEY"])
(. m content)

;; CON, CASE
(datatype Step
  [Exec (code Text)]          ; run this code, then show me the output
  [Done (code Text)])         ; this is my final answer
(Exec "print(1+1)")
(case s [(Exec c) (run c)] [(Done c) c])

;; ASK        oracle script: [ (Done "x = 42") ]
(ask claude Step (list (Message [role (User)] [content "set x to 42"])))

;; CALL       world: py is a fresh Python kernel
(call py exec "print(1+1)")

;; FAIL, CATCH
(fail 'no-solution)
(catch (call py exec "1/0") err (. err message))

;; BUDGET
(budget 20 (react claude py ctx))

;; PAR
(par (review claude diff) (review claude tests))
```

## Whole programs: the language's own benchmark suite

Before writing any laws, the language has to express these cleanly. They come
directly from the papers.

### P1. CP-Agent (Szeider): ReAct loop with a stateful tool

```scheme
(define react (model py ctx)
  (case (ask model Step ctx)
    [(Exec code)
       (let* ([obs (catch (call py exec code) e (. e message))]
              [ctx (append ctx (list (Message [role (Assistant)] [content code])
                                     (Message [role (Tool)]      [content obs])))])
         (react model py ctx))]
    [(Done code) code]))

(define cp-agent (model py task)
  (budget 30
    (react model py (list (Message [role (System)] [content cp-prompt])
                          (Message [role (User)]   [content task])))))
```

Example world: the oracle script is `[Exec "x = intvar(0,9)", Exec "print(x)", Done "…"]`
and `py` is a fresh kernel.

Two observations for later steps:

- `react` has **no base case except `Done`**. No argument gets smaller, so its
  laws aren't algorithmic until the budget is included (see the Step 6 preview).
- The whole CP-Agent is about ten lines. If a construct would make it longer,
  that construct is suspect.

### P2. Typed extraction with a deterministic fallback (Turn's "analyst", without `confidence`)

```scheme
(datatype Verdict
  [Buy  (reason Text)]
  [Hold (reason Text)]
  [Sell (reason Text)])

(define analyst (model filing)
  (catch (ask model Verdict (list (Message [role (System)] [content analyst-prompt])
                                  (Message [role (User)]   [content filing])))
         err
         (Hold "analysis unavailable")))
```

Example worlds: (a) the oracle replies `{"tag":"Sell","reason":"…"}`; (b) the
oracle replies `"I think you should sell"`, which fails validation and takes
the fallback.

### P3. A committee with partitioned authority (Turn's running example, re-cut)

```scheme
(define committee (model net fs ticker)
  (let* ([data  (call net get (quote-url ticker))]
         [views (par (analyst model data) (risk-officer model data))]
         [memo  (ask model Memo (memo-context data views))])
    (call fs write "memo.md" (. memo text))))
```

`analyst` and `risk-officer` receive **no capabilities**. The effect system
(Lesson 5, later) should be able to *prove* they can't touch `net` or `fs`.
That proof is also what makes `PAR` safe to reorder.

### P4. Human confirmation before an irreversible effect

```scheme
(datatype Answer [Yes] [No])

(define guarded-write (human fs path text)
  (case (call human confirm (string-append "Write " path "?"))
    [(Yes) (call fs write path text)]
    [(No)  (fail 'declined)]))
```

A human is just another capability. No new form is needed.

---

## Step 6 preview: why `BUDGET` earns its place

Write P1's loop with explicit fuel `n`, and the laws become Peano recursion
(Lesson 1):

```
(react m py ctx 0)       == (fail 'budget)
(react m py ctx (+ n 1)) == case (ask m Step ctx) of
                              (Exec c) -> (react m py (ctx ++ [code c, obs c]) n)
                              (Done c) -> c
```

Now "some input is getting smaller" holds, so the laws are algorithmic.
`BUDGET` is that same fuel, threaded implicitly by the evaluator.

---

## Decisions (updated)

Resolved and superseded items are recorded in
[`02-resources-and-concurrency.md`](02-resources-and-concurrency.md):

- 1: **explicit context**, decided.
- 2: budgets are a `(cost, time)` vector with reservation semantics.
- 3: **`ask` asks once**, decided; `retry` is library.
- 4: moot.
- 5: answered. Models are stateless given explicit context, and `PAR` is
  replaced by the dataflow form `WORKFLOW`.

The original list is kept below for the record.

1. **Explicit context (chosen here) or ambient context (Turn)?** Explicit keeps
   `ASK` referentially transparent *relative to the oracle* and makes context
   policies testable. The cost is verbosity; syntax sugar could address it later.
2. **Budget units:** steps, tokens or dollars? And is `BUDGET` a form, or only
   explicit fuel?
3. **Does `ASK` retry internally on invalid output?** (Turn does, with k = 3.)
   Proposed answer: no. `ASK` fails, and `retry` is a library function
   defined by laws.
4. **Is `ASK` just `CALL` on a model capability?** It could be
   `(call m complete τ ctx)`. It stays separate for now because its result
   type is *indexed by* `τ`, which needs its own typing rule.
5. **What does `PAR` mean when both branches `ASK` the same model?** Is the
   oracle shared state?
