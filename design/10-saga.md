# What SAGA asks of µNorman

[SAGA](00-reading-notes.md#7-du-et-al-accelerating-scientific-discovery-with-autonomous-goal-evolving-agents-saga-arxiv-251221782-2026)
(Du et al., 2026) is a bi-level agent: an inner loop optimizes candidates
against objectives, and an outer loop evolves the objectives themselves. Its
architecture is a *program*, so the cheapest way to test µNorman against it was
to write it.

That's [`../examples/saga-loop.nrm`](../examples/saga-loop.nrm). It runs, and
its seven tests pass, with **no change to the language**. What it couldn't
express is the useful part of the result.

> Ramsey's Step 2 in one line: write the example first, and let it tell you
> which forms you actually need. Three of the four things below were invisible
> until the program was written.

---

## 1. What already works

SAGA's outer loop needed nothing new:

| SAGA | µNorman | |
|---|---|---|
| Objective: candidate-wise, population-wise, or filter | a datatype with two shapes of constructor | `Weighted` carries a direction and a weight; `Filter` carries neither |
| Planner proposes objectives | `(ask m (List Objective 4) ctx 'planner)` | a bounded list of records, typed and validated |
| Analyzer decides whether to continue | `(ask m Progress ctx 'analyzer)`, then `case` | `Revise` carries the next objectives, `Done` doesn't |
| Planner reads the analyzer's report | the report is `Text`, passed to the next context | SAGA's own interface between the two is prose, so nothing is lost |
| The loop terminates | fuel for the Peano bound, `budget` for the real one | the test shows a penny budget stopping it before anything is spent |
| Selector ranks across every iteration | an archive threaded through the loop, and a fold | retrospective selection needs no new form |

So the claim in the guide — that a choice from a model becomes case analysis in
the program — holds one level up. The same form that drives CP-Agent's inner
loop drives SAGA's outer one. That is a genuine point in the design's favour,
and it was free.

---

## 2. What it couldn't express

### 2.1 `call` has no result type

**The asymmetry is visible in the abstract syntax** (`04` §3):

```
| ASK      (Site site, Exp model, Type ty, Exp context)
| CALL     (Exp cap, Name op, Explist actuals)
```

A tool result is always `Text` (`CALLOK`, and `Completion::ToolResult` in the
machine). Every SAGA scorer returns a number, so a scorer cannot be a tool.
In the example, scoring is an `ask` instead — which is wrong, and the header
says so.

This is the mission statement's own first row, unapplied. "Every `ask` names
the type of answer it expects" should be "every effect names the type of answer
it expects." The existing examples never stressed it because their tools return
text for a model to read; a tool whose answer the *program* consumes shows it
immediately.

**Proposal: the grant declares the operation's type.**

```scheme
(grant py  (kernel  [exec (Text 4000)] [fork kernel]))
(grant box (sandbox [define scorer] [run Num]))
```

The call site is unchanged — `(call py exec code)` still — but its result is
validated against the declared type, exactly as a reply is validated against an
ask's type. Why at the grant and not the call site:

- **The grant is already the host–program contract.** A model grant declares
  prices, ceiling and thinking allowance there. An operation's result type is
  the same kind of fact.
- **One declaration, many call sites.** A tool's interface shouldn't be
  restated at every use.
- **It's Lesson 6.** Outside its module an abstract type has no forms of data,
  only an interface. The grant is where that interface is written down.

What it costs: `validate_Θ` already exists; `CALLOK` gains one premise, the
same one `ASKOK` has. `Invalid` becomes reachable from `call`, and it is
already a failure form. Reservations don't change, because `cost_κ(op) = 0`
and hosts report their own latency, so no output bound is needed.

What it buys beyond SAGA: tool errors stop being stringly-typed, and the
trace gains the result type at each call site.

### 2.2 There is no way to consult a human

SAGA's three autonomy levels — co-pilot, semi-pilot, autopilot — differ *only*
in which module's output a person approves. µNorman has no form for this at
all, so the example is autopilot by necessity, not by choice.

This is the one genuine language gap. Everything else in SAGA is derivable, and
the project's own rule (`01`) is that a new construct must not be derivable.
Consulting a human isn't: a program has exactly two doors to the outside,
`ask` and `call`, and neither reaches a person.

**Proposal: one new effect.**

```scheme
(consult (Text 200) "Which objective should I drop?" 'triage)
```

It behaves like `ask` in every way that matters, which is what makes it cheap:

- It names the **type** of the answer it expects, and the answer is validated.
- It is subject to the **deadline** and recorded in the **trace**, at a named
  site.
- In **scripted mode it reads from the script**, so the three autonomy levels
  become ordinary deterministic tests.
- It costs **time but not money**. A person's time is the resource; the clock
  already models it.
- A refusal to answer is a new failure, `Declined`, and `retry` must not retry
  it — the same argument as `Refused` in `09` (decision B). Asking a second
  time usually gets the same answer and always annoys.

**The autonomy level itself needs no form.** I considered an `autonomy` scope
in the shape of `budget`, and rejected it: the level is ordinary data, and
`(if (reviewed? level) (consult …) fallback)` expresses all three of SAGA's
modes as library code. A scope would be a construct that earns nothing.

### 2.3 Model-authored tools, which turn out not to be a problem

SAGA's implementer *writes new scoring functions during the run*. The mission
table says "tools are capabilities; the model can never create one." These look
like a contradiction. They aren't, and the reason is worth writing down.

The model never produces a capability. It produces **text**. A capability the
host granted turns that text into another capability:

```scheme
(grant box (sandbox [define scorer] [run Num]))
(val s (call box define code))        ; `code` is Text, from a model
(call s run candidate)                ; `s` is a capability, from the host
```

`define` is an operation on a capability you already hold — exactly what `fork`
is for a kernel. Authority flows host → `box` → `s`, never through the model.
Theorem 2 is untouched, and `reach` follows `s` back to `box`, so the ownership
and concurrency rules apply to derived capabilities unchanged.

Two conditions make this safe, and both should be stated when it's specified:

1. **A derived capability is never more authoritative than its parent.** A
   sandbox mints sandboxed scorers; a kernel forks kernels. Neither mints a
   filesystem.
2. **A derived stateful capability shares its parent's owner,** unless the
   operation's whole purpose is to produce an independent one — which is what
   `fork` does and `define` does not.

This needs **no new form**: it is §2.1 (so a grant can declare that an
operation returns a capability) plus a host that offers `define`. Note also
that capabilities are *not askable*, so a tool result may be a capability and a
model answer may never be. That asymmetry is what makes the whole arrangement
safe, and it falls out of rules that already exist.

### 2.4 Two smaller things, already known

- **No value-to-text.** The objectives can't be rendered into the next
  iteration's prompt. The example gets away with it because SAGA's
  analyzer-to-planner interface really is a prose report, so `Text` flows
  through — but a faithful implementation needs `show`, with the round-trip law
  `validate(show v, τ) = v`.
- **`Num` is integral,** so the example carries weights as percentages and
  scores as thousandths. SAGA's scores are reals in [0, 1]. Nothing breaks;
  it's a conversion the standard library should own rather than every program.

---

## 3. What not to take

- **The domain content.** Molecular scorers, DFT, structure prediction: nothing
  for a language here.
- **Pareto fronts, clustering, tournament selection.** Ordinary library code.
  They would be µNorman functions with laws, not forms.
- **Docker and MCP isolation.** How a host implements a sandbox, not what the
  language says about one.
- **Machinery for many outer iterations.** SAGA's own ablation (§S2.3.1) says
  that for antibiotics the objectives proposed after iteration 1 don't visibly
  improve the result, and that the three autonomy modes perform about the same.
  Most of that task's gain comes from writing the objectives down at all. Other
  tasks move more across iterations, so this is a caution rather than a verdict
  — but it argues against optimizing the design for deep outer loops.

---

## 4. Recommendation

In order, and with the honest caveat that **none of this outranks what was
already next**: one real API call ([`09`](09-live-oracle.md) step 4), a real
tool host, and the type and effect system.

| | Work | Size | Why |
|---|---|---|---|
| 1 | **Typed grants** (§2.1) | small | Removes an asymmetry already visible in the abstract syntax, and makes §2.3 free. The validator exists; the rule gains one premise. |
| 2 | **`consult`** (§2.2) | medium | The only real gap. Needs its own design pass first: forms, rules, laws, and a decision on whether `Declined` is retryable. |
| 3 | **`show`** (§2.4) | small | Already on the list for other reasons; SAGA makes the case sharper. |
| — | Everything else | — | Library, host, or not ours. |

A note on sequencing: §2.1 and §2.2 both add premises about types to effect
rules. If the type and effect system is coming anyway, both are cheaper to do
*before* it than after, because each one is a rule the checker will have to
know about regardless.

---

## 5. Open questions

- **Q-C: is a `consult` charged money?** Argued above as time-only. But a
  program that consults a person fifty times is expensive in a way the budget
  can't see, and MCPP's whole point is that the resource model should match
  reality. An alternative is a per-consult price on the grant, like a model's.
- **Q-D: what does `consult` do with no human attached?** Options: fail
  `Declined` at once; block until the deadline; or require that the host grant
  a human the way it grants a model, so a program with no such grant can't
  typecheck. The third is the most µNorman-ish and the most work.
- **Q-E: should a derived capability be revocable?** SAGA's scorers are
  rebuilt every iteration; the old ones become garbage. Nothing in the
  semantics takes a capability away, and a long run accumulates them.
