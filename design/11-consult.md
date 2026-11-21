# `consult`: asking a person

> **Status: built** (all of §9). Scripts answer consults in tests; in live
> mode a person answers at the terminal, and a run refuses to start while any
> human grant is unbound (§4.1; `08`, deviation 11). [`../examples/saga-loop.nrm`](../examples/saga-loop.nrm)
> runs SAGA at all three autonomy levels. The rules are now in `04` §6.5 and `07` §4, which are the
> reference; this document keeps the argument, as `10` does for typed grants.
> [`../examples/consult.nrm`](../examples/consult.nrm) tests every rule of
> §2.2, and [`../tests/properties.rs`](../tests/properties.rs) checks K1–K4 on
> random cases.

[`10`](10-saga.md) §2.2 found the one real gap in the language: a program has
exactly two doors to the outside, `ask` and `call`, and neither reaches a
person. SAGA's three autonomy levels differ only in which module's output a
person approves, so without a third door the example could only be written
as autopilot.

This document designs that door. It follows `10`'s proposal closely: `consult`
behaves like `ask` in every way that matters. It also settles two of `10`'s
open questions (Q-C and Q-D) and adds one idea that wasn't in the proposal:
**what a person approves is what runs** (§3).

It comes before the type and effect checker on purpose. `consult` adds
premises about types to an effect rule, and the checker will need a rule for
it either way. It's cheaper to settle the rule first than to retrofit the
checker afterwards (`10` §4).

---

## 0. Decisions (please review)

| # | Question | Decision |
|---|---|---|
| H1 | Who is consulted? | **A granted human**, `(grant reviewer (human))`, passed to `consult` the way a model is passed to `ask`. Humans come from the host, never from the program, as models and capabilities do. |
| H2 | Q-C: is a consult charged money? | **Time by default. Money only if the grant states a price,** `(human [price $2.00])`. The price is reserved before the person is asked, like any other charge, so Theorem 1 is unchanged. A flat price per consult can't express hourly pay, so Q-C is settled for now, not for good (§10, Q-I). |
| H3 | Q-D: what if no human is attached? | **The program doesn't start.** In live mode, the operator must bind every human grant on the command line: to a terminal, or to `unattended`, under which any consult is a **checked run-time error**. There is no policy that approves everything, and none that pretends to be a person saying no. In scripted mode, the script answers. |
| H4 | Is a refusal retryable? | **No.** A refusal is a new failure, `Declined (reason Text)`. `retry`, `repair` and `attempt` don't recover it, by the same argument as `Refused` (`09`, decision B). A program that *revises* its proposal and consults again is making a new request, which is fine. |
| H5 | Is a human stateful? | **Not for ownership, but not stateless either.** Concurrent consults of one person are allowed. But a person's answer can depend on what they were shown before, so a workflow in which two nodes consult the same person falls outside Theorem 4, as one that reads `(remaining)` does (§2.4). Every consult should be **self-contained** (§2.5). |
| H6 | How does approval bind to the operation? | **An optional proposal, `[propose v]`, is shown and pre-filled as the answer.** Approving it unchanged returns exactly `v`, by the `show` round-trip law S1. The program runs the answer, not its own copy (§3). |
| H7 | Does waiting for a person count against the deadline? | **Yes.** The clock is the clock. A consult inside a one-minute scope will almost always be cut, and that's correct: the scope promised to finish in a minute. **This has a cost:** since `budget` only shrinks deadlines, a scope containing a consult needs a deadline of hours, and a model call inside it keeps a tight time limit only if it has its own inner `budget` (§6). Q-G is the real answer. |
| H8 | Is the autonomy level a form? | **No** (as `10` §2.2 already argued). It's ordinary data, and `case` on it is library code (§6). |

---

## 1. Syntax

### 1.1 Concrete

```
host-spec ::= … | (human {[price money]})        ; a literal, like a model grant's prices

exp       ::= … | (consult exp ty exp [propose exp] [quoted-symbol])
                  ; human, answer type, prompt, optional proposal, optional site
```

`consult` becomes a **reserved word**. `propose` and `price` don't: like `cost`
and `time`, they occur only in fixed bracket positions.

```scheme
(grant reviewer (human))

(consult reviewer (Text 200) "Which objective should I drop?" 'triage)

(consult reviewer (List Objective 4)
         "Approve, edit or decline the objectives for the next round."
         [propose objectives]
         'review-objectives)
```

### 1.2 Abstract

```
Exp* = … | CONSULT (Site site, Exp human, Type ty, Exp prompt, Exp? proposal)
```

Seventeen expression forms instead of sixteen. Free variables:

```
fv(CONSULT(s, e_h, τ, e_p, None))    = fv(e_h) ∪ fv(e_p)
fv(CONSULT(s, e_h, τ, e_p, Some e_v)) = fv(e_h) ∪ fv(e_p) ∪ fv(e_v)
```

### 1.3 Well-formedness

Rule 5 of `04` §3.1 extends to `consult`: **`τ` must be askable.** That's what
keeps Theorem 2 true. Whatever a person types, the answer is validated against
an askable type, so it can't contain a capability, a model or a human.

### 1.4 Values and the initial basis

- A new value form, `HUMAN(η)`, made only by a `human` grant. Like a model, it
  isn't stateful (H5): `reach(HUMAN(η)) = ∅`.
- `show` refuses it, as it refuses capabilities: a checked run-time error.
- `Failure` gains one constructor:

```scheme
(datatype Failure [Invalid (raw Text)] [ToolError (message Text)] [Refused (category Sym)]
                  [Declined (reason Text)]
                  [OverBudget] [PastDeadline] [Raised (reason Sym)])
```

`Declined` carries the person's reason, so a program can revise its proposal
and try again, or explain the refusal to a model.

---

## 2. Big-step semantics

### 2.1 The human oracle

A new auxiliary judgment, implemented by the host:

```
H ⊢ ⟨η, s, p, τ, v̂⟩ ⇝ ⟨answer j, δ⟩      the person answers with JSON text j after δ
H ⊢ ⟨η, s, p, τ, v̂⟩ ⇝ ⟨decline r, δ⟩     the person declines, giving reason r, after δ

price_η          the grant's stated price per consult (0 if none)
```

`p` is the prompt text, and `v̂` is `show v` when a proposal was given, or
nothing. What the person is shown is fixed by the host (§4), but it must
include `p`, `v̂` and the shape of `τ`.

Unlike the model oracle, this relation is **not** history-independent: what a
person answers may depend on what they were shown earlier in the run. The
semantics doesn't model that dependence; it only declines to promise anything
that would need its absence (§2.4).

**Conformance.** µNorman has no typing judgment at run time yet, so "`v` is a
`τ`" is defined by the round trip `show` already guarantees:

```
v ⊨ τ   iff   validate_Θ(show v, τ) = v
```

It's undefined (so false) when `v` can't be shown, for instance because it
contains a capability.

### 2.2 Rules

The premises are evaluated in order: human, prompt, then proposal.

```
⟨e_h, Θ, ρ, W⟩ ⇓ ⟨HUMAN(η), W₁⟩
⟨e_p, Θ, ρ, W₁⟩ ⇓ ⟨p, W₂⟩                    p is Text
⟨e_v, Θ, ρ, W₂⟩ ⇓ ⟨v, W₃⟩    v ⊨ τ           (only if a proposal is given; else W₃ = W₂)
W₃.now < W₃.deadline
ĉ = price_η ≤ W₃.pool
W₃.hosts ⊢ ⟨η, s, p, τ, show v⟩ ⇝ ⟨answer j, δ⟩
W₃.now + δ ≤ W₃.deadline
validate_Θ(j, τ) = a
────────────────────────────────────────────────────────────── (CONSULTOK)
⟨CONSULT(s, e_h, τ, e_p, e_v), Θ, ρ, W⟩ ⇓ ⟨a, W₃ ⊕ ⟨ĉ, δ, t⟩⟩
```

The failing variants share the first three premises:

| Rule | Replaced premise | Result | World |
|---|---|---|---|
| CONSULTEXPIRED | `W₃.now ≥ W₃.deadline` | `fail PastDeadline` | `W₃`: **nobody is asked, no charge** |
| CONSULTUNAFFORDABLE | `ĉ > W₃.pool` | `fail OverBudget` | `W₃`: nobody is asked, no charge |
| CONSULTDECLINED | the person `⇝ ⟨decline r, δ⟩` | `fail (Declined r)` | `W₃ ⊕ ⟨ĉ, δ, t⟩`: the person's time was spent, so a priced consult is charged |
| CONSULTLATE | `W₃.now + δ > W₃.deadline` | `fail PastDeadline` | `W₃[now := deadline]` plus a trace entry: **cut, and not charged** (see below) |
| CONSULTINVALID | `validate_Θ(j, τ)` undefined | `fail (Invalid j)` | `W₃ ⊕ ⟨ĉ, δ, t⟩` |

And two checked run-time errors, not failures:

- **A proposal that doesn't conform to `τ`** (`v ⊨ τ` fails). The program
  proposed something it then declared the wrong type for. That's a bug in the
  program, not something the person or the world did. The checker will catch
  it statically; until then it's caught here, before anyone is asked.
- **A consult of a human bound to `unattended`** (§4.1). That's the operator's
  configuration saying no person is available, which the program can't
  recover from and shouldn't try to.

Read these the way `04` reads ASK:

- **The person is asked only after the reservation succeeds,** and only once.
  No rule re-invokes the human oracle.
- **A consult that is cut or cancelled isn't charged,** unlike ASKLATE, which
  charges its reservation. A model's provider bills for the tokens it
  generated before the cut; a reviewer who never answered delivered nothing.
  That holds however the consult ends early: at its own deadline here, or
  because a sibling workflow node failed (§5, M-NODE-FAIL). Charging less than
  the reservation is always safe for Theorem 1.
- **CONSULTINVALID should be rare in live mode,** because the host validates
  the answer in its own form before the person can submit it (§4). The rule
  stays for scripts and for hosts that can't validate. When it does happen,
  `retry` will consult the person again, since `retry` recovers `Invalid`
  without knowing where it came from. That's accepted (K2): telling the two
  apart would mean tagging `Invalid` with its source, which costs more than
  this rare case is worth.

### 2.3 The trace

Each consult appends one entry:

```
⟨site, path, human η, prompt p, proposal show v, answer j | decline r | cut | cancelled, charge, δ, outcome⟩
```

That entry is the **audit record**: exactly what the program showed the
person, and exactly what they said. If a person answers a consult that was
already cut or cancelled, the host discards the answer and appends a second
entry marked `discarded`, so the record shows that someone answered and that
the answer was not used. The trace is part of the world (`04`, decision F4), so
the record is part of the semantics, not a log someone might forget to write.

### 2.4 Workflows

The WORKFLOW rule itself doesn't change. Because `reach(HUMAN(η)) = ∅`, two
nodes may consult the same person concurrently, as two nodes may ask the same
model. If the person can only attend to one prompt at a time, the second waits,
and that wait is part of its `δ`.

**Theorem 4 does change.** It says a workflow's value is the value of every
sequentialization of its nodes, and its proof uses the fact that models are
stateless. A person isn't: running the nodes in a different order changes
what the person sees first, and so possibly what they answer. So Theorem 4
gains a premise, in the same form as the one for `(remaining)`. It is
numbered (iv) because law W1 in `06` already lists three:

```
(iv) no two nodes consult the same human
```

A workflow that breaks (iv) is still allowed and still runs. It just falls
outside the law, so a refactoring tool may not reorder it. W4 and W5, which
inherit W1's premises, need it too, and so does `07`'s Theorem 5(b).

> **Correction to two earlier drafts.** The first draft made humans stateful,
> so that ownership would forbid two concurrent consults of one person, on the
> grounds that a person remembers. That proves too much: ownership forbids
> only *concurrent* use, and two *sequential* consults depend on memory just
> as much. The second draft concluded that a person's memory could simply be
> folded into the oracle relation, "so Theorem 4 holds for humans for the same
> reason it holds for models." That was wrong too. For a model, statelessness
> is literally true, since each request is independent. For a person it's
> false, and the §7 non-law about reordering consults contradicted it. The fix
> is the premise above: no ownership, and no promise about reordering either.

### 2.5 Every consult is self-contained

A person remembers only so much, and not for long. A consult that relies on
the person recalling an earlier one ("the plan from before, but cheaper") will
be answered from a memory the program can't see and the trace can't record.

So the host shows **only** the prompt and the proposal, and a program should
put everything the person needs into them. That's decision D1, explicit
context, applied to people: a model sees exactly the context it's given, and
so should a person. It also makes the trace entry (§2.3) a complete record of
everything the *program* gave the person to go on. It can't be a record of
everything the answer was based on, since the person brings their own memory
and judgment, and nothing can record those.

The semantics can't enforce this, since it can't see what a prompt *refers*
to. It is a design rule for programs, and the library examples in §6 follow
it: each round of `plan-with-review` shows the full new plan, not a diff.

---

## 3. What is approved is what runs

The risk worth designing against: a program shows a person one thing and then
does another. The person approves "delete the three stale branches," and the
program deletes a list it built separately, which has four.

µNorman can't stop a program from ignoring an answer. But it can make the
safe pattern the natural one, and give it a law:

```scheme
(let* ([plan (consult reviewer (List Step 10) "Approve this plan?" [propose plan₀] 'approve)])
  (run plan))          ; runs the answer, not plan₀
```

The host pre-fills the answer with `show plan₀`. A person who approves it
unchanged submits that text, and by S1 (`06` §5):

```
validate (show v) τ  =  v          for askable τ and v ⊨ τ
```

so the consult returns **exactly** `plan₀`. (The premise `v ⊨ τ` was already
checked before the person was asked, §2.2, so the law always applies.) A person who edits the plan returns
the edited plan, validated against the same type, and the program runs that.
A person who refuses gets `Declined`, and nothing runs. Approving,
editing and refusing are three outcomes of one form, and the first needs no
special case in the semantics.

What this does and doesn't guarantee:

- **It does guarantee** that the value returned by an unchanged approval is
  the proposal that was shown, bit for bit, and that the trace records both.
- **It doesn't cover the prompt.** The prompt is free text, and it can
  contradict the proposal: "Approve deleting the three stale branches" above
  a list of four. The person sees both, so a careful reader catches it, but
  the binding is to the proposal only. A program should keep its prompt
  generic ("Approve this plan?") and let the proposal carry the specifics.
- **It doesn't guarantee** that the program uses the answer. That's what the
  checker can help with later: a consult whose result is never used is almost
  always a mistake, and an effect system can warn about it (§8).

---

## 4. Hosts

### 4.1 Live mode

Every `human` grant must be bound on the command line, or the run refuses to
start (H3). It's a checked error before anything runs, so no money is spent:

```
cargo run -- --live --human reviewer=tty        examples/saga-loop.nrm
cargo run -- --live --human reviewer=unattended examples/saga-loop.nrm
```

- **`tty`** shows the site, the prompt, the proposal and the expected shape of
  the answer in the terminal, and reads a reply. It validates the reply
  against `τ` before accepting it, and asks the person again if it doesn't
  fit; that's UI, not a second consult. The person can approve the proposal
  as is, edit it, or decline with a reason.
- **`unattended`** makes any consult a **checked run-time error**: the run
  stops at the first point it needed a person, and no handler can intercept
  it. That's what makes a headless run fail loudly.

> **Correction to an earlier draft.** The headless policy was first called
> `decline`, and answered every consult with `Declined "no human attached"`.
> But `Declined` can be caught, and §6's own `plan-with-review` catches it on
> purpose: under that policy it would feed "no human attached" to the model
> as feedback and re-plan *n* times, spending real money arguing with a
> configuration setting. A missing person is not a person saying no, so it
> mustn't produce the same failure.

Every human grant must be bound even when the run never consults anyone, as
an autopilot run doesn't. Bind it to `unattended`: if the program does reach a
consult after all, the run stops rather than guessing.

There is deliberately **no `approve` policy**. Silence never means approval.
A run that should behave as if everything were approved is the autopilot
level, and that's a choice the program makes in its own code (§6), where it's
visible and testable.

**Withdrawing a consult.** When a consult is cut at its deadline or cancelled
by a failing sibling, the host must withdraw the prompt from the person, so
nobody keeps working on a question the program has stopped waiting for. An
answer that arrives anyway is discarded and recorded (§2.3).

> **As built** (`08`, deviation 11). The terminal host takes one line per
> answer: `a` approves, `d REASON` declines, and anything else is the answer,
> as JSON, or as plain text when τ is `Text`. Building it fixed four things
> this section left open. Questions are shown one at a time, in issue order.
> A line read before a question appeared never answers it, so a reply meant
> for one question can't land on the next, and a pipe that types ahead can't
> approve something nobody saw. A terminal whose input ends is treated like
> `unattended`: a checked error, never a decline, and never an endless wait.
> And because the terminal does withdraw a question, nothing typed afterwards
> can answer it, so the `discarded` trace entry is needed only by a host that
> can't withdraw.

Other hosts (a web form, a chat message) fit the same interface later. The
semantics doesn't care how the question reaches the person.

### 4.2 Scripted mode

Consult sites are scripted like ask sites, keyed by site and consumed in
order:

```scheme
(script co-pilot
  [review-objectives (approve) (latency 2min)
                     (answer "[{\"kind\":{\"tag\":\"Filter\"},\"name\":\"toxicity\",\"rationale\":\"Rule out toxic candidates first.\"}]")
                     (latency 5min)
                     (decline "These objectives ignore cost.") (latency 1min)])
```

| Entry | Meaning |
|---|---|
| `(answer j)` | the person submits JSON text `j` |
| `(approve)` | the person submits the proposal unchanged, i.e. `show v`; a **checked error** if the consult had no proposal |
| `(decline r)` | the person declines with reason `r` |
| `(latency d)` | how long the person took |

An unscripted consult is an exhausted script: a checked run-time error, as for
`ask` (`05`, convention 5). So the three autonomy levels become ordinary
deterministic tests.

---

## 5. Small-step machine

`07` gains three instant transitions, alongside M-ASK-…:

| Rule | Thread with expression… | Transition |
|---|---|---|
| **M-CONSULT-EXPIRED** | `E[(consult η τ p v̂ s)]`, `now ≥ deadline*(σ)` | `E[(fail PastDeadline)]` |
| **M-CONSULT-REFUSED** | …, `price_η > available(σ)` | `E[(fail OverBudget)]` |
| **M-CONSULT-ISSUE** | …, `price_η ≤ available(σ)`, the person chooses an outcome after `δ` | `reserved += price_η` on the chain; add request `rid` with `due = min(now + δ, deadline*(σ))`; status `blocked(rid)` |

and three rows in M-TIME's table:

| Outcome | Money | Thread resumes with |
|---|---|---|
| an answer arrives in time and validates to `a` | `reserved −= ĉ`, `spent += ĉ` | `a` |
| a decline arrives in time | `reserved −= ĉ`, `spent += ĉ` | `(fail (Declined r))` |
| `due` was cut to the deadline | `reserved −= ĉ` (**not** spent) | `(fail PastDeadline)` |

An answer that arrives in time but doesn't validate is charged and resumes
with `(fail (Invalid j))`, as for `ask`.

**M-NODE-FAIL changes too.** It currently charges every cancelled in-flight
request its reservation, which is right for `ask` (the provider bills for
tokens already generated) and wrong for a consult (§2.2). It gains a case:

| Cancelled request | Money |
|---|---|
| an `ask` or `call` | `reserved −= ĉ`, `spent += ĉ` (unchanged) |
| a `consult` | `reserved −= ĉ`, **not** spent; the host withdraws the prompt (§4.1) |

In live mode, a consult is one more kind of in-flight request. M-TIME already
blocks until the next completion or the earliest deadline, so a person who
takes an hour simply makes the next completion an hour away.

---

## 6. Autonomy levels as library code

SAGA's three levels, written as data and `case`:

```scheme
(datatype Autonomy [CoPilot] [SemiPilot] [AutoPilot])

;; Co-pilot and semi-pilot both review the planner. Co-pilot also reviews the
;; analyzer's report, which is written the same way with its own predicate.
(define reviews-planner? (level) (case level [AutoPilot #f] [_ #t]))

(define reviewed-objectives (level h objs)
  (if (reviews-planner? level)
      (consult h (List Objective 4)
               "Approve, edit or decline the objectives for the next round."
               [propose objs]
               'review-objectives)
      objs))
```

The tests for all three levels need no live person. Each test starts with
its script rewound (`05`, convention 1), so each outcome gets its own script:

```scheme
(script approves [review-objectives (approve) (latency 2min)])
(script declines [review-objectives (decline "These ignore cost.") (latency 1min)])
(script silent)                                    ; no consult sites at all

(under ([script silent] [cost $1.00] [time 1h])
  (check-expect (reviewed-objectives AutoPilot reviewer objs) objs))  ; nobody is asked

(under ([script approves] [cost $1.00] [time 1h])
  (check-expect (reviewed-objectives CoPilot reviewer objs) objs))    ; approved unchanged: K1

(under ([script declines] [cost $1.00] [time 1h])
  (check-fail (reviewed-objectives CoPilot reviewer objs) (Declined _)))
```

The first test passes only because nobody is asked: under `silent`, any
consult would exhaust the script, which is a checked error.

And refusal-then-revision is ordinary code, not a retry. The scope around
this function needs a deadline long enough for a person (H7), so the model
call gets its own one-minute `budget`. Without it, a slow model would have
the person's hours to use up:

```scheme
(define plan-with-review (h m ctx n)
  (let* ([proposal (budget ([time 1min]) (ask m Plan ctx 'planner))])
    (catch (consult h Plan "Approve this plan?" [propose proposal] 'approve)
           err
           (case err
             [(Declined why)
                (if (<= n 0)
                    (fail err)
                    (plan-with-review h m
                      (append ctx (list (Message [role User] [content why])))
                      (- n 1)))]
             [_ (fail err)]))))
```

Each round asks the model for a *new* plan with the person's reason in its
context, so the person never sees the same request twice. That's the
difference between revising and retrying, and it needs no new form.

---

## 7. Laws

Following `06`. `≡` is exact equivalence and `≡v` equivalence of value only
(`06` §0).

```
(K1) under a script answering (approve):
       (consult h τ p [propose v] s)  ≡  v                       for askable τ, v ⊨ τ   ; binding, from S1
(K2) (retry n (lambda () (fail (Declined r))))   ≡  (fail (Declined r))                  ; spec: never retried
     (attempt (lambda () (fail (Declined r))) c) ≡  (fail (Declined r))
     but (retry n (lambda () (consult …))) does re-consult on Invalid                    ; accepted (§2.2)
(K3) spend(consult h τ p …) ≤ price_h, and = 0 when h's grant has no price                ; resources
(K4) no CONSULTOK or CONSULTDECLINED trace entry finishes after its scope's deadline      ; Theorem 3, extended
```

K1 is the law that §3 is for. It's mutation-testable the way S1 is: break
`show`'s canonical form and K1 fails.

**Non-laws:**

| Tempting "law" | Why it's false |
|---|---|
| **Merge identical consults:** two consults with the same prompt equal one | A person can answer differently the second time, and the right side asks them half as often. Same as `ask` (`06` §5). |
| **Two consults of one person can be reordered** freely | Not even `≡v`: a person's answer may depend on what they saw just before. This is why Theorem 4 needs its premise about humans, W1's (iv) (§2.4). |
| **A proposal approved is a proposal executed** | Only if the program runs the answer. The semantics can't force that; the checker can warn (§8). |

---

## 8. What changes elsewhere

✅ marks a change that has been made. Where this document and those disagree,
those win: they are the reference, and this document keeps the argument.

| | Document | Change |
|---|---|---|
| ✅ | `04` §2, §3, §3.1 | the `human` host-spec, CONSULT in the grammar and abstract syntax, `fv`, the askable rule for `consult` |
| ✅ | `04` §6.2, §6.5 | the human oracle, `price_η` and conformance; CONSULT's rules |
| ✅ | `04` §7.2, §7.3 | `Declined` in `Failure`; `HUMAN(η)` unshowable |
| ✅ | `04` §8 | one new case in each of Theorems 1–3 (below); Theorem 4 gains a premise, no two nodes consult the same human (W1's (iv) in `06`; §2.4) |
| ✅ | `05` | convention 8 for consult sites; a coverage table for the CONSULT rules |
| ✅ | `06` §4, §5, §7 | premise (iv) on W1, W4 and W5; K1–K4 and the non-laws; problems #10 and #11 |
| ✅ | `07` §4, §6 | the three M-CONSULT transitions, the M-TIME rows, the consult case of M-NODE-FAIL; Theorems 1′ and 5(b) |
| ✅ | `08` | the rule-to-code map; deviations 11 and 12 |
| ✅ | `08`, `09` | the live human hosts and `--human`; refusing to start with an unbound grant; withdrawing cut and cancelled consults |
| ✅ | `10` §2.2, §4, §5 | point here; Q-C and Q-D settled |

**The new proof cases are short:**

- *Theorem 1, CONSULTOK, CONSULTDECLINED and CONSULTINVALID.* The charge is `ĉ = price_η ≤ W₃.pool`. Other cases charge nothing.
- *Theorem 2, CONSULTOK.* `a = validate_Θ(j, τ)` with `τ` askable, so `a` contains no `CAP`, `MODEL` or `HUMAN`. A person, like a model, can't hand the program authority.
- *Theorem 3, CONSULTOK and CONSULTDECLINED.* The premise `now + δ ≤ deadline`.

**What the checker inherits.** The point of doing this first: these are rules
the type and effect checker will now have from the start, rather than gain
later.

1. `τ` in a `consult` is askable (already rule 5; now stated for both forms).
2. The human expression has type `Human`, and a proposal has type `τ`.
3. A consult contributes an effect, *consults η*, to the expression's effect
   set. It isn't an ownership effect (H5), but it lets a policy say which
   parts of a program may involve a person at all, and it lets the checker
   **flag a workflow whose nodes consult the same human**, which falls
   outside Theorem 4. That's the same treatment as a node that reads
   `(remaining)`: allowed, but reported.
4. **A warning when a consult's answer is unused**, since an approval that is
   then discarded is almost always the §3 mistake.
5. A program that consults a human it was never granted doesn't typecheck.
   Today that's an unbound variable, found at run time.

---

## 9. Implementation sketch

In the order the code depends on it:

1. **Syntax.** `consult` in the lexer's reserved words; `Exp::Consult` in
   [`../src/ast.rs`](../src/ast.rs) with its free variables; parsing, including
   `[propose e]`, in [`../src/parser.rs`](../src/parser.rs); `(human [price $…])`
   grants.
2. **Values.** `Value::Human`, not stateful for `reach`; refused by `show`
   ([`../src/value.rs`](../src/value.rs), [`../src/types.rs`](../src/types.rs)).
   `Declined` in [`../src/prelude.nrm`](../src/prelude.nrm).
3. **Machine.** The M-CONSULT transitions, the completion rows and the
   uncharged cancellation in [`../src/machine.rs`](../src/machine.rs), sharing
   the request machinery `ask` and `call` already use.
4. **Scripts.** `answer`, `approve` and `decline` entries in the scripted host.
5. **Tests.** `examples/consult.nrm` with a test for every rule in §2.2, the
   three autonomy levels, and K1–K4 in [`../tests/properties.rs`](../tests/properties.rs).
   A deliberately wrong version in [`../tests/must-fail.nrm`](../tests/must-fail.nrm)
   (for instance, a `retry` that retries `Declined`).
6. **Live.** The `tty` and `unattended` hosts, withdrawal of cut and
   cancelled prompts, and the `--human` flag in
   [`../src/main.rs`](../src/main.rs) and [`../src/live.rs`](../src/live.rs).
   Last, because everything before it is testable without a person.
7. **SAGA.** Rewrite [`../examples/saga-loop.nrm`](../examples/saga-loop.nrm)
   with an autonomy level, so all three of SAGA's modes are tested.

---

## 10. Open questions

- **Q-E (from `10`): should a derived capability be revocable?** Unrelated to
  `consult`, and still open. It belongs with the checker, since revocation is
  a question about how long a capability may be used.
- **Q-F: should `Declined`'s reason be bounded?** `Invalid`'s raw text isn't,
  so this draft doesn't bound it either. But the reason often goes straight
  into a model's context (§6), where its size is money. A bound on the grant,
  `[reason 500]`, would make that cost predictable.
- **Q-G: should a consult be able to park?** This is more pressing than it
  first looked. Under H7, a person's wait spends the same deadline the
  program's model calls run against, so every program with a consult needs a
  long outer scope and careful inner ones (§6). A consult that suspends the
  run until the person returns, with the clock for model work stopped
  meanwhile, would separate the two. It's a different resource model (the run
  suspends and resumes, and its state has to survive that), and it's the
  biggest open question in this document.
- **Q-I: how is paid review priced?** H2's flat `[price …]` per consult can't
  express hourly pay. A price per unit of `δ` would, but then the charge is
  known only afterwards, and the reservation needs an upper bound: the time
  left until the deadline, times the rate. That works, and it's worth doing
  only if someone needs it.
- **Q-H: quorum and delegation.** "Two of these three reviewers must approve"
  looks like library code over several human grants and `workflow`, since
  different humans can be consulted concurrently. Worth writing as an example
  before deciding that it needs nothing more.
