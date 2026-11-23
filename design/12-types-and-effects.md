# Types and effects: checking a program before it runs

> **Status: being built.** Steps 0 to 4 of §9 are implemented: the checker
> infers types and effects for every form, checks ownership, and decides unit
> tests, but doesn't yet reject a program (step 5). The design was
> revised once after a critical review, and again where building it found a
> mistake; both kinds of correction are marked where they were made. It
> answers the second of Ramsey's three questions (`04`): *what checking does
> a program undergo before it runs?* Today the answer is "parse-time
> well-formedness, and nothing else", and every type mistake is found while
> the program runs, often after it has spent money.

`01` sketched a type grammar with effect sets on function types, and every
document since has deferred something to "the type and effect system": the
askability rule (`04` §3.1), the ownership check (`03` Q2), law W1's premises
about `(remaining)` and people (`06` §4), and the unused-answer warning for
`consult` (`11` §8). This document designs that system, so that it can be
built.

Three decisions were made with the user before writing (§0, T1–T3). The
rest follow from them and are open for review.

---

## 0. Decisions (please review)

| # | Question | Decision |
|---|---|---|
| T1 | How do programs get their types? | **Inference, as in Ramsey's µML** (Hindley-Milner, Chapter 8). No annotations. Every existing program keeps its meaning, and `define` and `lambda` stay as they are. |
| T2 | What happens to a program the checker finds wrong? | **Type errors reject the program before anything runs.** Programs that are legal but fall outside a law (a workflow node reading `(remaining)`, two nodes consulting one person, a consult answer that is never used, a non-exhaustive `case`) get **warnings**. |
| T3 | How fine-grained are effects? | **Per grant.** An effect says *which* model is asked, *which* capability is called and *which* person is consulted. That is what lets the checker prove `01` P3's claim (the analyst can't touch `fs`) and check ownership statically. |
| T4 | Generic code that uses overloaded or restricted operations | **Qualified types** (Jones, *Qualified Types*, 1994): a type scheme carries constraints, such as "α is a numeric type" or "this function's effect calls no stateful capability", that are checked when the scheme is instantiated. One mechanism covers overloaded arithmetic, `show`, capability interfaces, ownership and the law warnings (§3). |
| T5 | `Any` in the basis | **Replaced by type parameters.** `Option`, `Pair` and `Resources` become `(Option α)`, `(Pair α β)` and a record of `Money` and `Dur`. User datatypes and records may take parameters too: `(datatype (Option a) [None] [Some (value a)])`, and `(Option Num)` in a type. This is more than a checker change: the grammar, `validate`, `show`, `bound` and the JSON schema of `09` §2 all have to substitute type arguments (§9, step 0). |
| T6 | Size bounds, `(Text n)` and `(List τ n)` | **Erased by the checker.** Bounds describe what a model or person may *send*; they are part of a type's schema, checked by validation, and stay dynamic. Statically `(Text 400)` is `Text`. |
| T7 | Forward references | **A file is checked as a whole, before any of it runs,** in dependency order: mutually recursive definitions form a group and are generalized together, as in Haskell. Late binding (`08`, deviation 2) keeps working for functions. **A form that runs at load time** (a `val`, or a top-level expression) **may depend only on definitions above it,** including through the bodies of the functions it calls (§4.4); otherwise it would type-check and then find a name unbound. |
| T8 | Unit tests whose expression is ill-typed | **Each test's expressions are checked in isolation, so an ill-typed one doesn't reject the file, and the test fails, reporting the type error,** whatever kind of test it is. A new form, **`(check-type-error e)`**, passes only when `e` is ill-typed. The checked-error tests that are about types (`consult.nrm`, `typed-tools.nrm`) become `check-type-error`s; those about run-time conditions (an exhausted script) stay `check-error`s. Three Step 5 tests that *catch* the sharing failure are rewritten too (§5.1). |

> **T8, corrected in review.** An earlier draft let `check-error` pass on any
> static error. That repeats the mistake building `consult` found (`06` §7,
> #10): a test that passes whatever happens proves nothing. A `check-error`
> meant to catch an exhausted script would have passed on a typo instead.
> Hence a separate form: a type error satisfies only a test that asks for
> one.

---

## 1. Types

```
τ ::= α                             type variable
    | Text | Num | Bool | Sym       base types (bounds erased, T6)
    | Money | Dur                   ∞ is a value of both (it comes only from (remaining));
                                    it compares above every amount, but arithmetic on it is an error
    | (List τ)
    | (T τ …)                       a datatype or record, possibly with parameters
    | (τ … -> τ ! ε)                a function with latent effect ε
    | (Model μ) | (Human η)         a granted model or person, by name
    | (Kernel κ) | (Filesystem κ) | (Http κ)    a capability, by kind and name

μ, η, κ ::= g | ν                   a grant's name, or a name variable
```

**Names in types.** A grant binds a global whose type carries the grant's
name: `(grant claude (model))` gives `claude : (Model claude)`, and
`(grant py (kernel [exec (Text 4000)]))` gives `py : (Kernel py)`. A function
that takes a capability is generic in its name, through a name variable:

```scheme
(define observe (k code) (call k exec code))
;; observe : ∀ν. Offers(ν, exec, τ) ⇒ ((Kernel ν) Text -> τ ! {call ν})
```

The kind is part of the type because it decides statefulness: a kernel or a
filesystem is stateful, an HTTP client isn't (`04` §6.2).

**Names have kinds.** Every name, constant or variable, has a kind: `Model`,
`Human`, `Kernel`, `Filesystem` or `Http`. A grant's name gets its kind from
the grant, and a name variable gets its kind when it is created (from the
type it first appears in) and keeps it through unification, which fails on a
kind mismatch. Effect atoms carry only a name, so the kind is what tells the
footprint of §5 whether `call ν` touches something stateful, even when ν
reaches a function only through a latent effect.

**Askable types** (`04` §3.1, rule 5) are the ground types built from Text,
Num, Bool, Sym, List and datatypes and records whose fields, at the given
parameters, are askable. `(Option Num)` is now askable; it wasn't when
`Option` held `Any`. Money, Dur, functions, models, people and capabilities
are not.

**Showable types** are the askable types plus Money and Dur: everything
`show` can write (`04` §7.3).

---

## 2. Effects

```
ε ::= {a, …} ∪ φ                    a set of atoms, open with an effect variable φ, or closed
a ::= ask μ | call κ | consult η | observe
```

`observe` is reading `(remaining)`. A function's arrow carries its latent
effect; an expression's judgment carries the effect of evaluating it.

**Effects are inferred with inclusion constraints, not by unification.**
Every arrow's effect is a variable φ, and the rules generate constraints
`ε ⊆ φ` ("evaluating this may do at least ε"), as in Talpin and Jouvelot's
*The Type and Effect Discipline* (1992). That system was built for exactly
this case: effect atoms indexed by variables, which for them are regions
and here are grant names. Solving takes the least solution, and
generalization quantifies over φ together with the inclusions it is
subject to.

> **Corrected in review.** An earlier draft inferred effects by unifying
> open sets, "as in Koka". Koka's rows are labeled by constants. Here a label
> can contain a name variable, and unifying `{call ν} ∪ φ` with
> `{call py} ∪ φ′` has two incomparable solutions (ν = py, or `call py` goes
> into φ). With no principal unifier there are no principal types, and which
> programs are accepted would depend on the solver's order.

Higher-order functions such as `retry` are polymorphic in the effect of the
thunk they are given:

```
retry : ∀α φ. (Num (-> α ! φ) -> α ! φ)
```

`fail` and the failures of `ask` and `call` are **not** effects. Every
effect can fail, so tracking failure separately would say nothing new; what
matters to the laws is *what was touched*, and that is what the atoms record.

---

## 3. Constraints

A type scheme is `∀ᾱ ν̄ φ̄. C ⇒ τ`, where `C` is a set of constraints. A
constraint whose arguments are ground is checked at once; one that still
mentions variables when a binding group is generalized goes into the scheme,
and is checked again each time the scheme is instantiated. Constraints are
**only for checking**: the interpreter still dispatches on values, so unlike
Haskell's classes they compile to nothing.

| Constraint | Means | Instances | Where it comes from |
|---|---|---|---|
| `Arith(τ)` | `+` and `-` apply | Num, Money, Dur | `+`, `-` |
| `Ord(τ)` | `<` and `>` apply | Num, Money, Dur, Text | `<`, `>`, and through them the prelude's `<=` and `>=` |
| `Mul(τ₁, τ₂, τ₃)` | `*` takes τ₁ and τ₂ to τ₃ | (Num Num Num), (Money Num Money), (Num Money Money), (Dur Num Dur), (Num Dur Dur) | `*`; `/` has the analogous `Div` |
| `Showable(τ)` | `show` can write it | §1 | `show`; the arguments of a `call` |
| `Has(τ, f, τ′)` | record type τ has field f of type τ′ | the records of Θ | `(. e f)` |
| `Offers(κ, op, τ)` | capability κ's grant offers op, answering τ | the grants' declarations; an undeclared grant offers every op, answering Text | `call` |
| `F₁ # F₂` | two footprints share no stateful capability | §5 | `workflow` |
| `Stateless(ε)` | an effect calls no stateful capability | — | `workflow`, when a node's effect is a variable |
| `ε ⊆ φ` | an effect variable includes at least ε | the least solution | every rule with an effect (§2) |

**Defaulting.** When a binding group is generalized, an `Arith`, `Ord`,
`Mul` or `Div` constraint on a type variable that the group's type doesn't
mention is defaulted to Num, as SML does; one the type does mention stays in
the scheme, which is what keeps the prelude's `<=` generic. Defaulting per
group, not once for the whole program, keeps each definition's type
independent of what comes after it. Any other constraint that can't be
resolved or deferred is an error, such as a field `f` that two records have
and nothing determines.

`Offers` is what gives typed grants (`10` §2.1) a static check: calling an
operation a closed grant doesn't declare is a type error. `F₁ # F₂` and
`Stateless` give ownership one (§5). Using one mechanism for all of these is
the point of T4.

---

## 4. Typing rules

The judgment is

```
Γ ⊢ e : τ ! ε ; C       e has type τ and effect ε, under constraints C
```

Γ maps names to type schemes. The initial Γ₀ holds the primitives (§4.3),
the prelude's inferred schemes, and one entry per grant. Effects compose by
union, and evaluation order doesn't matter for them. Below, `ε₁ ∪ ε₂` is
union of open sets, and fresh variables are implicit.

### 4.1 The inherited forms

These are µML's rules (Ramsey, Chapter 8), with effects threaded through.
Only what differs is spelled out:

- **LAMBDA** moves the body's effect onto the arrow; the lambda itself has
  the empty effect. **APPLY** adds the function's latent effect to the
  effects of evaluating the function and the arguments.
- **IF** requires `Bool`. Both branches have the same type.
- **CASE** requires each pattern to match the scrutinee's type, and gives
  every branch the same type. A `case` that doesn't cover every constructor
  is a **warning** (T2). It stays a checked run-time error, which §6 lists.
- **LET** and top-level `val` and `define` generalize **type and effect
  variables freely, and name variables only for syntactic values.** ML's
  value restriction exists because of mutable references, and µNorman has
  none (`04`, decision F1), so ordinary type variables need no restriction.
  Name variables do, because of `fork`. If `(val k (call py fork))` were
  generalized, every use of `k` would get a fresh name, and two workflow
  nodes using the same `k`, which share one kernel, would pass the
  ownership check of §5. So a name variable is generalized only when the
  bound expression is a syntactic value (a lambda, literal, variable, or
  constructor or record of values). A `fork` inside a function body is
  fine: each *call* of the function forks anew, so a fresh name per call is
  exactly right.

  > **Corrected in review.** An earlier draft applied ML's value restriction
  > to everything, "because so many expressions have effects". That was the
  > wrong reason, it restricted more than needed, and it hid the one place
  > where generalizing really is unsound.

### 4.2 The agentic forms

| Form | Premises | Type and effect |
|---|---|---|
| `(ask e_m τ e_c 's)` | `e_m : (Model μ)`, `e_c : (List Message)`; τ ground and askable | `τ ! {ask μ} ∪ ε_m ∪ ε_c` |
| `(call e_k op e₁ …)` | `e_k : (K κ)` for a kind K; each `eᵢ : τᵢ` with `Showable(τᵢ)`; `Offers(κ, op, τ)` | `τ ! {call κ} ∪ …` |
| `(call e_k fork)` | `e_k : (Kernel κ)` | `(Kernel κ′) ! {call κ}`, with κ′ a **fresh** name variable (§5.2) |
| `(consult e_h τ e_p [propose e_v] 's)` | `e_h : (Human η)`, `e_p : Text`, `e_v : τ`; τ ground and askable | `τ ! {consult η} ∪ …` |
| `(fail e)` | `e : Failure` | `α ! ε_e`, for a fresh α |
| `(catch e₁ x e₂)` | `e₁ : τ`; `e₂ : τ` with `x : Failure` | `τ ! ε₁ ∪ ε₂` |
| `(budget ([cost e_c] [time e_t]) e)` | `e_c : Money`, `e_t : Dur` | the body's type, and the union of all three effects |
| `(remaining)` | — | `Resources ! {observe}` |
| `(workflow ([x₁ e₁] …) e)` | each `eᵢ` typed with its dependencies bound; the body with every `xᵢ` bound | the body's type; the union of every node's and the body's effects; plus the ownership constraints of §5 |

Four things in this table replace run-time checks:

- **The answer type of `ask` and `consult` is the written τ,** checked
  askable here rather than when the form is evaluated (`08`, deviation 1).
- **The proposal must have type τ,** so `v ⊨ τ` (`04` §6.2) can fail only
  on a bound, which T6 leaves dynamic.
- **A human that was never granted is an unbound variable,** now found
  before the run.
- **A closed grant's undeclared operation fails `Offers`.**

One check is **new, and changes behavior:** a `call`'s arguments must be
`Showable`. A live host receives non-text arguments as `show` writes them,
so passing a function already fails there, but a script ignores arguments,
so a scripted program that passes a function to `call` runs today and will
be rejected.

### 4.3 The primitives, in brief

```
+ - : ∀α. Arith(α) ⇒ (α α -> α)          < > : ∀α. Ord(α) ⇒ (α α -> Bool)
=   : ∀α. (α α -> Bool)                   show : ∀α. Showable(α) ⇒ (α -> Text)
cons : ∀α. (α (List α) -> (List α))       list : n-ary, every argument of one type
println : ∀α. (α -> α)                    string-append : n-ary Text
```

`=` stays total: on functions, models, people and capabilities it compares
identity, as it does now (`04` §7.2).

### 4.4 The top level, and load order

Checking the whole file at once (T7) lets a function call one defined below
it, because top-level functions are late-bound and only look the name up
when they run. A form that runs *while the file loads* has no such grace:

```scheme
(val x (f 1))          ; runs now, and f is not yet defined
(define f (n) (+ n 1))
```

Both forms type-check together, and the first then fails with an unbound
name. So the checker adds one rule: a `val` or top-level expression may
depend only on definitions that come before it, where "depends on" follows
the bodies of the functions it calls, transitively. The dependency graph
is already built for binding groups, so this costs one more pass. A unit
test is exempt: as in Ramsey's interpreters, a file's tests run after the
whole file has loaded.

> **Corrected in review.** An earlier draft claimed Theorem 7 ruled out
> every unbound name while accepting forward references everywhere, and so
> it didn't.

---

## 5. Ownership, statically

### 5.1 Footprints

A workflow node's **footprint** is everything stateful it could touch: the
`call κ` atoms in its effect whose κ is a kernel or filesystem, together
with the stateful capability names in the types of its free variables. The
second part matters because a node can *hand on* a capability without calling
it; anything it can ever use appears in either its effect or its types.

For every pair of nodes, the WORKFLOW rule adds `Fᵢ # Fⱼ`.

- **Ground footprints** are checked at once: two nodes that both call `py`
  are a type error, which is today's run-time `WORKFLOWSHARED` found before
  the run. **This changes what such a program means.** Today the sharing is
  a *failure*, `(Raised 'shared-stateful-capability)`, which a program can
  catch, and three Step 5 tests do catch it to inspect it
  (`step5-examples.nrm`, group 8). Under the checker the program is rejected,
  so there is nothing to catch, and those tests become `check-type-error`s. No
  program should rely on catching that failure: it reports a mistake in the
  program, not a condition of the world.
- **Footprints with variables** stay as constraints. `best-of*`, which runs
  the same thunk `f` in two nodes, gets the constraint `Stateless(φ)` on
  `f`'s effect, so `best-of` over a thunk that calls a kernel is rejected
  *where it is used*. A function that runs two different capability
  parameters concurrently gets `ν₁ # ν₂`, so calling it with the same kernel
  twice is rejected, and calling it with a kernel and a fork of it is
  accepted (§5.2).

The same machinery gives T2's **warnings**: a node whose effect contains
`observe` falls outside W1's premise (ii), and two nodes whose effects both
contain `consult η` fall outside premise (iv). A warning constraint that
mentions variables is likewise reported where its scheme is instantiated,
**once per call site and kind of warning**, naming the generic function it
came from. A helper used in twenty places would otherwise repeat the same
warning twenty times, and warnings that repeat get ignored.

### 5.2 Forks are new names

`fork` returns a capability with a **fresh name variable**, and generalizing
the enclosing definition makes that variable fresh at each use. So
`(let* ([k (call py fork)]) (workflow ([a (use py)] [b (use k)]) …))`
passes `py # κ′`, which is the idiom the guide recommends ("to work in
parallel, fork the kernel first").

> **Corrected in building.** This example used to fork *inside* node b:
> `[b (use (call py fork))]`. That node calls `py` to fork it while node a
> uses `py`, so both footprints contain `py`, and the program is rejected,
> rightly: the run-time check rejects it too, and forking a kernel another
> node is running is a race. The fork must happen before the workflow.

> **Corrected in building.** A fresh name alone loses the fork's
> operations: `Offers(κ′, op, τ)` on a fresh variable can never be decided,
> so a fork of a closed kernel would offer anything. A capability's type
> therefore carries two names, its own (for ownership, and fresh for a fork)
> and the grant whose operations it offers (kept by a fork). `(Kernel py)`
> is a granted kernel; `(Kernel κ′ py)` is a fork of it.

This is sound because a fresh name is introduced only where a new capability
is created, and no rule makes two existing capabilities share a name they
didn't already have. It relies on §4.1's rule that a name variable is never
generalized for an expression that isn't a value: without it, binding one
fork to a name and using it twice would give the same kernel two names. It is **incomplete** in the usual way: two forks made by
the same expression share its static name, so a program that puts several
forks in a list and runs two of them concurrently is rejected, though at run
time they differ. The run-time check would have accepted that program.

### 5.3 Static and dynamic ownership differ, in both directions

Neither check contains the other:

- **The checker accepts some programs the run-time check rejects.** Today's
  `reach` (`04` §6.2) counts every capability a node's closures *capture*,
  even ones they can never use. The static footprint counts only what the
  node can use. So a node that captures `py` in a closure it never calls
  passes the checker and is rejected by `WORKFLOWSHARED`.
- **The run-time check accepts some programs the checker rejects.** Two
  forks made by the same expression share a static name (§5.2), so running
  two of them concurrently is a type error, although at run time they are
  different capabilities. Under T2 such a program, which runs today, would
  be rejected. The fix is to restructure it, for example by forking inside
  each node.

**Recommendation:** a checked program skips the run-time ownership check.
That is safe, not because the static check is more precise (it isn't), but
because every program the run-time check would reject *and the checker
accepts* differs only in captures it can never use, and those can't
interfere. Theorem 4 needs non-interference, not non-capture. The machine
keeps the run-time check for programs that aren't checked, such as the
playground with the checker turned off. (Open question Q-1.)

> **Corrected in review.** An earlier draft called the static check "the
> more precise of the two" and said the checks differed in one direction.
> Both claims were wrong; the recommendation stands, on the argument above.

---

## 6. What the checker guarantees

**Theorem 7 (type soundness).** If a program checks, then no evaluation of
it reaches any of these checked run-time errors:

- an `if` on a non-Boolean; applying a non-function, or with the wrong
  number of arguments; an unbound name; an unknown constructor, record or
  field;
- `ask` on a non-model, `consult` on a non-person, `call` on a
  non-capability;
- an unaskable type in `ask` or `consult`; a proposal of the wrong shape;
- an undeclared operation on a closed grant; `show` of a function, model,
  person or capability;
- `fail` of a non-`Failure`;
- **`WORKFLOWSHARED`**: two nodes sharing a stateful capability.

**What remains dynamic,** stated so that it isn't mistaken for a gap:
division by zero and arithmetic overflow; `substring` out of range;
arithmetic on ∞ (`∞ − $1`; *comparing* ∞ is defined, and was fixed in the
interpreter when reviewing this document); a non-exhaustive `case` (a
warning, T2); a size bound
exceeded by a proposal (T6); an exhausted script; an unbound person in live
mode; and the step limit.

**Theorem 8 (effect soundness).** If `Γ ⊢ e : τ ! ε`, every entry the
evaluation of `e` appends to the trace is covered by ε: an ask of model μ
by `ask μ`, a call on κ by `call κ`, a consult of η by `consult η`. If `e`
reads `(remaining)`, ε contains `observe`.

Theorem 8 is what proves `01` P3's claim. In the committee, `analyst` and
`risk-officer` are given no capabilities, so their effects contain `ask` atoms
only. By Theorem 8 their traces contain no calls, so they can't touch `net`
or `fs`, whatever their models answer. Theorem 2 (no forged authority) says
a model can't *give* them a capability; Theorem 8 says they don't *use* one.

Both theorems are proved in the usual way, by preservation and progress over
`07`'s machine. The proof cases for the agentic forms are where the
interest lies; the inherited forms follow µML's. Preservation must cover the
machine's cancelling steps as well as its ordinary ones: M-NODE-FAIL and
M-TIME's cuts replace a thread's expression with a `fail` and append
`cancelled` or `past-deadline` trace entries, and Theorem 8 needs those
entries covered by the cancelled request's own effect atom, which it is.

---

## 7. Worked examples

**CP-Agent** checks, and the types say what the loop can do:

```
react : ∀μ κ. Offers(κ, exec, Text) ⇒ ((Model μ) (Kernel κ) (List Message) -> Text ! {ask μ, call κ})
```

**`best-of` over a kernel** is rejected where it is used, not inside
`best-of`:

```scheme
(best-of 3 (lambda () (observe py "x = 1")) accept)
;; error: best-of runs its thunk in concurrent nodes (Stateless(φ)),
;;        but this thunk calls the kernel py
```

Today that program is rejected at run time by `WORKFLOWSHARED`, after
whatever ran before it has spent money.

**SAGA's reviews** produce no warning: `reviewed-objectives` consults
`reviewer` in one node at a time. A version that put two reviews of the same
person in one `par` would get premise (iv)'s warning.

**A typo in a field name** stops the program before its first ask:
`(. verdict reson)` fails `Has(Verdict, reson, _)`.

---

## 8. Hard parts, and open questions

The difficult parts, in order of risk:

1. **Constraint solving with name variables.** `Offers`, `#` and `Stateless`
   are new, though each has a finite domain once names are ground.
   Simplification and entailment need care to keep error messages
   intelligible: Jones's framework says *what* to do, but not how to phrase
   "this thunk calls `py`" at the right place.
2. **Effect inference with inclusion constraints** (Talpin–Jouvelot) is
   standard, but combining it with qualified types, the fork restriction of
   §4.1 and binding groups (T7) has to be checked on the prelude and
   examples, not assumed. The inclusions are one more kind of constraint in
   §3's mechanism, which keeps the solver to one design.
3. **Error messages.** Hindley-Milner reports where unification failed, not
   where the mistake is. A first version should at least name both sides'
   origins.

Questions, **decided** with the user before implementation began:

- **Q-1, decided: a checked program skips `WORKFLOWSHARED`** (§5.3). What
  it would still reject can't interfere. The cost, which `04` and `07` must
  state, is that the dynamic rule then depends on whether the checker ran.
- **Q-2, decided: defining a top-level name twice is an error** (`08`,
  deviation 2). With whole-file checking, "which definition does this use?"
  has no good answer, and a second definition is usually an accident. µNorman
  has no interactive REPL, which is where Ramsey's redefinition habit earns
  its keep.
- **Q-3, decided: a non-exhaustive `case` is an error when its scrutinee is
  an answer from `ask` or `consult`, and a warning otherwise.** A missed
  constructor in an answer type is the commonest way an agent program gets
  stuck. Telling the two apart means following where the scrutinee came
  from, at least through `let`-bound names, which the checker records.
- **Q-4, decided: the unused-consult warning stays syntactic** (a consult in
  a `begin`, or bound to a name that is never used). Following the answer
  through data structures can come later.
- **`Any`, decided: it is removed once the checker is in use.** It has no
  static meaning, and nothing uses it now that the basis doesn't.
- **Q-E (from `10`)** remains open, and this design doesn't address it.
  Revoking a derived capability is about how long a name may be used, which
  is a question for a type system with lifetimes, not this one.

---

## 9. Implementation sketch

In dependency order, and each testable on its own:

0. ✅ **Done.** Built and tested in
   [`../examples/type-parameters.nrm`](../examples/type-parameters.nrm) and
   the `parameter_tests` in `src/types.rs`, with the schema tested in
   `src/anthropic.rs`. **Type parameters, before any checker** (T5). The grammar gains
   parameterized `datatype` and `record` declarations and type application
   in `ty`; [`../src/types.rs`](../src/types.rs) substitutes type arguments
   in `validate`, `show`, `bound` and `askable`; and the schema of `09` §2
   does the same, so `(ask m (Option Num) …)` works live. `Option`, `Pair`
   and `Resources` move off `Any`. This changes the running language, not
   just its checking, so it gets its own tests and its own commit: the
   `show` round trip on parameterized values, and the schema sent for one.
1. ✅ **Types and unification,** with effect variables, in
   [`../src/check.rs`](../src/check.rs). Name variables come with step 3,
   because nothing has a name-indexed type before grants do.
2. ✅ **Inference for the inherited forms,** with binding groups (T7),
   generalization (type variables freely, §4.1), the constraints of §3 with
   defaulting per group, the load-order rule (§4.4) and Q-2. `fail`,
   `catch`, `budget`, `(remaining)` and `workflow` are typed too, the last
   without ownership (step 4). The prelude checks, and every inferred type is
   asserted in [`../tests/check.rs`](../tests/check.rs); they are the types
   this document predicts, such as `retry : (Num (-> a ! e1) -> a ! e1)`.

   **Building it found two bugs, both now tested and mutation-checked.**
   Generalization copied each effect variable's *direct* lower bound, so an
   effect reaching a type only through a variable not in it was lost:
   `best-of` and anything calling `attempt` came out with no effect at all,
   which would have made Theorem 8 false. Bounds are now flattened when
   generalizing (`generalize` in `check.rs`). And resolving a field to its
   record dropped the field's constraint, which unlinked the field's type
   from the record's, so `swap` came out as `((Pair a b) -> (Pair c a))`.
   A field several records share now gives a qualified type,
   `Has a x b => (a -> b)`, resolved where the function is used.
3. ✅ **Done.** Tested in [`../tests/check.rs`](../tests/check.rs), whose
   `every_example_checks` reads every file in `examples/`, and by the
   `check-type-error`s now in the examples. **The agentic forms and constraints** of §3 and §4.2, and
   `check-type-error`. Tests: every example file checks; the checked-error
   tests that are about types (a non-person consulted, an unaskable type, a
   malformed proposal, an undeclared operation) become `check-type-error`s,
   and those about scripts (an exhausted script, `(approve)` with nothing
   proposed) stay `check-error`s (T8). A deliberately wrong version in
   `must-fail.nrm`: a `check-error` on an ill-typed expression, which must
   now fail.
   **Building it changed four things.**
   - *A capability's type has two names* (§5.2): its own, for ownership,
     and the grant whose operations it offers, for `Offers`. A fork gets a
     fresh name of its own but keeps its parent's offers, so a fork of a
     closed kernel is closed too, which `typed-tools.nrm` tests. With one
     name, `Offers` on a fork's fresh variable could never be decided.
     They print as one name when they agree: `(Kernel py)`, `(Kernel a py)`.
     `call` doesn't fix the kind, so a function generic in its capability
     has type `(Cap k a b)`.
   - *`(catch e x x)` is ill-typed* unless `e` is a `Failure`, and tests
     used it to look at the failure they expected. Decided with the user:
     `check-expect` passes when both sides fail with equal failures, so
     `(check-expect e (fail φ))` says exactly how `e` fails and type-checks,
     because `(fail φ)` has any type. `check-within` never needed the catch:
     it measures a failing run as it does a successful one.
   - *A type names exactly one grant* (T3), so `(list claude haiku)` is
     ill-typed. No example needed two grants of one kind in one value; one
     (`consult.nrm`'s `(list a b)` of Text and objectives) was heterogeneous
     anyway and is now a `Pair`.
   - *The driver runs the checker before loading,* ahead of step 5, but only
     to decide unit tests: a `check-type-error` passes when the checker says
     so, and an ill-typed test fails without running (T8). A file that
     doesn't check still loads. The must-fail suite is now two files,
     because `step5-examples.nrm` and `consult.nrm` both grant `claude`
     (Q-2), and each must check for its mutants to fail for the right reason.
4. ✅ **Done.** Tested in [`../tests/check.rs`](../tests/check.rs) (the
   `ownership` section) and by group 8 of `step5-examples.nrm`, whose three
   sharing tests are now `check-type-error`s. **Ownership** (§5). Tests: the Step 5 group 8 programs, rewritten as
   `check-type-error`s (§5.1); `best-of` over a kernel; the fork idiom; and
   `(val k (call py fork))` used in two nodes, which must be rejected (§4.1).
   **How it was built.** A `call` atom carries the capability's kind as
   well as its name, so a footprint knows what is stateful even when the
   name reaches it only through a latent effect (§1, "Names have kinds").
   One predicate, `F₁ # F₂`, does the work of both `#` and `Stateless`:
   when both footprints reach the same effect variable φ, every stateful
   call φ ever makes is in both, so `#` already demands that φ be
   stateless. It is printed as `Stateless e1` in that case, so `best-of`
   reads `Stateless e1 e2 => (Num (-> a ! e1) (a -> Bool ! e2) -> a ! e1 e2)`.
   A `#` that mentions a quantified variable goes into the scheme,
   flattened like the effect bounds, and is checked again where the scheme
   is used; one that can no longer change is checked and dropped.
   Q-1 (skipping the run-time check for a checked program) waits for step 5,
   since until then the checker doesn't stop a program it rejects.
5. **Warnings** (T2) and **wiring**: the driver checks a file tree before
   loading it; `norman --no-check` keeps today's behavior; the playground
   reports type errors. Two collisions with Q-2 must be fixed first: both
   `step5-examples.nrm` and `consult.nrm` define `spent` and `elapsed`, and
   both grant `claude`, and `tests/must-fail.nrm` uses both files.
6. **The properties.** Random well-typed programs must never hit a check
   Theorem 7 rules out, which is Theorem 7 tested the way `06`'s laws are.

---

## 10. What changes elsewhere

| Document | Change |
|---|---|
| `04` §0, §1 | Ramsey's second question gets its answer; the type language |
| `04` §2 | parameterized `datatype` and `record`; type application in `ty`; `check-type-error`, a new reserved word |
| `09` §2 | the JSON schema `S(τ)` of a parameterized type substitutes its arguments |
| `04` §3.1 | rule 5 (askability) moves from parse time to the checker |
| `04` §7.2 | `Option`, `Pair` and `Resources` get parameters (T5) |
| `04` §8 | Theorems 7 and 8 |
| `05` | convention: an ill-typed test fails unless it is a `check-type-error` (T8); group 8's three tests that catch the sharing failure become `check-type-error`s |
| `04` §6.5 | WORKFLOWSHARED: for a checked program, a type error rather than a catchable failure (§5.1, Q-1) |
| `06` §4 | W1's premises (ii) and (iv) become warnings the checker issues |
| `08` | deviations 1 and 2 revisited; a "skips run-time ownership" note if Q-1 is accepted |
| the guide | "What it doesn't do (yet)": the first bullet goes |
