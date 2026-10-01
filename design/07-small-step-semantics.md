# A small-step semantics for concurrency: the µNorman machine

`04` §6.5 admits one approximation. The big-step WORKFLOW rule threads the
money pool through nodes *one node at a time*, but real charges from concurrent
nodes interleave *event by event*. So when the budget is too tight for
everyone, big-step can't say **which** node gets `OverBudget`. Ramsey's
glossary gives the remedy: "Small-step semantics can express more kinds of
program behaviors than big-step semantics." This document gives that
semantics.

It's the semantics the Rust interpreter will implement for `workflow`. The
big-step rules of `04` remain the specification for everything else, and §7
states how the two relate.

---

## 1. The approach in one paragraph

A running program is a **machine**: a set of **threads**, one per active
workflow node, plus the main thread. There's a tree of **budget scopes**, a set
of **in-flight requests** (asks and calls that have been sent but haven't
returned), and a **clock**. Pure computation takes **zero virtual time**. A
thread computes, instantaneously, until it reaches an `ask` or a `call`. Then
it *issues* a request (reserving money) and blocks. When no thread can move,
the clock **jumps** to the earliest completion time, those requests complete
(charging actual money), and their threads resume. That's a discrete-event
semantics. In the control paper's vocabulary, it's a hybrid system: time
passes in *flows*, and the state changes in *jumps*.

**Why zero-time pure computation is a sound modelling choice.** Turn measured
its VM operations at 0.6–2.3 µs, "three to four orders of magnitude below the
minimum LLM network round-trip (100 ms–5 s)" (reading notes §2, Turn §10.5).
Pure µNorman code is negligible time next to the effects it orchestrates, so
the semantics treats it as instantaneous. The consequence is noted in §7.

---

## 2. Runtime syntax and evaluation contexts

The small-step machine uses **substitution** instead of environments, the
usual choice for small-step semantics. It adds two **runtime forms** that
never appear in source programs:

```
e ::= … (all of 04)
    | SCOPE(σ, e)      ; e is running inside budget scope σ
    | WAIT(w)          ; this thread is waiting for workflow w to finish
```

**Evaluation contexts** say where the next step happens: the leftmost
unevaluated subexpression, which is the same order the big-step rules thread
the world.

```
E ::= □
    | (K v… E e…) | (R [f v]… [f E] [f e]…) | (. E f)
    | (if E e e) | (let ([x E]) e) | (E e…) | (v v… E e…)
    | (case E branches)
    | (ask E τ e s) | (ask v τ E s)
    | (call E op e…) | (call v op v… E e…)
    | (fail E) | (catch E x e)
    | (budget ([cost E] [time e]) e) | (budget ([cost v] [time E]) e)
    | SCOPE(σ, E)
```

`workflow` bodies and `lambda` bodies aren't contexts. They're evaluated only
when the machine starts them.

---

## 3. Pure reduction, `e ⟶ e′` (takes no time and touches no machine state)

```
(if #t e₂ e₃) ⟶ e₂                     (if #f e₂ e₃) ⟶ e₃
(let ([x v]) e) ⟶ e[x ↦ v]
((lambda (x₁…xₙ) e) v₁…vₙ) ⟶ e[x₁ ↦ v₁, …, xₙ ↦ vₙ]
(. R{…, f = v, …} f) ⟶ v
(case v … [p e] …) ⟶ e[bindings of p]  if p is the first pattern that matches v
(p v₁ … vₙ) ⟶ the primitive's result     for primitives other than (remaining)
(catch v x h) ⟶ v
(catch F[(fail φ)] x h) ⟶ h[x ↦ φ]      F contains no catch frame
(fail φ) inside any other frame F′ ⟶ (fail φ)   (propagation, PROPAGATE's small-step form)
SCOPE(σ, v) ⟶ v   and   SCOPE(σ, fail φ) ⟶ fail φ   ; with the side effect "close σ" (§4)
```

These are the ordinary rules. The interesting ones are the machine
transitions.

---

## 4. The machine

```
M = ⟨ T, Σ, Ω, P, now, hosts, trace ⟩

T   threads     tid ↦ ⟨e, base-scope, path, status⟩        status ∈ {runnable, blocked(rid), waiting(w), done}
Σ   scopes      σ ↦ ⟨limit, spent, reserved, deadline, parent⟩
Ω   workflows   w ↦ ⟨parent tid, nodes, body⟩               each node: ⟨x, e, deps, pending | running(tid) | done(v)⟩
P   requests    rid ↦ ⟨tid, ĉ, σ, due, outcome⟩
```

Two functions on scope chains:

```
available(σ) = min over σ and its ancestors of (limit − spent − reserved)
deadline*(σ) = min over σ and its ancestors of deadline
```

**A thread's current scope** is the innermost `SCOPE` frame in its expression,
or its base scope if there isn't one. Charges go to **every scope on the
chain**, so a parent's `spent` always includes its children's spending. That's
what the big-step BUDGET rule computes by subtraction.

**Paths** name threads for tie-breaking. The main thread is `ε`, and node `x` of
a workflow started by a thread at path `p` has path `p.x`. Paths are ordered
lexicographically, with node names compared in **binding order**.

### 4.1 Instant transitions (no time passes)

The **scheduler policy** is part of the semantics, so that scripted mode is
deterministic. **The runnable thread with the least path moves.** Because it
stays least until it blocks or finishes, each thread in effect **runs until it
blocks**, and siblings go in binding order. Turn's scheduler is "non-preemptive
within a turn" for the same reason (Turn §9.5).

| Rule | Thread `tid` with expression… | Transition |
|---|---|---|
| **M-PURE** | `E[r]` with `r ⟶ r′` | the expression becomes `E[r′]` |
| **M-REMAINING** | `E[(remaining)]` in scope `σ` | `E[Resources{cost = available(σ), time = deadline*(σ) − now}]`: the **live shared value** (decision Q-A) |
| **M-BUDGET** | `E[(budget ([cost c] [time t]) e)]` in scope `σ` | create child `σ′ = ⟨c, 0, 0, now + t, σ⟩`; the expression becomes `E[SCOPE(σ′, e)]` |
| **M-ASK-EXPIRED** | `E[(ask μ τ ctx s)]`, `now ≥ deadline*(σ)` | `E[(fail PastDeadline)]`: no request, no charge |
| **M-ASK-REFUSED** | …, `ĉ > available(σ)` | `E[(fail OverBudget)]`: no request, no charge |
| **M-ASK-ISSUE** | …, `ĉ ≤ available(σ)`, and the oracle chooses `⟨j, n, δ⟩` or `error(m)` | `reserved += ĉ` on the chain; add request `rid` with `due = min(now + δ, deadline*(σ))`; status becomes `blocked(rid)` |
| **M-CALL-…** | `E[(call κ op v…)]` | the same three cases, with `ĉ = cost_κ(op)` and the host's reply |
| **M-WF-SHARED** | `E[(workflow bs e)]`, ownership violated | `E[(fail (Raised 'shared-stateful-capability))]` |
| **M-WF-START** | `E[(workflow bs e)]`, ownership holds | create `w`; spawn a thread for each node with no deps, in binding order, at path `p.x`, base scope = current scope; parent becomes `E[WAIT(w)]` with status `waiting(w)` |
| **M-NODE-DONE** | a node thread of `w` for `x` reaches a value `v` | mark `x` `done(v)`; spawn, in binding order, every pending node whose deps are now all done, substituting the dependency values into its expression |
| **M-WF-DONE** | every node of `w` is done | parent becomes `E[e_body[x₁ ↦ v₁ … xₙ ↦ vₙ]]`, runnable |
| **M-NODE-FAIL** | a node thread of `w` reaches `(fail φ)` | **cancel** every other running thread of `w`, and their descendants. Each cancelled in-flight request moves its `ĉ` from `reserved` to `spent` (charged the reservation, as in ASKLATE). Parent becomes `E[(fail φ)]`, runnable. |
| **M-SCOPE-EXIT** | `E[SCOPE(σ′, v)]` or `E[SCOPE(σ′, fail φ)]` | pure step (§3); `σ′` is closed, and its spending is already on its ancestors |

### 4.2 The timed transition (the clock jumps)

**M-TIME.** When **no thread is runnable** and `P` is non-empty:

1. Set `now := min { due(r) : r ∈ P }`.
2. Complete every request with that `due`, **in issue order** (`rid` order).
   For each one, with reservation `ĉ` on scope `σ`:

| Outcome | Money | Thread resumes with |
|---|---|---|
| reply arrives in time; `validate(j, τ) = v` | `reserved −= ĉ`, `spent += price(|ctx|, n)` on the chain | `v` |
| reply arrives in time; validation fails | same charge | `(fail (Invalid j))` |
| `due` was cut to the deadline (`now + δ > deadline*`) | `reserved −= ĉ`, `spent += ĉ` | `(fail PastDeadline)` |
| provider or host error | `reserved −= ĉ`, `spent += price(|ctx|, 0)` | `(fail (ToolError m))` |

Each completion also appends one trace entry.

**Final states.** The main thread is done with `v` or `fail φ`, and `T`, `P`
are empty. If no thread is runnable, `P` is empty, and threads are still
waiting, the machine is stuck. That can't happen for well-formed programs,
since workflow dependency graphs are acyclic.

---

## 5. What is now determined: the tight-budget race

This is the case big-step couldn't express. The pool is **$1.00**. Nodes `a`
and `b` each reserve **$0.60**.

**Scenario 1: both issue at t = 0.**

| t | Event | available | Outcome |
|---|---|---|---|
| 0 | `a` runs first (binding order): reserve $0.60 | $0.40 | `a` blocked |
| 0 | `b` runs: $0.60 > $0.40 | $0.40 | **M-ASK-REFUSED**: `b ⟶ (fail OverBudget)` |
| 0 | **M-NODE-FAIL**: cancel `a`, charge its $0.60 reservation | $0.40 | workflow fails `OverBudget` |

**Scenario 2: `b` first computes for one earlier ask of 6 s, so it issues its
big ask at t = 6.** `a`'s reply arrives at t = 5 and costs $0.10.

| t | Event | available |
|---|---|---|
| 0 | `a` reserves $0.60 | $0.40 |
| 5 | **M-TIME**: `a` completes; reservation released, $0.10 spent | $0.90 |
| 6 | `b` reserves $0.60 | $0.30 |
| … | both succeed | |

The same program with different *latencies* gets a different outcome. The
machine now says exactly which, and in scripted mode (latencies from the
script, ties broken by path) it's **deterministic**.

**A design question this exposes, answered conservatively for now.**
Scenario 1 charges `a` its full $0.60 when it's cancelled, even though the
provider might have billed less. The semantics has to charge *something* that
is guaranteed not to exceed what the provider bills. The reservation is the
only such number known in advance. A live implementation may later **refund
down** to provider-reported usage. Because the actual charge is always ≤ the
reservation, a refund can never break Theorem 1.

---

## 6. Theorems (to be proved; the first is now short)

**Theorem 1′ (budget invariant, small-step).** In every reachable machine
state, for every scope `σ`: `spent(σ) + reserved(σ) ≤ limit(σ)`. Moreover
`spent(root) ≤ B`.

*Proof sketch.* The proof is by induction on the length of the run, with one
case per transition that touches money.

- **M-ASK-ISSUE** adds `ĉ ≤ available(σ)` to `reserved` on the chain.
  `available` is a minimum over the chain, so no scope on the chain exceeds its
  limit.
- **M-TIME** replaces `ĉ` in `reserved` by an amount `≤ ĉ` in `spent`.
- **M-NODE-FAIL** replaces `ĉ` by exactly `ĉ`.
- No other transition touches money.

This is much shorter than the big-step argument, because interleaving is
built in rather than argued around.

*Checked at run time.* The invariant is also a representation invariant in
Lesson 6's sense, so the interpreter checks it: after every `reserve` and
`settle`, the machine checks the scopes on the chain it touched, and a
violation is a run-time error (`budget_invariant` in `src/machine.rs`). Every
scripted test therefore checks Theorem 1′ in every state it reaches. On a
live oracle the theorem rests on one assumption, that the provider never bills
more than the reservation (`09` §3). If that ever fails, the trace records it
as `over-reservation` and the check stops, because the theorem's premise no
longer holds.

**Theorem 3′ (deadline).** No request completes successfully after
`deadline*` of its scope: `due` is cut at `deadline*`, and cut requests fail.

**Theorem 5 (adequacy).** The small-step machine agrees with the big-step rules
wherever the big-step rules are exact:

- (a) For programs without `workflow`, the machine's final result and world
  equal big-step `⇓`.
- (b) For a workflow in which no reservation is refused and no node evaluates
  `(remaining)`, the machine's value, total spend and finishing time equal the
  big-step WORKFLOW rule's.

So `04` stays a correct and simpler specification in the common case, and this
machine settles the rest.

**Theorem 6 (scripted determinism).** Given a scripted oracle and hosts, the
machine has exactly one run. At every state at most one instant transition
applies (least-path policy), and M-TIME is deterministic (issue order).

---

## 7. Consequences and limitations

- **A pure infinite loop never lets time pass.** Pure steps take zero virtual
  time, so a thread looping without `ask` or `call` never yields, the clock
  never advances, and no deadline can stop it. This **corrects a claim in
  `02` §C1**. A budget bounds every loop *that performs effects*, since each
  `ask` costs at least some ε and each `call` takes at least some δ, but it
  doesn't bound pure divergence. That's the same as in every language Ramsey
  presents. The interpreter will impose a step limit as a **checked run-time
  error**. It lives outside the semantics, like Impcore's arithmetic-overflow
  check.
- **MCPP runs on this machine.** MCPP's state `s = (S, b, h)` is a projection of
  `M`: `S` is the done nodes of `Ω`, `b = available(σ)` and
  `h = deadline*(σ) − now`. Its decision epochs are the M-TIME jumps. A future
  MCPP-style scheduler is a *different scheduler policy* over the same
  machine. The value-level laws (`06` W1) still hold, while the timing and
  spending change. That's MCPP's "workflow vs. execution policy" separation,
  made precise.
- **RSTD's instrumentation points are the machine's events.** Every issue and
  completion is a subtask boundary. The trace is simply the log of M-ASK-ISSUE
  and M-TIME events.
- **Cancellation is a design commitment.** Fail-fast (`02`) plus conservative
  cancellation charging (§5) are now precise rules, M-NODE-FAIL, that can be
  tested.

---

## 8. Changes to earlier documents

- `04` §6.5: the "What it approximates" paragraph under WORKFLOW now defers to
  this machine.
- `02` §C1: the termination claim is limited to loops that perform effects (§7
  above).
- `05`: the scripted-mode conventions 2–3 (per-site order; ties broken by
  binding order) are **Theorem 6's scheduler policy**, now stated
  formally.
