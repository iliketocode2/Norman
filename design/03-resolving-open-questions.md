# Resolving the four open questions from `02`

Sources: the control-theory paper (Eslami & Yu, reading notes §5) and RSTD
(Asthana et al., §6), read alongside MCPP (§4) and CP-Agent (§3). Question 4
also follows your recommendation.

Summary:

| # | Question | Resolution |
|---|---|---|
| Q1 | Maximum output length: per model or per `ask`? | **Neither: per type.** The bound comes from `τ`, and the model only sets a ceiling. |
| Q2 | Stateful capabilities in a `workflow`: reject or serialize? | **Reject, never serialize.** An ownership check now; effect types later. |
| Q3 | Scripted oracle with tokens and latency? | **Yes, with three changes:** keyed by *site*, not global order; fault injection; a virtual clock. |
| Q4 | Absolute or relative deadline? | **Absolute: one clock shared by every concurrent branch** (your recommendation). |

---

## Q1. The output bound comes from the type

### The problem with both original options

- **Per model** is too coarse. Reserving a model's full ceiling (say 32k
  tokens) for an `ask` whose answer is `Yes | No` makes the reservation, and
  so the whole budget check, needlessly pessimistic. RSTD's main point is that
  good LLM calls are *narrowly scoped judgment operators* with small, typed
  outputs. Per-model bounds throw that information away.
- **Per ask** as a separate annotation is redundant. The `ask` already carries
  a precise description of its output: the type `τ`.

### Resolution

```
max_out(m, τ)  =  min( ceiling(m), bound(τ) )
```

`bound(τ)` is computed from the shape of the askable type. This is Ramsey's
idea again, applied to resources: *the shape of the data determines the cost*.

| Askable type | `bound(τ)` |
|---|---|
| `Bool`, `Sym`, a datatype of nullary constructors (`Yes`/`No`) | small constant (longest encoding) |
| `Num` | constant |
| `(Text n)`, text of at most `n` bytes | `n` + quoting overhead |
| `Text` with no bound | `ceiling(m)` (unbounded, so the model's limit applies) |
| record `{f₁: τ₁, …}` | `Σ bound(τᵢ)` + key/brace overhead |
| datatype `K₁ τ… \| K₂ τ…` | max over constructors |
| `(List τ n)`, at most `n` elements | `n · bound(τ)` + overhead |
| `(List τ)` with no bound | `ceiling(m)` |

**Soundness:** bounds are counted in *bytes of JSON*. Every token in a
byte-level BPE vocabulary covers at least one byte, so a byte bound is always a
valid token bound. It's conservative, but it's never wrong.

**What it's worth:**

1. **Tight reservations.** CP-Agent's `Step` (code text) still reserves a lot,
   but a verdict, a classification or a yes/no reserves almost nothing. Those
   are exactly the calls RSTD recommends.
2. **Per-ask latency bounds.** Latency is roughly time-to-first-token plus
   output length divided by throughput. The control paper shows that total
   delay is critical to stability (Prop. 1, eq. 43). A bounded output gives a
   bounded delay estimate for each decision point, which a deadline-aware
   scheduler (MCPP) can use.
3. **No new syntax.** The only addition is optional size indices on `Text` and
   `List`, which are type information anyway.

If the model exceeds the bound, the provider truncates the output, the JSON
fails to validate, and the result is `fail (Invalid raw)`. Nothing new is
needed.

---

## Q2. Stateful capabilities in a `workflow`: reject, never serialize

### Why serializing is wrong

Serialization (a lock around the Python kernel) prevents *data races* but not
*race conditions*. The order in which two nodes use the kernel would still
depend on which model replied first. The program's meaning would then depend
on timing, which breaks the defining law of `workflow`:

```
(workflow bindings body) == (let* (topo-sort bindings) body)   ; provided effects are disjoint
```

Two further arguments:

- RSTD insists that orchestration and state management be *deterministic*,
  *auditable* and *reproducible*.
- In the control paper's terms, a branch that reads a kernel another branch is
  mutating acts on stale information: a decision-induced delay nobody
  accounted for.

### Resolution: stateful capabilities are *owned*

Each capability has a **kind**, declared by the host that grants it:

| Kind | Examples | Rule inside one `workflow` |
|---|---|---|
| **stateless** | a model, an HTTP GET client, a clock | may be used by any number of nodes |
| **stateful** | a Python kernel, a filesystem writer, a git worktree | may be *reachable* from **at most one** node |

The check runs **once, when the `workflow` starts, before any node runs**. For
each binding, it collects the stateful capabilities reachable from the values
of that binding's free variables, following closure environments. If two
bindings overlap, it fails with `(Raised 'shared-stateful-capability)`. The
check is deterministic, because it can't depend on scheduling, and it's cheap.

Later, the effect system (Lesson 5) does the same check *statically*: a
stateful capability behaves like an affine resource, as in Rust ownership. The
runtime check is the executable specification the static check must agree
with.

Two supporting rules:

- **To get concurrency on stateful tools, fork them.** `(call py fork)`
  returns a *new* independent kernel capability, so two nodes can each own
  one. This matches Turn's per-process isolation, but made explicit.
- **For now, capabilities may be passed as arguments and captured by closures,
  but not stored in records or lists.** That keeps the reachability check
  simple and complete until types can track capabilities inside data.

Sequential use is unaffected. The `workflow` body runs after every node has
finished, so it may use any capability again.

---

## Q3. The scripted oracle: yes, with three changes

### Change 1: key the script by ask *site*, not global order

A single ordered list of replies is **ill-defined under concurrency**. If two
`workflow` nodes both `ask`, which one gets the first reply? It depends on
scheduling. So scripts are keyed by **site**:

```scheme
(script
  [analyst      (reply "{\"tag\":\"Sell\",\"reason\":\"…\"}") (out 40) (latency 2s)]
  [risk-officer (reply "{\"tag\":\"Hold\",\"reason\":\"…\"}") (out 35) (latency 5s)]
  [react        (reply "{\"tag\":\"Exec\",\"code\":\"…\"}")   (out 90) (latency 3s)
                (reply "{\"tag\":\"Done\",\"code\":\"…\"}")   (out 60) (latency 2s)])
```

Each site has its own sequence of replies, consumed in order. Order within a
site is deterministic; order across sites doesn't matter.

**This means sites need names.** Every `ask` gets a site identifier:

- by default, its **source location**;
- inside a `workflow`, the **binding name** of the node it belongs to;
- optionally, an explicit label for stability across edits.

The same site names key the trace. That gives RSTD's "subtask-level
monitoring" and MCPP's per-`(subtask, model)` profiles `ω_{v,m}` directly.

### Change 2: input tokens are computed, not scripted

Context is explicit, so input size is known exactly when the `ask` runs:
`|ctx|`, tokenized. A script only supplies what can't be computed: the reply,
the output tokens and the latency.

### Change 3: fault injection

RSTD had to *simulate* failures because natural failure rates were 0–2%. A test
fixture that can't inject faults can't test `retry`, `catch` or fallback. So a
script entry can also be a fault:

| Entry | Effect in the evaluator |
|---|---|
| `(reply j) (out n) (latency t)` | normal reply; then validated against `τ` |
| `(reply "not json") …` | reply that fails validation: `fail (Invalid …)` |
| `(provider-error "503")` | `fail (ToolError "503")` |
| `(latency 10min)` beyond the deadline | `fail (PastDeadline)`, charged the reservation |

Tool calls (`call`) get scripts on the same pattern (result, cost, latency,
or fault).

### A consequence: the scripted evaluator *is* a simulator

With a **virtual clock** (Q4) advanced by scripted latencies, running a program
against scripts computes exact cost and completion time with no real calls.
If the scripts are *sampled* from profiles taken from real traces, the same
evaluator becomes MCPP's Monte Carlo simulator. The test harness and a future
planner's simulator are the same machinery.

---

## Q4. One absolute clock shared by every concurrent branch

**Decision:** a `budget` scope carries an **absolute deadline**, a point in
time. Every branch inside it, however deeply nested in `workflow`s, checks
against the same instant.

Why this is right:

- **Time passes; it isn't consumed.** Cost is a pool that branches draw from.
  Time is the same for everyone. A relative remainder `h` per branch would
  have to be decremented consistently across concurrent branches, which
  amounts to re-deriving a shared clock the hard way.
- **The control paper requires it.** Delays from all sources add into one
  total (eq. 43). When decision authority is spread across agents, a coupled
  budget can only be kept by a **shared** budget or a supervisor (Remark 5).
  One clock per `budget` scope is that supervisor for time.
- **It makes work/span fall out.** With one clock, the time of a `workflow` is
  simply when its last node finishes. That's the critical path, with no extra
  accounting.

Rules:

```
deadline(budget ([time t]) e)  =  min( deadline(parent), now + t )
(remaining).time               =  deadline − now          ; the observer from 02 §C4
```

In scripted mode, `now` is the **virtual clock**. The evaluator advances it by
scripted latencies, and concurrent branches advance it as a discrete-event
simulation, so deadline tests are deterministic.

---

## What this means for Step 4 (the contract)

With these four settled, the evaluation judgment has everything it needs:

```
⟨ e, ρ, W ⟩  ⇓_site  ⟨ r, W′, t ⟩
```

- `e`: expression; `ρ`: environment
- `W = (pool, deadline, clock, capability-states, oracle)`: the world
  - `pool`: remaining µ$, shared and reserved atomically
  - `deadline`: an absolute instant, shared by all branches in scope
  - `clock`: real or virtual
  - `oracle`: live, or scripted by site
- `r ::= v | fail φ` with `φ : Failure`
- `t`: trace, a sequence of `(site, model, in, out, latency, outcome)` entries

Step 4 turns this into the evaluator's contract, in Ramsey's style: a simple,
clear sentence, then the judgment, then what each part guarantees. Step 5 then
writes example results against site-keyed scripts, including injected faults
and deadline cases.
