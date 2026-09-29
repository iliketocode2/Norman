//! Abstract syntax (04 §3) and free variables (04 Figure 1).

use crate::value::Value;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::rc::Rc;

pub type Name = Rc<str>;

#[derive(Debug, Clone)]
pub struct Loc {
    pub file: Rc<str>,
    pub line: u32,
    pub col: u32,
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.col)
    }
}

/// Types, as they appear in `ask`, `datatype`, and `record`.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Text(Option<u64>),
    Num,
    Bool,
    Sym,
    List(Box<Type>, Option<u64>),
    Named(Name),
    Any,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Type::Text(None) => write!(f, "Text"),
            Type::Text(Some(n)) => write!(f, "(Text {})", n),
            Type::Num => write!(f, "Num"),
            Type::Bool => write!(f, "Bool"),
            Type::Sym => write!(f, "Sym"),
            Type::List(t, None) => write!(f, "(List {})", t),
            Type::List(t, Some(n)) => write!(f, "(List {} {})", t, n),
            Type::Named(n) => write!(f, "{}", n),
            Type::Any => write!(f, "Any"),
        }
    }
}

/// Flat patterns (04 §2). `None` in a variable position is the wildcard `_`.
#[derive(Debug, Clone)]
pub enum Pattern {
    Con(Name, Vec<Option<Name>>),
    Nil,
    Cons(Option<Name>, Option<Name>),
    Wild,
}

impl Pattern {
    pub fn vars(&self) -> Vec<Name> {
        match self {
            Pattern::Con(_, xs) => xs.iter().flatten().cloned().collect(),
            Pattern::Cons(a, b) => a.iter().chain(b.iter()).cloned().collect(),
            Pattern::Nil | Pattern::Wild => vec![],
        }
    }
}

pub type ExpRef = Rc<Exp>;

/// The sixteen expression forms of 04 §3.
#[derive(Debug)]
pub enum Exp {
    Literal(Value),
    Var(Name),
    Con(Name, Vec<ExpRef>),
    Record(Name, Vec<(Name, ExpRef)>),
    Field(ExpRef, Name),
    If(ExpRef, ExpRef, ExpRef),
    Let(Name, ExpRef, ExpRef),
    Lambda(Rc<Lambda>),
    Apply(ExpRef, Vec<ExpRef>),
    Case(ExpRef, Vec<(Pattern, ExpRef)>),
    Ask { site: Name, model: ExpRef, ty: Type, ctx: ExpRef },
    Call { cap: ExpRef, op: Name, args: Vec<ExpRef> },
    Fail(ExpRef),
    Catch(ExpRef, Name, ExpRef),
    Budget { cost: Option<ExpRef>, time: Option<ExpRef>, body: ExpRef },
    Workflow(Rc<WorkflowDef>),
}

#[derive(Debug)]
pub struct Lambda {
    pub formals: Vec<Name>,
    pub body: ExpRef,
    /// fv(body) − formals, precomputed for the ownership check (`reach`).
    pub fv: Vec<Name>,
}

impl Lambda {
    pub fn new(formals: Vec<Name>, body: ExpRef) -> Lambda {
        let mut s = free_vars(&body);
        for x in &formals {
            s.remove(x);
        }
        Lambda { formals, body, fv: s.into_iter().collect() }
    }
}

#[derive(Debug)]
pub struct Node {
    pub name: Name,
    pub exp: ExpRef,
    /// Indices of the nodes this node depends on: fv(e) ∩ {x₁ … xₙ}.
    pub deps: Vec<usize>,
    pub fv: Vec<Name>,
}

#[derive(Debug)]
pub struct WorkflowDef {
    pub nodes: Vec<Node>,
    pub body: ExpRef,
}

fn union(a: &mut BTreeSet<Name>, b: BTreeSet<Name>) {
    a.extend(b);
}

fn minus(mut a: BTreeSet<Name>, xs: &[Name]) -> BTreeSet<Name> {
    for x in xs {
        a.remove(x);
    }
    a
}

/// Figure 1 of 04: free variables of an expression.
pub fn free_vars(e: &Exp) -> BTreeSet<Name> {
    let mut s = BTreeSet::new();
    match e {
        Exp::Literal(_) => {}
        Exp::Var(x) => {
            s.insert(x.clone());
        }
        Exp::Con(_, es) => es.iter().for_each(|e| union(&mut s, free_vars(e))),
        Exp::Record(_, fs) => fs.iter().for_each(|(_, e)| union(&mut s, free_vars(e))),
        Exp::Field(e, _) | Exp::Fail(e) => s = free_vars(e),
        Exp::If(a, b, c) => {
            union(&mut s, free_vars(a));
            union(&mut s, free_vars(b));
            union(&mut s, free_vars(c));
        }
        Exp::Let(x, e1, e2) => {
            s = free_vars(e1);
            union(&mut s, minus(free_vars(e2), std::slice::from_ref(x)));
        }
        Exp::Lambda(l) => s = l.fv.iter().cloned().collect(),
        Exp::Apply(f, es) => {
            s = free_vars(f);
            es.iter().for_each(|e| union(&mut s, free_vars(e)));
        }
        Exp::Case(e, bs) => {
            s = free_vars(e);
            for (p, b) in bs {
                union(&mut s, minus(free_vars(b), &p.vars()));
            }
        }
        Exp::Ask { model, ctx, .. } => {
            s = free_vars(model);
            union(&mut s, free_vars(ctx));
        }
        Exp::Call { cap, args, .. } => {
            s = free_vars(cap);
            args.iter().for_each(|e| union(&mut s, free_vars(e)));
        }
        Exp::Catch(e1, x, e2) => {
            s = free_vars(e1);
            union(&mut s, minus(free_vars(e2), std::slice::from_ref(x)));
        }
        Exp::Budget { cost, time, body } => {
            s = free_vars(body);
            cost.iter().chain(time.iter()).for_each(|e| union(&mut s, free_vars(e)));
        }
        Exp::Workflow(w) => {
            let mut t = free_vars(&w.body);
            for n in &w.nodes {
                t.extend(n.fv.iter().cloned());
            }
            let names: Vec<Name> = w.nodes.iter().map(|n| n.name.clone()).collect();
            s = minus(t, &names);
        }
    }
    s
}

#[derive(Debug, Clone)]
pub struct ConDef {
    pub name: Name,
    pub fields: Vec<(Name, Type)>,
}

/// True definitions (04 §6.6).
#[derive(Debug)]
pub enum Def {
    Val(Name, ExpRef),
    Exp(ExpRef),
    Define(Name, Rc<Lambda>),
    Datatype(Name, Vec<ConDef>),
    Record(Name, Vec<(Name, Type)>),
}

/// What a `grant` hands to the program.
#[derive(Debug, Clone)]
pub enum HostSpec {
    /// Prices are integer micro-dollars per token. Omitted fields take the
    /// defaults in `crate::defaults` (Claude Sonnet 5).
    Model { id: String, in_price: i64, out_price: i64, ceiling: u64, think: u64 },
    Kernel,
    Filesystem,
    Http,
}

/// One scripted reply (05, "Conventions for scripted mode").
#[derive(Debug, Clone)]
pub enum Entry {
    Reply { json: String, out: u64, latency: i64 },
    Refusal { category: String, out: u64, latency: i64 },
    ProviderError { msg: String, latency: i64 },
    Result { text: String, latency: i64 },
    Error { msg: String, latency: i64 },
}

impl Entry {
    pub fn latency(&self) -> i64 {
        match self {
            Entry::Reply { latency, .. }
            | Entry::Refusal { latency, .. }
            | Entry::ProviderError { latency, .. }
            | Entry::Result { latency, .. }
            | Entry::Error { latency, .. } => *latency,
        }
    }
}

/// A script: per-site sequences of replies. Keys are ask sites (`analyst`) or
/// call sites (`py/exec`).
#[derive(Debug, Default)]
pub struct Script {
    pub sites: HashMap<String, Vec<Entry>>,
}

/// The configuration of an `under` block.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub script: Option<Name>,
    pub cost: Option<i64>,
    pub time: Option<i64>,
}

/// Grades of equivalence for `check-equiv` (06 §0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grade {
    Exact,
    Resource,
    Value,
}

#[derive(Debug)]
pub enum TestKind {
    Expect(ExpRef, ExpRef),
    Assert(ExpRef),
    Error(ExpRef),
    Fail(ExpRef, Pattern),
    Within(ExpRef, Option<i64>, Option<i64>),
    Equiv(ExpRef, ExpRef, Grade),
}

#[derive(Debug)]
pub struct UnitTest {
    pub kind: TestKind,
    /// Source text of the tested expressions, for messages.
    pub texts: Vec<String>,
    pub loc: Loc,
}

/// A top-level form: a true definition or an extended definition.
#[derive(Debug)]
pub enum Top {
    Def(Def),
    Use(String),
    Grant(Name, HostSpec),
    Script(Name, Script),
    Under(Config, Vec<UnitTest>),
    Test(UnitTest),
}
