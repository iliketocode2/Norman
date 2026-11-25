# Eval results

One file per run worth keeping: the `summary.md` a run wrote, with a short
hand-written reading on top (design/14 §4).

**Latest:** [after `13` step 3's first fixes](2026-11-25-step3.md). Every
baseline failure cause the fixes targeted is gone (0 of 457 trials), and the
re-run found a checker bug, now fixed. The Haiku runs are partial: the
account ran out of API credit.

## The baseline (2026-11-25)

norman 0.4.0, the reference at sha256 `33481c07dc0e`, 20 tasks × 3 trials,
up to 3 repair rounds, effort medium, adaptive thinking.

| model, condition | check@1 | pass@1 | pass@R | repair rounds | spend |
|---|---|---|---|---|---|
| [Sonnet 5.5, `ref`](2026-11-25-claude-sonnet-5-5-ref.md) | 95% | 95% | 100% | 0.05 | $0.43 |
| [Sonnet 5.5, `ref+examples`](2026-11-25-claude-sonnet-5-5-ref+examples.md) | 100% | 100% | 100% | 0.00 | $1.04 |
| [Haiku 5.5, `ref`](2026-11-25-claude-haiku-5-5-ref.md) | 92% | 93% | 100% | 0.10 | $0.06 |
| [Haiku 5.5, `ref+examples`](2026-11-25-claude-haiku-5-5-ref+examples.md) | 90% | 95% | 100% | 0.10 | $0.10 |

Spend is estimated from reported usage at list prices; the four runs came to
about $1.64.

### Reading

**Both models write µNorman well from the reference alone.** All 240 trials
pass in the end. Of the first programs, 226 pass the checker and their own
tests, and 230 pass the hidden tests. Every program that failed was fixed by
the interpreter's own message within two rounds. Haiku does almost as well
as Sonnet, at a seventh to a tenth of the cost, which by `14` §5 means the
reference is doing its job.

**None of the gates in `14` §5 fires.**

| Gate | Baseline | Fires? |
|---|---|---|
| unbound names over a quarter of first-round errors | none at all, apart from one knock-on error | no |
| wrong sites in more than 1 trial in 10 | none | no |
| syntax and nested-pattern failures common | 6 of 240 first programs had a syntax error: 5 are the site rule below, 1 an unbalanced parenthesis | no |
| pass@R on tiers A and B below 60% (Sonnet) | 100% | no |
| `ref+examples` beats `ref` by more than 15 points | +5 (Sonnet), +2 (Haiku) | no |
| Haiku close to Sonnet | 93–95% beside 95–100% pass@1 | **yes** |

**The corpus is at ceiling, and that's the main finding.** As written, it
can't steer `13`. Step 3's measure ("mean repair rounds fall from the
baseline") starts at 0.05, and step 4's ("tier A pass@R rises") starts at
100%. Neither number has room to move. The evidence for the prelude is also
thin: in 240 trials no model reached for a list function µNorman doesn't
have. Before the eval can judge the next steps, it needs tasks hard enough to
fail. See *What next* below.

### The failure causes

Fourteen first programs failed, out of 240. Four of them still passed the
hidden tests; they failed only their own. Sonnet's `ref` run had only one
cause, so this list pools all four runs. Most common first:

1. **A site passed as an argument (5: Sonnet 3, Haiku 2; every one in
   `E01`).** The natural refactor is a helper
   `(summarize model doc site)` with `(ask ... site)` inside it. µNorman
   requires the site to be a quoted symbol written at the `ask`. Three
   things make this worse than it needs to be:
   - The reference never says so. Its grammar shows `['site]`, and its prose
     calls the site "a name for this place in the program".
   - The parser's message, "the site of an ask is a quoted symbol", doesn't
     say why, or what to write instead.
   - The rejected `define` produces a second error, "unbound name summarize",
     at the use in `compare`, where nothing is wrong.

   Every trial fixed it in one round by inlining the `ask`s. Sonnet with the
   examples in front of it never made the mistake.
2. **Redefining the interface (4, all Haiku).** The task shows the grants
   and datatypes under "Already defined for you (don't redefine these)", and
   Haiku pastes them into its answer anyway. The checker rejects a
   redefined grant, and the model fixes it in one round. It does **not**
   reject a redefined record or datatype (`C04` redefined `Quote`), which is
   the gap already listed in `13` step 3.
3. **Mistakes in the model's own tests (4, Haiku).**
   - **Bare-text replies:** `(reply "v1")` where a `(Text n)` ask needs
     `(reply "\"v1\"")`. A reply is read as JSON, and the reference's script
     table says so for `answer` but not for `reply`.
   - **A test under a script that doesn't exist** (`[script none]`). It isn't
     caught until the test runs, though nothing about it needs running to
     find.
   - **A script that runs out of tool results.**
   - **An ill-typed test:** a `catch` whose handler returns a symbol where
     the body returns a list.

   In all four the program itself was right and passed the hidden tests.
4. **An unbalanced parenthesis (1, Haiku),** in a 45-line program.

That's all of them. Fourteen failures fall into four causes, so there is no
fifth to report. No first program failed the hidden tests for a reason of
its own: every program that failed them also failed the checker.

### What it points at

Cheap fixes, all in `13` step 3's territory (diagnostics and the reference):

- In the reference: say that a site is written literally at the `ask` (and
  why: scripts, traces and budgets are keyed by it before the program runs),
  and that a scripted `reply` is the model's raw output, read as JSON.
- In the parser's message for a non-literal site: say what to do instead.
  Drop the knock-on "unbound name" for a definition that failed to parse
  (§1.2's "no knock-on errors").
- In the checker: reject a test `under` a script that isn't defined, and
  reject a redefined datatype or record.

One language question for a design note, not a quick fix: should a helper be
able to take a site as an argument, say as a value of a `Site` type the
checker can still see statically? Every model reached for that helper
first. `13` §1.3 (stable site names) is the place for it.

### What next for the eval

The baseline shows that writing µNorman is not the bottleneck for these
models on tasks of this size. For the eval to steer anything, it needs at
least one of these:

- **A harder tier (G):** whole agents of 50–150 lines that combine tiers:
  a plan, a tool loop under a budget, a fallback model, a `consult` before
  anything irreversible, and tests the model must write itself.
- **No interface given:** the model writes the grants and datatypes from
  prose, which is what an agent writing its own limits has to do.
- **Stricter first-round metrics:** for example, a program's own tests must
  pass too, or measure at effort `low`.

Stage 3 (the Python comparison in `14` §6) can now be decided. µNorman's
pass rates are in range, so the open question is no longer whether models
can write it. The open question is whether writing it changes how often a
program breaks its own limits.
