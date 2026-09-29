# Norman
Mr. Shannon, meet Mr. Ramsey

**µNorman** is a small programming language for AI agents. It has typed model
calls, budgets in money and time, capabilities for tools, and dataflow
concurrency. It's designed in the style of Norman Ramsey's *Programming
Languages: Build, Prove, and Compare*, and implemented as a definitional
interpreter in Rust.

Start with the guide: [design/README.md](design/README.md).

```
cargo run -- examples/step5-examples.nrm    # load a program and run its unit tests
cargo test                                  # the design's example results and laws
```

## Bibliography

The source papers are in [`theory/`](theory/). Each one is reviewed critically
in [`design/00-reading-notes.md`](design/00-reading-notes.md), and the section
number there is given after each entry.

### Program design and programming-language theory

- Norman Ramsey. *Seven Lessons in Program Design.* Tufts University.
  [PDF](theory/Seven_Lessons_in_Program_Design.pdf). The nine-step design
  process the whole project follows.
- Norman Ramsey. "An Imperative Core." Chapter 1 of *Programming Languages:
  Build, Prove, and Compare.* Cambridge University Press, 2022.
  [PDF](theory/Ramsey%20-%20An%20Imperative%20Core.pdf). The model for µNorman's
  syntax, operational semantics, metatheory and definitional interpreter.

### Agentic systems

- Huanting Wang, Jingzhi Gong, Huawei Zhang, Jie Xu, and Zheng Wang. "AI
  Agentic Programming: A Survey of Techniques, Challenges, and Opportunities."
  arXiv:2508.11126v2, September 2025.
  [PDF](theory/AI%20Agentic%20Programming%20-%20A%20Survey.pdf). Reading notes §1.
- Muyukani Kizito. "Turn: A Language for Agentic Computation."
  arXiv:2603.08755v1, March 2026.
  [PDF](theory/Turn%20-%20A%20language%20for%20Agentic%20Computation.pdf).
  Reading notes §2.
- Stefan Szeider. "CP-Agent: Agentic Constraint Programming." In *3rd
  International Workshop on Large Language Models for Code (LLM4Code '26)*,
  Rio de Janeiro, 2026. doi:10.1145/3786181.3788711.
  [PDF](theory/CP%20Agent.pdf). Reading notes §3.
- Xinglin Wang, Zishen Liu, Shaoxiong Feng, Peiwen Yuan, Yiwei Li, Jiayi Shi,
  Yueqi Zhang, Chuyi Tan, Ji Zhang, Boyuan Pan, Yao Hu, and Kan Li. "On Time,
  Within Budget: Constraint-Driven Online Resource Allocation for Agentic
  Workflows." arXiv:2605.06110v2, May 2026.
  [PDF](theory/Constraint-Driven%20Online%20Resource%20Allocation%20for%20Agentic%20Workflows.pdf).
  Reading notes §4.
- Ali Eslami and Jiangbo Yu. "A Control-Theoretic Foundation for Agentic
  Systems." arXiv:2603.10779v4, March 2026.
  [PDF](theory/Control-Theoretic%20Foundation%20for%20Agentic%20Development.pdf).
  Reading notes §5.
- Shubhi Asthana, Bing Zhang, Chad DeLuca, Hima Patel, and Ruchi Mahindru.
  "Runtime-Structured Task Decomposition for Agentic Coding Systems." In *ACM
  Workshop on Agentic Software Engineering (ACM CAIS 2026)*, San Jose, 2026.
  arXiv:2605.15425v1.
  [PDF](theory/Runtime-Structured%20Task%20Decomposition%20Agentic%20Coding%20Systems.pdf).
  Reading notes §6.
