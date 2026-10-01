# Contributing to µNorman

Thank you for your interest in contributing to µNorman. This document
describes how to get started.

µNorman is designed the way Norman Ramsey designs languages: the meaning of
every construct is written down (in [`design/`](design/)) before it's
implemented (in [`src/`](src/)), and tests are written before the code they
test. Contributions follow the same order. If you're new to the project,
read the guide first: [`design/README.md`](design/README.md).

## Quick Start

1. **Fork and clone** the repository.
2. **Install Rust** (stable, 1.85 or later, for the 2024 edition):
   - macOS / Linux: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
   - Windows: `winget install Rustlang.Rustup` (needs the Visual Studio C++ Build Tools for the linker)
3. **Build and test**:
   ```bash
   cargo build
   cargo test
   cargo run -- examples/step5-examples.nrm
   ```
   The last command loads a µNorman file and runs its unit tests. It should
   print `All 35 tests passed.`

No API key is needed for any of this. Models answer from scripts, and the live
client is tested against a local stub server. Only `--live` talks to a real
model (see [`design/09-live-oracle.md`](design/09-live-oracle.md)).

## Development Workflow

1. Create a branch from `main`.
2. **For any change to what µNorman programs mean, write the tests first.** Add
   `check-expect`, `check-fail`, `check-within` or `check-equiv` tests to a
   file in [`examples/`](examples/), and see them fail (Ramsey's Step 5).
   A new algebraic law in [`design/06`](design/06-algebraic-laws.md) gets an
   instance in [`examples/step6-laws.nrm`](examples/step6-laws.nrm) and, if it
   holds for all scripts, a random test in
   [`tests/properties.rs`](tests/properties.rs). A law that has only been
   tested on one hand-written script hasn't really been tested: W1 passed that
   way and was false.
3. Make your changes.
4. Run `cargo fmt` and `cargo clippy --all-targets` before committing.
5. Ensure all tests pass: `cargo test`.
6. Open a pull request.

## Code Style

- Follow the formatting in [`rustfmt.toml`](rustfmt.toml) (`cargo fmt`; 120
  columns).
- Address all Clippy warnings (`cargo clippy --all-targets` should print
  nothing).
- **Keep the code close to the semantics.** Each evaluation rule in
  [`design/04-formal-definition.md`](design/04-formal-definition.md) and
  [`design/07-small-step-semantics.md`](design/07-small-step-semantics.md) is
  one visible piece of code, commented with the rule's name (`// M-ASK-ISSUE`,
  `// CATCHFAIL`). A new rule gets a new arm, not a special case inside an old
  one.
- **Distinguish the three kinds of error**, as the semantics does:
  - a µNorman **failure** (`fail φ`: `Invalid`, `ToolError`, `Refused`,
    `OverBudget`, `PastDeadline`, `Raised`) is a *result* the program can
    `catch`;
  - a **checked run-time error** (`RtErr`) means the program is stuck, for
    example an unbound variable or a bad API key;
  - a Rust **panic** is only for internal invariants that can't fail (use
    `expect` with a message saying why).

  Library code returns `Result` and never panics on bad input.
- Errors are plain `String` messages, with a source location where there is
  one. There's no `anyhow` or `thiserror`. Keep dependencies few: every new
  crate needs a reason in the PR.
- Money is integer micro-dollars and time is integer milliseconds. Never use
  floats for either.

## Areas of Contribution

- **Design** ([`design/`](design/)): changes to syntax, semantics or laws need
  justification and discussion in an issue first. A design change must update
  every document it affects. When a later document corrects an earlier one,
  note the correction at the place it corrects, as the existing documents do.
  A new construct should be derivable (library code or syntactic sugar)
  unless it genuinely can't be. That's the design rule from
  [`design/01-forms-of-data.md`](design/01-forms-of-data.md).
- **Implementation** ([`src/`](src/)): bug fixes, performance, and features
  that the design documents already specify. The deferred items in
  [`design/09-live-oracle.md`](design/09-live-oracle.md) §6–7 are good places
  to start: streaming, asynchronous counting, real tool hosts, and prompt
  caching.
- **The library** ([`src/prelude.nrm`](src/prelude.nrm)): new predefined
  functions are written in µNorman, each with its algebraic laws stated in
  [`design/06-algebraic-laws.md`](design/06-algebraic-laws.md).
- **Documentation**: clarifications, examples, tutorials. The audience for
  [`design/README.md`](design/README.md) is programmers who know AI agents
  but not programming-language theory.
- **Tests**: coverage for edge cases and error paths. Every evaluation rule
  should have at least one example (see the coverage table in
  [`design/05-steps-3-to-5.md`](design/05-steps-3-to-5.md)). A new test
  should be able to fail: add a deliberately wrong version to
  [`tests/must-fail.nrm`](tests/must-fail.nrm) when it's not obvious that it
  can.

## Pull Request Process

1. Ensure formatting, Clippy and tests pass locally (`cargo fmt --check`,
   `cargo clippy --all-targets`, `cargo test`). There's no CI yet, so this is
   on you.
2. If your change is user-facing, update the documentation it affects: the
   guide, the relevant design document, and
   [`design/08-implementation-notes.md`](design/08-implementation-notes.md)
   if you add or remove a deviation from the specification.
3. Keep PRs focused; split large changes into smaller ones. A design change
   and its implementation can be separate PRs, with the design first.
4. **Never commit an API key.** Tests that call a real model must be opt-in
   (`#[ignore]`) and say what they cost.
5. Request review from maintainers.

By submitting a contribution, you agree that it's dual licensed under the MIT
and Apache-2.0 licenses, as described in [`README.md`](README.md#license).
Don't add third-party papers or other copyrighted material to
[`theory/`](theory/) unless its license allows redistribution; cite it in the
README's Bibliography instead.

## Questions?

Open an issue for discussion. We welcome contributions of all kinds: a
question that exposes an unclear rule is as valuable as a patch.
