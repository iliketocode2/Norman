//! The read-eval-print loop for files: true definitions (04 §6.6), extended
//! definitions (`use`, `grant`, `script`, `under`), and unit tests. As in
//! Ramsey's interpreters, a file's unit tests run after the whole file is loaded.

use crate::ast::*;
use crate::lexer::read_all;
use crate::machine::{self, Outcome, Res, WorldConfig, match_pattern};
use crate::parser::Parser;
use crate::types::TypeEnv;
use crate::value::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Interp {
    pub theta: TypeEnv,
    pub globals: HashMap<Name, Value>,
    scripts: HashMap<Name, Rc<Script>>,
    next_cap: u64,
    /// Print each failing test's message (on by default).
    pub verbose: bool,
    /// When set, `ask` is answered by a real model (design/09); otherwise by
    /// scripts. Native only: wasm has no sockets, so there it is scripts alone.
    #[cfg(not(target_arch = "wasm32"))]
    pub live: Option<crate::live::LiveConfig>,
    /// Print the trace of each top-level evaluation: one line per ask or call.
    pub trace: bool,
    /// Everything the interpreter reports, kept as a value rather than only
    /// written to the terminal. Natively it is echoed to stdout and stderr as
    /// well; on wasm, where there is no terminal, it is the only output there
    /// is, and the documentation playground displays it.
    out: RefCell<String>,
}

impl Interp {
    /// Report a line: to the transcript always, and to the terminal natively.
    fn say(&self, line: impl std::fmt::Display) {
        #[cfg(not(target_arch = "wasm32"))]
        println!("{}", line);
        let mut out = self.out.borrow_mut();
        out.push_str(&line.to_string());
        out.push('\n');
    }

    /// Report a problem: like `say`, but to stderr natively.
    fn complain(&self, line: impl std::fmt::Display) {
        #[cfg(not(target_arch = "wasm32"))]
        eprintln!("{}", line);
        let mut out = self.out.borrow_mut();
        out.push_str(&line.to_string());
        out.push('\n');
    }

    /// The transcript so far.
    pub fn transcript(&self) -> String {
        self.out.borrow().clone()
    }

    /// Forget the transcript, so the next load starts clean.
    pub fn clear_transcript(&self) {
        self.out.borrow_mut().clear();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Summary {
    pub passed: usize,
    pub total: usize,
}

impl Summary {
    pub fn all_passed(&self) -> bool {
        self.passed == self.total
    }
}

struct PendingTest {
    test: UnitTest,
    cfg: Config,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    /// A fresh interpreter with the initial basis loaded.
    pub fn new() -> Interp {
        let mut i = Interp {
            theta: TypeEnv::default(),
            globals: HashMap::new(),
            scripts: HashMap::new(),
            next_cap: 1,
            verbose: true,
            #[cfg(not(target_arch = "wasm32"))]
            live: None,
            trace: false,
            out: RefCell::new(String::new()),
        };
        for p in Prim::ALL {
            i.globals.insert(Rc::from(p.name()), Value::Prim(p));
        }
        let summary = i.load_str(include_str!("prelude.nrm"), "prelude.nrm", Path::new("."));
        assert!(summary.is_ok(), "the prelude failed to load: {:?}", summary.err());
        i
    }

    pub fn load_file(&mut self, path: &Path) -> Result<Summary, String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {}", path.display(), e))?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.load_str(&src, &name, &dir)
    }

    /// Read and evaluate every definition in `src`, then run its unit tests.
    pub fn load_str(&mut self, src: &str, file: &str, dir: &Path) -> Result<Summary, String> {
        let forms = read_all(src, file)?;
        let mut tests: Vec<PendingTest> = vec![];
        for sx in &forms {
            let top = Parser::new(&self.theta).top(sx);
            let top = match top {
                Ok(t) => t,
                Err(msg) => {
                    self.complain(format!("syntax error: {}", msg));
                    continue;
                }
            };
            match top {
                Top::Def(d) => {
                    if let Err(msg) = self.evaldef(d) {
                        self.complain(format!("{}: {}", sx.loc(), msg));
                    }
                }
                Top::Use(f) => match self.load_file(&dir.join(&f)) {
                    Ok(_) => {}
                    Err(msg) => self.complain(format!("{}: {}", sx.loc(), msg)),
                },
                Top::Grant(x, spec) => {
                    let v = match spec {
                        HostSpec::Model { id, in_price, out_price, ceiling, think } => {
                            Value::Model(Rc::new(ModelSpec {
                                name: x.clone(),
                                id,
                                think,
                                in_price,
                                out_price,
                                ceiling,
                            }))
                        }
                        HostSpec::Kernel(ops) => self.cap(&x, CapKind::Kernel, ops),
                        HostSpec::Filesystem(ops) => self.cap(&x, CapKind::Filesystem, ops),
                        HostSpec::Http(ops) => self.cap(&x, CapKind::Http, ops),
                    };
                    self.globals.insert(x, v);
                }
                Top::Script(name, s) => {
                    self.scripts.insert(name, Rc::new(s));
                }
                Top::Under(cfg, ts) => tests.extend(ts.into_iter().map(|test| PendingTest { test, cfg: cfg.clone() })),
                Top::Test(test) => tests.push(PendingTest { test, cfg: Config::default() }),
            }
        }
        Ok(self.run_tests(file, tests))
    }

    fn cap(&mut self, key: &Name, kind: CapKind, ops: crate::ast::Ops) -> Value {
        self.next_cap += 1;
        Value::Cap(Cap { id: self.next_cap, kind, key: key.clone(), ops })
    }

    /// The definition judgment ⟨d, Θ, ρ, W⟩ → ⟨Θ′, ρ′, W′⟩. Top-level
    /// expressions run with no script and no limits.
    fn evaldef(&mut self, d: Def) -> Result<(), String> {
        match d {
            Def::Val(x, e) => {
                let v = self.eval_top(e)?;
                self.globals.insert(x, v);
            }
            Def::Exp(e) => {
                // As in Ramsey's interpreters, a top-level expression's value is printed.
                let v = self.eval_top(e)?;
                self.say(&v);
                self.globals.insert(Rc::from("it"), v);
            }
            Def::Define(f, lambda) => {
                self.globals.insert(f, Value::Closure(Rc::new(Closure { lambda, env: Env::default() })));
            }
            Def::Datatype(t, cons) => self.theta.add_datatype(t, cons)?,
            Def::Record(r, fields) => self.theta.add_record(r, fields)?,
        }
        Ok(())
    }

    fn eval_top(&self, e: ExpRef) -> Result<Value, String> {
        match self.run_in(&Config::default(), &e)? {
            Outcome { result: Res::Val(v), .. } => Ok(v),
            Outcome { result: Res::Fail(phi), .. } => Err(format!("evaluation failed: (fail {})", phi)),
        }
    }

    /// Evaluate an expression in a fresh world built from a test configuration.
    /// In live mode, asks go to the model; the configuration's money and time
    /// limits still apply, and its script (if any) is not consulted.
    pub fn run_in(&self, cfg: &Config, e: &ExpRef) -> Result<Outcome, String> {
        let script = match &cfg.script {
            Some(name) => Some(self.scripts.get(name).cloned().ok_or_else(|| format!("unknown script {}", name))?),
            None => None,
        };
        let world = WorldConfig { cost: cfg.cost, time: cfg.time, script };
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = match &self.live {
            None => machine::run(&self.globals, &self.theta, &world, e.clone()),
            Some(live) => {
                let oracle = Box::new(crate::live::LiveOracle::new(&self.theta, live.clone()));
                machine::run_with(&self.globals, &self.theta, &world, oracle, e.clone())
            }
        }
        .map_err(|e| format!("run-time error: {}", e.0))?;
        #[cfg(target_arch = "wasm32")]
        let outcome = machine::run(&self.globals, &self.theta, &world, e.clone())
            .map_err(|e| format!("run-time error: {}", e.0))?;
        if self.trace {
            for line in trace_lines(&outcome) {
                self.say(line);
            }
        }
        Ok(outcome)
    }

    /// Evaluate `e` in live mode with real tool hosts.
    ///
    /// Separate from `run_in` because hosts are borrowed for the length of the
    /// run, and because a program that reaches a real side effect should say
    /// so at the call site rather than depend on interpreter state.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_live_with_hosts(
        &self,
        cfg: &Config,
        e: &ExpRef,
        live: crate::live::LiveConfig,
        hosts: &crate::tools::HostRegistry,
    ) -> Result<Outcome, String> {
        let world = WorldConfig { cost: cfg.cost, time: cfg.time, script: None };
        let oracle = Box::new(crate::live::LiveOracle::with_hosts(&self.theta, live, hosts));
        let outcome = machine::run_with(&self.globals, &self.theta, &world, oracle, e.clone())
            .map_err(|e| format!("run-time error: {}", e.0))?;
        if self.trace {
            for line in trace_lines(&outcome) {
                self.say(line);
            }
        }
        Ok(outcome)
    }

    /// Parse and evaluate one expression under `cfg`: for tests and embedding.
    pub fn eval_source(&self, src: &str, cfg: &Config) -> Result<Outcome, String> {
        let forms = read_all(src, "<source>")?;
        let [sx] = forms.as_slice() else {
            return Err(format!("expected exactly one expression, found {}", forms.len()));
        };
        let e = Parser::new(&self.theta).exp(sx)?;
        self.run_in(cfg, &e)
    }

    fn run_tests(&self, file: &str, tests: Vec<PendingTest>) -> Summary {
        let mut s = Summary { passed: 0, total: tests.len() };
        for t in &tests {
            match self.run_test(&t.test, &t.cfg) {
                Ok(()) => s.passed += 1,
                Err(msg) => {
                    if self.verbose {
                        self.complain(format!("{}: {}", t.test.loc, msg));
                    }
                }
            }
        }
        if s.total > 0 {
            match (s.passed, s.total) {
                (p, t) if p == t && t == 1 => self.say(format!("{}: The only test passed.", file)),
                (p, t) if p == t => self.say(format!("{}: All {} tests passed.", file, t)),
                (0, t) => self.say(format!("{}: All {} tests failed.", file, t)),
                (p, t) => self.say(format!("{}: {} of {} tests passed.", file, p, t)),
            }
        }
        s
    }

    fn run_test(&self, t: &UnitTest, cfg: &Config) -> Result<(), String> {
        let text = |i: usize| t.texts.get(i).cloned().unwrap_or_default();
        match &t.kind {
            TestKind::Expect(e1, e2) => {
                let o1 = self.run_in(cfg, e1)?;
                let o2 = self.run_in(cfg, e2)?;
                let (Res::Val(v1), Res::Val(v2)) = (&o1.result, &o2.result) else {
                    return Err(format!(
                        "Check-expect failed: expected {} to evaluate to {}, but it's {}.",
                        text(0),
                        o2.result,
                        o1.result
                    ));
                };
                if v1.equal(v2) {
                    Ok(())
                } else {
                    Err(format!("Check-expect failed: expected {} to evaluate to {}, but it's {}.", text(0), v2, v1))
                }
            }
            TestKind::Assert(e) => match self.run_in(cfg, e)?.result {
                Res::Val(Value::Bool(true)) => Ok(()),
                other => Err(format!("Check-assert failed: {} evaluated to {}.", text(0), other)),
            },
            TestKind::Error(e) => match self.run_in(cfg, e) {
                Err(_) => Ok(()),
                Ok(o) => Err(format!(
                    "Check-error failed: evaluating {} was expected to cause a run-time error, but it produced {}.",
                    text(0),
                    o.result
                )),
            },
            TestKind::Fail(e, p) => match self.run_in(cfg, e)?.result {
                Res::Fail(phi) if match_pattern(p, &phi).is_some() => Ok(()),
                other => Err(format!(
                    "Check-fail failed: expected {} to fail matching {}, but it's {}.",
                    text(0),
                    text(1),
                    other
                )),
            },
            TestKind::Within(e, cost, time) => {
                let o = self.run_in(cfg, e)?;
                if let Some(c) = cost
                    && o.spent > *c
                {
                    return Err(format!(
                        "Check-within failed: {} spent {}, more than {}.",
                        text(0),
                        fmt_money(o.spent),
                        fmt_money(*c)
                    ));
                }
                if let Some(d) = time
                    && o.elapsed > *d
                {
                    return Err(format!(
                        "Check-within failed: {} took {}, longer than {}.",
                        text(0),
                        fmt_dur(o.elapsed),
                        fmt_dur(*d)
                    ));
                }
                Ok(())
            }
            TestKind::Equiv(e1, e2, grade) => {
                let a = self.run_in(cfg, e1)?;
                let b = self.run_in(cfg, e2)?;
                let describe = |o: &Outcome| {
                    format!(
                        "{} (spent {}, took {}, {} trace entries)",
                        o.result,
                        fmt_money(o.spent),
                        fmt_dur(o.elapsed),
                        o.trace.len()
                    )
                };
                let same_value = match (&a.result, &b.result) {
                    (Res::Val(x), Res::Val(y)) => x.equal(y),
                    (Res::Fail(_), Res::Fail(_)) => true,
                    _ => false,
                };
                let same_resources = a.result.equal(&b.result) && a.spent == b.spent && a.elapsed == b.elapsed;
                let same_trace = a.trace.len() == b.trace.len() && a.trace.iter().zip(&b.trace).all(|(x, y)| x.same(y));
                let ok = match grade {
                    Grade::Value => same_value,
                    Grade::Resource => same_resources,
                    Grade::Exact => same_resources && same_trace,
                };
                if ok {
                    Ok(())
                } else {
                    Err(format!(
                        "Check-equiv failed ({:?}): {} gave {}, but {} gave {}.",
                        grade,
                        text(0),
                        describe(&a),
                        text(1),
                        describe(&b)
                    ))
                }
            }
        }
    }
}

/// One line per ask or call: site, tokens, cost, when it started and ended, and how it ended.
fn trace_lines(o: &Outcome) -> Vec<String> {
    let mut lines = vec![];
    for t in &o.trace {
        lines.push(format!(
            "  trace: {} {:<12} in {:>6}  out {:>6}  {:>10}  {:>7} → {:<7} {}",
            t.kind,
            t.site,
            t.in_tokens,
            t.out_tokens,
            fmt_money(t.cost),
            fmt_dur(t.start),
            fmt_dur(t.end),
            t.outcome
        ));
    }
    if !o.trace.is_empty() {
        lines.push(format!("  total: spent {}, took {}", fmt_money(o.spent), fmt_dur(o.elapsed)));
    }
    lines
}
