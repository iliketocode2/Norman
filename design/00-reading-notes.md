# Reading notes: three papers, read against Ramsey

The yardstick is the *Seven Lessons* booklet. For each paper we ask four questions.
What does it say the forms of agent data are? Does it give semantics, and are
they well-formed? Does it state laws? What does it actually measure?

---

## 1. Wang et al., *AI Agentic Programming: A Survey* (arXiv 2508.11126, 2025)

**What it is.** A systematic literature review of LLM coding agents (152 papers),
with a taxonomy, lists of enablers, challenges and future directions.

**Useful for us**

- §5.5 and §6.6 ask for our project almost word for word: *"a structured language
  that developers can use to express their intent clearly … what the agent should
  do, what it must avoid, and what counts as a valid solution,"* checkable by
  "static analyzers or type checkers." That's a contract (Step 4) plus a type
  system (Lesson 5).
- §6.2 and §6.6 list runtime requirements that belong in the *semantics*, not
  in libraries: trace and replay, backtracking, interruption and resumption,
  undo, and audit trails.
- §6.2 proposes a memory hierarchy (short-term interaction, mid-term subgoals,
  long-term knowledge). That's a design hint for how context should be structured.

**Critique**

- **The taxonomy classifies behavior, not structure.** Reactive/proactive,
  single/multi-turn, static/adaptive: none of these axes can be tested by
  writing code, so they fail Ramsey's Step 1 requirement that forms be
  *distinguishable*. A taxonomy of forms would split on things like "does the
  loop terminate on a verifier or a budget?" and "is tool state persistent?"
- **The categories overlap and mix levels.** "Claude Opus 4-powered agent" is a
  model, not a system architecture. SWE-agent is described (§4.2.4) as a
  multi-agent Architect/Coder/Reviewer system. As far as I know, the original
  SWE-agent is a single agent built around an Agent-Computer Interface, so check
  the claim before relying on it.
- **Some tables are unsourced or garbled** (model sizes, the SWE-Bench task-type
  proportions in Table 7). Treat them as unreliable.
- **There is no semantics and there are no laws.** It maps the territory but
  gives us nothing to reason *with*. That gap is the opening for this project.

**Takeaway:** use it as a requirements list, not as design guidance.

---

## 2. Kizito, *Turn: A Language for Agentic Computation* (arXiv 2603.08755, 2026)

**What it is.** The closest prior art: a compiled, actor-based DSL with a Rust
bytecode VM. It has five constructs: typed `infer`, a `confidence` operator,
Erlang-style processes with structured context, capability-based `Identity`,
and compile-time schema import. We have to position this project against it.

**Worth taking**

- **Inference as a typed primitive.** `infer T { e }` generates a schema from a
  type and validates model output before binding it. This is right: the
  schema is the model's contract.
- **Object capabilities for authority** (Dennis & Van Horn, then Miller's E).
  Credentials are unforgeable handles that never enter program memory.
- **Isolated context per agent process,** so a child never inherits the
  parent's context by accident.
- **Durable suspend/resume** through a serializable VM state.

**Critique (Ramsey lens)**

1. **"Cognitive Type Safety" is a runtime check with a theorem attached.**
   Theorem 1 says: if `infer` completes, the value conforms to `T`. But the VM
   validates before it binds, so the theorem holds by construction, like saying
   `parse` returns a parse. Nothing is proven *statically*, because everything
   outside `infer` is dynamically typed ("targeted strictness"). A progress and
   preservation argument over a dynamically typed language doesn't show much.
2. **The operational rules aren't well-formed.** `Infer-Retry` tests
   `retries < k`, but `retries` doesn't appear in the configuration `⟨e, σ⟩`.
   The rule references state the judgment doesn't carry. `Infer-Ok` writes
   `llm(e, s) = j` as if the model were a *function*, which quietly assumes
   determinism. Lesson 1's "subtle mistake #1" applies: the right-hand side uses
   a variable the left-hand side never binds.
3. **The confidence algebra is two calculi glued together.** Arithmetic
   multiplies confidences (probability under an independence assumption),
   while `and` takes the minimum (Zadeh fuzzy logic). The paper doesn't
   justify combining them. When the provider exposes no logprobs, confidence
   defaults to 0.5, so `if confidence v < 0.7` *always* takes the fallback. And
   token log-probability is not calibrated semantic confidence.
4. **The experiments test the implementation against itself.** E2 checks that
   0.8 × 0.5 = 0.4. E4 checks that a HashMap is O(1). E5 checks that serde
   round-trips. In Lesson 2's terms, these test *algorithmic* laws against the
   algorithm. None measures the actual claim, that agents written in Turn are
   more reliable than framework agents. The "350+ lines in LangChain" figure is
   an estimate.
5. **"Bounded context" is bounded in entries, not tokens.** P1 = 100 entries
   and P2 = 200 entries, so 300 long entries can still overflow the window. A
   heuristic from one empirical paper (Lost in the Middle) is hard-coded into
   the semantics.
6. **There are no sum types.** Turn's only structured type is the struct
   (a product). But an agent's decision is a *choice* (call a tool, answer,
   give up). That's a sum, and case analysis on it is exactly Ramsey's Step 7.
   See our forms-of-data doc.
7. **Capabilities protect credentials, not effects in general.** Nothing
   statically states which effects a turn may perform, or tracks information
   flow from tool output into context.

**Takeaway:** keep typed inference, object capabilities and context isolation.
Differentiate on (a) static effect and capability types, (b) semantics that
treat the model as a nondeterministic oracle honestly, (c) algebraic laws with
replay-based testing, (d) sum types for model decisions, and (e) evaluation on
real agent tasks.

---

## 3. Szeider, *CP-Agent: Agentic Constraint Programming* (LLM4Code '26)

**What it is.** A ReAct loop with a persistent IPython kernel, three tools
(`python_exec`, `save_code`, optional `todo_write`) and a project prompt under
50 lines. It reports 101/101 on a corrected CP-Bench with Sonnet 4.5.

**What it teaches the language design**

- **The winning program is tiny.** A loop, a stateful tool, a termination
  signal and a short prompt. That's the first benchmark program our language
  must express, and it should take about 10 lines. See
  `01-forms-of-data.md`, example P1.
- **Correctness came from the symbolic checker, not the prompt.** The LLM
  proposes and the solver decides. The central construct is "propose, check,
  refine," and it should be *derivable* in the language, not bolted on.
- **More procedure in the prompt hurt.** The 800-line prompt did no better than
  the 50-line one. Imposed structure hurt too: the todo tool took hard-problem
  success from 52/60 to 45/60. So the language should **not** hard-wire
  planning scaffolds. It should make them optional and cheap to ablate. That's
  an argument for swappable effect handlers.
- **The termination signal is a form of data.** `save_code` exists to mark the
  end of the session. The model's output is a sum: `Exec code | Done code`.
- **Tool state persists** across calls (the kernel). The semantics needs a
  world/store component, not just environments.
- **Specifications mattered as much as the agent.** The author clarified 31
  problems, fixed 19 ground-truth models and added JSON output schemas to
  every problem. That's Ramsey's Step 4: if the contract isn't precise, you
  can't evaluate the code.

**Critique**

- The author edited the benchmark and then scored 100% on it. The edits may be
  justified, but they happened after seeing failures, so there's a risk of
  overfitting and there's no comparable baseline (acknowledged in §5.2).
- Three runs, no confidence intervals. The todo ablation used only Haiku. The
  prompt was tuned for Sonnet, which may explain Opus < Sonnet.
- "Temperature 0" still gave varying run times and outcomes, which shows
  nondeterminism is real even at T=0. Our semantics must not assume
  `ask` is a function.

---

## 4. Wang et al., *On Time, Within Budget: Constraint-Driven Online Resource Allocation for Agentic Workflows* (arXiv 2605.06110, 2026)

**What it is.** The workflow is fixed in advance as a DAG of subtasks. The
executor must decide, as execution proceeds, which model to use for each
*ready* subtask and how many parallel samples `k` to draw, subject to a hard
budget `B` (dollars) and a hard deadline `D` (wall-clock time). The objective is
`Pr(T_solve ≤ D ∧ C ≤ B)`, the probability that the *whole workflow* finishes
within both limits. The method, MCPP, is closed-loop Monte Carlo rollout
planning. At each state it simulates candidate actions followed by a portfolio
of simple continuation policies, executes the best one, observes the result and
replans.

**The formal model (§3.1), which transfers almost directly into our semantics**

- State `s = (S, b, h)`: completed subtasks, remaining budget, remaining time.
- Ready set `R(S) = {v ∉ S : Pred(v) ⊆ S}`. This is dataflow: a node may run as
  soon as its inputs exist.
- Cost of an action: `C(a) = Σ_v k_v · c_{v,m_v}`. **Costs add across concurrent work.**
- Duration of an action: `τ(a) = max_v τ_{v,m_v}(k_v)`. **Time takes the max
  across concurrent work.**
- An action that would exceed `b` or `h` has value 0. **Budget and deadline are
  success conditions, not soft penalties.**
- `k` parallel samples succeed with probability `q(k) = 1 − (1 − p)^k`.

The cost/duration pair is the **work/span** model from parallel cost semantics
(Blelloch & Greiner, NESL). Sequential composition adds both. Parallel
composition adds work and takes the max of span. That gives our resource
accounting a known algebraic basis.

**Worth taking**

- **Two resource dimensions, not one.** Money and time behave differently under
  concurrency: `par` costs the same but finishes sooner. A single "budget"
  number can't express that trade-off.
- **Hard constraints.** Going over either limit is failure. That gives a clean
  semantics (a failure form) and a clean objective.
- **Policies that depend on state beat static ones.** Uniform (static split)
  and Retry (a fixed local rule) both lose to a planner that looks at the
  remaining `(b, h)`. So programs, or their schedulers, need to be able to
  *observe* remaining resources.
- **Separate the workflow from its execution policy.** The paper takes the
  workflow as given and optimizes only how it's run: which model, how many
  samples. This matches the separation of algorithm and schedule in Halide.
  Programs should be parametric in models, so a scheduler can bind them.
- **Model diversity helps** (Fig. 3). Cheap models on easy nodes and strong
  models on bottleneck nodes beat any single model.

**Critique**

1. **The workflow must be a fixed, finite DAG** (Assumption 4). A ReAct loop
   isn't one: you don't know the number of steps in advance, and each step
   depends on the previous answer. MCPP doesn't apply to CP-Agent-style
   programs without unrolling them against a bound. Our language has to cover
   both static DAGs *and* dynamic loops.
2. **Cost counts only output tokens** (App. C.2: "according to official
   output-token prices"). In agent loops, input dominates: CP-Agent averaged
   37k input tokens to 3k output. The context is re-sent every step, so input
   cost grows with the number of steps. A cost model that ignores input
   underestimates exactly the workloads we care about. With explicit context,
   our `ask` knows `|ctx|` *before* the call, so input cost can be charged
   precisely.
3. **The independence assumption is optimistic.** `q(k) = 1 − (1 − p)^k`
   assumes the k samples are independent. Samples from one model on one prompt
   are correlated, so real gains from larger `k` will be smaller than
   predicted.
4. **Best-of-k silently requires a verifier.** "At least one sample succeeds"
   only helps if something can tell *which* one. ProofFlow uses Lean and
   CodeFlow uses tests. This backs up the CP-Agent lesson: the checker is
   central, so best-of-k has to take a `check` function.
5. **The evaluation is simulated.** Outcomes are drawn from offline empirical
   pools (512 samples per pair, gathered on 512 H800 GPUs), with idealized
   parallel latency. There's no live execution, and the numbers depend on
   profiles that cost a lot to collect.
6. **The baselines are weak.** Only Uniform and Retry are compared. None of
   the budget-aware methods cited in §2 (AgentTTS, EvoRoute, budget-aware
   routing) is used as a baseline.
7. **The theory is sound but modest.** The "safe improvement" result is the
   standard rollout-policy improvement bound. It guarantees improvement over
   the best base policy in the portfolio, not optimality. The authors say so.

**Takeaway:** resources are a *vector* (cost, time) with a work/span algebra,
limits are hard, workflows are dataflow graphs, and allocation policy is
separate from the workflow. The trace our semantics already produces is
exactly the profile data (per-site tokens, latency, success) that a planner
like MCPP needs.

---

## 5. Eslami & Yu, *A Control-Theoretic Foundation for Agentic Systems* (arXiv 2603.10779, 2026)

**What it is.** Agency is defined as **runtime decision authority** over parts
of a feedback-control architecture: controller parameters `θ`, controller
choice `σ`, workflow configuration `c` and goal `γ`. That gives a five-level
hierarchy:

- L1: fixed policy
- L2: internal adaptation and memory
- L3: tool and strategy switching
- L4: workflow reconfiguration
- L5: goal re-planning

Each level adds a richer class of dynamical system (time-varying, switched,
hybrid). The main result (Theorem 2, eq. 51) is a **stability budget**:
adaptation rate, total delay, switching frequency and reconfiguration
frequency all draw on one shared decay margin `λ`. Stability needs
`λ > κL_θ + L_d + μτ̄ + ln μ_σ/τ_a,σ + ln μ_c/τ_a,c`.

**Useful for us**

- **Agency as authority over decision variables.** That maps onto our syntax
  and suggests a static analysis:
  - A model's answer that only flows into *data* is L1/L2.
  - An answer that flows into a `case` that picks what to do next (strategy
    switching) is L3.
  - An answer that flows into *which workflow nodes run* is L4.
  - An answer that flows into the *context or goal of later asks* is L5.

  So agency level is an **information-flow property** of a program, something
  an effect system could compute and a policy could limit. (A future Lesson 5
  extension, noted here so it isn't lost.)
- **Delay is critical to stability, and it accumulates.** Total delay is the
  sum of inference, tool, adaptation and reconfiguration latencies (eq. 43).
  Latency is a correctness issue, not just a performance one. That supports
  modeling time as a first-class resource with a single shared deadline.
- **A shared budget needs a supervisor** (Remark 5). When several agents each
  control part of the budget, no single agent can check the global condition.
  Stability has to be kept through a *shared* budget or a supervisory
  mechanism. Our `budget` scope is exactly such a supervisor: one pool and one
  clock, shared by every concurrent branch inside it.
- **Hysteresis and dwell time** (Proposition 2). Flip-flopping between
  strategies can destabilize a loop even when each strategy is fine on its
  own. In agent terms, that's a ReAct loop oscillating between approaches.
  This suggests a library combinator later, not a core form.

**Critique**

1. **No LLM appears anywhere in the analysis or the experiments.** The
   simulations are second-order linear switched systems with made-up
   matrices. The paper borrows agent vocabulary but analyzes classical
   control.
2. **The defining feature of LLM agents is assumed away.** The
   semantic-ambiguity term `ε(t)` is "set to zero in this paper." The
   constants the theorems need (`L_θ`, `κ`, `μ`, Lyapunov functions) can't
   currently be estimated for a language model, which the authors admit
   (§V-D).
3. **The results are conservative sufficient conditions,** not a
   characterization. Most software agents have no physical plant `x(t)` and
   no obvious Lyapunov function, so "stability" needs a new meaning before
   these theorems apply. Candidates would be progress toward a verifier or
   non-oscillation.

**Takeaway:** keep the vocabulary (agency as authority, a shared budget under a
supervisor, delay as critical to stability, dwell time). Don't expect the
theorems to transfer.

---

## 6. Asthana et al., *Runtime-Structured Task Decomposition for Agentic Coding Systems* (arXiv 2605.15425, ACM CAIS 2026)

**What it is.** An architectural pattern (RSTD) with three parts:

1. A decomposition engine: deterministic code that controls branching.
2. Judgment operators: narrowly scoped LLM calls with schema-validated output.
3. A state manager: validated outputs stored by subtask key, with each
   downstream subtask getting only the context it needs.

It's compared against monolithic prompting and static decomposition on two
tasks (multi-file debugging and Kubernetes root-cause analysis), 10 runs each,
GPT-4 at T=0, built on the Mellea framework.

**Useful for us: RSTD is what our `workflow` + `retry` semantics already gives by construction**

- *"A failed subtask's output is not written to the State Manager and is never
  visible to downstream subtasks."* In `workflow`, a binding that fails is
  never bound, so downstream nodes never start. That's validation-gated state,
  for free.
- *"Retry only the failed subtask."* `retry` wraps one node's thunk, so it
  can't re-execute siblings or upstream nodes. It becomes a law, not a
  convention:
  `(workflow (… [x (retry n f)] …) body)`: failures of `f` re-run only `f`.
- *"Each subtask receives only predecessor outputs"* (bounded context). That's
  explicit context plus dataflow.
- Their RCA pipeline (Fig. 3) has a **skip arc** (triage feeds both subtasks 2
  and 3), so it's a DAG, not a chain. That's more evidence for `workflow` over
  `par`.
- *"Subtask-level monitoring … each subtask boundary is a natural
  instrumentation point."* This argues for **named ask sites** in the trace
  (see `03`).
- Their failures had to be **simulated**, because natural failure rates were
  0–2%. That's the case for fault injection in the scripted oracle (see `03`).

**Critique**

1. **By the paper's own numbers, RSTD never pays for itself in tokens.**
   Expected cost with a failure probability `f` of one failure per run:
   - RCA: monolithic `904 + 904f`, RSTD `2716 + 436f`. Break-even needs
     `f ≈ 3.9`, which is impossible for a probability.
   - Debugging: monolithic `703 + 703f`, RSTD `2225 + 460f`. Break-even needs
     `f ≈ 6.3`.

   Even if every run failed once (`f = 1`), RSTD would cost more (3152 vs.
   1808 tokens in RCA). The paper's headline "51.7% retry-cost reduction"
   compares only the retry component. Latency is also worse (23–29 s vs.
   10 s). The real arguments for the pattern are debuggability and
   isolation, not cost, and the paper doesn't measure those.
2. **The key finding holds by construction.** Retrying one subtask is cheaper
   than rerunning three downstream ones, and the experiment injects exactly
   one failure at a chosen subtask. The result is arithmetic, not an
   empirical discovery.
3. **The evaluation is small:** 2 hand-built scenarios, 10 runs, one model,
   100% correctness in every configuration. So it can't show any effect of
   decomposition on *correctness*.
4. **It branches on confidence thresholds,** with the same calibration problem
   as Turn's `confidence`.
5. **Mellea's retry appends the validation error** ("validation-and-repair"),
   which is `repair`, not `retry`. The paper merges the two. We keep them
   separate: `retry` re-asks the same context, and `repair` asks with the
   error added to the context.

**Takeaway:** the pattern is right and our semantics produces it by
construction. The paper's cost argument fails on its own data.

---

## Synthesis: requirements the six papers put on our design

| Requirement | Source | Where it lands in Ramsey's process |
|---|---|---|
| Model output has a declared shape | Turn, CP-Agent (JSON schemas) | Step 1 (forms) and Lesson 5 (typing rule for `ask`) |
| Model decisions are *choices* | CP-Agent (`Exec`/`Done`) | Step 1: sum types; Step 7: case analysis on oracle output |
| Model is opaque and nondeterministic | CP-Agent T=0 variance; Turn's ill-formed rule | Lesson 3: "all you can do with a model is ask it"; semantics uses a *relation* |
| Authority is explicit and unforgeable | Turn `Identity`, survey §5.4 | Capabilities as abstract values (Lesson 6); effect types |
| Loops must terminate | CP-Agent timeouts, survey cost model | Budget as the decreasing measure (Lesson 2: "some input gets smaller") |
| Verification drives refinement | CP-Agent solver feedback | Derived form `refine`, defined by laws |
| Tool state persists | CP-Agent kernel | World/store component in the evaluation judgment |
| Replay, audit, undo | Survey §6.2, §6.6 | Traces as a semantic output; effect handlers |
| Scaffolding must be optional | CP-Agent todo ablation | Handlers and library, not core forms |
| Resources are (cost, time) with hard limits | MCPP | Work/span cost semantics; `OverBudget` and `PastDeadline` failure forms |
| Workflows are DAGs; a node runs when its inputs are ready | MCPP ready set `R(S)` | Dataflow `workflow` form (see `02-resources-and-concurrency.md`) |
| Input tokens dominate cost in loops | CP-Agent token counts, a gap in MCPP | Explicit context means `ask` charges `|ctx|` before the call |
| Allocation policy is separate from the workflow | MCPP | Models are parameters; schedulers bind them |
| One shared budget and clock under a supervisor | Control paper, Remark 5 | `budget` scope = one pool + one absolute deadline for all branches |
| Delay is critical to stability | Control paper, eq. 43 and Prop. 1 | Output bounds come from types, giving per-ask latency bounds |
| Failed subtasks are invisible downstream; retry is local | RSTD | Already true of `workflow` + `retry`; stated as a law |
| Subtask boundaries are instrumentation points | RSTD | Named ask sites in the trace |
| Failures must be injectable to be tested | RSTD (0–2% natural rate) | Fault entries in the scripted oracle |
| Agency level = where model output flows | Control paper hierarchy | Future information-flow analysis in the effect system |
