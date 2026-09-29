//! The µNorman machine (design/07-small-step-semantics.md).
//!
//! A running program is a set of threads, a tree of budget scopes, a set of
//! in-flight requests and a clock. Each thread is a CEK-style state: a control
//! (`Ctl`), an environment, and a stack of continuation frames. Pure steps take
//! no virtual time. A thread runs until it issues an `ask` or `call` and
//! blocks. When no thread can move, the clock jumps to the earliest completion
//! (M-TIME).
//!
//! Rule names in comments refer to 04 §6 and 07 §4.

use crate::ast::*;
use crate::host::ScriptState;
use crate::types::TypeEnv;
use crate::value::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A checked run-time error: the machine is stuck (04 §6.4).
#[derive(Debug, Clone)]
pub struct RtErr(pub String);

pub type Rt<T> = Result<T, RtErr>;

fn stuck<T>(msg: impl Into<String>) -> Rt<T> {
    Err(RtErr(msg.into()))
}

/// Pure computation is bounded by a step limit, not by the budget (07 §7).
pub const STEP_LIMIT: u64 = 20_000_000;

/// The result `r` of an evaluation: a value, or a failure `fail φ`.
#[derive(Debug, Clone)]
pub enum Res {
    Val(Value),
    Fail(Value),
}

impl Res {
    pub fn equal(&self, other: &Res) -> bool {
        match (self, other) {
            (Res::Val(a), Res::Val(b)) | (Res::Fail(a), Res::Fail(b)) => a.equal(b),
            _ => false,
        }
    }
}

impl std::fmt::Display for Res {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Res::Val(v) => write!(f, "{}", v),
            Res::Fail(v) => write!(f, "(fail {})", v),
        }
    }
}

/// One entry of the trace: an `ask` or `call`, from issue to completion.
#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub site: String,
    pub path: Vec<u32>,
    pub kind: &'static str,
    pub in_tokens: i64,
    pub out_tokens: i64,
    pub cost: i64,
    pub start: i64,
    pub end: i64,
    pub outcome: String,
}

impl TraceEntry {
    /// Equality for 'exact equivalence. Paths are ignored, because the same
    /// computation may run at different workflow paths.
    pub fn same(&self, other: &TraceEntry) -> bool {
        self.site == other.site
            && self.kind == other.kind
            && self.cost == other.cost
            && self.start == other.start
            && self.end == other.end
            && self.outcome == other.outcome
    }
}

/// The outcome of running an expression to completion.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub result: Res,
    pub spent: i64,
    pub elapsed: i64,
    pub trace: Vec<TraceEntry>,
}

/// The initial world of a run.
pub struct WorldConfig {
    pub cost: Option<i64>,
    pub time: Option<i64>,
    pub script: Option<Rc<Script>>,
}

enum Ctl {
    Eval(ExpRef, Env),
    Ret(Value),
    Raise(Value),
    Idle,
}

/// Continuation frames: the evaluation contexts `E` of 07 §2, one per hole.
enum Frame {
    ConArgs { e: ExpRef, done: Vec<Value>, env: Env },
    RecordArgs { e: ExpRef, done: Vec<Value>, env: Env },
    Field(Name),
    If { e: ExpRef, env: Env },
    Let { e: ExpRef, env: Env },
    ApplyFn { e: ExpRef, env: Env },
    ApplyArgs { e: ExpRef, f: Value, done: Vec<Value>, env: Env },
    Case { e: ExpRef, env: Env },
    AskModel { e: ExpRef, env: Env },
    AskCtx { e: ExpRef, model: Rc<ModelSpec> },
    CallCap { e: ExpRef, env: Env },
    CallArgs { e: ExpRef, cap: Cap, done: Vec<Value>, env: Env },
    Fail,
    Catch { e: ExpRef, env: Env },
    BudgetCost { e: ExpRef, env: Env },
    BudgetTime { e: ExpRef, cost: Option<i64>, env: Env },
    /// SCOPE(σ, E): the thread is running inside budget scope σ.
    Scope(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Runnable,
    Blocked,
    Waiting,
    Done,
}

struct Thread {
    ctl: Ctl,
    k: Vec<Frame>,
    base_scope: usize,
    path: Vec<u32>,
    status: Status,
    /// The workflow node this thread evaluates, if any: (workflow, node index).
    node: Option<(usize, usize)>,
    /// The workflow this thread is waiting for, if any.
    waiting_on: Option<usize>,
    result: Option<Res>,
}

struct Scope {
    limit: Option<i64>,
    spent: i64,
    reserved: i64,
    deadline: Option<i64>,
    parent: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeState {
    Pending,
    Running(usize),
    Done,
}

struct Wf {
    parent: usize,
    def: Rc<WorkflowDef>,
    env: Env,
    scope: usize,
    state: Vec<NodeState>,
    values: Vec<Option<Value>>,
    remaining: usize,
    finished: bool,
}

enum ReqKind {
    Ask { model: Rc<ModelSpec>, ty: Type, in_tokens: i64, max_out: i64 },
    Call,
}

struct Request {
    rid: u64,
    tid: usize,
    site: String,
    reserve: i64,
    scope: usize,
    start: i64,
    due: i64,
    /// The reply would arrive after the deadline, so `due` was cut to it (ASKLATE).
    cut: bool,
    entry: Entry,
    kind: ReqKind,
}

pub struct Machine<'a> {
    globals: &'a HashMap<Name, Value>,
    theta: &'a TypeEnv,
    threads: Vec<Thread>,
    scopes: Vec<Scope>,
    wfs: Vec<Wf>,
    pending: Vec<Request>,
    next_rid: u64,
    next_cap: u64,
    now: i64,
    script: ScriptState,
    trace: Vec<TraceEntry>,
    steps: u64,
}

fn failure(name: &str, fields: Vec<Value>) -> Value {
    Value::con(name, fields)
}

/// Run `e` in a fresh world. This is how every definition and unit test is
/// evaluated.
pub fn run(globals: &HashMap<Name, Value>, theta: &TypeEnv, cfg: &WorldConfig, e: ExpRef) -> Rt<Outcome> {
    let mut m = Machine {
        globals,
        theta,
        threads: vec![],
        scopes: vec![Scope { limit: cfg.cost, spent: 0, reserved: 0, deadline: cfg.time, parent: None }],
        wfs: vec![],
        pending: vec![],
        next_rid: 0,
        next_cap: 1 << 40,
        now: 0,
        script: ScriptState::new(cfg.script.clone()),
        trace: vec![],
        steps: 0,
    };
    m.threads.push(Thread {
        ctl: Ctl::Eval(e, Env::default()),
        k: vec![],
        base_scope: 0,
        path: vec![],
        status: Status::Runnable,
        node: None,
        waiting_on: None,
        result: None,
    });
    let result = m.run_main()?;
    Ok(Outcome { result, spent: m.scopes[0].spent, elapsed: m.now, trace: m.trace })
}

impl<'a> Machine<'a> {
    // ------------------------------------------------------------ scheduler

    /// The scheduler policy of 07 §4.1: the runnable thread with the least path
    /// moves, and it runs until it blocks or finishes. When no thread can
    /// move, M-TIME advances the clock.
    fn run_main(&mut self) -> Rt<Res> {
        loop {
            if self.threads[0].status == Status::Done {
                return Ok(self.threads[0].result.clone().expect("main thread finished without a result"));
            }
            let pick = (0..self.threads.len())
                .filter(|&t| self.threads[t].status == Status::Runnable)
                .min_by(|&a, &b| self.threads[a].path.cmp(&self.threads[b].path));
            if let Some(t) = pick {
                while self.threads[t].status == Status::Runnable {
                    self.step(t)?;
                }
                continue;
            }
            if self.pending.is_empty() {
                return stuck("the machine is stuck: no thread can run and no request is in flight");
            }
            self.advance_time()?;
        }
    }

    /// M-TIME: jump to the earliest due time and complete every request due
    /// then, in issue order.
    fn advance_time(&mut self) -> Rt<()> {
        let due = self.pending.iter().map(|r| r.due).min().expect("pending is non-empty");
        self.now = due;
        let mut ready = vec![];
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].due == due {
                ready.push(self.pending.remove(i));
            } else {
                i += 1;
            }
        }
        ready.sort_by_key(|r| r.rid);
        for r in ready {
            self.complete(r)?;
        }
        Ok(())
    }

    fn step(&mut self, tid: usize) -> Rt<()> {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            return stuck(format!(
                "step limit of {} exceeded: a pure computation may not terminate (budgets bound effects, not pure loops)",
                STEP_LIMIT
            ));
        }
        match std::mem::replace(&mut self.threads[tid].ctl, Ctl::Idle) {
            Ctl::Eval(e, env) => self.eval(tid, e, env),
            Ctl::Ret(v) => self.ret(tid, v),
            Ctl::Raise(phi) => self.raise(tid, phi),
            Ctl::Idle => stuck("internal error: stepped an idle thread"),
        }
    }

    fn set(&mut self, tid: usize, c: Ctl) {
        self.threads[tid].ctl = c;
    }

    fn push(&mut self, tid: usize, f: Frame) {
        self.threads[tid].k.push(f);
    }

    fn raise_con(&mut self, tid: usize, name: &str, fields: Vec<Value>) {
        self.set(tid, Ctl::Raise(failure(name, fields)));
    }

    // ------------------------------------------------------------- scopes

    /// A thread's current scope: its innermost SCOPE frame, or its base scope.
    fn current_scope(&self, tid: usize) -> usize {
        let t = &self.threads[tid];
        t.k.iter()
            .rev()
            .find_map(|f| if let Frame::Scope(s) = f { Some(*s) } else { None })
            .unwrap_or(t.base_scope)
    }

    fn chain(&self, mut s: usize) -> Vec<usize> {
        let mut out = vec![s];
        while let Some(p) = self.scopes[s].parent {
            out.push(p);
            s = p;
        }
        out
    }

    /// available(σ) = min over the chain of (limit − spent − reserved).
    fn available(&self, s: usize) -> i64 {
        self.chain(s)
            .iter()
            .filter_map(|&c| self.scopes[c].limit.map(|l| l - self.scopes[c].spent - self.scopes[c].reserved))
            .min()
            .unwrap_or(i64::MAX)
    }

    /// deadline*(σ) = min over the chain of the deadlines.
    fn deadline_star(&self, s: usize) -> Option<i64> {
        self.chain(s).iter().filter_map(|&c| self.scopes[c].deadline).min()
    }

    fn reserve(&mut self, s: usize, amount: i64) {
        for c in self.chain(s) {
            self.scopes[c].reserved += amount;
        }
    }

    /// Release a reservation and charge the actual amount, on the whole chain.
    fn settle(&mut self, s: usize, reserved: i64, charge: i64) {
        for c in self.chain(s) {
            self.scopes[c].reserved -= reserved;
            self.scopes[c].spent += charge;
        }
    }

    // ---------------------------------------------------------- evaluation

    fn lookup(&self, env: &Env, x: &str) -> Option<Value> {
        env.lookup(x).cloned().or_else(|| self.globals.get(x).cloned())
    }

    fn eval(&mut self, tid: usize, e: ExpRef, env: Env) -> Rt<()> {
        match &*e {
            // LITERAL
            Exp::Literal(v) => self.set(tid, Ctl::Ret(v.clone())),
            // VAR
            Exp::Var(x) => match self.lookup(&env, x) {
                Some(v) => self.set(tid, Ctl::Ret(v)),
                None => return stuck(format!("unbound variable {}", x)),
            },
            // CON
            Exp::Con(k, args) => {
                let Some(def) = self.theta.con_def(k) else {
                    return stuck(format!("unknown constructor {}", k));
                };
                if def.fields.len() != args.len() {
                    return stuck(format!("constructor {} expects {} argument(s), got {}", k, def.fields.len(), args.len()));
                }
                if args.is_empty() {
                    self.set(tid, Ctl::Ret(Value::Con(k.clone(), Rc::new(vec![]))));
                } else {
                    let first = args[0].clone();
                    self.push(tid, Frame::ConArgs { e: e.clone(), done: vec![], env: env.clone() });
                    self.set(tid, Ctl::Eval(first, env));
                }
            }
            // RECORD
            Exp::Record(r, fields) => {
                if !self.theta.is_record(r) {
                    return stuck(format!("unknown record {}", r));
                }
                if fields.is_empty() {
                    return self.build_record(tid, &e, vec![]);
                }
                let first = fields[0].1.clone();
                self.push(tid, Frame::RecordArgs { e: e.clone(), done: vec![], env: env.clone() });
                self.set(tid, Ctl::Eval(first, env));
            }
            // FIELD
            Exp::Field(r, f) => {
                self.push(tid, Frame::Field(f.clone()));
                self.set(tid, Ctl::Eval(r.clone(), env));
            }
            // IFTRUE / IFFALSE
            Exp::If(c, ..) => {
                self.push(tid, Frame::If { e: e.clone(), env: env.clone() });
                self.set(tid, Ctl::Eval(c.clone(), env));
            }
            // LET
            Exp::Let(_, rhs, _) => {
                self.push(tid, Frame::Let { e: e.clone(), env: env.clone() });
                self.set(tid, Ctl::Eval(rhs.clone(), env));
            }
            // LAMBDA
            Exp::Lambda(l) => self.set(tid, Ctl::Ret(Value::Closure(Rc::new(Closure { lambda: l.clone(), env })))),
            // APPLYCLOSURE, APPLY-primitive
            Exp::Apply(f, _) => {
                self.push(tid, Frame::ApplyFn { e: e.clone(), env: env.clone() });
                self.set(tid, Ctl::Eval(f.clone(), env));
            }
            // CASE
            Exp::Case(scrutinee, _) => {
                self.push(tid, Frame::Case { e: e.clone(), env: env.clone() });
                self.set(tid, Ctl::Eval(scrutinee.clone(), env));
            }
            // ASK: model first, then context
            Exp::Ask { model, .. } => {
                self.push(tid, Frame::AskModel { e: e.clone(), env: env.clone() });
                self.set(tid, Ctl::Eval(model.clone(), env));
            }
            // CALL: capability first, then arguments
            Exp::Call { cap, .. } => {
                self.push(tid, Frame::CallCap { e: e.clone(), env: env.clone() });
                self.set(tid, Ctl::Eval(cap.clone(), env));
            }
            // FAIL
            Exp::Fail(x) => {
                self.push(tid, Frame::Fail);
                self.set(tid, Ctl::Eval(x.clone(), env));
            }
            // CATCHOK / CATCHFAIL
            Exp::Catch(body, ..) => {
                self.push(tid, Frame::Catch { e: e.clone(), env: env.clone() });
                self.set(tid, Ctl::Eval(body.clone(), env));
            }
            // BUDGET: cost, then time, then the body in a new scope
            Exp::Budget { cost, time, body } => {
                if let Some(c) = cost {
                    self.push(tid, Frame::BudgetCost { e: e.clone(), env: env.clone() });
                    self.set(tid, Ctl::Eval(c.clone(), env));
                } else if let Some(t) = time {
                    self.push(tid, Frame::BudgetTime { e: e.clone(), cost: None, env: env.clone() });
                    self.set(tid, Ctl::Eval(t.clone(), env));
                } else {
                    self.enter_scope(tid, None, None, body.clone(), env);
                }
            }
            // M-WF-START / M-WF-SHARED
            Exp::Workflow(w) => self.start_workflow(tid, w.clone(), env)?,
        }
        Ok(())
    }

    /// Return `v` to the innermost frame.
    fn ret(&mut self, tid: usize, v: Value) -> Rt<()> {
        let Some(frame) = self.threads[tid].k.pop() else {
            return self.finish(tid, Res::Val(v));
        };
        match frame {
            Frame::ConArgs { e, mut done, env } => {
                let Exp::Con(k, args) = &*e else { unreachable!() };
                done.push(v);
                if done.len() < args.len() {
                    let next = args[done.len()].clone();
                    self.push(tid, Frame::ConArgs { e: e.clone(), done, env: env.clone() });
                    self.set(tid, Ctl::Eval(next, env));
                } else {
                    self.set(tid, Ctl::Ret(Value::Con(k.clone(), Rc::new(done))));
                }
            }
            Frame::RecordArgs { e, mut done, env } => {
                let Exp::Record(_, fields) = &*e else { unreachable!() };
                done.push(v);
                if done.len() < fields.len() {
                    let next = fields[done.len()].1.clone();
                    self.push(tid, Frame::RecordArgs { e: e.clone(), done, env: env.clone() });
                    self.set(tid, Ctl::Eval(next, env));
                } else {
                    self.build_record(tid, &e, done)?;
                }
            }
            Frame::Field(f) => match &v {
                Value::Record(_, fs) => match fs.iter().find(|(g, _)| *g == f) {
                    Some((_, x)) => self.set(tid, Ctl::Ret(x.clone())),
                    None => return stuck(format!("{} has no field {}", v, f)),
                },
                _ => return stuck(format!("(. e {}) needs a record, but got {}", f, v)),
            },
            Frame::If { e, env } => {
                let Exp::If(_, t, f) = &*e else { unreachable!() };
                match v {
                    Value::Bool(true) => self.set(tid, Ctl::Eval(t.clone(), env)),
                    Value::Bool(false) => self.set(tid, Ctl::Eval(f.clone(), env)),
                    other => return stuck(format!("if needs a Boolean, but got {}", other)),
                }
            }
            Frame::Let { e, env } => {
                let Exp::Let(x, _, body) = &*e else { unreachable!() };
                self.set(tid, Ctl::Eval(body.clone(), env.extend(x.clone(), v)));
            }
            Frame::ApplyFn { e, env } => {
                let Exp::Apply(_, args) = &*e else { unreachable!() };
                if args.is_empty() {
                    self.apply(tid, v, vec![])?;
                } else {
                    let first = args[0].clone();
                    self.push(tid, Frame::ApplyArgs { e: e.clone(), f: v, done: vec![], env: env.clone() });
                    self.set(tid, Ctl::Eval(first, env));
                }
            }
            Frame::ApplyArgs { e, f, mut done, env } => {
                let Exp::Apply(_, args) = &*e else { unreachable!() };
                done.push(v);
                if done.len() < args.len() {
                    let next = args[done.len()].clone();
                    self.push(tid, Frame::ApplyArgs { e: e.clone(), f, done, env: env.clone() });
                    self.set(tid, Ctl::Eval(next, env));
                } else {
                    self.apply(tid, f, done)?;
                }
            }
            Frame::Case { e, env } => {
                let Exp::Case(_, branches) = &*e else { unreachable!() };
                for (p, body) in branches {
                    if let Some(binds) = match_pattern(p, &v) {
                        let env = binds.into_iter().fold(env.clone(), |env, (x, val)| env.extend(x, val));
                        self.set(tid, Ctl::Eval(body.clone(), env));
                        return Ok(());
                    }
                }
                return stuck(format!("no case branch matches {}", v));
            }
            Frame::AskModel { e, env } => {
                let Exp::Ask { ctx, .. } = &*e else { unreachable!() };
                match v {
                    Value::Model(model) => {
                        let ctx = ctx.clone();
                        self.push(tid, Frame::AskCtx { e: e.clone(), model });
                        self.set(tid, Ctl::Eval(ctx, env));
                    }
                    other => return stuck(format!("ask needs a model, but got {}", other)),
                }
            }
            Frame::AskCtx { e, model } => {
                let Exp::Ask { site, ty, .. } = &*e else { unreachable!() };
                self.issue_ask(tid, site, model, ty, &v)?;
            }
            Frame::CallCap { e, env } => {
                let Exp::Call { op, args, .. } = &*e else { unreachable!() };
                let Value::Cap(cap) = v else {
                    return stuck(format!("call needs a capability, but got {}", v));
                };
                if args.is_empty() {
                    self.issue_call(tid, cap, op, vec![])?;
                } else {
                    let first = args[0].clone();
                    self.push(tid, Frame::CallArgs { e: e.clone(), cap, done: vec![], env: env.clone() });
                    self.set(tid, Ctl::Eval(first, env));
                }
            }
            Frame::CallArgs { e, cap, mut done, env } => {
                let Exp::Call { op, args, .. } = &*e else { unreachable!() };
                done.push(v);
                if done.len() < args.len() {
                    let next = args[done.len()].clone();
                    self.push(tid, Frame::CallArgs { e: e.clone(), cap, done, env: env.clone() });
                    self.set(tid, Ctl::Eval(next, env));
                } else {
                    self.issue_call(tid, cap, op, done)?;
                }
            }
            Frame::Fail => {
                let is_failure = matches!(&v, Value::Con(k, _) if self.theta.con_type(k).is_some_and(|t| &**t == "Failure"));
                if !is_failure {
                    return stuck(format!("fail needs a value of datatype Failure, but got {}", v));
                }
                self.set(tid, Ctl::Raise(v));
            }
            // CATCHOK: a value passes through.
            Frame::Catch { .. } => self.set(tid, Ctl::Ret(v)),
            Frame::BudgetCost { e, env } => {
                let Exp::Budget { time, body, .. } = &*e else { unreachable!() };
                let cost = match v {
                    Value::Money(m) => Some(m),
                    Value::Inf => None,
                    other => return stuck(format!("a budget's cost must be money, but got {}", other)),
                };
                if let Some(t) = time {
                    let t = t.clone();
                    self.push(tid, Frame::BudgetTime { e: e.clone(), cost, env: env.clone() });
                    self.set(tid, Ctl::Eval(t, env));
                } else {
                    self.enter_scope(tid, cost, None, body.clone(), env);
                }
            }
            Frame::BudgetTime { e, cost, env } => {
                let Exp::Budget { body, .. } = &*e else { unreachable!() };
                let time = match v {
                    Value::Dur(d) => Some(d),
                    Value::Inf => None,
                    other => return stuck(format!("a budget's time must be a duration, but got {}", other)),
                };
                self.enter_scope(tid, cost, time, body.clone(), env);
            }
            // Leaving a scope: its spending is already on its ancestors.
            Frame::Scope(_) => self.set(tid, Ctl::Ret(v)),
        }
        Ok(())
    }

    /// PROPAGATE and CATCHFAIL: unwind to the innermost `catch`.
    fn raise(&mut self, tid: usize, phi: Value) -> Rt<()> {
        while let Some(frame) = self.threads[tid].k.pop() {
            if let Frame::Catch { e, env } = frame {
                let Exp::Catch(_, x, handler) = &*e else { unreachable!() };
                self.set(tid, Ctl::Eval(handler.clone(), env.extend(x.clone(), phi)));
                return Ok(());
            }
        }
        self.finish(tid, Res::Fail(phi))
    }

    fn build_record(&mut self, tid: usize, e: &ExpRef, vals: Vec<Value>) -> Rt<()> {
        let Exp::Record(r, fields) = &**e else { unreachable!() };
        let declared = self.theta.records.get(r).expect("checked in eval").clone();
        let given: HashMap<&str, &Value> = fields.iter().map(|(f, _)| &**f).zip(vals.iter()).collect();
        if given.len() != fields.len() {
            return stuck(format!("a field appears twice in a {} record", r));
        }
        let mut out = vec![];
        for (f, _) in declared.iter() {
            match given.get(&**f) {
                Some(v) => out.push((f.clone(), (*v).clone())),
                None => return stuck(format!("record {} is missing field {}", r, f)),
            }
        }
        if given.len() != declared.len() {
            return stuck(format!("record {} has no such field among {:?}", r, fields.iter().map(|(f, _)| f.to_string()).collect::<Vec<_>>()));
        }
        self.set(tid, Ctl::Ret(Value::Record(r.clone(), Rc::new(out))));
        Ok(())
    }

    fn apply(&mut self, tid: usize, f: Value, args: Vec<Value>) -> Rt<()> {
        match f {
            Value::Closure(c) => {
                if c.lambda.formals.len() != args.len() {
                    return stuck(format!("function expects {} argument(s), got {}", c.lambda.formals.len(), args.len()));
                }
                let env = c.lambda.formals.iter().zip(args).fold(c.env.clone(), |env, (x, v)| env.extend(x.clone(), v));
                self.set(tid, Ctl::Eval(c.lambda.body.clone(), env));
            }
            Value::Prim(p) => {
                let v = self.prim(tid, p, args)?;
                self.set(tid, Ctl::Ret(v));
            }
            other => return stuck(format!("tried to apply {}, which is not a function", other)),
        }
        Ok(())
    }

    fn prim(&mut self, tid: usize, p: Prim, args: Vec<Value>) -> Rt<Value> {
        use Value::{Bool, Cons, Dur, Inf, Money, Num, Record, Str};
        let arity = |n: usize| -> Rt<()> {
            if args.len() == n {
                Ok(())
            } else {
                stuck(format!("{} expects {} argument(s), got {}", p.name(), n, args.len()))
            }
        };
        let overflow = || RtErr(format!("arithmetic overflow in {}", p.name()));
        Ok(match p {
            Prim::Add | Prim::Sub => {
                arity(2)?;
                let op = |a: i64, b: i64| if p == Prim::Add { a.checked_add(b) } else { a.checked_sub(b) };
                match (&args[0], &args[1]) {
                    (Num(a), Num(b)) => Num(op(*a, *b).ok_or_else(overflow)?),
                    (Money(a), Money(b)) => Money(op(*a, *b).ok_or_else(overflow)?),
                    (Dur(a), Dur(b)) => Dur(op(*a, *b).ok_or_else(overflow)?),
                    (a, b) => return stuck(format!("{} needs two numbers, amounts of money or durations of the same kind, got {} and {}", p.name(), a, b)),
                }
            }
            Prim::Mul => {
                arity(2)?;
                match (&args[0], &args[1]) {
                    (Num(a), Num(b)) => Num(a.checked_mul(*b).ok_or_else(overflow)?),
                    (Money(a), Num(b)) | (Num(b), Money(a)) => Money(a.checked_mul(*b).ok_or_else(overflow)?),
                    (Dur(a), Num(b)) | (Num(b), Dur(a)) => Dur(a.checked_mul(*b).ok_or_else(overflow)?),
                    (a, b) => return stuck(format!("* can't multiply {} by {}", a, b)),
                }
            }
            Prim::Div => {
                arity(2)?;
                match (&args[0], &args[1]) {
                    (_, Num(0)) => return stuck("division by zero"),
                    (Num(a), Num(b)) => Num(a / b),
                    (Money(a), Num(b)) => Money(a / b),
                    (Dur(a), Num(b)) => Dur(a / b),
                    (a, b) => return stuck(format!("/ can't divide {} by {}", a, b)),
                }
            }
            Prim::Eq => {
                arity(2)?;
                Bool(args[0].equal(&args[1]))
            }
            Prim::Lt | Prim::Gt => {
                arity(2)?;
                let ord = match (&args[0], &args[1]) {
                    (Num(a), Num(b)) | (Money(a), Money(b)) | (Dur(a), Dur(b)) => a.cmp(b),
                    (Str(a), Str(b)) => a.cmp(b),
                    (a, b) => return stuck(format!("{} can't compare {} with {}", p.name(), a, b)),
                };
                Bool(if p == Prim::Lt { ord.is_lt() } else { ord.is_gt() })
            }
            Prim::Cons => {
                arity(2)?;
                let mut it = args.into_iter();
                Cons(Rc::new(it.next().unwrap()), Rc::new(it.next().unwrap()))
            }
            Prim::List => Value::list(args),
            Prim::StringAppend => {
                let mut s = String::new();
                for a in &args {
                    match a {
                        Str(x) => s.push_str(x),
                        other => return stuck(format!("string-append needs strings, got {}", other)),
                    }
                }
                Value::str(&s)
            }
            Prim::StringLength => {
                arity(1)?;
                match &args[0] {
                    Str(s) => Num(s.len() as i64),
                    other => return stuck(format!("string-length needs a string, got {}", other)),
                }
            }
            Prim::Println => {
                arity(1)?;
                println!("{}", args[0]);
                args.into_iter().next().unwrap()
            }
            // M-REMAINING: the live shared value (decision Q-A).
            Prim::Remaining => {
                arity(0)?;
                let s = self.current_scope(tid);
                let cost = match self.available(s) {
                    i64::MAX => Inf,
                    c => Money(c),
                };
                let time = match self.deadline_star(s) {
                    Some(d) => Dur((d - self.now).max(0)),
                    None => Inf,
                };
                Record(Rc::from("Resources"), Rc::new(vec![(Rc::from("cost"), cost), (Rc::from("time"), time)]))
            }
        })
    }

    // ------------------------------------------------------------- budget

    /// M-BUDGET: a child scope whose limits are at most its parent's (via the chain).
    fn enter_scope(&mut self, tid: usize, cost: Option<i64>, time: Option<i64>, body: ExpRef, env: Env) {
        let parent = self.current_scope(tid);
        let id = self.scopes.len();
        self.scopes.push(Scope { limit: cost, spent: 0, reserved: 0, deadline: time.map(|t| self.now + t), parent: Some(parent) });
        self.push(tid, Frame::Scope(id));
        self.set(tid, Ctl::Eval(body, env));
    }

    // --------------------------------------------------------- ask and call

    /// M-ASK-EXPIRED, M-ASK-REFUSED, M-ASK-ISSUE.
    fn issue_ask(&mut self, tid: usize, site: &str, model: Rc<ModelSpec>, ty: &Type, ctx: &Value) -> Rt<()> {
        let s = self.current_scope(tid);
        if self.deadline_star(s).is_some_and(|d| self.now >= d) {
            self.raise_con(tid, "PastDeadline", vec![]);
            return Ok(());
        }
        self.theta.askable(ty).map_err(RtErr)?;
        let in_tokens = (context_bytes(ctx)? + 3) / 4;
        let max_out = self.theta.bound(ty).unwrap_or(u64::MAX).min(model.ceiling) as i64;
        let reservation = in_tokens * model.in_price + max_out * model.out_price;
        if reservation > self.available(s) {
            self.raise_con(tid, "OverBudget", vec![]);
            return Ok(());
        }
        let entry = self
            .script
            .next(site)
            .ok_or_else(|| RtErr(format!("script exhausted: no reply left for ask site '{}'", site)))?;
        if !matches!(entry, Entry::Reply { .. } | Entry::ProviderError { .. }) {
            return stuck(format!("the script entry for ask site '{}' is a tool result, not a model reply", site));
        }
        let kind = ReqKind::Ask { model, ty: ty.clone(), in_tokens, max_out };
        self.enqueue(tid, s, site.to_string(), reservation, entry, kind);
        Ok(())
    }

    /// The CALL rules. `fork` on a kernel is implemented by the host itself.
    fn issue_call(&mut self, tid: usize, cap: Cap, op: &str, _args: Vec<Value>) -> Rt<()> {
        let s = self.current_scope(tid);
        if self.deadline_star(s).is_some_and(|d| self.now >= d) {
            self.raise_con(tid, "PastDeadline", vec![]);
            return Ok(());
        }
        if op == "fork" && cap.kind == CapKind::Kernel {
            self.next_cap += 1;
            let forked = Cap { id: self.next_cap, kind: cap.kind, key: cap.key.clone() };
            self.set(tid, Ctl::Ret(Value::Cap(forked)));
            return Ok(());
        }
        let site = format!("{}/{}", cap.key, op);
        let entry = self
            .script
            .next(&site)
            .ok_or_else(|| RtErr(format!("script exhausted: no result left for call site '{}'", site)))?;
        if !matches!(entry, Entry::Result { .. } | Entry::Error { .. }) {
            return stuck(format!("the script entry for call site '{}' is a model reply, not a tool result", site));
        }
        // cost_κ(op) = 0 for every host so far.
        self.enqueue(tid, s, site, 0, entry, ReqKind::Call);
        Ok(())
    }

    fn enqueue(&mut self, tid: usize, s: usize, site: String, reservation: i64, entry: Entry, kind: ReqKind) {
        let mut due = self.now + entry.latency();
        let mut cut = false;
        if let Some(d) = self.deadline_star(s)
            && due > d {
                due = d;
                cut = true;
            }
        self.reserve(s, reservation);
        let rid = self.next_rid;
        self.next_rid += 1;
        self.pending.push(Request { rid, tid, site, reserve: reservation, scope: s, start: self.now, due, cut, entry, kind });
        self.threads[tid].status = Status::Blocked;
    }

    /// M-TIME, for one request: settle its money, log it, resume its thread.
    fn complete(&mut self, r: Request) -> Rt<()> {
        let (in_tokens, price_in, price_out, max_out, ty) = match &r.kind {
            ReqKind::Ask { model, ty, in_tokens, max_out } => (*in_tokens, model.in_price, model.out_price, *max_out, Some(ty)),
            ReqKind::Call => (0, 0, 0, 0, None),
        };
        let (charge, out_tokens, ctl, outcome) = if r.cut {
            // ASKLATE / CALLLATE: cancelled at the deadline, charged the reservation.
            (r.reserve, 0, Ctl::Raise(failure("PastDeadline", vec![])), "past-deadline".to_string())
        } else {
            match &r.entry {
                Entry::Reply { json, out, .. } => {
                    let n = (*out as i64).min(max_out);
                    let charge = in_tokens * price_in + n * price_out;
                    let parsed = if (*out as i64) > max_out {
                        None // the provider truncates at max_out, so the JSON is incomplete
                    } else {
                        serde_json::from_str::<serde_json::Value>(json).ok().and_then(|j| self.theta.validate(&j, ty.unwrap()))
                    };
                    match parsed {
                        Some(v) => (charge, n, Ctl::Ret(v), "ok".to_string()),
                        None => (charge, n, Ctl::Raise(failure("Invalid", vec![Value::str(json)])), "invalid".to_string()),
                    }
                }
                Entry::ProviderError { msg, .. } => {
                    (in_tokens * price_in, 0, Ctl::Raise(failure("ToolError", vec![Value::str(msg)])), format!("error: {}", msg))
                }
                Entry::Result { text, .. } => (0, 0, Ctl::Ret(Value::str(text)), "ok".to_string()),
                Entry::Error { msg, .. } => (0, 0, Ctl::Raise(failure("ToolError", vec![Value::str(msg)])), format!("error: {}", msg)),
            }
        };
        self.settle(r.scope, r.reserve, charge);
        self.trace.push(TraceEntry {
            site: r.site,
            path: self.threads[r.tid].path.clone(),
            kind: if ty.is_some() { "ask" } else { "call" },
            in_tokens,
            out_tokens,
            cost: charge,
            start: r.start,
            end: self.now,
            outcome,
        });
        let t = &mut self.threads[r.tid];
        t.ctl = ctl;
        t.status = Status::Runnable;
        Ok(())
    }

    // ----------------------------------------------------------- workflows

    /// `reach(v)` (04 §6.2): stateful capabilities reachable from `v`, following
    /// only the free variables of closure bodies.
    fn reach(&self, v: &Value, acc: &mut HashSet<u64>, seen: &mut HashSet<*const Closure>) {
        match v {
            Value::Cap(c) if c.kind.stateful() => {
                acc.insert(c.id);
            }
            Value::Closure(c) => {
                if !seen.insert(Rc::as_ptr(c)) {
                    return;
                }
                for x in &c.lambda.fv {
                    if let Some(w) = self.lookup(&c.env, x) {
                        self.reach(&w, acc, seen);
                    }
                }
            }
            Value::Cons(a, b) => {
                self.reach(a, acc, seen);
                self.reach(b, acc, seen);
            }
            Value::Con(_, xs) => xs.iter().for_each(|x| self.reach(x, acc, seen)),
            Value::Record(_, fs) => fs.iter().for_each(|(_, x)| self.reach(x, acc, seen)),
            _ => {}
        }
    }

    fn start_workflow(&mut self, tid: usize, def: Rc<WorkflowDef>, env: Env) -> Rt<()> {
        // Ownership (03 Q2): each stateful capability is reachable from at most one node.
        let names: HashSet<&str> = def.nodes.iter().map(|n| &*n.name).collect();
        let mut owned: Vec<HashSet<u64>> = vec![];
        for node in &def.nodes {
            let mut acc = HashSet::new();
            let mut seen = HashSet::new();
            for x in &node.fv {
                if names.contains(&**x) {
                    continue;
                }
                if let Some(v) = self.lookup(&env, x) {
                    self.reach(&v, &mut acc, &mut seen);
                }
            }
            owned.push(acc);
        }
        for i in 0..owned.len() {
            for j in i + 1..owned.len() {
                if !owned[i].is_disjoint(&owned[j]) {
                    // M-WF-SHARED
                    self.raise_con(tid, "Raised", vec![Value::Sym(Rc::from("shared-stateful-capability"))]);
                    return Ok(());
                }
            }
        }
        if def.nodes.is_empty() {
            self.set(tid, Ctl::Eval(def.body.clone(), env));
            return Ok(());
        }
        // M-WF-START
        let n = def.nodes.len();
        let wid = self.wfs.len();
        let scope = self.current_scope(tid);
        self.wfs.push(Wf {
            parent: tid,
            def: def.clone(),
            env,
            scope,
            state: vec![NodeState::Pending; n],
            values: vec![None; n],
            remaining: n,
            finished: false,
        });
        self.threads[tid].status = Status::Waiting;
        self.threads[tid].waiting_on = Some(wid);
        for i in 0..n {
            if def.nodes[i].deps.is_empty() {
                self.spawn(wid, i);
            }
        }
        Ok(())
    }

    /// Start node `i` of workflow `wid`, with its dependencies' values bound.
    fn spawn(&mut self, wid: usize, i: usize) {
        let wf = &self.wfs[wid];
        let node = &wf.def.nodes[i];
        let env = node.deps.iter().fold(wf.env.clone(), |env, &d| {
            env.extend(wf.def.nodes[d].name.clone(), wf.values[d].clone().expect("dependency is done"))
        });
        let mut path = self.threads[wf.parent].path.clone();
        path.push(i as u32);
        let thread = Thread {
            ctl: Ctl::Eval(node.exp.clone(), env),
            k: vec![],
            base_scope: wf.scope,
            path,
            status: Status::Runnable,
            node: Some((wid, i)),
            waiting_on: None,
            result: None,
        };
        let tid = self.threads.len();
        self.threads.push(thread);
        self.wfs[wid].state[i] = NodeState::Running(tid);
    }

    /// A thread's frame stack is empty: record its result, and if it was a
    /// workflow node, apply M-NODE-DONE, M-WF-DONE or M-NODE-FAIL.
    fn finish(&mut self, tid: usize, res: Res) -> Rt<()> {
        self.threads[tid].status = Status::Done;
        self.threads[tid].result = Some(res.clone());
        let Some((wid, i)) = self.threads[tid].node else {
            return Ok(());
        };
        if self.wfs[wid].finished {
            return Ok(());
        }
        match res {
            Res::Val(v) => {
                let wf = &mut self.wfs[wid];
                wf.values[i] = Some(v);
                wf.state[i] = NodeState::Done;
                wf.remaining -= 1;
                if wf.remaining == 0 {
                    // M-WF-DONE
                    wf.finished = true;
                    let env = wf
                        .def
                        .nodes
                        .iter()
                        .zip(wf.values.iter())
                        .fold(wf.env.clone(), |env, (n, v)| env.extend(n.name.clone(), v.clone().unwrap()));
                    let (parent, body) = (wf.parent, wf.def.body.clone());
                    let p = &mut self.threads[parent];
                    p.ctl = Ctl::Eval(body, env);
                    p.status = Status::Runnable;
                    p.waiting_on = None;
                } else {
                    // M-NODE-DONE: start every node whose dependencies are now done.
                    let ready: Vec<usize> = (0..wf.def.nodes.len())
                        .filter(|&j| {
                            wf.state[j] == NodeState::Pending && wf.def.nodes[j].deps.iter().all(|&d| wf.state[d] == NodeState::Done)
                        })
                        .collect();
                    for j in ready {
                        self.spawn(wid, j);
                    }
                }
            }
            Res::Fail(phi) => {
                // M-NODE-FAIL: fail fast, cancelling the siblings.
                self.wfs[wid].finished = true;
                let running: Vec<usize> = self.wfs[wid]
                    .state
                    .iter()
                    .filter_map(|s| if let NodeState::Running(t) = s { Some(*t) } else { None })
                    .filter(|&t| t != tid)
                    .collect();
                for t in running {
                    self.cancel(t);
                }
                let parent = self.wfs[wid].parent;
                let p = &mut self.threads[parent];
                p.ctl = Ctl::Raise(phi);
                p.status = Status::Runnable;
                p.waiting_on = None;
            }
        }
        Ok(())
    }

    /// Cancel a thread and everything it's waiting for. An in-flight request is
    /// charged its reservation (07 §5).
    fn cancel(&mut self, tid: usize) {
        let status = self.threads[tid].status;
        self.threads[tid].status = Status::Done;
        match status {
            Status::Blocked => {
                if let Some(pos) = self.pending.iter().position(|r| r.tid == tid) {
                    let r = self.pending.remove(pos);
                    self.settle(r.scope, r.reserve, r.reserve);
                    self.trace.push(TraceEntry {
                        site: r.site,
                        path: self.threads[tid].path.clone(),
                        kind: if matches!(r.kind, ReqKind::Ask { .. }) { "ask" } else { "call" },
                        in_tokens: 0,
                        out_tokens: 0,
                        cost: r.reserve,
                        start: r.start,
                        end: self.now,
                        outcome: "cancelled".into(),
                    });
                }
            }
            Status::Waiting => {
                if let Some(w) = self.threads[tid].waiting_on {
                    self.wfs[w].finished = true;
                    let running: Vec<usize> = self.wfs[w]
                        .state
                        .iter()
                        .filter_map(|s| if let NodeState::Running(t) = s { Some(*t) } else { None })
                        .collect();
                    for t in running {
                        self.cancel(t);
                    }
                }
            }
            Status::Runnable | Status::Done => {}
        }
    }
}

/// Match a flat pattern, returning its bindings.
pub fn match_pattern(p: &Pattern, v: &Value) -> Option<Vec<(Name, Value)>> {
    match (p, v) {
        (Pattern::Wild, _) => Some(vec![]),
        (Pattern::Nil, Value::Nil) => Some(vec![]),
        (Pattern::Cons(a, b), Value::Cons(h, t)) => {
            let mut out = vec![];
            if let Some(a) = a {
                out.push((a.clone(), (**h).clone()));
            }
            if let Some(b) = b {
                out.push((b.clone(), (**t).clone()));
            }
            Some(out)
        }
        (Pattern::Con(k, xs), Value::Con(j, vs)) if k == j && xs.len() == vs.len() => {
            Some(xs.iter().zip(vs.iter()).filter_map(|(x, v)| x.clone().map(|x| (x, v.clone()))).collect())
        }
        _ => None,
    }
}

/// Bytes of a context under the test tokenizer's serialization: for each
/// message, its role, its content, and 4 bytes of framing. Tokens = ⌈bytes ÷ 4⌉.
fn context_bytes(ctx: &Value) -> Rt<i64> {
    let items = ctx.to_vec().ok_or_else(|| RtErr(format!("an ask's context must be a list of Message, got {}", ctx)))?;
    let mut bytes = 0;
    for m in items {
        let Value::Record(r, fs) = &m else {
            return stuck(format!("an ask's context must contain Message records, got {}", m));
        };
        if &**r != "Message" {
            return stuck(format!("an ask's context must contain Message records, got {}", m));
        }
        for (f, v) in fs.iter() {
            match (&**f, v) {
                ("role", Value::Con(k, _)) => bytes += k.len() as i64,
                ("content", Value::Str(s)) => bytes += s.len() as i64,
                _ => return stuck(format!("malformed Message {}", m)),
            }
        }
        bytes += 4;
    }
    Ok(bytes)
}
