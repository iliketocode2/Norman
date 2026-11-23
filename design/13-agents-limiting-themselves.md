# Agents limiting themselves: a plan

> **Status: plan, not built.** This document follows a critical review of the
> language as of 0.4.0. It reframes what µNorman is for, and orders the work
> that framing needs into phases. Each phase that changes the language gets
> its own design document first, as every earlier change did. Decisions marked
> **(open)** in §0 need the user's review before that phase starts.

## The use case

An AI agent, such as a coding agent, is about to do something open-ended: try
to fix a failing build, research a question, run an optimization. It knows
roughly what that's worth: *spend at most $2 and 15 minutes, touch only this
directory, call only these hosts, and ask me before deleting anything.* Today
it has nowhere to write those limits down where something will **enforce**
them. It relies on its own judgment mid-task, which is exactly when judgment
is least reliable.

µNorman could be that place. The agent writes a short program that states the
task's structure and its limits, hands it to `norman`, and gets back a
structured result: the answer, or a failure that says which limit stopped it
and what was spent. The program is the agent's commitment; the interpreter
holds it to it.

Three parties are involved, and the plan depends on keeping them apart:

| Party | Writes | Trusted? |
|---|---|---|
| **Operator** (the person running the agent) | a *policy*: the most any program may spend, and which hosts exist | yes |
| **Agent** (the program's author) | the program, including its own `budget`s and `grant`s | partly: it may limit itself further, never extend itself |
| **Model** (inside the program's `ask`s) | answers | no: it can't create authority (already a theorem) |

The language already proves the third row. Nothing yet enforces the second:
today a program grants itself whatever it names. That is the central gap.

### What this reframing changes

- **The author is a model.** S-expression syntax, verbosity and nested `case`
  matter less; precise error messages, a single short reference and
  machine-readable output matter more.
- **The program is short-lived.** Most will run once. Running the program
  must be as easy as running its tests is today.
- **Limits must mean something on the real machine.** A capability model
  whose only live tool is unsandboxed Python limits nothing.

---

## 0. Decisions

| # | Question | Proposal |
|---|---|---|
| A1 | How does an agent run a program? | **A CLI first** (`norman run`), with JSON in and JSON out. An MCP server (§7) wraps the same entry point later. |
| A2 | Who has the final say on limits? | **The operator's policy.** A program's budgets and grants are checked against it before the program starts, and the policy's ceiling also wraps the whole run. A program may only *attenuate*. |
| A3 | Is the Python kernel kept? | **Kept, but no longer the default tool.** Narrow hosts (§2) come first; Python runs only under a sandbox the operator names in the policy. **(open)**: which sandbox (container, `bwrap`, WASI Python) to support first. |
| A4 | Does the core language grow? | **As little as possible.** New list and string functions go in the prelude or as primitives; `par-map` is the only new form, and it's defined by translation where it can be. |
| A5 | Do we keep integer-only `Num`? | **Yes, for now.** Add fixed-point helpers in the prelude. **(open)**: a `Dec` type is a separate design question. |
| A6 | Other providers | **(open)**: an OpenAI-compatible provider is cheap to add behind the existing provider seam, but it isn't on the critical path for self-limiting. Deferred to §7. |

---

## Phase 0: make the documentation true (small, do first)

Agents read the reference ([`../docs/munorman-reference.md`](../docs/munorman-reference.md))
as context, so a false statement there becomes a wrong program.

1. Remove every claim that types are checked at run time or that the checker
   is planned: [`../docs/munorman-reference.md`](../docs/munorman-reference.md)
   §"What is checked before a program runs", [`README.md`](README.md) lines
   59–64 and the reading-map row for `12`, and their source in
   `docs/docs.html`.
2. Add a CI check that greps the generated reference and the guide for
   "planned type", "draft, not built" and similar phrases, so the drift can't
   return silently.
3. Move the agent-facing reference toward being **complete and short**: one
   page an agent can be given, with every form, every failure and one
   example. Design rationale stays in the numbered documents.

**Done when:** the reference and guide describe 0.4.0 exactly, and CI fails
on the known stale phrases.

> **Progress.** Items 1 and 2 are done. The site, the generated reference and
> the guide now say what the checker checks; `docs/verify.mjs` has a `STALE`
> list that fails CI if a known-false claim returns. It caught all ten in the
> files as they were. Item 3 is partly done: `consult`, `human` grants and
> the script entries that answer a consult were missing from the reference
> entirely, and are now there, with a runnable example `verify.mjs` checks.
> Still to do for item 3: document `(use file)`, and shorten the page by
> moving rationale (the "Why 3 and 4 differ" box, the reserved-words
> history) into the design documents.

---

## Phase 1: run a program, not just its tests

Today `norman FILE` prints top-level values and runs unit tests. There's no
entry point, no input and no machine-readable output.

### 1.1 The entry point

```
(define main (input) ...)                 ; input : an askable type, inferred
```

```
norman run FILE.nrm --input input.json [--cost $2] [--time 15min] [--policy P] [--live]
norman run FILE.nrm --input -              ; JSON on stdin
```

- `input.json` is validated against `main`'s parameter type exactly as an
  `ask` answer is (`validate`, `04` §3.1), so a malformed input is `Invalid`
  before anything is spent. The parameter's type must be askable; the checker
  already infers it.
- `--cost` and `--time` wrap `main` in a `budget`. They are capped by the
  policy (Phase 2).
- Unit tests do **not** run under `run`. `norman test FILE` keeps today's
  behaviour, and plain `norman FILE` stays an alias for it.

### 1.2 The result

`run` prints exactly one JSON object to stdout:

```json
{
  "outcome": "value",                       // or "failure" or "error"
  "value": { "tag": "Done", "answer": "5050" },
  "failure": null,                           // e.g. {"tag":"OverBudget"}
  "spent": { "cost": "$0.0031", "time": "6s" },
  "limits": { "cost": "$0.50", "time": "5min" },
  "trace": "run-2026-11-23T10-02-11.jsonl"
}
```

- `value` is `show`n, so it reads back (the `show` law, `04`).
- A **failure** is a value the program didn't catch; an **error** is a checked
  run-time error or a rejected program. They map to distinct exit codes (0,
  1, 2) so a caller needn't parse JSON to branch.
- Every diagnostic (parse, type, policy, run-time) has a JSON form under
  `--format json`: file, line, column, message, and for type errors the two
  types. An agent repairs its own program from these.

### 1.3 Stable site names

A default site is `file:line:col`, so editing a file silently re-keys every
unnamed `ask` and breaks the scripts that test it. Change the default to
**the enclosing top-level definition plus an ordinal**, `react/1`, which
survives edits elsewhere in the file. Warn when a script names a site that
no `ask`, `call` or `consult` has. (This is a change to the `ask` sugar in
`04` §"Syntactic sugar", so it needs a law check: sites appear in traces,
and `'exact` equivalence compares traces.)

**Done when:** `examples/cp-agent.nrm` runs live from `norman run` with the
task as JSON input and returns the result above; every diagnostic has a JSON
form.

---

## Phase 2: limits that mean something

### 2.1 The operator's policy

```toml
# norman-policy.toml
[ceiling]
cost = "$5.00"
time = "30min"

[models.claude]
id = "claude-sonnet-5"
in = 3
out = 15

[hosts.work]
kind = "filesystem"
root = "./sandbox"
ops = ["read", "write", "list"]

[hosts.web]
kind = "http"
allow = ["https://api.github.com/*", "https://docs.rs/*"]
ops = ["get"]

[humans.reviewer]
binding = "tty"
```

The rules, all checked **before the program starts**:

1. **Every `grant` must name a host the policy defines, and may only narrow
   it.** A program may grant `work` with `[read Text]` only; it may not add
   `delete`, or a root outside `./sandbox`. A grant not in the policy rejects
   the program.
2. **Prices come from the policy, not the program.** Today a grant states its
   own prices, and wrong prices make budget arithmetic wrong. An agent
   shouldn't be able to understate a price to fit a budget. A program's
   price, if given, must equal the policy's.
3. **The policy's ceiling wraps everything,** outside `--cost`/`--time` and
   outside every `budget` in the program. Nesting already takes the smaller
   limit, so no new semantics is needed: the ceiling is one more scope.
4. Without `--policy`, `run --live` refuses to start. Scripted runs need no
   policy.

This is the piece that makes "limiting themselves" true: the agent's limits
are inside the operator's, and both are enforced by the interpreter rather
than by the agent's restraint.

### 2.2 Narrow hosts

Add live hosts, in this order. Each declares its operations' result types
exactly as `typed-tools` grants do today, and each is stateless or owned as
`03` Q2 describes.

| Host | Ops | Enforced by the host |
|---|---|---|
| `filesystem` | `read`, `write`, `list`, `stat` | paths resolved and confined to `root` (no `..`, no symlink escape); a per-file byte bound from the grant |
| `http` | `get`, `post` | URL allowlist; response body bound; the request's own timeout is the scope's deadline |
| `command` | one op per allowed executable, e.g. `[cargo-test Text]` | an allowlist of exact argv prefixes, a working directory, an environment allowlist; no shell |
| `python` | `exec` | only under the sandbox the policy names (A3) |

`command` matters most for coding agents: "run the tests" is a capability,
"run anything" is not.

### 2.3 Approval gates

`consult` already exists. Add a policy option that **requires** a consult
before named operations (`write` on `work`, say): the host refuses those
calls unless the program's call site is dominated by a consult whose answer
is the call's argument. Whether that's checkable statically or must be a
run-time token is **(open)**; it gets its own design note.

**Done when:** a program that grants itself a host or operation outside the
policy is rejected before it runs; each new host has scripted tests and a
live test confined to a temp directory or a local HTTP server.

---

## Phase 3: a standard library you can write agents with

### 3.1 In the prelude, each with laws (`06` style)

| Function | Notes |
|---|---|
| `map`, `foldl`, `foldr`, `zip`, `reverse`, `take`, `nth`, `any?`, `all?` | missing today; `foldr` replaces hand-written folds like SAGA's `best` |
| `sort-by` | stable merge sort on a key function |
| `assoc`, `lookup`, `update` | an association list keyed by `Sym` or `Text`; enough for agents without a new type |
| `max-by`, `min-by` | answer `(Option a)` |

### 3.2 As primitives

| Primitive | Notes |
|---|---|
| `string-split`, `string-join`, `string-trim` | byte-safe: never split inside a UTF-8 sequence |
| `string->num` | answers `(Option Num)`; no exceptions |
| `num->string` | `show` already does this; added for readability |
| `json-parse` | `(json-parse τ text)`, validated like an `ask` answer, failing with `Invalid`. This is how a program reads a tool's JSON output at a type other than the grant's. |
| `(cost-of model ty ctx)` | the reservation an `ask` would make, without making it (§5) |

### 3.3 `par-map`

```
(par-map f xs)
```

Run `(f x)` for every `x` concurrently, under the enclosing scope, answering
with the list of results in order. It's the dynamic fan-out `workflow` can't
express. Semantically it's the recursive `par` tree `best-of*` builds by
hand, so it's defined by translation and inherits the concurrency theorem.
The checker applies the existing ownership rule: `f`'s effect may call no
stateful capability, or the program is rejected (`12` §5).

### 3.4 Recursive askable types

`json_schema` rejects recursive types today. Structured outputs accept
`$defs` and `$ref`. Investigate whether the provider accepts recursive
references, and what `bound` (`03` Q1) means for them: the reservation needs
a worst case, so a recursive type is askable only with a depth or size
bound, for example `(Tree a 50)` for at most 50 nodes. **(open)**: the bound's
syntax.

**Done when:** `saga-loop.nrm` and `cp-agent.nrm` are rewritten using these
functions and get shorter; each function has the laws and tests `06` would
require.

---

## Phase 4: record, replay and resume

Everything non-deterministic in a µNorman run happens at `ask`, `call` and
`consult`. Everything between them is a pure, deterministic machine (`07`).
So a log of what each effect answered is enough to re-run a program to
exactly the same point. That one observation gives three features.

1. **The journal.** `run` writes each completed effect to a JSONL file as it
   completes: site, ordinal, request, answer, usage, cost, time. This
   replaces today's printed `--trace`.
2. **Record to script.** `norman script-from JOURNAL` writes a `(script …)`
   form from a journal. A live run becomes a free, exact regression test, and
   nobody hand-escapes JSON inside strings again.
3. **Resume.** `norman run --resume JOURNAL` re-runs the program, answering
   each effect from the journal until the journal runs out, then continues
   live. A crash two hours into a SAGA run loses one effect, not two hours.
   Spending already recorded counts against the budget on resume, so
   resuming can't reset a limit.

Resume is sound only if the program hasn't changed. The journal records a
hash of the program and of the policy, and `--resume` refuses on mismatch.
Concurrency is the hard part: the journal must record the order completions
were *delivered*, and replay must deliver them in that order (`07` §4). That
interleaving is what needs a careful design note.

**Done when:** killing a live `cp-agent` run mid-loop and resuming it gives
the same answer as an uninterrupted scripted run with the same answers, and
total spend is the sum of both parts.

---

## Phase 5: budgets an agent can choose

The reservation rule is what makes "never overspent" a theorem, so it stays.
What changes is how much of it the author has to work out alone.

1. **Say why.** An `OverBudget` failure carries what was needed and what was
   available: `(OverBudget [needed $0.041] [available $0.03] [site react/1])`.
   This changes the `Failure` datatype, so `04`, the prelude and every
   `check-fail` pattern on `OverBudget` move together.
2. **Estimate before running.** `norman estimate FILE --input …` reports,
   for each `ask` site, the reservation of one call: schema tokens, worst-case
   answer, thinking allowance, price. With `cost-of` (§3.2), a program can
   also compute this itself and choose its fuel.
3. **Narrow the slack.** The cp-agent test reserves about 80 times what it
   spends. The largest parts are the default thinking allowance and unbounded
   text. Make `think` default to 0 unless the grant asks for it, and have the
   checker **warn** on an unbounded `Text` in an asked type, with the
   reservation it causes.
4. **Report both numbers.** The result (§1.2) and the journal show reserved
   and spent per site, so an agent learns what its budgets should be from its
   own runs.

**Done when:** an agent given only the reference and an `estimate` report can
pick a budget that runs `cp-agent` to completion on its first try.

---

## Phase 6: simplify

Each item removes something an author has to know.

| Today | Change | Cost |
|---|---|---|
| Flat patterns force nested `case` | **Nested patterns as sugar,** translated to nested flat `case`s, as `04` always allowed | parser and checker only; no new semantics |
| Constructor names are global (SAGA had to rename `Done`) | **Qualified names:** `Step.Done` always works; a bare `Done` works when unambiguous | checker change; old programs keep their meaning |
| `call` returning `Text` takes raw bytes, `ask` takes a JSON string | **Keep the rule, rename the mechanism:** a grant op declared `Raw` takes bytes; `Text` means a JSON string, as in `ask` | one rename across examples; removes the reference's "Why 3 and 4 differ" box |
| Byte offsets everywhere | Keep byte semantics, but add `string-take` and `string-drop` that never split a character, and make them the documented ones | primitives only |
| `check-equiv` with three grades | Keep it for the law suite; drop it from the agent reference | docs only |
| The type costs money twice, and authors must reason about it | §5's warnings and `estimate` carry this, so the reference can say it in two sentences | docs |

Not proposed: dropping S-expressions. Models write them reliably, and the
parser and every proof depend on them.

---

## Phase 7: integration (after the core is useful)

1. **An MCP server,** `norman mcp --policy P`, exposing `run`, `estimate`,
   `check` and `resume` as tools. A coding agent then limits a subtask by
   calling one tool, and the policy belongs to the person who configured the
   server, not to the agent. This is the most natural deployment for the use
   case, and it's thin once Phase 1 and 2 exist.
2. **MCP tools as hosts.** An MCP server the operator lists in the policy
   becomes a stateless host whose operations are its tools, typed from their
   input and output schemas.
3. **Another provider** (A6), behind the existing request-building seam in
   [`../src/anthropic.rs`](../src/anthropic.rs).
4. **Images and documents in context.** `Message.content` becomes a list of
   parts. This touches the reservation's token count, so it waits.

---

## Order and dependencies

```
Phase 0 ──► Phase 1 ──► Phase 2 ──► Phase 7.1 (MCP server)
                │
                ├──► Phase 3 (independent of 2; can run alongside)
                ├──► Phase 4 (needs 1's journal format)
                └──► Phase 5 (needs 1's result format; 5.2 needs 3's cost-of)
Phase 6 can be done any time; nested patterns are the most useful item.
```

The minimum useful version for an agent limiting itself is **Phases 0–2**:
a program it can run with input, whose limits the operator caps and the
interpreter enforces, over tools narrower than "arbitrary Python". Phases 3–5
make it pleasant; 7 makes it easy to adopt.

## What we're deliberately not doing

- **Probabilistic budgets** ("probably under $1"). The guarantee is the
  point; §5 shrinks the slack instead.
- **Letting a program raise its own ceiling** mid-run. A program that needs
  more can fail with `OverBudget` carrying the amount, and its caller decides.
- **A general-purpose language.** No mutation, no exceptions besides
  failures, no foreign-function interface. Every new effect is a host.
