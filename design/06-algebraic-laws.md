# Step 6: algebraic laws

> "Algebraic laws are the single most powerful tool you will learn in Comp 105.
> They occupy a middle ground between vague English and executable code."
> — *Seven Lessons*, rationale for Step 6

> "The programmer—or perhaps a specialist—uses operational semantics to show
> that the laws are sound, and after that, programming proceeds by appealing to
> the laws, not to the operational semantics directly."
> — Ramsey, *An Imperative Core*, §1.7.4

This document has two kinds of law, following Lesson 2 §2.3:

- **Algorithmic laws** define the predefined functions (`retry`, `repair`,
  `attempt`, `best-of`, and a context policy). Each set is checked against
  Lesson 2's four hallmarks.
- **Properties** of the core forms (`catch`, `fail`, `budget`, `workflow`,
  `par`, `ask`). Each property is sound by appeal to the rules of `04` and
  labeled with its use: testing, refactoring, code improvement or specification.

Writing the laws found **five problems** in the earlier documents (§7).

---

## 0. What `==` means in a nondeterministic language

Lesson 3 defines equality of functions by their results on equal arguments.
µNorman needs a matching definition for expressions, and it has two new
complications.

**Nondeterminism.** The oracle is a relation (`04` §6.1). We handle it by
quantifying over **scripted worlds**. A scripted oracle is deterministic, and
every behavior of the live oracle is the behavior of *some* script. So:

> `e₁` and `e₂` are equivalent if, for **every** `Θ`, `ρ` and scripted world
> `W`, they produce equivalent outcomes.

**Resources.** An outcome has several parts (value, money, time, tool state,
trace), and different laws preserve different parts. So there are **three
grades of equivalence**, plus one refinement:

| Symbol | Name | `e₁` and `e₂` must agree on… |
|---|---|---|
| `≡` | exact | the result, and the entire world: pool, clock, hosts, trace |
| `≡$` | resource | the result, spend, elapsed time and host state; the trace may be reordered |
| `≡v` | value | the value on success, and failure vs. success (but not *which* failure) |
| `e₁ ⊑t e₂` | faster-or-equal | `e₁ ≡v e₂`, spend is equal on success, and `elapsed(e₁) ≤ elapsed(e₂)` |

Every law below is labeled with its grade. A law stated without side
conditions holds for all expressions. Side conditions follow Lesson 1.7:
**a variable stands for any expression**, so a condition like "`e` does not
fail" must be written out explicitly, never assumed.

**The basis must not be redefined.** Ramsey's "terrifying transcript" (§1.7.2)
redefines `+`, and then `(+ x 0) = x` is false. In the same way, every law that
mentions `retry`, `par` or `remaining` assumes the initial-basis definition is
in force.

---

## 1. Algorithmic laws for the predefined functions

### Lesson 2's four hallmarks, checked for each set

1. Each left-hand side applies the function to **variables or forms of data**.
2. The laws are **mutually exclusive**.
3. The laws **cover every input** the contract permits.
4. In every recursive call, **some input gets smaller.**

### `retry`: Peano recursion on the number of retries

**Contract:** `(retry n f)` calls the thunk `f` up to `n + 1` times, stopping at
the first success. It retries only `Invalid` and `ToolError`. `n` is a natural
number.

```
(retry 0 f)       == (f)
(retry (+ n 1) f) == (catch (f) err
                       (case err
                         [(Invalid _)   (retry n f)]
                         [(ToolError _) (retry n f)]
                         [_             (fail err)]))
```

Hallmarks: the forms are `0` and `(+ n 1)` (Peano), so they're exclusive and
cover every natural. `n` gets smaller. `f` appears as a variable, because
"you can't break a function down by cases" (Lesson 2 §2.4).

> **Problem found (§7, #1).** `Num` includes negatives, but the laws cover only
> naturals. The code in `04` tested `(= n 0)`, so `(retry -1 f)` never reaches
> the base case. It keeps calling `f`, which spends money on every call, until
> `OverBudget` stops it *after the entire budget is gone*. This is Lesson 1.7's
> mistake: "a permissible form of data that does not match the left-hand side
> of any law." The fix: distinguish forms with `(<= n 0)`, so a negative count
> behaves as 0 (Table 1.2's test, made robust).

**Properties of `retry`:**

```
(retry n (lambda () e))  ≡  e        if e never fails                         ; testing
(retry n (lambda () (fail OverBudget)))    ≡ (fail OverBudget)                ; spec: never retries budget
(retry n (lambda () (fail PastDeadline)))  ≡ (fail PastDeadline)
(retry m (lambda () (retry n f)))  ≡  (retry (- (* (+ m 1) (+ n 1)) 1) f)      ; refactoring
```

The last law says nested retries multiply attempts: `(m+1)(n+1)` in total. It's
exact (`≡`), because the sequence of calls to `f` is identical.

### `repair`: like `retry`, with the context growing

**Contract:** `(repair n f ctx)` calls `(f ctx)`. If the reply is `Invalid raw`,
it tries again (at most `n` more times) with `raw` and a correction appended to
the context. It never repairs any other failure.

```
(repair 0 f ctx)       == (f ctx)
(repair (+ n 1) f ctx) == (catch (f ctx) err
                            (case err
                              [(Invalid raw) (repair n f (append ctx (correction raw)))]
                              [_             (fail err)]))
```

Here `(correction raw)` is the two-message list shown in `04` §7.2.

**Cost property** (specification; this is what explicit context buys):
each repair attempt's context is strictly longer, so **its reservation is
strictly larger** (law A2 below). Repairing costs more per attempt than
retrying. That's the trade-off RSTD's Mellea hides by merging the two.

### `attempt`: recoverable failures become `None`

```
(attempt (lambda () e) check)  where e ⇓ v                ==  (if (check v) (Some v) None)
(attempt (lambda () e) check)  where e ⇓ fail (Invalid r)   ==  None
(attempt (lambda () e) check)  where e ⇓ fail (ToolError m) ==  None
(attempt (lambda () e) check)  where e ⇓ fail φ, otherwise  ==  (fail φ)
```

These laws split on the **outcome** of `e`, which is a form of data (a result
is a value or a failure). So they're mutually exclusive and exhaustive.

### `best-of`: Peano recursion on the sampling width

> **Problem found (§7, #2).** The version in `04` recursed on `k` with base case
> `(= k 1)`. `(best-of* 0 f check)` therefore recursed forever, and each level
> forked another `attempt`. It would spend until `OverBudget`, just like
> `retry -1`. Rewriting the laws in Peano form fixes it and simplifies the code.

```
(best-of* 0 f check)       == None
(best-of* (+ k 1) f check) == (let* ([p (par (attempt f check) (best-of* k f check))])
                                (first-some (. p fst) (. p snd)))

(best-of k f check) == (case (best-of* k f check)
                         [(Some x) x]
                         [None     (fail 'none-accepted)])
```

**Properties:**

```
(best-of 1 f check) ≡ (case (attempt f check) [(Some x) x] [None (fail 'none-accepted)])   ; refactoring
spent   (best-of k f check) = Σ spent of the k attempts      (on success)                   ; work
elapsed (best-of k f check) = max elapsed of the k attempts  (on success)                   ; span
```

> **Problem found (§7, #3).** `02` §C6 stated
> `(best-of 1 f check) == (let* ([x (f)]) (if (check x) x (fail …)))`. That's
> **false**. When `f` fails with `Invalid`, the left side gives
> `(fail 'none-accepted)`, because `attempt` absorbed the `Invalid`, while the
> right side gives `(fail (Invalid …))`. The two are `≡v` but not `≡`, since the
> failures differ. The correct law goes through `attempt`, as above.

**Probabilistic property, with its assumption stated** (MCPP §3.1, critiqued
in the reading notes): *if* the `k` attempts succeed independently with
probability `p`, then `Pr[best-of k succeeds] = 1 − (1 − p)^k`. This is **not**
an algebraic law. It depends on an independence assumption that correlated
samples from one model on one prompt violate. It's recorded so that nobody
mistakes it for one.

### `window`: a context policy (Turn's P0/P1/P2 as ten lines of library)

Turn builds a tiered context manager into its VM. With explicit context, it's
a list function with Lesson 2 laws. **Contract:** `(window n ctx)` keeps every
`System` message, in order, followed by the last `n` of the other messages.

```
(drop 0 xs)                == xs
(drop (+ k 1) '())         == '()
(drop (+ k 1) (cons y ys)) == (drop k ys)

(last-n n xs) == (drop (- (length xs) n) xs)      ; drop treats a count ≤ 0 as 0

(window n ctx) == (append (filter system? ctx)
                          (last-n n (filter (lambda (m) (not (system? m))) ctx)))
```

**Properties** (for testing):

```
(length (window n ctx)) ≤ (+ n (length (filter system? ctx)))
(window n ctx) == ctx                     when (length ctx) ≤ n
reservation(ask m τ (window n ctx)) ≤ reservation(ask m τ ctx)       ; by law A2
```

So context management needs no primitive, no VM tier and no fixed `W = 100`.
It's a function whose effect on cost follows from a law.

---

## 2. Laws for `catch` and `fail`

```
(C1) (catch e₀ x h) ≡ e₀                   e₀ a literal, variable or lambda   [CATCHOK]
(C2) (catch (fail e) x h) ≡ (let* ([x e]) h)       if e cannot fail             [FAIL, CATCHFAIL]
(C3) (catch e x (fail x)) ≡ e                                                  [CATCHOK, CATCHFAIL]
(C4) (K e₁ … (fail φ) … eₙ) ≡ (begin e₁ … eᵢ₋₁ (fail φ))                       [PROPAGATE]
```

- **C2's side condition is Lesson 1.7's lesson.** If `e` itself can fail with
  `φ′`, the left side *catches* `φ′` (the catch surrounds the whole `fail e`),
  while the right side lets `φ′` escape. Without the side condition, the law
  is false.
- **C3 is the "rethrow" law,** µNorman's analogue of Ramsey's Exercise 13,
  `(if x x 0) ≡ x`. It's proved in §6.
- **C4 is strictness, from left to right.** Everything before the failure
  still runs and still spends. Nothing after it runs.

---

## 3. Laws for `budget`

```
(B1) (budget (c₁ t₁) (budget (c₂ t₂) e))  ≡  (budget ((min c₁ c₂) (min t₁ t₂)) e)   limits are values   [BUDGET]
(B2) (budget (∞ ∞) e)                     ≡  e                                                       [BUDGET]
(B3) (budget r v)                         ≡  v       v a value                                        [BUDGET, LITERAL]
(B4) spent (budget (c t) e)  ≤  c                    ; spec: Theorem 1, localized
(B5) (budget (c t₀) (ask …))  ≡  (fail PastDeadline)   when t₀ = 0s: nothing charged   [ASKEXPIRED, new]
```

**B1 needs its side condition for the same reason as C2:** if the limit
expressions had effects (say, `(budget ([cost (ask …)]) e)`), merging the
scopes would change when those effects happen.

**B5 found a problem (§7, #4).** Under `04`'s rules, an `ask` evaluated *after*
its deadline has passed still consulted the oracle and then failed by ASKLATE,
**charging its full reservation**. So a `PastDeadline` handler that tried a
fallback `ask` would pay for a call it could never use. The fix is a new rule,
**ASKEXPIRED**: if `W₂.now ≥ W₂.deadline` *before* the call, fail
`PastDeadline` with no oracle premise and no charge. CALLEXPIRED is the same
for tools. B5 is then exact.

**Monotonicity: a property that holds only conditionally.**

```
(B6) if (budget (c t) e) ⇓ v and c ≤ c′, t ≤ t′,
     then (budget (c′ t′) e) ⇓ v         — PROVIDED e never evaluates (remaining)
```

With more money and more time, every reservation and deadline premise that held
still holds, so the same derivation goes through. **But `(remaining)` breaks
this.** A program that reads its budget (like `pick-model` in `02` §C4) may take
a different branch with more money. MCPP's central result is that
state-dependent policies win, and the price of that is monotonicity. The law
makes the trade-off explicit: *budget-aware programs aren't monotone in their
budget*.

---

## 4. Laws for `workflow` and `par`

```
(W1) (workflow ([x₁ e₁] … [xₙ eₙ]) e)  ≡v  (let* (topologically ordered bindings) e)
        provided  (i) nodes own disjoint stateful capabilities   [WORKFLOW ownership premise]
                  (ii) no node evaluates (remaining)             [see Q-A below]

(W2) (par e₁ e₂)  ≡$  (swap (par e₂ e₁))     where swap p = (Pair [fst (. p snd)] [snd (. p fst)])
        except when both fail at the same virtual instant (then ≡v)

(W3) (par v e)  ≡  (let* ([y e]) (Pair [fst v] [snd y]))      v a value

(W4) flattening:     (workflow (bs₁) (workflow (bs₂) e))  ⊒t  (workflow (bs₁ ++ bs₂) e)
                     when (a) the names of bs₂ are distinct from those of bs₁,
                          (b) no node of bs₁ mentions a name bound by bs₂ (no capture), and
                          (c) ownership holds across bs₁ ++ bs₂ together
                     (in the nested form, bs₁ and bs₂ run in separate phases, so they may share a kernel;
                      flattened, they may not)

(W5) parallelizing:  (let* ([x e₁] [y e₂]) e)  ⊒t  (workflow ([x e₁] [y e₂]) e)
                     when x ∉ fv(e₂), and (i) and (ii) of W1 hold

(W6) work:  spent   (workflow …) = Σ spent(eᵢ) + spent(e)         (on success)
     span:  elapsed (workflow …) = longest path (by elapsed) through the DAG + elapsed(e)
```

- **W1 is Theorem 4,** and writing it as a law found a problem (§7, #5).
  Condition (ii) wasn't in `04`. A node that reads `(remaining)` sees how much
  its *siblings* have spent so far, which depends on scheduling. The
  sequentialization lets earlier nodes spend first, so a node could compute a
  different value. `04`'s Theorem 4 was stated without this premise, and it's
  false without it. See question Q-A.
- **W4 and W5 are code improvements** (Lesson 2: "rewriting code to improve its
  performance, without changing its semantics"). They're the language's
  **parallelization optimizations**. The Step 5 test "40 s vs. 60 s" is W4 in
  action: the fork-join version is `n-graph-par`, and the flattened dataflow is
  `n-graph`.
- **W2's exception is real.** Fail-fast reports the failure that happens first,
  with ties broken by binding order. Swapping the branches swaps the
  tie-break. The trade is deliberate: the alternative is nondeterministic
  failure identity even in scripted mode.

---

## 5. Laws for `ask`, and some tempting non-laws

```
(A1) reservation(ask m τ ctx) = price_m(|ctx|, max_out(m, τ))           ; spec, by definition
(A2) |ctx| ≤ |ctx′|  ⇒  reservation(ask m τ ctx) ≤ reservation(ask m τ ctx′)   ; monotone in context
(A3) bound(τ) ≤ bound(τ′) ⇒ reservation(ask m τ ctx) ≤ reservation(ask m τ′ ctx)
```

A3 has a **hidden trade-off**. Narrowing `Text` to `(Text 400)` makes asks
*cheaper to reserve*, so fewer of them fail with `OverBudget`. But it makes
validation *stricter*, so more of them fail with `Invalid`. Narrowing a type is
**not** value-preserving. It's a policy choice, and the law says exactly which
failure you're trading for which.

### Non-laws

Lesson 2 teaches that some properties look true and aren't. These are the
mistakes an optimizer or a refactoring tool would make.

| Tempting "law" | Why it's false | Rules that break it |
|---|---|---|
| **Common-subexpression elimination:** `(let* ([a (ask m τ c s)] [b (ask m τ c s)]) (Pair a b))` = `(let* ([a (ask m τ c s)]) (Pair a a))` | The oracle is a relation. Two asks may answer differently, and the right side spends half as much. (CP-Agent observed variation even at T = 0.) | ASKOK (two oracle premises vs. one) |
| **Retry commutes with budget:** `(budget r (retry n f))` = `(retry n (lambda () (budget r (f))))` | The left side shares one budget across *all* attempts. The right side gives *each* attempt `r`, capped only by the parent. Both are useful; they're different. | BUDGET |
| **Catch commutes with budget:** `(catch (budget r e) x h)` = `(budget r (catch e x h))` | On the left, the handler runs *outside* the scope with the parent's remaining money. That's the fallback idiom (`02` §C6). On the right, it runs *inside*, where an `OverBudget` scope has nothing left. | BUDGET, CATCHFAIL |
| **A failing workflow's cost doesn't depend on scheduling** | Fail-fast cancels siblings at different points (`04` §8). | WORKFLOW (fail-fast) |
| **Moving an `ask` to a different site changes nothing** | Scripts and traces are keyed by site, and MCPP-style profiles are per site. The value is `≡v`, but the trace is not `≡`. | ASKOK (trace entry) |

The first row is the most important. **Caching model answers is never an
automatic optimization in µNorman.** A cache is state, so it's a *stateful
capability*: `(call cache get key)` and `(call cache put key v)`. The program
opts into it explicitly, and ownership and the trace both account for it. See
question Q-B.

---

## 6. Two soundness proofs, in Ramsey's template (§1.7.3)

**Law C3: `(catch e x (fail x)) ≡ e`.** The proof is by cases on the last rule
of the derivation for the left side. Only two rules conclude in a CATCH.

- *When the last rule is CATCHOK*, the derivation has the form
  `⟨e, Θ, ρ, W⟩ ⇓ ⟨v, W′⟩` over `⟨CATCH(e, x, FAIL(VAR x)), Θ, ρ, W⟩ ⇓ ⟨v, W′⟩`.
  The premise is itself a derivation for `e` with the same outcome `⟨v, W′⟩`.
  Our obligation is met.
- *When the last rule is CATCHFAIL*, the premises are
  `⟨e, Θ, ρ, W⟩ ⇓ ⟨fail φ, W₁⟩` and
  `⟨FAIL(VAR x), Θ, ρ{x ↦ φ}, W₁⟩ ⇓ ⟨r, W′⟩`. The second premise is
  justified only by FAIL over VAR. VAR gives `⟨φ, W₁⟩`, since VAR doesn't
  change the world. FAIL then gives `r = fail φ` and `W′ = W₁`. So the
  conclusion is `⟨fail φ, W₁⟩`, exactly the outcome of `e`. Our obligation is
  met.

Both cases give identical results and worlds, so the law is exact (`≡`).

**Law B1: budget nesting.** Take the limit expressions to be values, so their
evaluation doesn't change the world (LITERAL). Let `P = W.pool`,
`D = W.deadline` and `N = W.now`.

- *Left side.* The outer BUDGET sets pool `min(c₁, P)` and deadline
  `min(D, N + t₁)`. The inner BUDGET then sets pool
  `min(c₂, min(c₁, P)) = min(min(c₁, c₂), P)` and deadline
  `min(min(D, N + t₁), N + t₂) = min(D, N + min(t₁, t₂))`, because `now` hasn't
  changed.
- *Right side.* The single BUDGET sets exactly those values.
- So the body `e` is evaluated in the **same world** on both sides. By
  determinism of the scripted world, it produces the same `⟨r, W₄⟩`.
- *Charges back to the parent.* Let `s` be the body's spend. The inner scope
  returns `min(c₁, P) − s` to the middle scope, and the outer scope returns
  `P − s`. The single scope also returns `P − s`. Deadlines are restored to `D`
  on both sides. The trace is identical. Our obligation is met: `≡`.

---

## 7. Problems the laws found

| # | Problem | Where | Fix |
|---|---|---|---|
| 1 | `retry` with a negative count loops until the whole budget is spent | `04` §7.2 | test with `(<= n 0)` |
| 2 | `best-of* 0` forks without end | `04` §7.2 | Peano laws with base case `0 → None` |
| 3 | `02`'s law for `best-of 1` is false when `f` fails `Invalid` | `02` §C6 | the law goes through `attempt` |
| 4 | an `ask` after the deadline still calls the oracle and pays its reservation | `04` §6.5 | new rules ASKEXPIRED and CALLEXPIRED (no call, no charge) |
| 5 | Theorem 4 (workflow sequentialization) is false if a node reads `(remaining)` | `04` §8 | add premise (ii), pending Q-A |

This is Impcore §1.7's point in practice: "Proofs … are interesting primarily
when they are wrong. Like a bug in a program, a wrong proof tells you that you
made a mistake (in your language design, not your code)."

---

## 8. How the laws will be used

These are Lesson 2's four uses for properties, applied here:

- **Testing.** Instances of the laws become unit tests in
  [`examples/step6-laws.nrm`](../examples/step6-laws.nrm). Later, random
  scripts can drive property-based tests. Because the laws quantify over
  scripted worlds, a random script *is* a random test case.
- **Refactoring.** C3, W2, W3 and nested `retry`.
- **Code improvement.** W4 and W5 (parallelization), and A3 (narrowing types to
  shrink reservations, with the stated trade-off).
- **Specification.** B4, the `retry` budget properties and A1. These are the
  contracts that future users rely on without reading `04`.

---

## Decisions

- **Q-A, decided: `(remaining)` is the live shared value, even inside
  concurrent nodes.** Budget-aware choices use real information (MCPP's central
  result). The price is that W1/Theorem 4 keeps premise (ii): a node that
  reads `(remaining)` isn't covered by the sequentialization law. The future
  effect system will flag such nodes statically, as an `observe` effect.
  Theorem 1 holds either way.
- **Q-B, decided: no deterministic-model declarations.** CSE and memoization
  of `ask` are never laws. Caching is an explicit stateful capability
  (`call cache get/put`), which ownership and the trace account for.
