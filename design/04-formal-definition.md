# The formal definition of µNorman

This document follows the structure of Ramsey's *An Imperative Core*
(Chapter 1 of *Programming Languages: Build, Prove, and Compare*):

1. lexical structure
2. concrete syntax
3. abstract syntax
4. metavariables
5. environments and the world
6. operational semantics
7. syntactic sugar and the initial basis
8. metatheory

It consolidates `01`–`03`. Where this document and an earlier one disagree on a
syntactic detail, this one wins.

Ramsey's three questions for any language (§1.1) frame everything:

1. **How is code formed?** Sections 1–4.
2. **What checking does it undergo before it runs?** Parse-time well-formedness
   now (§3.1), a type/effect system later.
3. **What happens when it runs?** Sections 5–6.

---

## 0. Design decisions this document makes (please review)

Reading Impcore against our design brought out four decisions that the earlier
documents left implicit.

| # | Decision | Impcore does | µNorman does | Why |
|---|---|---|---|---|
| F1 | **No `set`, no `while`** | both are core | neither exists | All mutable state lives behind capabilities. That keeps `workflow`'s law sound (the only shared state is capabilities, and ownership controls those). Loops are recursion, bounded by `budget` (the Peano measure from `02`). |
| F2 | **Capitalized names are constructors, record names or types** | a token is a name unless it's a numeral or reserved | lowercase = variable; `Capitalized` = constructor/record/type | Without this, the parser can't tell `(Message …)` (a record) from `(f …)` (an application). ML uses the same convention. |
| F3 | **`par` is syntactic sugar, not a predefined function** | `and`/`or` are predefined *functions*, so they evaluate both arguments | `par` must not evaluate its arguments before forking | A function's arguments are evaluated *before* the call (rule APPLYUSER), sequentially. So `par` as a function would already have run both branches, one after the other. Only sugar can delay them. |
| F4 | **The trace lives in the world `W`** | printing is left out of the semantics "to avoid clutter" | trace entries are part of `W` | Scripts, replay and profiles (`03`) depend on the trace, so it has to be in the formal state, not an informal side effect. |

---

## 1. Lexical structure

This is Ramsey's lexical structure, with the extra literals agents need.

- A **semicolon** starts a comment that runs to the end of the line.
- Each **bracket**, `(` `)` `[` `]`, is a token by itself. Round and square
  brackets are interchangeable as long as they match.
- A **string literal** is `"…"` with the escapes `\"`, `\\` and `\n`. Impcore
  has no strings; we need them for prompts.
- Every other character joins tokens that are as long as possible, ending only
  at a bracket, a semicolon, whitespace or a `"`.

Token classes:

| Class | Form | Examples | Meaning |
|---|---|---|---|
| numeral | digits, optional sign | `42`, `-7` | 64-bit integer |
| money | `$` digits [`.` up to 6 digits] | `$0.50`, `$20` | integer micro-dollars: `$0.50` = 500 000 µ$ |
| duration | digits followed by `ms` \| `s` \| `min` \| `h` | `250ms`, `30s`, `10min` | integer milliseconds |
| Boolean | `#t` \| `#f` | | |
| quoted symbol | `'` name | `'declined` | a symbol value |
| empty list | `'()` | | |
| name | starts with a lowercase letter or symbol character | `react`, `best-of`, `+` | a variable or function |
| Name | starts with an uppercase letter | `Step`, `Exec`, `Message`, `Text` | a constructor, record name or type |

Money and durations are **distinct kinds of value**, not plain integers. You
can add money to money but not money to seconds, which rules out a whole class
of budget bugs at the level of values.

---

## 2. Concrete syntax

This is in the EBNF of Ramsey's Figure 1.1: `{ x }` means zero or more, and
`[ x ]` means optional. **Keywords** are reserved.

```
def       ::= (val variable-name exp)
            | exp
            | (define function-name (formals) exp)
            | (datatype Type-name {[Con-name {(field-name ty)}]})
            | (record Record-name {(field-name ty)})
            | xdef

xdef      ::= (use file-name)
            | (grant variable-name host-spec)
            | (script Script-name {site-script})
            | (under ({config}) {unit-test})
            | unit-test

unit-test ::= (check-expect exp exp)
            | (check-assert exp)
            | (check-error exp)                        ; a checked run-time error (stuck)
            | (check-fail exp pattern)                 ; evaluates to (fail φ), φ matching pattern
            | (check-within exp ([cost exp] [time exp])) ; spends at most, takes at most
            | (check-equiv exp exp quoted-symbol)        ; 'exact | 'resource | 'value (06 §0);
                                                         ; each side runs in its own fresh world

exp       ::= literal
            | variable-name
            | Con-name                                  ; nullary constructor
            | (Con-name {exp})                          ; constructor application
            | (Record-name {[field-name exp]})          ; record construction
            | (. exp field-name)                        ; field selection
            | (if exp exp exp)
            | (let* ({[variable-name exp]}) exp)
            | (lambda (formals) exp)
            | (case exp {[pattern exp]})
            | (ask exp ty exp [quoted-symbol])          ; model, type, context, optional site
            | (call exp op-name {exp})                  ; capability, operation, arguments
            | (fail exp)
            | (catch exp variable-name exp)
            | (budget ({[cost exp] | [time exp]}) exp)
            | (workflow ({[variable-name exp]}) exp)
            | (begin {exp})                              ; sugar
            | (par exp exp)                              ; sugar
            | (and exp exp) | (or exp exp)               ; sugar (short-circuit)
            | (exp {exp})                                ; application

pattern   ::= Con-name | (Con-name {variable-name | _}) | '() | (cons variable-name variable-name) | _

ty        ::= Text | (Text numeral) | Num | Bool | Sym
            | (List ty) | (List ty numeral)
            | Type-name | Record-name | Any

formals   ::= {variable-name}
literal   ::= numeral | string | money | duration | #t | #f | quoted-symbol | '()
```

**Reserved words:** `val define datatype record use grant script under
check-expect check-assert check-error check-fail check-within check-equiv if let* lambda
case ask call fail catch budget workflow begin par and or _`.

`cost` and `time` are **not** reserved. They occur only in fixed bracket
positions (`[cost e]` in `budget`, `under` and `check-within`), and they're the
field names of the predefined `Resources` record. An earlier draft reserved
them, which made `(. (remaining) cost)` a syntax error. Running the Step 5
tests caught this.

**Patterns are flat:** a constructor applied to variables or `_`. Nested
patterns can come later as sugar. Flat patterns keep the CASE rule, and every
proof about it, to a single step of matching.

---

## 3. Abstract syntax

Written in Ramsey's AST-description notation (§1.3, chunk 42c). As in his
interpreter, forms whose names clash with keywords of the implementation
language get an `X` suffix.

```
Def*    = VAL      (Name name, Exp exp)
        | EXP      (Exp)
        | DEFINE   (Name name, Lambda lambda)
        | DATATYPE (Name name, Conlist cons)
        | RECORD   (Name name, Fieldlist fields)

Lambda  = (Namelist formals, Exp body)          ; invariant: formals all distinct

Exp*    = LITERAL  (Value)
        | VAR      (Name)
        | CON      (Name con, Explist args)
        | RECORDX  (Name record, Bindinglist fields)
        | FIELD    (Exp record, Name field)
        | IFX      (Exp cond, Exp truex, Exp falsex)
        | LETX     (Name name, Exp rhs, Exp body)
        | LAMBDAX  (Lambda)
        | APPLY    (Exp fn, Explist actuals)
        | CASEX    (Exp scrutinee, Branchlist branches)
        | ASK      (Site site, Exp model, Type ty, Exp context)
        | CALL     (Exp cap, Name op, Explist actuals)
        | FAIL     (Exp)
        | CATCH    (Exp body, Name name, Exp handler)
        | BUDGET   (Exp cost, Exp time, Exp body)       ; an omitted limit becomes LITERAL(∞)
        | WORKFLOW (Bindinglist nodes, Exp body)

Pattern = PCON (Name con, Namelist vars) | PNIL | PCONS (Name head, Name tail) | PWILD
Type    = TEXT (Bound) | NUM | BOOL | SYM | LIST (Type, Bound) | TYNAME (Name) | ANY
Bound   = UNBOUNDED | AT_MOST (int)
Site    = a quoted symbol, or the source location "file:line:col"
```

There are **sixteen expression forms**: the ten inherited from µScheme/ML, plus
the six agentic forms from `01`/`02` (ASK, CALL, FAIL, CATCH, BUDGET,
WORKFLOW). `let*`, `begin`, `par`, `and` and `or` are *not* here. They're
sugar (§7).

### 3.1 Well-formedness checked by the parser

These are the analogue of Impcore's `check_def_duplicates`. Each is decidable
from syntax alone.

1. The formals of a `lambda`/`define` are distinct. So are the constructor names
   in a `datatype` and the field names in a `record`.
2. The binding names in a `workflow` are distinct.
3. **The dependency graph of each `workflow` is acyclic.** For node `xᵢ = eᵢ`,
   `deps(xᵢ) = fv(eᵢ) ∩ {x₁…xₙ}`, using `fv` from Figure 1.
4. Patterns are flat, and each variable appears at most once per pattern.
5. `ty` in an `ask` is **askable**: built only from Text, Num, Bool, Sym, List,
   and datatype/record names whose fields are askable. `Any` is **not**
   askable. That's the side condition that keeps capabilities unforgeable
   (`01` §1c).

```
fv(LITERAL v)              = ∅
fv(VAR x)                  = {x}
fv(CON(K, es))             = ∪ fv(eᵢ)
fv(RECORDX(R, fs))         = ∪ fv(eᵢ)
fv(FIELD(e, f))            = fv(e)
fv(IFX(e₁, e₂, e₃))        = fv(e₁) ∪ fv(e₂) ∪ fv(e₃)
fv(LETX(x, e₁, e₂))        = fv(e₁) ∪ (fv(e₂) − {x})
fv(LAMBDAX(xs, e))         = fv(e) − xs
fv(APPLY(e, es))           = fv(e) ∪ ∪ fv(eᵢ)
fv(CASEX(e, bs))           = fv(e) ∪ ∪ (fv(eᵢ) − vars(pᵢ))
fv(ASK(s, e_m, τ, e_c))    = fv(e_m) ∪ fv(e_c)
fv(CALL(e, op, es))        = fv(e) ∪ ∪ fv(eᵢ)
fv(FAIL e)                 = fv(e)
fv(CATCH(e₁, x, e₂))       = fv(e₁) ∪ (fv(e₂) − {x})
fv(BUDGET(e_c, e_t, e))    = fv(e_c) ∪ fv(e_t) ∪ fv(e)
fv(WORKFLOW([xᵢ = eᵢ], e)) = (∪ fv(eᵢ) ∪ fv(e)) − {x₁…xₙ}
```
*Figure 1: Free variables (compare Ramsey's Figure 1.8).*

---

## 4. Metavariables

Compare Ramsey's Table 1.4.

| Metavariable | Stands for |
|---|---|
| `e`, `eᵢ` | an expression |
| `d` | a definition |
| `x`, `f` | a variable name, a function name |
| `K`, `R`, `T` | a constructor, a record name, a datatype name |
| `τ` | a type (in `ask`) |
| `p` | a pattern |
| `s` | a site |
| `v`, `vᵢ` | a value |
| `φ` | a failure: a value of datatype `Failure` |
| `r` | a result: either `v` or `fail φ` |
| `μ` | a model (host-provided) |
| `κ` | a capability (host-provided) |
| `c`, `ĉ` | an amount of money in µ$ (`ĉ` is a reservation) |
| `δ` | a duration in ms |
| `ρ` | a value environment |
| `Θ` | a datatype/record environment |
| `W` | a world |

**Values:**

```
v ::= n | str | #t | #f | 'sym | '() | (cons v v)
    | R{f₁ = v₁, …} | K(v₁, …, vₙ)
    | CLOSURE(⟨x₁…xₙ⟩, e, ρ) | PRIMITIVE(name)
    | MODEL(μ) | CAP(κ)
    | money(c) | dur(δ) | ∞
```

---

## 5. Environments and the world

Compare Ramsey §1.4. Impcore needs three environments (ξ for globals, φ for
functions, ρ for parameters) because it has separate namespaces and mutable
variables. µNorman has neither, so it needs **one** value environment `ρ` like
µScheme, plus two new components:

- **`Θ`**: datatype and record definitions. It maps each type name to its
  constructors or fields, and each constructor to its datatype. Expressions
  never change `Θ`, just as Impcore expressions never change `φ`. It's needed
  by CON, CASE, and `validate` in ASK.
- **`W`**: the **world**. It's everything outside the program that `ask` and
  `call` read or change:

```
W = ⟨ pool, deadline, now, hosts, oracle, trace ⟩

  pool      remaining money (µ$) in the current budget scope
  deadline  absolute instant; shared by every branch in the scope   (03, Q4)
  now       the clock: real, or virtual in scripted mode             (03, Q3)
  hosts     the private state behind each capability κ
  oracle    the relation that models answer by; live or scripted
  trace     append-only list of ⟨site, path, model-or-cap, in, out, cost, δ, outcome⟩
```

Notation: `W.pool` projects a component, `W[pool := c]` updates one, and

```
W ⊕ ⟨c, δ, t⟩  =  W[pool := W.pool − c, now := W.now + δ, trace := W.trace ++ [t]]
```

charges money `c` and time `δ` and logs trace entry `t`.

**Capabilities and models come from the host, never from the program.** The
initial basis `ρ₀` contains the primitives, the predefined functions, and one
binding for each `grant`. No expression form creates a `CAP` or `MODEL` value.
§8 makes that a theorem.

---

## 6. Operational semantics

### 6.1 Judgment forms

```
⟨e, Θ, ρ, W⟩ ⇓ ⟨r, W′⟩            evaluating e produces result r and changes the world to W′
⟨d, Θ, ρ, W⟩ → ⟨Θ′, ρ′, W′⟩       evaluating definition d produces a new basis and world
```

The form of the expression judgment already tells us a few things, in the way
Ramsey reads Impcore's judgment in §1.5.1:

- **An expression may fail.** The result `r` is either a value `v` or
  `fail φ`. Impcore has no counterpart, because its errors are simply "stuck."
  In µNorman, running out of budget, running out of time and bad model output
  are *results*, not crashes.
- **An expression never changes `Θ` or `ρ`**, since neither appears on the
  right. Only definitions extend the basis.
- **An expression may change the world:** it may spend money, advance the
  clock, change capability state and extend the trace.

Ramsey notes that "a judgment describes a relation, not a function" and that
all his languages are deterministic. **µNorman is not.** The oracle is a
relation (`⇝`), so `⇓` is one too. In scripted mode the oracle is a function,
and then µNorman is deterministic. That's what makes `check-expect` possible.

### 6.2 Auxiliary judgments (implemented by the host, not the evaluator)

```
O ⊢ ⟨μ, s, ctx, τ⟩ ⇝ ⟨j, n, δ⟩       the oracle answers with JSON text j, n output tokens, after δ
O ⊢ ⟨μ, s, ctx, τ⟩ ⇝ error(m)          the provider failed with message m
H ⊢ ⟨κ, op, vs⟩ ⇝ ⟨ok v, H′, δ⟩        the capability's host performs op
H ⊢ ⟨κ, op, vs⟩ ⇝ ⟨err m, H′, δ⟩       ... or reports an error
validate_Θ(j, τ) = v                    j parses as JSON and conforms to τ; otherwise undefined
price_μ(nᵢₙ, nₒᵤₜ) = inₘ·nᵢₙ + outₘ·nₒᵤₜ  money for a call (µ$ per token × tokens)
|ctx|_μ                                  input tokens of ctx under μ's tokenizer
max_out(μ, τ) = min(ceiling(μ), bound(τ))                                 (03, Q1)
cost_κ(op)                               the declared up-front cost of op (usually 0)
reach(v)                                 stateful capabilities reachable from v (defined below)
```

`reach` follows **only the free variables** of a closure's body, never the
whole closure environment:

```
reach(CAP(κ))              = {κ} if κ is stateful, else ∅
reach(CLOSURE(xs, e, ρ_c)) = ∪ { reach(ρ_c(y)) : y ∈ fv(e) − xs }   ; least fixed point, for recursive closures
reach(K(v₁…vₙ)), reach(R{…}), reach(cons v v′) = ∪ of the components
reach(anything else)       = ∅
reach(ρ|X)                 = ∪ { reach(ρ(x)) : x ∈ X }
```

The restriction to free variables matters. Every top-level function's closure
environment is the global `ρ`, which contains *every* granted capability. If
`reach` followed whole environments, any two workflow nodes that called any
top-level functions would appear to share every kernel, and every `workflow`
would be rejected. Writing the Step 5 tests is what exposed this (`05`,
group 8).

### 6.3 The failure convention

Almost every rule has evaluation premises, and any of them may produce
`fail φ`. Writing a separate propagation rule for every premise of every form
would double the size of the semantics without adding understanding. It's the
same judgment Ramsey makes about printing. So there is **one rule schema**:

> **(PROPAGATE)** If a rule's evaluation premise, evaluated in order, yields
> `⟨fail φ, Wᵢ⟩`, then the conclusion yields `⟨fail φ, Wᵢ⟩`. The remaining
> premises aren't evaluated. The only exceptions are the rules that handle
> failure: CATCH-FAIL, and WORKFLOW's cancellation rule.

### 6.4 Inherited forms

These are the µScheme/ML rules, stated briefly because nothing new happens in
them. The world is threaded left to right; that threading is what fixes the
order of evaluation (Ramsey §1.5.5).

```
─────────────────────────────── (LITERAL)
⟨LITERAL(v), Θ, ρ, W⟩ ⇓ ⟨v, W⟩

          x ∈ dom ρ
─────────────────────────────── (VAR)
⟨VAR(x), Θ, ρ, W⟩ ⇓ ⟨ρ(x), W⟩

───────────────────────────────────────────────────── (LAMBDA)
⟨LAMBDAX(⟨xs⟩, e), Θ, ρ, W⟩ ⇓ ⟨CLOSURE(⟨xs⟩, e, ρ), W⟩

⟨e, Θ, ρ, W₀⟩ ⇓ ⟨CLOSURE(⟨x₁…xₙ⟩, e_c, ρ_c), W₁⟩
⟨eᵢ, Θ, ρ, Wᵢ⟩ ⇓ ⟨vᵢ, Wᵢ₊₁⟩   for i = 1…n
⟨e_c, Θ, ρ_c{x₁ ↦ v₁, …, xₙ ↦ vₙ}, Wₙ₊₁⟩ ⇓ ⟨r, W′⟩
──────────────────────────────────────────────────── (APPLYCLOSURE)
⟨APPLY(e, e₁…eₙ), Θ, ρ, W₀⟩ ⇓ ⟨r, W′⟩

⟨e₁, Θ, ρ, W⟩ ⇓ ⟨#t, W₁⟩    ⟨e₂, Θ, ρ, W₁⟩ ⇓ ⟨r, W′⟩
───────────────────────────────────────────────── (IFTRUE)
⟨IFX(e₁, e₂, e₃), Θ, ρ, W⟩ ⇓ ⟨r, W′⟩
                                        (IFFALSE is symmetric, with #f and e₃)

⟨e₁, Θ, ρ, W⟩ ⇓ ⟨v, W₁⟩    ⟨e₂, Θ, ρ{x ↦ v}, W₁⟩ ⇓ ⟨r, W′⟩
──────────────────────────────────────────────────────── (LET)
⟨LETX(x, e₁, e₂), Θ, ρ, W⟩ ⇓ ⟨r, W′⟩

K ∈ dom Θ with arity n    ⟨eᵢ, Θ, ρ, Wᵢ₋₁⟩ ⇓ ⟨vᵢ, Wᵢ⟩  for i = 1…n
────────────────────────────────────────────────────────── (CON)
⟨CON(K, e₁…eₙ), Θ, ρ, W₀⟩ ⇓ ⟨K(v₁…vₙ), Wₙ⟩
                              (RECORD and FIELD follow the same pattern)

⟨e, Θ, ρ, W⟩ ⇓ ⟨v, W₁⟩    pᵢ is the first pattern that matches v, binding ρ_p
⟨eᵢ, Θ, ρ ⊎ ρ_p, W₁⟩ ⇓ ⟨r, W′⟩
──────────────────────────────────────────────────────── (CASE)
⟨CASEX(e, [(p₁, e₁) … (pₘ, eₘ)]), Θ, ρ, W⟩ ⇓ ⟨r, W′⟩
```

Notes:

- `if` requires a **Boolean**. Any other test value leaves the machine stuck,
  which is a checked run-time error. (µScheme treats everything except `#f` as
  true.) We're strict because model outputs arrive typed, and a truthy string
  where a Bool was meant is exactly the kind of bug we want to catch.
- A `case` with no matching pattern is stuck.
- **Recursion.** `DEFINE(f, λ)` binds `f` to a closure whose environment
  already contains `f`: `ρ′ = ρ{f ↦ CLOSURE(xs, e, ρ′)}`. That's a recursive
  environment, defined as a fixed point. µScheme gets the same effect with a
  mutable store. We don't need one, because nothing else in the language is
  mutable (F1).

### 6.5 The agentic forms

#### ASK

The premises are evaluated in order: model, then context. Then comes the
reservation check, the oracle's answer, the deadline check and validation.

```
⟨e_m, Θ, ρ, W⟩ ⇓ ⟨MODEL(μ), W₁⟩
⟨e_c, Θ, ρ, W₁⟩ ⇓ ⟨ctx, W₂⟩          ctx is a list of Message records
ĉ = price_μ(|ctx|_μ, max_out(μ, τ))    ĉ ≤ W₂.pool
W₂.oracle ⊢ ⟨μ, s, ctx, τ⟩ ⇝ ⟨j, n, δ⟩    n ≤ max_out(μ, τ)
W₂.now + δ ≤ W₂.deadline
validate_Θ(j, τ) = v
────────────────────────────────────────────────────────────────────── (ASKOK)
⟨ASK(s, e_m, τ, e_c), Θ, ρ, W⟩ ⇓ ⟨v, W₂ ⊕ ⟨price_μ(|ctx|_μ, n), δ, t⟩⟩
```

The failing variants share the first two premises. Each replaces the premise
that fails:

| Rule | Replaced premise | Result | World |
|---|---|---|---|
| ASKEXPIRED | `W₂.now ≥ W₂.deadline` (checked *before* the reservation) | `fail PastDeadline` | `W₂`, unchanged: **no call, no charge** (added by `06`, problem #4) |
| ASKUNAFFORDABLE | `ĉ > W₂.pool` | `fail OverBudget` | `W₂`, unchanged: **no call, no charge** |
| ASKPROVIDERERROR | oracle `⇝ error(m)` after `δ` | `fail (ToolError m)` | `W₂ ⊕ ⟨price_μ(|ctx|_μ, 0), δ, t⟩` |
| ASKLATE | `W₂.now + δ > W₂.deadline` | `fail PastDeadline` | `W₂[pool −= ĉ, now := deadline]`: cancelled, charged the reservation |
| ASKINVALID | `validate_Θ(j, τ)` undefined | `fail (Invalid j)` | `W₂ ⊕ ⟨price_μ(|ctx|_μ, n), δ, t⟩` |

Read these rules the way Ramsey reads IFTRUE:

- **The oracle is consulted only after the reservation succeeds.** In
  ASKUNAFFORDABLE there is no oracle premise at all, so nothing is sent to
  the provider.
- **`ask` asks once.** No rule re-invokes the oracle (decision D3).
- **The charge uses the actual `n`, never more than the reservation.** Since
  `n ≤ max_out(μ, τ)`, the charge is at most `ĉ`. That inequality is the
  heart of Theorem 1 (§8).

#### CALL

```
⟨e, Θ, ρ, W₀⟩ ⇓ ⟨CAP(κ), W₁⟩     ⟨eᵢ, Θ, ρ, Wᵢ⟩ ⇓ ⟨vᵢ, Wᵢ₊₁⟩  for i = 1…n
ĉ = cost_κ(op) ≤ Wₙ₊₁.pool
Wₙ₊₁.hosts ⊢ ⟨κ, op, v₁…vₙ⟩ ⇝ ⟨ok v, H′, δ⟩     Wₙ₊₁.now + δ ≤ Wₙ₊₁.deadline
────────────────────────────────────────────────────────────────── (CALLOK)
⟨CALL(e, op, e₁…eₙ), Θ, ρ, W₀⟩ ⇓ ⟨v, Wₙ₊₁[hosts := H′] ⊕ ⟨ĉ, δ, t⟩⟩
```

CALLERROR (the host reports `err m`, giving `fail (ToolError m)`),
CALLEXPIRED, CALLUNAFFORDABLE and CALLLATE follow the ASK pattern.

#### FAIL and CATCH

```
⟨e, Θ, ρ, W⟩ ⇓ ⟨φ, W′⟩     φ is a value of datatype Failure
─────────────────────────────────────────── (FAIL)
⟨FAIL(e), Θ, ρ, W⟩ ⇓ ⟨fail φ, W′⟩

⟨e₁, Θ, ρ, W⟩ ⇓ ⟨v, W′⟩
─────────────────────────────────────── (CATCHOK)
⟨CATCH(e₁, x, e₂), Θ, ρ, W⟩ ⇓ ⟨v, W′⟩

⟨e₁, Θ, ρ, W⟩ ⇓ ⟨fail φ, W₁⟩    ⟨e₂, Θ, ρ{x ↦ φ}, W₁⟩ ⇓ ⟨r, W′⟩
───────────────────────────────────────────────────────────── (CATCHFAIL)
⟨CATCH(e₁, x, e₂), Θ, ρ, W⟩ ⇓ ⟨r, W′⟩
```

The handler runs in the world *after* the failure. So in a handler for
`PastDeadline`, `W₁.now` is already at the deadline, and any `ask` in the
handler fails too. No handler can buy back time, and none should.

#### BUDGET

```
⟨e_c, Θ, ρ, W⟩ ⇓ ⟨money(c), W₁⟩     ⟨e_t, Θ, ρ, W₁⟩ ⇓ ⟨dur(δ), W₂⟩
W₃ = W₂[pool := min(c, W₂.pool), deadline := min(W₂.deadline, W₂.now + δ)]
⟨e, Θ, ρ, W₃⟩ ⇓ ⟨r, W₄⟩
───────────────────────────────────────────────────────────────────── (BUDGET)
⟨BUDGET(e_c, e_t, e), Θ, ρ, W⟩ ⇓ ⟨r, W₄[pool := W₂.pool − (W₃.pool − W₄.pool),
                                        deadline := W₂.deadline]⟩
```

The body sees the smaller limits. The parent is charged exactly what the body
spent, `W₃.pool − W₄.pool`, and gets its own deadline back. The result `r`
passes through unchanged, so an `OverBudget` inside the scope is visible (and
catchable) outside it. That's the fallback idiom from `02` §C6.

#### WORKFLOW

```
nodes x₁ = e₁ … xₙ = eₙ, acyclic and distinct                (checked by the parser, §3.1)
for all i ≠ j:  reach(ρ|fv(eᵢ)) ∩ reach(ρ|fv(eⱼ)) = ∅         (ownership, 03 Q2)
t₀ = W.now;   order the nodes by ready time, ties broken by binding order:  π = i₁ … iₙ
for each i in π:
    readyᵢ  = max(t₀, max{ finishⱼ : xⱼ ∈ deps(xᵢ) })
    ⟨eᵢ, Θ, ρ{xⱼ ↦ vⱼ : xⱼ ∈ deps(xᵢ)}, W_prev[now := readyᵢ]⟩ ⇓ ⟨vᵢ, Wᵢ⟩
    finishᵢ = Wᵢ.now
⟨e, Θ, ρ{x₁ ↦ v₁ … xₙ ↦ vₙ}, W_last[now := max finishᵢ]⟩ ⇓ ⟨r, W′⟩
──────────────────────────────────────────────────────────────────── (WORKFLOW)
⟨WORKFLOW([x₁ = e₁ … xₙ = eₙ], e), Θ, ρ, W⟩ ⇓ ⟨r, W′⟩
```

Here `W_prev` is the world after the node before `i` in `π`, and `W_last` is
the world after the last node.

```
there exist i ≠ j with reach(ρ|fv(eᵢ)) ∩ reach(ρ|fv(eⱼ)) ≠ ∅
───────────────────────────────────────────────────────────────────────── (WORKFLOWSHARED)
⟨WORKFLOW(…), Θ, ρ, W⟩ ⇓ ⟨fail (Raised 'shared-stateful-capability), W⟩
```

**What the WORKFLOW rule gets exactly right.** This is the honesty section
that Ramsey's §1.7 insists on.

- **The value is exact.** Each node sees exactly the values of its
  dependencies. Ownership rules out interference through capabilities. Models
  are stateless. So the order `π` can't affect any `vᵢ`.
- **Time is exact.** Each node starts at its ready time, so the workflow
  finishes at `max finishᵢ`: the critical path, which is span.
- **Total spend is exact when every reservation succeeds.** Charges add up
  (work).

**What it approximates.** The pool is threaded through the nodes in `π` order.
In reality, charges from concurrent nodes interleave *event by event*. When the
pool is too tight for everyone, *which* node gets `OverBudget` depends on that
interleaving, and a big-step rule can't express interleaving. Ramsey's
glossary: "Small-step semantics can express more kinds of program behaviors
than big-step semantics." A precise account needs a small-step or
discrete-event semantics. **That semantics is now
[`07-small-step-semantics.md`](07-small-step-semantics.md).** By its Theorem 5,
this big-step rule agrees with it whenever no reservation is refused and no node
reads `(remaining)`. Two things hold anyway:

- In scripted mode, event order is fixed by virtual time with ties broken by
  workflow path, so the outcome is deterministic.
- Theorem 1 (the budget is never overdrawn) holds for *every* interleaving.

**Fail-fast (WORKFLOWFAIL, stated informally for the same reason).** If a
node yields `fail φ`, the node that fails *first in time* determines `φ`.
Nodes still running are cancelled. Each cancelled `ask` in flight is charged
its reservation, as in ASKLATE, and nodes not yet started are never run.

### 6.6 Definitions

```
⟨e, Θ, ρ, W⟩ ⇓ ⟨v, W′⟩
────────────────────────────────────────── (DEFINEGLOBAL)
⟨VAL(x, e), Θ, ρ, W⟩ → ⟨Θ, ρ{x ↦ v}, W′⟩

⟨e, Θ, ρ, W⟩ ⇓ ⟨v, W′⟩
──────────────────────────────────────────── (EVALEXP)
⟨EXP(e), Θ, ρ, W⟩ → ⟨Θ, ρ{it ↦ v}, W′⟩

ρ′ = ρ{f ↦ CLOSURE(⟨xs⟩, e, ρ′)}
─────────────────────────────────────────────────── (DEFINEFUNCTION)
⟨DEFINE(f, ⟨xs⟩, e), Θ, ρ, W⟩ → ⟨Θ, ρ′, W⟩

constructor names fresh in Θ     field types well formed in Θ
───────────────────────────────────────────────────────────── (DEFINEDATATYPE)
⟨DATATYPE(T, cons), Θ, ρ, W⟩ → ⟨Θ{T ↦ cons, Kᵢ ↦ T}, ρ, W⟩
                                        (DEFINERECORD is analogous)
```

If a top-level `val` or expression evaluates to `fail φ`, the interpreter
reports the failure and the binding isn't made, just as with Impcore's
`runerror`. The world keeps whatever was spent.

**Extended definitions aren't formalized.** These are `use`, `grant`,
`script`, `under` and the five `check-` forms. This follows Ramsey's sidebar
"True definitions and extended definitions": they're instructions to the
interpreter, and formalizing them "would distract us from what the semantics is
meant to do." Informally:

- `grant` binds a name in `ρ₀` to a `MODEL` or `CAP` the host provides.
- `script` defines a site-keyed oracle and host fixture.
- `under` runs its tests, each in a **fresh** world built from its
  configuration. Scripts restart for every test, so tests are independent.

---

## 7. Syntactic sugar and the initial basis

### 7.1 Sugar

Sugar is defined by translation. Ramsey (sidebar "What is syntactic sugar and
who benefits?"): new forms that are *just sugar* inherit every metatheorem
already proved about the core, with no new cases.

```
(let* () e)                        =  e
(let* ([x e₁] rest…) e)            =  LETX(x, e₁, (let* (rest…) e))
(begin)                            =  #f
(begin e₁ … eₙ)                    =  (let* ([_₁ e₁] … [_ₙ₋₁ eₙ₋₁]) eₙ)            ; _ᵢ fresh
(par e₁ e₂)                        =  (workflow ([a e₁] [b e₂]) (Pair [fst a] [snd b]))  ; a, b fresh
(and e₁ e₂)                        =  (if e₁ e₂ #f)
(or e₁ e₂)                         =  (if e₁ #t e₂)
(fail 'sym)                        =  (fail (Raised 'sym))
(ask e_m τ e_c)                    =  (ask e_m τ e_c '<file:line:col>)
(budget ([cost e]) body)           =  BUDGET(e, ∞, body)                           ; likewise for time only
```

`par` has to be sugar (decision F3). The fresh names `a` and `b` can't occur in
`e₁` or `e₂`, so neither branch depends on the other and both start at once.
`par` returns a `Pair` record rather than a list, so callers use
`(. p fst)`/`(. p snd)`.

### 7.2 Initial basis

**Predefined datatypes and records:**

```
(datatype Role    [System] [User] [Assistant] [Tool])
(record   Message (role Role) (content Text))
(datatype Failure [Invalid (raw Text)] [ToolError (message Text)]
                  [OverBudget] [PastDeadline] [Raised (reason Sym)])
(datatype Option  [None] [Some (value Any)])
(record   Pair    (fst Any) (snd Any))
(record   Resources (cost Any) (time Any))
```

`Any` is a placeholder until types exist. It makes `Option`, `Pair` and
`Resources` unaskable, which is correct: nothing should ask a model for a
`Pair` of arbitrary things.

**Primitives** (implemented by the interpreter):

- arithmetic `+ - * /` and comparisons `= < >` on Num, and on money and
  durations of the same kind
- `cons`, `list` (variadic), `string-append`, `string-length`, `println`
- `(remaining)`, which returns a `Resources` record for the current scope
  (`02` §C4)

**Predefined** (written in µNorman, like Ramsey's Figure 1.3):

```scheme
(define not (b)     (if b #f #t))
(define <= (x y)    (not (> x y)))
(define >= (x y)    (not (< x y)))
(define append (xs ys)
  (case xs ['() ys] [(cons z zs) (cons z (append zs ys))]))

;; retry: re-ask the same thunk; never retry budget or deadline failures   (02 §C6)
(define retry (n f)
  (if (<= n 0)                           ; a count ≤ 0 means "no retries" (06, problem #1)
      (f)
      (catch (f) err
        (case err
          [(Invalid _)   (retry (- n 1) f)]
          [(ToolError _) (retry (- n 1) f)]
          [_             (fail err)]))))

;; repair: like retry, but the failed reply and a correction go into the context.
;; Explicit context (D1) is what makes this an ordinary library function.
(define repair (n f ctx)
  (if (<= n 0)
      (f ctx)
      (catch (f ctx) err
        (case err
          [(Invalid raw)
             (repair (- n 1) f
               (append ctx (list (Message [role Assistant] [content raw])
                                 (Message [role User]
                                          [content "That reply did not match the required format. Try again."]))))]
          [_ (fail err)]))))

;; best-of: MCPP's sampling width k, needing a verifier (02 §C6).
;; A recursive par gives dynamic fan-out without any new form.
(define attempt (f check)
  (catch (let* ([x (f)]) (if (check x) (Some x) None))
         err
         (case err [(Invalid _) None] [(ToolError _) None] [_ (fail err)])))

(define first-some (a b) (case a [(Some _) a] [None b]))

(define best-of* (k f check)             ; Peano on k, with base case 0 (06, problem #2)
  (if (<= k 0)
      None
      (let* ([p (par (attempt f check) (best-of* (- k 1) f check))])
        (first-some (. p fst) (. p snd)))))

(define best-of (k f check)
  (case (best-of* k f check)
    [(Some x) x]
    [None     (fail 'none-accepted)]))
```

`best-of` shows sugar and recursion together. `k` concurrent attempts come from
a recursive `par`. The workflow law makes their cost add up and their time the
maximum. `attempt` turns *recoverable* failures into `None`, so one bad sample
doesn't cancel its siblings, but it still lets `OverBudget` and `PastDeadline`
through.

---

## 8. Metatheory: claims to prove, and one that fails

Following Ramsey §1.7.3, each claim is to be proved by induction on the
structure of derivations, with one case per rule, using his six-step
template. Proof sketches are given where they're short.

**Theorem 1 (budget safety).** If `⟨e, Θ, ρ, W⟩ ⇓ ⟨r, W′⟩`, then
`W′.pool ≥ 0` and `W.pool − W′.pool` is the sum of the charges in the new trace
entries.

- *Case ASKOK.* The charge is `price_μ(|ctx|_μ, n)`. Since
  `n ≤ max_out(μ, τ)` and prices are non-negative, the charge is at most
  `ĉ ≤ W₂.pool`. So `W′.pool ≥ 0`. Our obligation is met.
- *Case ASKUNAFFORDABLE.* The world is unchanged. Our obligation is met.
- *Case BUDGET.* By the induction hypothesis on the body,
  `W₄.pool ≥ 0` and `W₃.pool − W₄.pool` is the body's spend. Since
  `W₃.pool ≤ W₂.pool`, the parent's new pool
  `W₂.pool − (W₃.pool − W₄.pool)` is at least `W₄.pool ≥ 0`. Our obligation is
  met.
- *Case WORKFLOW.* Each node's derivation is smaller. Apply the induction
  hypothesis in `π` order. Because reservations are atomic, the same argument
  covers any interleaving.

**Theorem 2 (no forged authority).** If `⟨e, Θ, ρ, W⟩ ⇓ ⟨v, W′⟩` and `CAP(κ)`
occurs in `v`, then `CAP(κ)` is reachable from `ρ|fv(e)`. The analogue is
Impcore's "evaluating an expression can't create a new variable."

The ASK case is the interesting one. `validate_Θ(j, τ)` produces only values
of askable types (§3.1, rule 5), and those never contain `CAP`. So a model
cannot hand the program authority, whatever text it emits.

**Theorem 3 (deadline safety).** No trace entry produced by ASKOK or CALLOK
finishes after the deadline of its scope. It follows directly from the premise
`now + δ ≤ deadline`, together with the fact that BUDGET only ever *shrinks*
the deadline.

**Theorem 4 (workflow determinism of value).** If the WORKFLOW rule derives
`⟨v, W′⟩`, then for *every* topological order `π′` of the nodes, the
sequentialization `(let* (π′-ordered bindings) e)` derives the same `v`,
**provided no node evaluates `(remaining)`**. The proof uses the ownership
premise and the statelessness of models. The `(remaining)` premise was missing
from the first draft. Stating the theorem as law W1 in `06` exposed that it's
false without it: a node that reads `(remaining)` observes how much its
siblings have spent, which depends on scheduling. See `06`, question Q-A. This is the
law from `02`, made into a theorem.

**A conjecture that fails** (the counterpart of Ramsey's "every variable in `e`
is defined"). *"A workflow's total spend doesn't depend on scheduling."* It's
true when every node succeeds. It fails under fail-fast. If node `a` fails at
t = 5 s, whether sibling `b` has already made its second `ask` depends on `b`'s
latencies. So the spend of a *failing* workflow depends on timing. As in
Ramsey's example, the failed proof teaches something: **cost is deterministic
on the success path and only bounded on the failure path.** Theorem 1 still
supplies that bound.

---

## 9. Correspondence between semantics and implementation

Compare Ramsey's Table 1.5. The interpreter will be written in Rust (`01`).

| Semantics | Concept | Interpreter (planned) |
|---|---|---|
| `d`, `e` | definition, expression | `enum Def`, `enum Exp` |
| `v`, `r` | value, result | `enum Value`, `Result<Value, Failure>` |
| `ρ`, `Θ` | environments | `Env`, `TypeEnv` |
| `W` | world | `struct World { pool, deadline, clock, hosts, oracle, trace }` |
| `⟨e,Θ,ρ,W⟩ ⇓ ⟨r,W′⟩` | expression evaluation | `eval(&Exp, &TypeEnv, &Env, &mut World) -> Result<Value, Failure>` |
| `⟨d,Θ,ρ,W⟩ → ⟨Θ′,ρ′,W′⟩` | definition evaluation | `evaldef(&Def, &mut TypeEnv, &mut Env, &mut World)` |
| `O ⊢ … ⇝ …` | oracle | `trait Oracle` (`Live`, `Scripted`) |
| `H ⊢ … ⇝ …` | capability hosts | `trait Host` |
| `validate_Θ(j, τ)` | schema validation | `validate(&Json, &Type, &TypeEnv)` |
| `max_out(μ, τ)` | output bound | `bound(&Type) -> Bytes` |
| `reach(v)` | ownership check | `stateful_caps(&Value) -> Set<CapId>` |
