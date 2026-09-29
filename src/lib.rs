//! µNorman: a definitional interpreter.
//!
//! The modules follow the formal definition in `design/04-formal-definition.md`
//! and the machine in `design/07-small-step-semantics.md`:
//!
//! | Module    | Implements                                             |
//! |-----------|--------------------------------------------------------|
//! | `lexer`   | lexical structure (04 §1), S-expressions               |
//! | `parser`  | concrete → abstract syntax, sugar, well-formedness (§2, §3, §7.1) |
//! | `ast`     | abstract syntax (§3), free variables (Figure 1)        |
//! | `value`   | values (§4) and environments                           |
//! | `types`   | Θ, `askable`, `bound(τ)`, `validate_Θ(j, τ)`           |
//! | `host`    | scripted oracle and capability hosts (05, Step 5)      |
//! | `machine` | the µNorman machine (07): threads, scopes, requests, clock |
//! | `driver`  | definitions (§6.6), extended definitions, unit tests   |

pub mod ast;
pub mod defaults;
pub mod driver;
pub mod host;
pub mod lexer;
pub mod machine;
pub mod parser;
pub mod types;
pub mod value;
