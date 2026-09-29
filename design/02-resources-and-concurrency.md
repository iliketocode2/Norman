# Resources and concurrency: `par`, `workflow`, `budget`

This revises Step 1 (forms of data). It uses four sources: the CP-Agent token
numbers, MCPP's formal model (reading notes §4), Ramsey Lessons 1–3, and the
two decisions below.

## Decisions recorded

| # | Question | Decision |
|---|---|---|
| D1 | Explicit or ambient context? | **Explicit.** A context is an ordinary list of `Message` values, passed to `ask`. |
| D3 | Does `ask` retry internally? | **No.** `ask` does exactly what its name says: it asks once. Continuing after a failure is `retry`'s job, and `retry` is a library function defined by laws. |

---

## Part A. What `par` is, and what it's for

### The problem it solves

Some pieces of work don't depend on each other. In Turn's investment
committee, the analyst and the risk officer both read the same market data,
and neither needs the other's answer. Written sequentially:

```scheme
(let* ([v1 (analyst m data)]        ; $0.02, 20 s
       [v2 (risk-officer m data)])  ; $0.02, 20 s
  (list v1 v2))
;; total: $0.04, 40 s
```

The second model call waits for the first for no reason. `par` says "these two
are independent; run them at the same time":

```scheme
(par (analyst m data) (risk-officer m data))
;; total: $0.04, 20 s
```

**Same answer, same cost, less time.** That's all `par` is for. It exchanges
nothing for wall-clock time. Under MCPP's hard deadlines, that can decide
whether a workflow succeeds.

### What it means

`(par e1 e2)` starts `e1` and `e2` concurrently, waits for **both** to finish
(fork-join), and returns the list of their two values. If either fails, the
`par` fails.

### The law that defines it

```
(par e1 e2)  ==  (let* ([a e1] [b e2]) (list a b))
    provided e1 and e2 have disjoint effects
```

This is a Lesson 2 *property*, not an algorithmic law. It says `par` changes
**when** things happen, never **what** the answer is. The side condition is
what makes that true:

- **Safe:** two `ask`s. A model has no hidden conversation state, because we
  made context explicit (D1). The input to `ask` is the whole triple
  `(m, τ, ctx)`, so two concurrent asks can't interfere with each other.
- **Unsafe:** two `call`s on the same *stateful* capability, for example both
  branches running code in the same Python kernel. The result would depend on
  the interleaving. The effect system (Lesson 5, later) is what will reject
  this.

That answers the earlier open question, "Is the model shared state when both
branches ask it?" **No.** Explicit context makes models stateless. The only
state that concurrent branches share is (1) the resource pool (Part C) and
(2) stateful capabilities (the effect system).

### How it's accounted for (from MCPP)

| Composition | Cost | Time |
|---|---|---|
| sequential `(let* ([a e1] [b e2]) …)` | c₁ + c₂ | t₁ + t₂ |
| parallel `(par e1 e2)` | c₁ + c₂ | max(t₁, t₂) |

This is the **work/span** cost model from parallel algorithms (Blelloch &
Greiner). Cost is *work*, the total effort you pay for. Time is *span*, the
length of the critical path. `par` leaves work unchanged and shortens span.

---

## Part B. Why `par` isn't the right primitive: workflows are DAGs

MCPP's workflows are arbitrary dependency DAGs, and a node becomes ready the
moment its predecessors finish (`R(S) = {v : Pred(v) ⊆ S}`). Fork-join `par`
can only express **series-parallel** graphs. Here's a small DAG it can't
express:

```
   a ──► c ◄── b
   │
   └──► d            (c needs a and b; d needs only a)
```

With `par` you're forced to write `(par a b)` and then `(par c d)`, so `d`
waits for `b` even though it doesn't need it. With a = c = 10 s and b = d = 30 s:

| Encoding | Critical path | Time |
|---|---|---|
| `(par a b)` then `(par c d)` | max(10,30) + max(10,30) | **60 s** |
| Dataflow: `d` starts when `a` finishes | a→d = 40, b→c = 40 | **40 s** |

The cost is identical. Only the dataflow version is optimal.

### Proposal: replace `PAR` with `WORKFLOW`, a dataflow `let`

```scheme
(workflow ([x₁ e₁] … [xₙ eₙ]) body)
```

- Each `eᵢ` may mention any other `xⱼ`. **The dependency graph is read off the
  free variables.** The programmer never draws it.
- The graph must be acyclic. That's a static check, and it's our first
  compile-time judgment.
- Evaluation: start every binding whose dependencies have values (MCPP's ready
  set), repeat as bindings finish, and evaluate `body` once all are done.
- If any binding fails, cancel the ones still running and fail with that
  failure (fail-fast). Resources already spent stay spent.

The laws:

```
(workflow bindings body)  ==  (let* (topo-sort bindings) body)
    provided the bindings have pairwise-disjoint effects       ; meaning unchanged

cost (workflow …) = Σ cost(eᵢ)                                   ; work
time (workflow …) = longest path through the DAG                 ; span

(par e1 e2)  ==  (workflow ([a e1] [b e2]) (list a b))           ; par is derived
```

So `par` moves to the library as sugar, and the core still has **six forms**:
`ASK`, `CALL`, `FAIL`, `CATCH`, `BUDGET`, `WORKFLOW`.

### Example P5: ProofFlow (the MCPP benchmark) in µNorman

```scheme
(define prove-step (model stmt deps)
  (let* ([lean  (ask model LeanStmt  (formalize-ctx stmt deps))]
         [proof (ask model LeanProof (prove-ctx lean deps))])
    proof))

(define proof-graph (model s1 s2 s3 s4)
  (workflow ([a (prove-step model s1 '())]
             [b (prove-step model s2 '())]
             [c (prove-step model s3 (list a b))]
             [d (prove-step model s4 (list a))])
    (list a b c d)))
```

The DAG from Part B falls out of the variable references, and `d` starts as
soon as `a` has a value.

---

## Part C. Budgets

### C1. Units: resources are a vector, and its two parts behave differently

A resource bound is `r = (cost, time)`.

| | **Cost** | **Time** |
|---|---|---|
| Unit | money: integer micro-dollars, never floats (`$0.50` is sugar for 500 000 µ$) | wall-clock milliseconds (`30s`, `10min` are sugar) |
| Kind of resource | a **pool** that is *consumed* | a **clock** that *passes* |
| Under `workflow` | concurrent draws add up | every branch sees the same deadline, so time is effectively a max |
| Source of charges | `ask`: input tokens × input price + output tokens × output price. `call`: tool-declared cost, usually 0. | measured latency |

Steps and tokens are not user-facing units:

- **Tokens** are converted to money per model. Input and output are priced
  separately. That fixes MCPP's gap, since input tokens dominate agent loops
  (37k in to 3k out in CP-Agent).
- **Step counts** aren't a resource anyone pays for. Termination follows from
  the real resources anyway: every `ask` costs at least some ε > 0 and every
  `call` takes at least some δ > 0 of time. So a budgeted loop is always
  bounded, and the budget is the Peano measure from `01`.
  **Correction (`07` §7):** this holds only for loops that *perform effects*.
  A pure loop never advances the clock, so it's bounded by the interpreter's
  step limit, not by the budget.

### C2. The `BUDGET` form, revised

```scheme
(budget ([cost $0.50] [time 10min]) e)   ; either component may be omitted
```

This runs `e` inside a **sub-scope** carved out of the enclosing one. The
effective limit is the smaller of the requested limit and what's left in the
parent. Whatever `e` actually uses is charged to the parent, and anything
unused goes back.

Laws:

```
(budget r₁ (budget r₂ e))  ==  (budget (min r₁ r₂) e)     ; componentwise min
(budget ∞ e)               ==  e
cost-to-parent (budget r e)  ≤  r.cost                     ; a sub-scope never overdraws
```

### C3. How `ask` spends money: reservation, which makes the limit truly hard

We don't know an answer's length until it arrives, so how can a hard
limit hold? By **reserving the worst case before calling**. Because context is
explicit, the input size `|ctx|` is known exactly before the call, and each
model has a maximum output length `max_out`.

Draft rules, written as relations. The oracle `⇝` is a relation, not a
function. That's the fix for Turn's `llm(e, s) = j`.

```
 ĉ = price_m(|ctx|, max_out_m)    ĉ ≤ b    O_m(ctx, τ) ⇝ (j, n_out, done_at)
 done_at ≤ deadline    validate(j, τ) = v
 ───────────────────────────────────────────────────────────────── (Ask-Ok)
 ⟨ask m τ ctx, b⟩ ⇓ ⟨v, b − price_m(|ctx|, n_out)⟩

 ĉ = price_m(|ctx|, max_out_m)    ĉ > b
 ───────────────────────────────────────────────── (Ask-Unaffordable)
 ⟨ask m τ ctx, b⟩ ⇓ ⟨fail (OverBudget), b⟩        ; no call made, nothing charged

 … validate(j, τ) = error
 ───────────────────────────────────────────────── (Ask-Invalid)
 ⟨ask m τ ctx, b⟩ ⇓ ⟨fail (Invalid j), b − price_m(|ctx|, n_out)⟩

 … the deadline arrives before the reply
 ───────────────────────────────────────────────── (Ask-Late)
 ⟨ask m τ ctx, b⟩ ⇓ ⟨fail (PastDeadline), b − ĉ⟩  ; cancelled; charge the reservation
```

**Invariant (to be proved in Step 6): total spend never exceeds `B`.** Every
charge is at most its reservation, every reservation is at most what remains,
and reservations are taken atomically, so concurrent `workflow` branches can't
overdraw the shared pool.

This is a real theorem about a well-formed judgment, unlike Turn's Theorem 1.
The cost is pessimism: a call that would have fit may be refused because its
*worst case* doesn't. MCPP already treats exceeding the limit as failure, so
that trade matches the "hard constraint" philosophy.

### C4. Observing what's left

MCPP's central empirical result is that policies which look at the remaining
`(b, h)` beat static ones. So programs need an **observer** (Liskov's term):

```scheme
(remaining)   ; returns the record (Resources [cost µ$] [time ms]) for the current scope
```

This is a primitive *function*, not a new syntactic form. It reads the world,
like any other primitive. Example of a budget-aware choice:

```scheme
(define pick-model (strong cheap)
  (if (> (. (remaining) cost) $0.10) strong cheap))
```

### C5. Failure is a form of data

Running out of budget, running out of time, bad model output and a failed
tool are *different* failures. The program has to be able to ask a failure how
it was formed. So `fail` carries a datatype:

```scheme
(datatype Failure
  [Invalid      (raw Text)]      ; model output didn't match τ
  [ToolError    (message Text)]  ; a call failed
  [Refused      (category Sym)]  ; the model declined (added by 09, decision B)
  [OverBudget]                   ; a reservation didn't fit
  [PastDeadline]                 ; the clock ran out
  [Raised       (reason Sym)])   ; the program said (fail 'reason)
```

This matters immediately, in the laws for `retry`.

### C6. `retry`, `best-of` and fallback: library functions with laws

**`retry`** takes a *thunk*. It needs a function to call again, not a value
(Lesson 3: functions as arguments). It retries only failures where trying again
can help:

```
(retry 0 f)       == (f)
(retry (+ n 1) f) == (catch (f) err
                       (case err
                         [(Invalid _)   (retry n f)]
                         [(ToolError _) (retry n f)]
                         [_             (fail err)]))   ; never retry OverBudget or PastDeadline
```

This is Peano recursion on `n`, so the laws are algorithmic and terminate.
Retrying `OverBudget` would be pointless, because the next attempt can't
afford a reservation either. The laws say so explicitly rather than leaving it
to chance.

**`best-of`** is MCPP's sampling width `k`. It needs a verifier; the paper
assumes one without saying so.

```
(best-of k f check)   ; run k copies of (f) as a workflow; return any result check accepts
(best-of 1 f check) == (let* ([x (f)]) (if (check x) x (fail (Raised 'rejected))))
    ; ✗ FALSE when (f) fails Invalid: see 06 §1 (problem #3) for the correct law
cost (best-of k …) = k · cost (f)          time (best-of k …) ≈ time (f)
```

**Fallback under a sub-budget** is the idiom that makes nested budgets useful:

```scheme
(catch (budget ([cost $1.00]) (expensive-approach task))
       err
       (case err
         [(OverBudget) (cheap-approach task)]   ; the parent scope still has money
         [_            (fail err)]))
```

### C7. Where estimates come from (for a future MCPP-style scheduler)

MCPP needs, for each (subtask, model) pair, a success rate and a length
estimate. It collected these by running 512 samples per pair offline. Our
semantics already produces a **trace** (from `01`, Step 1d). Each entry records
the `ask` site, model, input tokens, output tokens, latency and outcome. That
is MCPP's profile `ω_{v,m}`, collected as a side effect of ordinary runs.

Keeping models as *parameters* (never hard-coded at `ask` sites) leaves room
for a later separation of **workflow** (the program) from **schedule** (which
model and how many samples per site). That's deferred, but the core must not
rule it out.

---

## Revised core, going into Step 3

| Core form | Status |
|---|---|
| `ASK (m, τ, ctx)` | unchanged; charges by reservation; asks once |
| `CALL (c, op, es)` | unchanged; may declare a cost |
| `FAIL e` | now carries a `Failure` value |
| `CATCH (e₁, x, e₂)` | unchanged |
| `BUDGET (r, e)` | `r = (cost, time)`; sub-scope semantics |
| `WORKFLOW (bindings, body)` | **replaces `PAR`**; dataflow over an acyclic dependency graph |

In the library, defined by laws: `par`, `retry`, `best-of`, `refine`, `react`,
and context policies. Primitive observer: `(remaining)`.

## Open questions for Step 4 (the contract)

> **Resolved in [`03-resolving-open-questions.md`](03-resolving-open-questions.md):**
> Q1: the output bound comes from the type. Q2: stateful capabilities are
> owned; reject, never serialize. Q3: site-keyed scripts with fault injection
> and a virtual clock. Q4: one absolute clock per `budget` scope.


1. **Maximum output length for reservation:** per model (simple) or per `ask`
   (more precise and more verbose)?
2. **Stateful capabilities inside a `workflow`:** reject them statically (needs
   the effect system first), or serialize access at run time until then?
3. **Scripted oracle entries** will need `(reply, input tokens, output tokens,
   latency)` so that Step 5's example results can check budget and deadline
   behavior deterministically. Is that the right test fixture?
4. **Deadline representation:** an absolute instant (natural for a clock
   shared across concurrent branches, and my recommendation) or a relative
   remainder `h` as in MCPP?
