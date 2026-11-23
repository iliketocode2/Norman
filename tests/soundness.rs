//! Step 6 of design/12 §9: Theorem 7 (type soundness), tested the way the
//! laws of design/06 are. Random programs are generated from a fixed seed;
//! every one the checker accepts is run, and it may reach only the run-time
//! errors §6 says remain dynamic. None of the errors the checker rules out
//! (an `if` on a number, applying a non-function, an unbound name, a shared
//! kernel, ...) may happen.
//!
//! The generator is type-directed, so most programs check, but a few
//! subterms get the wrong type on purpose, so some don't. Those are run too,
//! without the checker, and some of them must hit an error the checker rules
//! out. That is the must-fail idea again: it shows the generator reaches the
//! errors in question, so a passing run of the theorem means something.
//!
//! A failing case prints its seed and its program, which reproduces it.

use munorman::ast::Config;
use munorman::check::Checker;
use munorman::driver::Interp;
use std::path::Path;

/// Programs generated per run. Fixed seeds keep every run identical.
const CASES: u64 = 400;

/// SplitMix64, as in tests/properties.rs.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.range(0, xs.len() as i64 - 1) as usize]
    }
}

/// The grants and script every program shares. Script entries run out if a
/// program asks often enough, which is one of the errors that remain.
const PREAMBLE: &str = r#"
(grant claude (model))
(grant py (kernel [exec (Text 40)] [count Num]))
(script s
  [n (reply "7") (out 1) (latency 1s) (reply "-2") (out 1) (latency 1s) (reply "0") (out 1) (latency 1s)]
  [t (reply "\"ok\"") (out 1) (latency 1s) (reply "\"two words\"") (out 1) (latency 1s)]
  [py/exec  (result "ran") (latency 1s) (result "ran again") (latency 1s) (result "x") (latency 1s)]
  [py/count (result "3") (latency 1s) (result "0") (latency 1s) (result "12") (latency 1s)])
"#;

#[derive(Clone, Copy, PartialEq, Debug)]
enum T {
    Num,
    Bool,
    Text,
    Nums,
    OptNum,
    Money,
}

const TYPES: [T; 6] = [T::Num, T::Bool, T::Text, T::Nums, T::OptNum, T::Money];

struct Gen {
    r: Rng,
    env: Vec<(String, T)>,
    fresh: u32,
}

impl Gen {
    fn var(&mut self, t: T) -> Option<String> {
        let found: Vec<String> = self.env.iter().filter(|(_, u)| *u == t).map(|(x, _)| x.clone()).collect();
        if found.is_empty() || self.r.chance(50) {
            return None;
        }
        Some(found[self.r.range(0, found.len() as i64 - 1) as usize].clone())
    }

    fn name(&mut self) -> String {
        self.fresh += 1;
        format!("v{}", self.fresh)
    }

    /// Bind a fresh name of type `bound` around a body of type `t`.
    fn with_let(&mut self, t: T, bound: T, d: u32) -> String {
        let rhs = self.exp(bound, d - 1);
        let x = self.name();
        self.env.push((x.clone(), bound));
        let body = self.exp(t, d - 1);
        self.env.pop();
        format!("(let* ([{} {}]) {})", x, rhs, body)
    }

    fn leaf(&mut self, t: T) -> String {
        if let Some(x) = self.var(t) {
            return x;
        }
        if self.r.chance(3) {
            return "(fail (Raised 'boom))".into();
        }
        match t {
            T::Num => self.r.range(-3, 9).to_string(),
            T::Bool => self.r.pick(&["#t", "#f"]).into(),
            T::Text => self.r.pick(&["\"hi\"", "\"\"", "\"héllo\"", "\"x = 1\""]).into(),
            T::Nums => self.r.pick(&["'()", "(list 1 2)", "(list 0)"]).into(),
            T::OptNum => self.r.pick(&["None", "(Some 4)", "(Some 0)"]).into(),
            T::Money => self.r.pick(&["$0.10", "$1.00", "$0"]).into(),
        }
    }

    /// An expression of type `t`, except that now and then it deliberately
    /// isn't: that is how some programs come out ill-typed.
    fn exp(&mut self, t: T, d: u32) -> String {
        let t = if self.r.chance(4) { TYPES[self.r.range(0, 5) as usize] } else { t };
        if d == 0 || self.r.chance(20) {
            return self.leaf(t);
        }
        let any = TYPES[self.r.range(0, 5) as usize];
        // Forms that work at every type.
        match self.r.range(0, 10) {
            0 => {
                // Now and then the test isn't a Boolean.
                let ct = if self.r.chance(15) { any } else { T::Bool };
                let (c, a, b) = (self.exp(ct, d - 1), self.exp(t, d - 1), self.exp(t, d - 1));
                return format!("(if {} {} {})", c, a, b);
            }
            1 => return self.with_let(t, any, d),
            2 => {
                let (a, h) = (self.exp(t, d - 1), self.exp(t, d - 1));
                return format!("(catch {} e {})", a, h);
            }
            3 => {
                let x = self.name();
                self.env.push((x.clone(), any));
                let body = self.exp(t, d - 1);
                self.env.pop();
                let arg = self.exp(any, d - 1);
                return format!("((lambda ({}) {}) {})", x, body, arg);
            }
            4 => {
                let (a, b) = (self.exp(t, d - 1), self.exp(any, d - 1));
                let side = if self.r.chance(50) { "fst" } else { "snd" };
                return if side == "fst" {
                    format!("(. (par {} {}) fst)", a, b)
                } else {
                    format!("(. (par {} {}) snd)", b, a)
                };
            }
            5 => return format!("(budget ([time 1min]) {})", self.exp(t, d - 1)),
            // A thunk, called; now and then something that isn't a function.
            6 => {
                return if self.r.chance(85) {
                    format!("((lambda () {}))", self.exp(t, d - 1))
                } else {
                    format!("({} {})", self.leaf(any), self.leaf(any))
                };
            }
            _ => {}
        }
        match t {
            T::Num => match self.r.range(0, 9) {
                0 => format!("(+ {} {})", self.exp(T::Num, d - 1), self.exp(T::Num, d - 1)),
                1 => format!("(* {} {})", self.exp(T::Num, d - 1), self.exp(T::Num, d - 1)),
                2 => format!("(/ {} {})", self.exp(T::Num, d - 1), self.exp(T::Num, d - 1)),
                3 => format!("(length {})", self.exp(T::Nums, d - 1)),
                4 => format!("(string-length {})", self.exp(T::Text, d - 1)),
                5 => {
                    let (o, z) = (self.exp(T::OptNum, d - 1), self.exp(T::Num, d - 1));
                    format!("(case {} [(Some v) (+ v 1)] [None {}])", o, z)
                }
                6 => {
                    // Sometimes missing a branch: a warning, and a run-time
                    // error that remains.
                    let xs = self.exp(T::Nums, d - 1);
                    if self.r.chance(30) {
                        format!("(case {} [(cons h _) h])", xs)
                    } else {
                        format!("(case {} [(cons h _) h] ['() 0])", xs)
                    }
                }
                7 => "(ask claude Num (list) 'n)".into(),
                8 => format!("(call py count {})", self.exp(T::Text, d - 1)),
                _ => format!("(retry 1 (lambda () {}))", self.exp(T::Num, d - 1)),
            },
            T::Bool => match self.r.range(0, 4) {
                0 => format!("(< {} {})", self.exp(T::Num, d - 1), self.exp(T::Num, d - 1)),
                1 => format!("(= {} {})", self.exp(any, d - 1), self.exp(any, d - 1)),
                2 => format!("(not {})", self.exp(T::Bool, d - 1)),
                3 => format!("(string-contains? {} {})", self.exp(T::Text, d - 1), self.exp(T::Text, d - 1)),
                _ => format!("(<= {} {})", self.exp(T::Money, d - 1), self.exp(T::Money, d - 1)),
            },
            T::Text => match self.r.range(0, 6) {
                0 => format!("(show {})", self.exp(any, d - 1)),
                1 => format!("(string-append {} {})", self.exp(T::Text, d - 1), self.exp(T::Text, d - 1)),
                2 => format!(
                    "(substring {} {} {})",
                    self.exp(T::Text, d - 1),
                    self.exp(T::Num, d - 1),
                    self.exp(T::Num, d - 1)
                ),
                3 => format!("(call py exec {})", self.exp(T::Text, d - 1)),
                4 => "(ask claude (Text 20) (list) 't)".into(),
                // Two nodes on one kernel: rejected. A fork first: accepted.
                5 => format!("(. (par (call py exec {}) (call py exec \"b\")) fst)", self.exp(T::Text, d - 1)),
                _ => format!(
                    "(let* ([k (call py fork)]) (. (par (call py exec {}) (call k exec \"b\")) snd))",
                    self.exp(T::Text, d - 1)
                ),
            },
            T::Nums => match self.r.range(0, 4) {
                0 => format!("(cons {} {})", self.exp(T::Num, d - 1), self.exp(T::Nums, d - 1)),
                1 => format!("(list {} {})", self.exp(T::Num, d - 1), self.exp(T::Num, d - 1)),
                2 => format!("(append {} {})", self.exp(T::Nums, d - 1), self.exp(T::Nums, d - 1)),
                3 => format!("(filter (lambda (x) (> x {})) {})", self.exp(T::Num, d - 1), self.exp(T::Nums, d - 1)),
                _ => format!("(drop {} {})", self.exp(T::Num, d - 1), self.exp(T::Nums, d - 1)),
            },
            T::OptNum => match self.r.range(0, 2) {
                0 => format!("(Some {})", self.exp(T::Num, d - 1)),
                1 => format!("(first-some {} {})", self.exp(T::OptNum, d - 1), self.exp(T::OptNum, d - 1)),
                _ => format!("(case {} [(cons h _) (Some h)] ['() None])", self.exp(T::Nums, d - 1)),
            },
            T::Money => match self.r.range(0, 3) {
                0 => format!("(+ {} {})", self.exp(T::Money, d - 1), self.exp(T::Money, d - 1)),
                1 => format!("(* {} {})", self.exp(T::Money, d - 1), self.exp(T::Num, d - 1)),
                // Outside a budget this is ∞, and arithmetic on ∞ remains an error.
                2 => format!("(- (. (remaining) cost) {})", self.exp(T::Money, d - 1)),
                _ => "(. (remaining) cost)".to_string(),
            },
        }
    }
}

/// The run-time errors that remain for a checked program (design/12 §6).
const REMAINING: &[&str] = &[
    "division by zero",
    "arithmetic overflow",
    "substring",
    "string-truncate needs a length of zero or more",
    "no case branch matches",
    "the proposal",
    "script exhausted",
    "requests are in flight but the oracle has nothing to deliver",
    "step limit",
    "∞",
];

/// What running `src`'s `main` led to, if it was an error Theorem 7 rules out.
fn ruled_out(src: &str, check: bool) -> Option<String> {
    let mut i = Interp::new();
    i.verbose = false;
    i.check = check;
    let loaded = i.load_str(src, "case.nrm", Path::new(".")).expect("the case reads");
    assert_eq!(loaded.errors, 0, "the case didn't load:\n{}\n{}", src, i.transcript());
    let cfg = Config { script: Some("s".into()), cost: Some(5_000_000), time: Some(3_600_000) };
    match i.eval_source("(main)", &cfg) {
        Err(msg) if REMAINING.iter().any(|ok| msg.contains(ok)) => None,
        Err(msg) => Some(msg),
        Ok(o) if o.result.to_string().contains("shared-stateful-capability") => Some(o.result.to_string()),
        Ok(_) => None,
    }
}

fn program(seed: u64) -> String {
    let mut g = Gen { r: Rng::new(seed), env: vec![], fresh: 0 };
    let t = TYPES[g.r.range(0, 5) as usize];
    let depth = g.r.range(2, 5) as u32;
    format!("{}(define main () {})\n", PREAMBLE, g.exp(t, depth))
}

#[test]
fn theorem_7_well_typed_programs_never_reach_a_ruled_out_error() {
    let (mut accepted, mut rejected, mut reached) = (0, 0, 0);
    for seed in 0..CASES {
        let src = program(seed);
        if Checker::new().check_str(&src, "case.nrm", Path::new(".")).ok() {
            accepted += 1;
            if let Some(err) = ruled_out(&src, true) {
                panic!("Theorem 7 fails for seed {}: a program that checks reached {}\n{}", seed, err, src);
            }
        } else {
            rejected += 1;
            if ruled_out(&src, false).is_some() {
                reached += 1;
            }
        }
    }
    // The generator has to be good for something both ways.
    assert!(
        accepted >= CASES as usize / 2,
        "only {} of {} programs checked: the generator is too loose",
        accepted,
        CASES
    );
    assert!(rejected >= CASES as usize / 10, "only {} programs were ill-typed: the generator is too tame", rejected);
    assert!(
        reached >= 10,
        "only {} ill-typed programs reached an error the checker rules out: the generator is too weak",
        reached
    );
    eprintln!(
        "{} checked and ran clean; {} rejected, of which {} reached a ruled-out error",
        accepted, rejected, reached
    );
}
