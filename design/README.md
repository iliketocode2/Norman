# µNorman: a guide

µNorman is a small programming language for writing **AI agents**: programs
that ask language models questions, use tools, and decide what to do next based
on the answers. It's being designed the way Norman Ramsey designs languages in
*Programming Languages: Build, Prove, and Compare*. We write down precisely what
every construct means before implementing it, and we prove the properties we
promise.

This guide is the place to start. It explains the language in plain terms and
by example. The formal definitions, rules and proofs are in the numbered
documents. The [reading map](#reading-map) at the end says which one to open
for what.

> **Status:** designed, not yet implemented. The syntax, semantics and laws are
> written down, and the example programs and tests exist. The interpreter
> (in Rust) is the next step.

---

## Why a language, and not another agent library?

Today's agent frameworks are libraries on top of Python or TypeScript. The
things that matter most in an agent live in those libraries as *conventions*,
and nothing checks that they're followed:

- how much it may spend
- how long it may run
- which tools it may touch
- what shape the model's answer must have

Six papers shaped this design (reviewed in
[`00-reading-notes.md`](00-reading-notes.md)). Each points to something a
language can guarantee and a library can't:

| Problem | What the papers found | What µNorman does |
|---|---|---|
| Model answers have no reliable shape | Turn makes typed inference a language primitive | every `ask` names the **type** of answer it expects |
| Agents decide between *options* | CP-Agent's loop ends by choosing "run code" or "done" | answers can be **choices** (sum types), and programs branch on them |
| Cost and deadlines are hard limits | MCPP: success means finishing *within* budget and deadline | **money and time** are built in, and the budget can never be overspent |
| Latency is safety-critical | the control-theory paper: delays add up and can destabilize | one **shared clock** per budget scope, with deadlines everywhere |
| Failed steps shouldn't rerun everything | RSTD: retry only the step that failed | failures are **values**, and `retry` wraps one step |
| Agents shouldn't touch tools they weren't given | Turn: credentials as unforgeable handles | tools are **capabilities**; the model can never create one |

---

## A tour by example

µNorman looks like Ramsey's teaching languages (Scheme-style parentheses) with
a handful of new forms. Every example below is taken from the test files in
[`../examples/`](../examples/).

### 1. Ask a model for a typed answer

```scheme
(datatype Verdict
  [Buy  (reason (Text 400))]
  [Hold (reason (Text 400))]
  [Sell (reason (Text 400))])

(ask claude Verdict (list (Message [role User] [content filing])))
```

`ask` takes a model, the **type** of answer you want, and the **context** (the
list of messages the model sees). The answer comes back as a real `Verdict`
value, such as `(Sell "Margins are compressing.")`, or the `ask` fails.

- **Context is explicit.** It's an ordinary list that you build and pass.
  Nothing accumulates behind your back.
- **`ask` asks exactly once.** Trying again is a separate, visible decision
  (see `retry` below).

### 2. Failure is a value you can handle

```scheme
(define analyst (model filing)
  (catch (ask model Verdict (analyst-ctx filing))
         err
         (Hold "analysis unavailable")))
```

If the model's reply doesn't fit `Verdict`, or the provider is down, or the
money or time runs out, `ask` produces a **failure**. A failure is one of
`Invalid`, `ToolError`, `OverBudget`, `PastDeadline` or `Raised`, and `catch`
handles it. Agents fail often, so failure is part of the language, not a crash.

### 3. Budgets: money and time

```scheme
(budget ([cost $0.50] [time 10min])
  (react claude py task-context))
```

Everything inside gets at most $0.50 and 10 minutes. Nested budgets take the
smaller limit, and whatever the inside spends is charged to the outside.

**How the budget can never be overspent.** Before calling the model, `ask`
*reserves* the worst-case cost. That's the input size (known, because the
context is explicit) plus the largest answer the **type** allows. If the
reservation doesn't fit, the model is never called and nothing is charged.
Afterward, only the actual cost is kept.

This is why types carry size bounds. A `Verdict` with 400-byte reasons
reserves about $0.007. The same type with unlimited text reserves the model's
whole output limit, about $0.12, and a $0.05 budget refuses it.

### 4. An agent loop in ten lines

This is the CP-Agent design: ask the model for the next step, run any code it
writes, feed back the result, and stop when it says it's done.

```scheme
(datatype Step
  [Exec (code (Text 4000))]    ; "run this code and show me the output"
  [Done (code (Text 4000))])   ; "this is my final answer"

(define react (model py ctx)
  (case (ask model Step ctx)
    [(Exec code)
       (react model py
              (append ctx (list (Message [role Assistant] [content code])
                                (Message [role Tool]      [content (observe py code)]))))]
    [(Done code) code]))
```

The loop is just a `case` on the model's answer. The enclosing `budget`
guarantees it stops.

### 5. Doing things at the same time

```scheme
(workflow ([a (ask m (Text 20) (step-ctx "a"))]
           [b (ask m (Text 20) (step-ctx "b"))]
           [c (begin a b (ask m (Text 20) (step-ctx "c")))]   ; needs a and b
           [d (begin a   (ask m (Text 20) (step-ctx "d")))])  ; needs only a
  (list a b c d))
```

A `workflow` is a set of named steps. **Dependencies come from the names each
step mentions.** Any step whose inputs are ready runs immediately, alongside
the others. Here `d` starts the moment `a` finishes; it doesn't wait for `b`.

With step times of a = c = 10 s and b = d = 30 s, this takes **40 s**. Forcing
it into "run two, wait, run two" takes **60 s**. The cost is identical either
way: concurrency saves time, never money.

`par` is a shorthand for a two-step workflow: `(par e₁ e₂)`.

### 6. Tools are capabilities

```scheme
(grant py (kernel))                  ; the host hands the program a Python kernel
(call py exec "print(6 * 7)")        ; the only way to use it
```

Tools come only from the host, through `grant`. A model's answer can never
contain one, because tool handles aren't allowed in the types you can `ask`
for. So no model output can give a program authority it wasn't granted.

**Stateful tools have one owner at a time.** Two concurrent workflow steps
can't share a kernel, because the outcome would depend on timing. The program
is rejected before either step runs. To work in parallel, `fork` the kernel
first.

### 7. The rest is library, with laws

`retry`, `repair`, `best-of` and context policies are ordinary µNorman
functions, each defined by algebraic laws:

| Function | What it does |
|---|---|
| `(retry n f)` | tries again on bad output or tool errors, up to `n` more times. It **never** retries running out of money or time. |
| `(repair n f ctx)` | like `retry`, but shows the model its bad reply and asks it to fix it. It costs more per attempt, because the context grows. |
| `(best-of k f check)` | runs `k` attempts concurrently and keeps one that passes `check`. |
| `(window n ctx)` | keeps the system messages plus the last `n` others. Turn builds this into its VM; here it's a few lines. |

---

## What µNorman guarantees

These are stated as theorems in [`04`](04-formal-definition.md) §8 and
[`07`](07-small-step-semantics.md) §6. In plain words:

1. **The budget is never overspent,** even with many steps running at once.
2. **A model can't hand your program a tool.** Authority comes only from the
   host.
3. **No answer is accepted after the deadline.**
4. **Concurrency doesn't change answers.** A workflow gives the same result as
   running its steps one at a time. (The fine print: this holds for steps that
   don't read their own remaining budget.)
5. **Every model call and tool call is recorded** in a trace. That trace is what
   makes testing, replay and cost profiling possible.

## What it doesn't do (yet)

- **No static types yet.** Types appear only where `ask` needs them. A full
  type and effect checker is planned. It will turn several run-time checks into
  compile-time ones.
- **Models are not deterministic, and µNorman doesn't pretend they are.** It
  never merges two identical-looking `ask`s or caches answers automatically.
  Caching is an explicit tool.
- **A loop that never calls a model or tool isn't stopped by the budget.** Pure
  computation takes no time in the model. The interpreter will enforce a step
  limit instead.

## How testing works

Tests run against **scripts**: canned model replies and tool results, keyed by
where in the program each call happens, with a simulated clock. A script can
also inject faults: garbled replies, outages and slow answers. Tests are
therefore exact and repeatable:

```scheme
(script sell
  [analyst (reply "{\"tag\":\"Sell\",\"reason\":\"Margins are compressing.\"}")
           (out 20) (latency 2s)])

(under ([script sell] [cost $1.00] [time 1min])
  (check-expect (analyst claude "FY2025 10-K") (Sell "Margins are compressing."))
  (check-within (analyst claude "FY2025 10-K") ([cost $0.01] [time 2s])))
```

---

## How it was designed

The design follows the nine-step process from Ramsey's *Seven Lessons in
Program Design*. Here the "function" being designed is the language's
evaluator.

| Ramsey's step | Where |
|---|---|
| 1. Forms of data | [`01`](01-forms-of-data.md), [`02`](02-resources-and-concurrency.md), [`03`](03-resolving-open-questions.md), [`04`](04-formal-definition.md) |
| 2. Example inputs | [`01`](01-forms-of-data.md) (programs P1–P5) |
| 3. Names · 4. Contracts · 5. Example results | [`05`](05-steps-3-to-5.md), [`examples/step5-examples.nrm`](../examples/step5-examples.nrm) |
| 6. Algebraic laws | [`06`](06-algebraic-laws.md), [`examples/step6-laws.nrm`](../examples/step6-laws.nrm) |
| (the semantics the code will follow) | [`04`](04-formal-definition.md) (big-step), [`07`](07-small-step-semantics.md) (concurrency) |
| 7–8. Case analysis and code | next: the Rust interpreter |
| 9. Revisit tests | after that |

Twice, writing things down precisely caught real mistakes before any code
existed:

- **Writing tests** caught an ownership check that would have rejected every
  workflow.
- **Writing laws** caught five problems. Two were loops that could drain a whole
  budget, one was a false law, one was a charge for calls that could never
  succeed, and one was a theorem missing a condition.

Ramsey's point exactly: proofs are most useful when they fail.

---

## Reading map

| Document | What's in it | Read it if you want to… |
|---|---|---|
| [`00-reading-notes.md`](00-reading-notes.md) | critical reviews of the six papers, and the requirements they impose | know *why* each design choice was made |
| [`01-forms-of-data.md`](01-forms-of-data.md) | the first cut: forms, values, example programs | see the design's starting point |
| [`02-resources-and-concurrency.md`](02-resources-and-concurrency.md) | `par` explained; `workflow`; budgets as money + time; reservations | understand budgets and concurrency in depth |
| [`03-resolving-open-questions.md`](03-resolving-open-questions.md) | output bounds from types; tool ownership; test scripts; the shared clock | understand those four decisions |
| [`04-formal-definition.md`](04-formal-definition.md) | **the reference:** lexical rules, grammar, abstract syntax, big-step semantics, built-ins, theorems | look up exactly what a construct means |
| [`05-steps-3-to-5.md`](05-steps-3-to-5.md) | function names, contracts, test conventions, rule-by-rule test coverage | write or read tests |
| [`06-algebraic-laws.md`](06-algebraic-laws.md) | laws for every construct; what `==` means; tempting non-laws; two proofs | refactor, optimize, or reason about programs |
| [`07-small-step-semantics.md`](07-small-step-semantics.md) | the event-by-event machine for concurrency and shared budgets | implement the interpreter, or see who wins a budget race |

Where documents disagree, the later one wins. Each correction is also noted at
the place it corrects.

---

## Glossary

- **ask**: consult a model once, for an answer of a given type.
- **call**: use a tool (a capability) once.
- **capability**: an unforgeable handle to a tool, granted by the host. It's
  *stateful* (a Python kernel, a filesystem) or *stateless* (an HTTP GET
  client).
- **context**: the list of messages a model sees. It's always explicit.
- **reservation**: the worst-case cost of an `ask`, set aside before the call.
  Unused money is returned.
- **budget scope**: a region of the program with its own money limit and
  deadline, both at most its parent's.
- **site**: a name for a place in the program where an `ask` or `call` happens.
  Scripts and traces are organized by site.
- **script**: canned replies and faults used in tests, with a simulated clock.
- **trace**: the log of every `ask` and `call`: what was sent, what came back,
  what it cost, how long it took.
- **workflow**: named steps that run as soon as their inputs are ready.
- **work / span**: total cost (the sum over all steps) versus total time (the
  longest chain of dependent steps).
- **world**: everything outside the program that `ask` and `call` affect:
  money, clock, tool state, trace.
