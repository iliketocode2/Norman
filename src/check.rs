//! The type and effect checker (design/12), steps 1 and 2: inference for
//! every form except the three that reach a model, a tool or a person.
//!
//! - Types are inferred as in Ramsey's µML (Hindley-Milner), with type
//!   parameters on datatypes and records (design/12, T5).
//! - Overloaded and restricted operations are **qualified types** (T4): a
//!   scheme carries predicates such as `Ord a`, checked when it is
//!   instantiated, and defaulted per binding group as SML does (§3).
//! - Effects follow Talpin and Jouvelot (§2): every arrow carries an effect
//!   *variable*, and the rules add lower bounds to it (`ε ⊆ φ`). Unifying two
//!   arrows unifies their effect variables. The only atom so far is
//!   `observe`, for `(remaining)`; step 3 adds `ask μ`, `call κ` and
//!   `consult η` with their grant names.
//! - A file is checked as a whole before it runs (T7): definitions are
//!   grouped into strongly connected components and generalized together,
//!   a name defined twice is an error (Q-2), and a form that runs at load
//!   time may depend only on definitions above it (§4.4).
//!
//! `ask`, `call`, `consult` and grants need name-indexed types, which are
//! step 3; until then the checker reports them as not yet supported.

use crate::ast::*;
use crate::lexer::read_all;
use crate::parser::Parser;
use crate::types::TypeEnv;
use crate::value::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

type TV = u32;
type EV = u32;

/// A type during inference.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Var(TV),
    /// A base type (`Num`), `List`, or a datatype or record, with arguments.
    Con(Name, Vec<Ty>),
    /// A function: arguments, result, and the variable for its latent effect.
    Fun(Vec<Ty>, Box<Ty>, EV),
}

fn con(n: &str) -> Ty {
    Ty::Con(Rc::from(n), vec![])
}
fn list_of(t: Ty) -> Ty {
    Ty::Con(Rc::from("List"), vec![t])
}

/// An effect atom. Step 3 adds `ask μ`, `call κ` and `consult η`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Atom {
    /// Reading `(remaining)`.
    Observe,
}

/// An effect: some atoms, and the effect variables whose effects it includes.
#[derive(Clone, Debug, Default, PartialEq)]
struct Eff {
    atoms: BTreeSet<Atom>,
    vars: BTreeSet<EV>,
}

impl Eff {
    fn var(e: EV) -> Eff {
        Eff { atoms: BTreeSet::new(), vars: [e].into() }
    }
    fn union(mut self, other: &Eff) -> Eff {
        self.atoms.extend(other.atoms.iter().copied());
        self.vars.extend(other.vars.iter().copied());
        self
    }
}

/// A predicate of a qualified type (design/12 §3).
#[derive(Clone, Debug, PartialEq)]
pub enum Pred {
    Arith(Ty),
    Ord(Ty),
    Mul(Ty, Ty, Ty),
    Div(Ty, Ty, Ty),
    Showable(Ty),
    Has(Ty, Name, Ty),
}

/// `∀ tvs evs. preds ⇒ ty`. Quantified effect variables carry their lower
/// bounds with them, so a scheme is self-contained.
#[derive(Clone, Debug)]
pub struct Scheme {
    tvs: Vec<TV>,
    evs: Vec<(EV, Eff)>,
    preds: Vec<Pred>,
    ty: Ty,
}

impl Scheme {
    fn mono(ty: Ty) -> Scheme {
        Scheme { tvs: vec![], evs: vec![], preds: vec![], ty }
    }
}

/// A global: a primitive, whose type is made fresh at each use, or a scheme.
#[derive(Clone, Debug)]
enum Global {
    Prim(&'static str),
    Scheme(Scheme),
}

/// What a predicate reduces to.
enum Step {
    Done,
    Keep,
    Then(Vec<Pred>),
    Unify(Vec<(Ty, Ty)>),
    /// Unify, then go on with these predicates.
    UnifyThen(Vec<(Ty, Ty)>, Vec<Pred>),
}

struct EffVar {
    parent: EV,
    lower: Eff,
}

/// The inference state: the substitution, the effect variables and the
/// predicates not yet discharged.
#[derive(Default)]
struct State {
    tsub: Vec<Option<Ty>>,
    evars: Vec<EffVar>,
    preds: Vec<Pred>,
}

type R<T> = Result<T, String>;

impl State {
    fn fresh(&mut self) -> Ty {
        self.tsub.push(None);
        Ty::Var((self.tsub.len() - 1) as TV)
    }

    fn fresh_ev(&mut self) -> EV {
        let e = self.evars.len() as EV;
        self.evars.push(EffVar { parent: e, lower: Eff::default() });
        e
    }

    fn find(&mut self, e: EV) -> EV {
        let p = self.evars[e as usize].parent;
        if p == e {
            return e;
        }
        let root = self.find(p);
        self.evars[e as usize].parent = root;
        root
    }

    /// `ε ⊆ φ`: add lower bounds to an effect variable.
    fn include(&mut self, phi: EV, eff: &Eff) {
        let r = self.find(phi);
        let vars: Vec<EV> = eff.vars.iter().map(|&v| self.find(v)).filter(|&v| v != r).collect();
        let low = &mut self.evars[r as usize].lower;
        low.atoms.extend(eff.atoms.iter().copied());
        low.vars.extend(vars);
    }

    fn union_ev(&mut self, a: EV, b: EV) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let moved = std::mem::take(&mut self.evars[rb as usize].lower);
        self.evars[rb as usize].parent = ra;
        self.include(ra, &moved);
    }

    /// Every atom an effect variable includes, and every variable it reaches.
    fn closure(&mut self, phi: EV) -> (BTreeSet<Atom>, BTreeSet<EV>) {
        let mut atoms = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut todo = vec![self.find(phi)];
        while let Some(v) = todo.pop() {
            if !seen.insert(v) {
                continue;
            }
            atoms.extend(self.evars[v as usize].lower.atoms.iter().copied());
            let next: Vec<EV> = self.evars[v as usize].lower.vars.iter().copied().collect();
            for n in next {
                todo.push(self.find(n));
            }
        }
        (atoms, seen)
    }

    /// Resolve the outer variable of a type.
    fn prune(&self, t: &Ty) -> Ty {
        let mut t = t.clone();
        while let Ty::Var(v) = t {
            match &self.tsub[v as usize] {
                Some(u) => t = u.clone(),
                None => break,
            }
        }
        t
    }

    /// Resolve a type completely, with effect variables at their roots.
    fn zonk(&mut self, t: &Ty) -> Ty {
        match self.prune(t) {
            Ty::Var(v) => Ty::Var(v),
            Ty::Con(n, args) => Ty::Con(n, args.iter().map(|a| self.zonk(a)).collect()),
            Ty::Fun(args, res, e) => {
                let args = args.iter().map(|a| self.zonk(a)).collect();
                let res = Box::new(self.zonk(&res));
                Ty::Fun(args, res, self.find(e))
            }
        }
    }

    fn occurs(&self, v: TV, t: &Ty) -> bool {
        match self.prune(t) {
            Ty::Var(w) => v == w,
            Ty::Con(_, args) => args.iter().any(|a| self.occurs(v, a)),
            Ty::Fun(args, res, _) => args.iter().any(|a| self.occurs(v, a)) || self.occurs(v, &res),
        }
    }

    fn unify(&mut self, a: &Ty, b: &Ty) -> R<()> {
        let (a, b) = (self.prune(a), self.prune(b));
        match (&a, &b) {
            (Ty::Var(x), Ty::Var(y)) if x == y => Ok(()),
            (Ty::Var(x), t) | (t, Ty::Var(x)) => {
                if self.occurs(*x, t) {
                    let (sa, sb) = self.show_pair(&a, &b);
                    return Err(format!("{} would have to contain itself, as {}", sa, sb));
                }
                self.tsub[*x as usize] = Some(t.clone());
                Ok(())
            }
            (Ty::Con(m, xs), Ty::Con(n, ys)) if m == n && xs.len() == ys.len() => {
                xs.iter().zip(ys).try_for_each(|(x, y)| self.unify(x, y))
            }
            (Ty::Fun(xs, r1, e1), Ty::Fun(ys, r2, e2)) if xs.len() == ys.len() => {
                xs.iter().zip(ys).try_for_each(|(x, y)| self.unify(x, y))?;
                self.unify(r1, r2)?;
                self.union_ev(*e1, *e2);
                Ok(())
            }
            (Ty::Fun(xs, ..), Ty::Fun(ys, ..)) => {
                Err(format!("a function of {} argument(s) is used as one of {}", xs.len(), ys.len()))
            }
            _ => {
                let (sa, sb) = self.show_pair(&a, &b);
                Err(format!("expected {}, but found {}", sb, sa))
            }
        }
    }

    fn show_pair(&mut self, a: &Ty, b: &Ty) -> (String, String) {
        let (a, b) = (self.zonk(a), self.zonk(b));
        let mut p = Printer::default();
        (p.ty(self, &a, false), p.ty(self, &b, false))
    }

    fn ftv(&self, t: &Ty, out: &mut BTreeSet<TV>) {
        match self.prune(t) {
            Ty::Var(v) => {
                out.insert(v);
            }
            Ty::Con(_, args) => args.iter().for_each(|a| self.ftv(a, out)),
            Ty::Fun(args, res, _) => {
                args.iter().for_each(|a| self.ftv(a, out));
                self.ftv(&res, out);
            }
        }
    }

    fn fev(&mut self, t: &Ty, out: &mut BTreeSet<EV>) {
        match self.prune(t) {
            Ty::Var(_) => {}
            Ty::Con(_, args) => args.iter().for_each(|a| self.fev(a, out)),
            Ty::Fun(args, res, e) => {
                args.iter().for_each(|a| self.fev(a, out));
                self.fev(&res, out);
                let r = self.find(e);
                out.insert(r);
            }
        }
    }

    fn pred_ftv(&self, p: &Pred, out: &mut BTreeSet<TV>) {
        match p {
            Pred::Arith(t) | Pred::Ord(t) | Pred::Showable(t) => self.ftv(t, out),
            Pred::Mul(a, b, c) | Pred::Div(a, b, c) => {
                self.ftv(a, out);
                self.ftv(b, out);
                self.ftv(c, out);
            }
            Pred::Has(a, _, b) => {
                self.ftv(a, out);
                self.ftv(b, out);
            }
        }
    }

    fn zonk_pred(&mut self, p: &Pred) -> Pred {
        match p {
            Pred::Arith(t) => Pred::Arith(self.zonk(t)),
            Pred::Ord(t) => Pred::Ord(self.zonk(t)),
            Pred::Showable(t) => Pred::Showable(self.zonk(t)),
            Pred::Mul(a, b, c) => Pred::Mul(self.zonk(a), self.zonk(b), self.zonk(c)),
            Pred::Div(a, b, c) => Pred::Div(self.zonk(a), self.zonk(b), self.zonk(c)),
            Pred::Has(a, f, b) => Pred::Has(self.zonk(a), f.clone(), self.zonk(b)),
        }
    }
}

/// The type of a primitive, fresh at each use. `list` and `string-append`
/// take any number of arguments and are typed at the application instead.
fn prim_type(name: &str, st: &mut State) -> Option<Ty> {
    let pure = |st: &mut State, args: Vec<Ty>, res: Ty| Ty::Fun(args, Box::new(res), st.fresh_ev());
    let (num, text, bool_) = (con("Num"), con("Text"), con("Bool"));
    Some(match name {
        "+" | "-" => {
            let a = st.fresh();
            st.preds.push(Pred::Arith(a.clone()));
            pure(st, vec![a.clone(), a.clone()], a)
        }
        "*" | "/" => {
            let (a, b, c) = (st.fresh(), st.fresh(), st.fresh());
            st.preds.push(if name == "*" {
                Pred::Mul(a.clone(), b.clone(), c.clone())
            } else {
                Pred::Div(a.clone(), b.clone(), c.clone())
            });
            pure(st, vec![a, b], c)
        }
        "<" | ">" => {
            let a = st.fresh();
            st.preds.push(Pred::Ord(a.clone()));
            pure(st, vec![a.clone(), a], bool_)
        }
        "=" => {
            let a = st.fresh();
            pure(st, vec![a.clone(), a], bool_)
        }
        "cons" => {
            let a = st.fresh();
            pure(st, vec![a.clone(), list_of(a.clone())], list_of(a))
        }
        "show" => {
            let a = st.fresh();
            st.preds.push(Pred::Showable(a.clone()));
            pure(st, vec![a], text)
        }
        "println" => {
            let a = st.fresh();
            pure(st, vec![a.clone()], a)
        }
        "string-length" => pure(st, vec![text], num),
        "substring" => pure(st, vec![text.clone(), num.clone(), num], text),
        "string-truncate" => pure(st, vec![text.clone(), num], text),
        "string-contains?" => pure(st, vec![text.clone(), text], bool_),
        "remaining" => {
            let e = st.fresh_ev();
            st.include(e, &Eff { atoms: [Atom::Observe].into(), vars: BTreeSet::new() });
            Ty::Fun(vec![], Box::new(con("Resources")), e)
        }
        _ => return None,
    })
}

/// Names for type and effect variables, assigned in order of appearance.
#[derive(Default)]
struct Printer {
    tnames: HashMap<TV, String>,
    enames: HashMap<EV, String>,
    /// Effect variables of argument positions: shown by name. An arrow in a
    /// result position shows the argument effects it includes, so `retry`
    /// reads `(Num (-> a ! e1) -> a ! e1)`.
    shown: BTreeSet<EV>,
}

impl Printer {
    fn tname(&mut self, v: TV) -> String {
        let n = self.tnames.len();
        self.tnames
            .entry(v)
            .or_insert_with(|| {
                let letter = (b'a' + (n % 26) as u8) as char;
                if n < 26 { letter.to_string() } else { format!("{}{}", letter, n / 26) }
            })
            .clone()
    }

    fn ename(&mut self, e: EV) -> String {
        let n = self.enames.len() + 1;
        self.enames.entry(e).or_insert_with(|| format!("e{}", n)).clone()
    }

    /// Collect the effect variables of argument arrows (odd polarity).
    fn collect(&mut self, st: &mut State, t: &Ty, negative: bool) {
        if let Ty::Fun(args, res, e) = st.prune(t) {
            if negative {
                let r = st.find(e);
                self.shown.insert(r);
            }
            for a in &args {
                self.collect(st, a, !negative);
            }
            self.collect(st, &res, negative);
        } else if let Ty::Con(_, args) = st.prune(t) {
            for a in &args {
                self.collect(st, a, negative);
            }
        }
    }

    fn ty(&mut self, st: &mut State, t: &Ty, top: bool) -> String {
        if top {
            self.collect(st, t, false);
        }
        match st.prune(t) {
            Ty::Var(v) => self.tname(v),
            Ty::Con(n, args) if args.is_empty() => n.to_string(),
            Ty::Con(n, args) => {
                let args: Vec<String> = args.iter().map(|a| self.ty(st, a, false)).collect();
                format!("({} {})", n, args.join(" "))
            }
            Ty::Fun(args, res, e) => {
                let args: Vec<String> = args.iter().map(|a| self.ty(st, a, false)).collect();
                let res = self.ty(st, &res, false);
                let (atoms, vars) = st.closure(e);
                let mut eff: Vec<String> = atoms.iter().map(|a| format!("{:?}", a).to_lowercase()).collect();
                for v in vars {
                    if self.shown.contains(&v) {
                        eff.push(self.ename(v));
                    }
                }
                let eff = if eff.is_empty() { String::new() } else { format!(" ! {}", eff.join(" ")) };
                if args.is_empty() {
                    format!("(-> {}{})", res, eff)
                } else {
                    format!("({} -> {}{})", args.join(" "), res, eff)
                }
            }
        }
    }

    fn pred(&mut self, st: &mut State, p: &Pred) -> String {
        let mut t = |st: &mut State, x: &Ty| self.ty(st, x, false);
        match p {
            Pred::Arith(a) => format!("Arith {}", t(st, a)),
            Pred::Ord(a) => format!("Ord {}", t(st, a)),
            Pred::Showable(a) => format!("Showable {}", t(st, a)),
            Pred::Mul(a, b, c) => format!("Mul {} {} {}", t(st, a), t(st, b), t(st, c)),
            Pred::Div(a, b, c) => format!("Div {} {} {}", t(st, a), t(st, b), t(st, c)),
            Pred::Has(a, f, b) => format!("Has {} {} {}", t(st, a), f, t(st, b)),
        }
    }
}

/// One top-level form, as the checker sees it.
enum Item {
    Define(Name, Rc<Lambda>),
    Val(Name, ExpRef),
    Expr(ExpRef),
}

/// What checking a program found.
#[derive(Debug, Default)]
pub struct Report {
    /// Errors that reject the program.
    pub errors: Vec<String>,
    /// Unit tests whose expressions are ill-typed. Each fails on its own,
    /// without rejecting the file (design/12, T8).
    pub test_errors: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// The checker: Θ and the global type environment, built as files are read.
pub struct Checker {
    theta: TypeEnv,
    globals: HashMap<Name, Global>,
    /// Where each global was defined, for the redefinition rule (Q-2).
    defined_in: HashMap<Name, String>,
    st: State,
    used: HashSet<PathBuf>,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    /// A checker that knows the primitives and has checked the prelude.
    pub fn new() -> Checker {
        let mut c = Self::bare();
        let report = c.check_str(include_str!("prelude.nrm"), "prelude.nrm", Path::new("."));
        assert!(report.ok(), "the prelude does not type-check: {:?}", report.errors);
        c
    }

    /// A checker with the primitives only.
    pub fn bare() -> Checker {
        let mut globals = HashMap::new();
        for p in crate::value::Prim::ALL {
            globals.insert(Rc::from(p.name()), Global::Prim(p.name()));
        }
        Checker {
            theta: TypeEnv::default(),
            globals,
            defined_in: HashMap::new(),
            st: State::default(),
            used: HashSet::new(),
        }
    }

    /// The inferred type of a global, written out: `Ord a => (a a -> Bool)`.
    pub fn scheme_of(&mut self, name: &str) -> Option<String> {
        let g = self.globals.get(name)?.clone();
        // Instantiate, so the printer can follow the effect bounds, and keep
        // the predicates that instantiating created.
        let before = self.st.preds.len();
        let ty = match g {
            Global::Scheme(s) => self.instantiate(&s),
            Global::Prim(p) => prim_type(p, &mut self.st)?,
        };
        let preds: Vec<Pred> = self.st.preds.drain(before..).collect();
        let mut p = Printer::default();
        p.collect(&mut self.st, &ty, false);
        let shown = p.ty(&mut self.st, &ty, false);
        let preds: Vec<String> = preds.iter().map(|q| p.pred(&mut self.st, q)).collect();
        Some(if preds.is_empty() { shown } else { format!("{} => {}", preds.join(", "), shown) })
    }

    pub fn check_file(&mut self, path: &Path) -> Report {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !self.used.insert(key) {
            return Report::default();
        }
        match std::fs::read_to_string(path) {
            Ok(src) => {
                let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.check_str(&src, &name, &dir)
            }
            Err(e) => Report { errors: vec![format!("can't read {}: {}", path.display(), e)], ..Default::default() },
        }
    }

    /// Check a whole file before any of it runs (T7).
    pub fn check_str(&mut self, src: &str, file: &str, dir: &Path) -> Report {
        let mut report = Report::default();
        let forms = match read_all(src, file) {
            Ok(f) => f,
            Err(e) => {
                report.errors.push(e);
                return report;
            }
        };
        // Read every form, extending Θ as datatypes and records appear, so
        // later forms parse with the records they mention.
        let mut items: Vec<(Loc, Item)> = vec![];
        let mut tests: Vec<UnitTest> = vec![];
        for sx in &forms {
            let top = match Parser::new(&self.theta).top(sx) {
                Ok(t) => t,
                Err(e) => {
                    report.errors.push(format!("syntax error: {}", e));
                    continue;
                }
            };
            let loc = sx.loc().clone();
            match top {
                Top::Def(Def::Datatype(t, ps, cons)) => {
                    if let Err(e) = self.theta.add_datatype(t, ps, cons) {
                        report.errors.push(format!("{}: {}", loc, e));
                    }
                }
                Top::Def(Def::Record(r, ps, fs)) => {
                    if let Err(e) = self.theta.add_record(r, ps, fs) {
                        report.errors.push(format!("{}: {}", loc, e));
                    }
                }
                Top::Def(Def::Define(f, l)) => items.push((loc, Item::Define(f, l))),
                Top::Def(Def::Val(x, e)) => items.push((loc, Item::Val(x, e))),
                Top::Def(Def::Exp(e)) => items.push((loc, Item::Expr(e))),
                Top::Use(f) => {
                    let used = self.check_file(&dir.join(&f));
                    report.errors.extend(used.errors);
                    report.test_errors.extend(used.test_errors);
                }
                Top::Grant(x, _) => {
                    // Step 3 gives grants name-indexed types. Until then a
                    // grant is a name the checker knows but can't use.
                    self.define_global(&x, Global::Scheme(Scheme::mono(con("Grant"))), &loc.to_string(), &mut report);
                }
                Top::Script(..) => {}
                Top::Under(_, ts) => tests.extend(ts),
                Top::Test(t) => tests.push(t),
            }
        }
        self.check_items(file, items, &mut report);
        for t in &tests {
            if let Err(e) = self.check_test(t) {
                report.test_errors.push(format!("{}: {}", t.loc, e));
            }
        }
        report
    }

    fn define_global(&mut self, x: &Name, g: Global, where_: &str, report: &mut Report) {
        if let Some(prev) = self.defined_in.get(x) {
            report.errors.push(format!(
                "{}: {} is already defined, at {} (a name is defined once: design/12, Q-2)",
                where_, x, prev
            ));
            return;
        }
        if matches!(self.globals.get(x), Some(Global::Prim(_))) {
            report.errors.push(format!("{}: {} is a primitive and can't be redefined", where_, x));
            return;
        }
        self.defined_in.insert(x.clone(), where_.to_string());
        self.globals.insert(x.clone(), g);
    }

    /// Binding groups, generalization, the load-order rule, and then the
    /// top-level expressions.
    fn check_items(&mut self, file: &str, items: Vec<(Loc, Item)>, report: &mut Report) {
        // The names this file defines, in order. A second definition is an
        // error (Q-2), here or against an earlier file.
        let mut index: HashMap<Name, usize> = HashMap::new();
        let mut defs: Vec<usize> = vec![];
        for (i, (loc, item)) in items.iter().enumerate() {
            let name = match item {
                Item::Define(f, _) => f,
                Item::Val(x, _) => x,
                Item::Expr(_) => continue,
            };
            if let Some(prev) = self.defined_in.get(name) {
                report.errors.push(format!(
                    "{}: {} is already defined, at {} (a name is defined once: design/12, Q-2)",
                    loc, name, prev
                ));
                continue;
            }
            if matches!(self.globals.get(name), Some(Global::Prim(_))) {
                report.errors.push(format!("{}: {} is a primitive and can't be redefined", loc, name));
                continue;
            }
            if let Some(&j) = index.get(name) {
                report.errors.push(format!(
                    "{}: {} is already defined, at {} (a name is defined once: design/12, Q-2)",
                    loc, name, items[j].0
                ));
                continue;
            }
            index.insert(name.clone(), i);
            defs.push(i);
        }
        let deps = |item: &Item| -> BTreeSet<Name> {
            let fv = match item {
                Item::Define(_, l) => l.fv.iter().cloned().collect(),
                Item::Val(_, e) | Item::Expr(e) => free_vars(e),
            };
            fv.into_iter().filter(|x| index.contains_key(x)).collect()
        };

        // The load-order rule (§4.4): a form that runs at load time may
        // depend, through the bodies of what it calls, only on definitions
        // above it. A `val` bound to a lambda doesn't run anything.
        for (i, (loc, item)) in items.iter().enumerate() {
            let runs = match item {
                Item::Define(..) => false,
                Item::Val(_, e) => !matches!(&**e, Exp::Lambda(_)),
                Item::Expr(_) => true,
            };
            if !runs {
                continue;
            }
            let mut seen = BTreeSet::new();
            let mut todo: Vec<Name> = deps(item).into_iter().collect();
            while let Some(x) = todo.pop() {
                if !seen.insert(x.clone()) {
                    continue;
                }
                let j = index[&x];
                if j > i {
                    report.errors.push(format!(
                        "{}: this runs while {} loads, and needs {}, which is defined below it at {} \
                         (move the definition up: design/12 §4.4)",
                        loc, file, x, items[j].0
                    ));
                    break;
                }
                todo.extend(deps(&items[j].1));
            }
        }

        // Strongly connected components of the definitions, dependencies first.
        let graph: Vec<Vec<usize>> = items.iter().map(|(_, it)| deps(it).iter().map(|x| index[x]).collect()).collect();
        for group in sccs(&defs, &graph) {
            self.check_group(&items, &group, report);
        }

        // Top-level expressions, each on its own.
        for (loc, item) in &items {
            if let Item::Expr(e) = item {
                let mark = self.st.preds.len();
                let result = self.infer_closed(e, mark);
                if let Err(err) = result {
                    report.errors.push(format!("{}: {}", loc, err));
                }
            }
        }
    }

    /// Infer `e` with no local variables, then discharge its predicates.
    fn infer_closed(&mut self, e: &ExpRef, mark: usize) -> R<Ty> {
        let mut env = vec![];
        let result =
            self.infer(&mut env, e).and_then(|(t, _)| self.settle(mark, &BTreeSet::new(), &BTreeSet::new()).map(|_| t));
        // Nothing about a closed expression is owed to anything later.
        self.st.preds.truncate(mark);
        result
    }

    /// Check one binding group: monomorphic inside, generalized together.
    fn check_group(&mut self, items: &[(Loc, Item)], group: &[usize], report: &mut Report) {
        let mark = self.st.preds.len();
        let mut mono: Vec<(Name, Ty)> = vec![];
        for &i in group {
            let name = match &items[i].1 {
                Item::Define(f, _) => f.clone(),
                Item::Val(x, _) => x.clone(),
                Item::Expr(_) => unreachable!(),
            };
            let t = self.st.fresh();
            self.globals.insert(name.clone(), Global::Scheme(Scheme::mono(t.clone())));
            mono.push((name, t));
        }
        let mut failed = false;
        for (&i, (_, t)) in group.iter().zip(&mono) {
            let (loc, item) = &items[i];
            let result = (|| -> R<()> {
                let mut env = vec![];
                let got = match item {
                    Item::Define(_, l) => self.infer_lambda(&mut env, l)?.0,
                    Item::Val(_, e) => self.infer(&mut env, e)?.0,
                    Item::Expr(_) => unreachable!(),
                };
                self.st.unify(t, &got)
            })();
            if let Err(e) = result {
                let name = &mono[group.iter().position(|&j| j == i).unwrap()].0;
                report.errors.push(format!("{}: in the definition of {}: {}", loc, name, e));
                failed = true;
            }
        }
        // Generalize: the group's types share variables, so quantify each
        // over the variables of all of them.
        let mut gen_tvs = BTreeSet::new();
        for (_, t) in &mono {
            self.st.ftv(t, &mut gen_tvs);
        }
        if let Err(e) = self.settle(mark, &BTreeSet::new(), &gen_tvs) {
            report.errors.push(format!("{}: {}", items[group[0]].0, e));
            failed = true;
        }
        let preds: Vec<Pred> = self.st.preds.drain(mark..).collect();
        for (name, t) in mono {
            let loc = &items[group[mono_index(group, items, &name)]].0;
            let sc =
                if failed { Scheme::mono(t) } else { self.generalize(&t, &preds, &BTreeSet::new(), &BTreeSet::new()) };
            self.globals.insert(name.clone(), Global::Scheme(sc));
            self.defined_in.insert(name, loc.to_string());
        }
    }

    /// `∀` over the variables of `t` not free in the environment.
    fn generalize(&mut self, t: &Ty, preds: &[Pred], env_tvs: &BTreeSet<TV>, env_evs: &BTreeSet<EV>) -> Scheme {
        let ty = self.st.zonk(t);
        let mut tvs = BTreeSet::new();
        self.st.ftv(&ty, &mut tvs);
        let tvs: Vec<TV> = tvs.difference(env_tvs).copied().collect();
        let mut roots = BTreeSet::new();
        self.st.fev(&ty, &mut roots);
        let quantified: BTreeSet<EV> = roots.difference(env_evs).copied().collect();
        // Each quantified effect variable takes its lower bound along,
        // *flattened*: every atom it reaches, every other quantified variable
        // it reaches, and every variable the environment can still extend,
        // which must stay shared. Copying only the direct bound would lose
        // effects that reach the type through a variable not in it: in
        // `(define g (f c) (attempt f c))`, g's effect includes the effect of
        // the call, which includes f's, and only g's and f's are in g's type.
        let mut evs: Vec<(EV, Eff)> = vec![];
        for &r in &quantified {
            let (atoms, reached) = self.st.closure(r);
            let vars =
                reached.into_iter().filter(|v| *v != r && (quantified.contains(v) || env_evs.contains(v))).collect();
            evs.push((r, Eff { atoms, vars }));
        }
        let mut kept = vec![];
        for p in preds {
            let z = self.st.zonk_pred(p);
            let mut v = BTreeSet::new();
            self.st.pred_ftv(&z, &mut v);
            if v.iter().any(|x| tvs.contains(x)) {
                kept.push(z);
            }
        }
        Scheme { tvs, evs, preds: kept, ty }
    }

    fn instantiate(&mut self, sc: &Scheme) -> Ty {
        let tmap: HashMap<TV, Ty> = sc.tvs.iter().map(|&v| (v, self.st.fresh())).collect();
        let emap: HashMap<EV, EV> = sc.evs.iter().map(|(e, _)| (*e, self.st.fresh_ev())).collect();
        for (e, lower) in &sc.evs {
            let mapped = Eff {
                atoms: lower.atoms.clone(),
                vars: lower.vars.iter().map(|v| *emap.get(v).unwrap_or(v)).collect(),
            };
            self.st.include(emap[e], &mapped);
        }
        let ty = subst_ty(&sc.ty, &tmap, &emap);
        for p in &sc.preds {
            let q = subst_pred(p, &tmap, &emap);
            self.st.preds.push(q);
        }
        ty
    }

    // ------------------------------------------------------------ predicates

    /// Discharge the predicates added since `mark`: simplify to a fixed
    /// point, default the numeric ones on variables that `keep` doesn't
    /// mention and the environment doesn't hold, and report what can never
    /// hold. Predicates still on `keep`'s variables stay for the scheme.
    fn settle(&mut self, mark: usize, env_tvs: &BTreeSet<TV>, keep: &BTreeSet<TV>) -> R<()> {
        loop {
            self.simplify_from(mark)?;
            // Default per binding group (design/12 §3).
            let mut defaulted = false;
            let pending: Vec<Pred> = self.st.preds[mark..].to_vec();
            for p in pending {
                let mut vs = BTreeSet::new();
                self.st.pred_ftv(&p, &mut vs);
                let ambiguous = |v: &TV| !keep.contains(v) && !env_tvs.contains(v);
                match &p {
                    Pred::Arith(t) | Pred::Ord(t) => {
                        if let Ty::Var(v) = self.st.prune(t)
                            && ambiguous(&v)
                        {
                            self.st.unify(&Ty::Var(v), &con("Num"))?;
                            defaulted = true;
                        }
                    }
                    Pred::Mul(..) | Pred::Div(..) => {
                        for v in vs.iter().filter(|v| ambiguous(v)) {
                            self.st.unify(&Ty::Var(*v), &con("Num"))?;
                            defaulted = true;
                        }
                    }
                    _ => {}
                }
            }
            if !defaulted {
                break;
            }
        }
        // What remains must be about variables the scheme or the environment keeps.
        let mut i = mark;
        while i < self.st.preds.len() {
            let p = self.st.zonk_pred(&self.st.preds[i].clone());
            // A field of a record nothing can ever decide is ambiguous, even
            // when the field's own type is kept: no use of the scheme could
            // say which record it is.
            if let Pred::Has(r, f, _) = &p {
                let mut rv = BTreeSet::new();
                self.st.ftv(r, &mut rv);
                if !rv.is_empty() && !rv.iter().any(|v| keep.contains(v) || env_tvs.contains(v)) {
                    return Err(format!("can't tell which record's field {} this is; several records have it", f));
                }
            }
            let mut vs = BTreeSet::new();
            self.st.pred_ftv(&p, &mut vs);
            if vs.iter().any(|v| keep.contains(v) || env_tvs.contains(v)) {
                i += 1;
                continue;
            }
            match p {
                // `show` of something nothing pins down: any showable value
                // will do, and the interpreter checks the one it gets.
                Pred::Showable(_) => {
                    self.st.preds.remove(i);
                }
                Pred::Has(_, f, _) => {
                    return Err(format!("can't tell which record's field {} this is; several records have it", f));
                }
                _ => i += 1,
            }
        }
        Ok(())
    }

    fn simplify_from(&mut self, mark: usize) -> R<()> {
        loop {
            let mut changed = false;
            let mut i = mark;
            while i < self.st.preds.len() {
                let p = self.st.zonk_pred(&self.st.preds[i].clone());
                match self.step(&p)? {
                    Step::Keep => {
                        self.st.preds[i] = p;
                        i += 1;
                    }
                    Step::Done => {
                        self.st.preds.remove(i);
                        changed = true;
                    }
                    Step::Then(ps) => {
                        self.st.preds.remove(i);
                        self.st.preds.extend(ps);
                        changed = true;
                    }
                    Step::Unify(eqs) => {
                        self.st.preds.remove(i);
                        for (a, b) in eqs {
                            self.st.unify(&a, &b)?;
                        }
                        changed = true;
                    }
                    Step::UnifyThen(eqs, more) => {
                        self.st.preds.remove(i);
                        for (a, b) in eqs {
                            self.st.unify(&a, &b)?;
                        }
                        self.st.preds.extend(more);
                        changed = true;
                    }
                }
            }
            if !changed {
                return Ok(());
            }
        }
    }

    /// One step of simplifying a predicate.
    fn step(&mut self, p: &Pred) -> R<Step> {
        let name = |t: &Ty| match t {
            Ty::Con(n, a) if a.is_empty() => Some(n.to_string()),
            _ => None,
        };
        let is_var = |t: &Ty| matches!(t, Ty::Var(_));
        let class = |t: &Ty, ok: &[&str], what: &str, st: &mut State| -> R<Step> {
            if is_var(t) {
                return Ok(Step::Keep);
            }
            match name(t) {
                Some(n) if ok.contains(&n.as_str()) => Ok(Step::Done),
                _ => {
                    let mut pr = Printer::default();
                    Err(format!("{} works on {}, not on {}", what, ok.join(", "), pr.ty(st, t, false)))
                }
            }
        };
        match p {
            Pred::Arith(t) => class(t, &["Num", "Money", "Dur"], "+ and -", &mut self.st),
            Pred::Ord(t) => class(t, &["Num", "Money", "Dur", "Text"], "< and >", &mut self.st),
            Pred::Mul(a, b, c) | Pred::Div(a, b, c) => {
                let table: &[[&str; 3]] = if matches!(p, Pred::Mul(..)) {
                    &[
                        ["Num", "Num", "Num"],
                        ["Money", "Num", "Money"],
                        ["Num", "Money", "Money"],
                        ["Dur", "Num", "Dur"],
                        ["Num", "Dur", "Dur"],
                    ]
                } else {
                    &[["Num", "Num", "Num"], ["Money", "Num", "Money"], ["Dur", "Num", "Dur"]]
                };
                let have = [a, b, c];
                let fits: Vec<&[&str; 3]> = table
                    .iter()
                    .filter(|row| have.iter().zip(row.iter()).all(|(t, n)| is_var(t) || name(t).as_deref() == Some(*n)))
                    .collect();
                match fits.len() {
                    0 => {
                        let mut pr = Printer::default();
                        let shown: Vec<String> = have.iter().map(|t| pr.ty(&mut self.st, t, false)).collect();
                        let op = if matches!(p, Pred::Mul(..)) { "*" } else { "/" };
                        Err(format!("{} can't take {} and {} to {}", op, shown[0], shown[1], shown[2]))
                    }
                    1 => {
                        Ok(Step::Unify(have.iter().zip(fits[0].iter()).map(|(t, n)| ((*t).clone(), con(n))).collect()))
                    }
                    _ => Ok(Step::Keep),
                }
            }
            Pred::Showable(t) => match t {
                Ty::Var(_) => Ok(Step::Keep),
                Ty::Fun(..) => Err("a function can't be shown".into()),
                Ty::Con(n, args) => match &**n {
                    "Text" | "Num" | "Bool" | "Sym" | "Money" | "Dur" => Ok(Step::Done),
                    "List" => Ok(Step::Then(vec![Pred::Showable(args[0].clone())])),
                    "Grant" => Err("a model, a person or a capability can't be shown".into()),
                    _ => {
                        let fields = self.field_types(n, args)?;
                        Ok(Step::Then(fields.into_iter().map(Pred::Showable).collect()))
                    }
                },
            },
            Pred::Has(t, f, ft) => match t {
                Ty::Con(r, args) => {
                    if !self.theta.is_record(r) {
                        return Err(format!("{} is not a record, so it has no field {}", r, f));
                    }
                    let fields = self.record_fields(r, args)?;
                    match fields.into_iter().find(|(g, _)| g == f) {
                        Some((_, t)) => Ok(Step::Unify(vec![(ft.clone(), t)])),
                        None => Err(format!("record {} has no field {}", r, f)),
                    }
                }
                Ty::Fun(..) => Err(format!("a function has no field {}", f)),
                Ty::Var(_) => {
                    // A field that only one record has decides the record.
                    let owners: Vec<Name> = self
                        .theta
                        .records
                        .iter()
                        .filter(|(_, fs)| fs.iter().any(|(g, _)| g == f))
                        .map(|(r, _)| r.clone())
                        .collect();
                    match owners.len() {
                        0 => Err(format!("no record has a field {}", f)),
                        1 => {
                            // Decide the record, and keep the predicate: it
                            // still has to tie the field's type to the
                            // record's. Dropping it here lost that link, so
                            // `swap` came out as ((Pair a b) -> (Pair c a)).
                            let rt = self.fresh_instance(&owners[0]);
                            Ok(Step::UnifyThen(vec![(t.clone(), rt)], vec![p.clone()]))
                        }
                        _ => Ok(Step::Keep),
                    }
                }
            },
        }
    }

    // ------------------------------------------------------------ Θ

    fn convert(&mut self, t: &Type, params: &HashMap<Name, Ty>) -> R<Ty> {
        Ok(match t {
            Type::Text(_) => con("Text"),
            Type::Num => con("Num"),
            Type::Bool => con("Bool"),
            Type::Sym => con("Sym"),
            Type::Money => con("Money"),
            Type::Dur => con("Dur"),
            Type::List(e, _) => list_of(self.convert(e, params)?),
            Type::Named(n, args) => {
                let args = args.iter().map(|a| self.convert(a, params)).collect::<R<Vec<_>>>()?;
                Ty::Con(n.clone(), args)
            }
            Type::Var(v) => params.get(v).cloned().ok_or_else(|| format!("unknown type variable {}", v))?,
            Type::Any => return Err("Any has no static meaning (design/12, T5)".into()),
        })
    }

    fn params(&mut self, n: &Name) -> (Vec<Ty>, HashMap<Name, Ty>) {
        let ps: Vec<Name> = self.theta.params.get(n).map(|p| (**p).clone()).unwrap_or_default();
        let args: Vec<Ty> = ps.iter().map(|_| self.st.fresh()).collect();
        let map = ps.into_iter().zip(args.iter().cloned()).collect();
        (args, map)
    }

    fn fresh_instance(&mut self, n: &Name) -> Ty {
        let (args, _) = self.params(n);
        Ty::Con(n.clone(), args)
    }

    fn sigma(&self, n: &Name, args: &[Ty]) -> R<HashMap<Name, Ty>> {
        let ps: Vec<Name> = self.theta.params.get(n).map(|p| (**p).clone()).unwrap_or_default();
        if ps.len() != args.len() {
            return Err(format!("{} takes {} type argument(s), but was given {}", n, ps.len(), args.len()));
        }
        Ok(ps.into_iter().zip(args.iter().cloned()).collect())
    }

    fn record_fields(&mut self, r: &Name, args: &[Ty]) -> R<Vec<(Name, Ty)>> {
        let sigma = self.sigma(r, args)?;
        let fs = self.theta.records.get(r).cloned().ok_or_else(|| format!("unknown record {}", r))?;
        fs.iter().map(|(f, t)| Ok((f.clone(), self.convert(t, &sigma)?))).collect()
    }

    /// Every field type of a datatype or record at its arguments.
    fn field_types(&mut self, n: &Name, args: &[Ty]) -> R<Vec<Ty>> {
        if self.theta.is_record(n) {
            return Ok(self.record_fields(n, args)?.into_iter().map(|(_, t)| t).collect());
        }
        let sigma = self.sigma(n, args)?;
        let cons = self.theta.datatypes.get(n).cloned().ok_or_else(|| format!("unknown type {}", n))?;
        let mut out = vec![];
        for c in cons.iter() {
            for (_, t) in &c.fields {
                out.push(self.convert(t, &sigma)?);
            }
        }
        Ok(out)
    }

    /// A constructor's datatype at fresh arguments, and its field types.
    fn constructor(&mut self, k: &Name) -> R<(Ty, Vec<Ty>)> {
        let (dt, _) = self.theta.con_owner.get(k).cloned().ok_or_else(|| format!("unknown constructor {}", k))?;
        let def = self.theta.con_def(k).cloned().expect("owner implies definition");
        let (args, map) = self.params(&dt);
        let fields = def.fields.iter().map(|(_, t)| self.convert(t, &map)).collect::<R<Vec<_>>>()?;
        Ok((Ty::Con(dt, args), fields))
    }

    // ------------------------------------------------------------ inference

    fn lookup(&mut self, env: &[(Name, Scheme)], x: &Name) -> R<Ty> {
        if let Some((_, sc)) = env.iter().rev().find(|(y, _)| y == x) {
            let sc = sc.clone();
            return Ok(self.instantiate(&sc));
        }
        match self.globals.get(x).cloned() {
            Some(Global::Scheme(sc)) => Ok(self.instantiate(&sc)),
            Some(Global::Prim(p)) => match prim_type(p, &mut self.st) {
                Some(t) => Ok(t),
                None => Err(format!("{} takes any number of arguments, so it can only be applied", p)),
            },
            None => Err(format!("unbound name {}", x)),
        }
    }

    fn env_tvs(&self, env: &[(Name, Scheme)]) -> BTreeSet<TV> {
        let mut out = BTreeSet::new();
        for (_, sc) in env {
            let mut vs = BTreeSet::new();
            self.st.ftv(&sc.ty, &mut vs);
            out.extend(vs.into_iter().filter(|v| !sc.tvs.contains(v)));
        }
        out
    }

    /// Effect variables the environment holds, and everything they reach:
    /// the effects of functions bound to local names, which later uses may
    /// still extend.
    fn env_evs(&mut self, env: &[(Name, Scheme)]) -> BTreeSet<EV> {
        let mut out = BTreeSet::new();
        for (_, sc) in env {
            let mut es = BTreeSet::new();
            self.st.fev(&sc.ty, &mut es);
            for e in es {
                if !sc.evs.iter().any(|(q, _)| *q == e) {
                    let (_, reached) = self.st.closure(e);
                    out.extend(reached);
                }
            }
        }
        out
    }

    fn infer_lambda(&mut self, env: &mut Vec<(Name, Scheme)>, l: &Lambda) -> R<(Ty, Eff)> {
        let args: Vec<Ty> = l.formals.iter().map(|_| self.st.fresh()).collect();
        let n = env.len();
        for (x, t) in l.formals.iter().zip(&args) {
            env.push((x.clone(), Scheme::mono(t.clone())));
        }
        let result = self.infer(env, &l.body);
        env.truncate(n);
        let (body, eff) = result?;
        let phi = self.st.fresh_ev();
        self.st.include(phi, &eff);
        Ok((Ty::Fun(args, Box::new(body), phi), Eff::default()))
    }

    /// `Γ ⊢ e : τ ! ε`.
    fn infer(&mut self, env: &mut Vec<(Name, Scheme)>, e: &ExpRef) -> R<(Ty, Eff)> {
        let pure = Eff::default();
        match &**e {
            Exp::Literal(v) => Ok((
                match v {
                    Value::Num(_) => con("Num"),
                    Value::Str(_) => con("Text"),
                    Value::Bool(_) => con("Bool"),
                    Value::Sym(_) => con("Sym"),
                    Value::Money(_) => con("Money"),
                    Value::Dur(_) => con("Dur"),
                    Value::Nil => list_of(self.st.fresh()),
                    other => return Err(format!("unexpected literal {}", other)),
                },
                pure,
            )),
            Exp::Var(x) => Ok((self.lookup(env, x)?, pure)),
            Exp::Con(k, args) => {
                let (t, fields) = self.constructor(k)?;
                if fields.len() != args.len() {
                    return Err(format!("constructor {} expects {} argument(s), got {}", k, fields.len(), args.len()));
                }
                let mut eff = Eff::default();
                for (a, ft) in args.iter().zip(&fields) {
                    let (at, ae) = self.infer(env, a)?;
                    self.st.unify(&at, ft).map_err(|m| format!("in an argument of {}: {}", k, m))?;
                    eff = eff.union(&ae);
                }
                Ok((t, eff))
            }
            Exp::Record(r, given) => {
                let rt = self.fresh_instance(r);
                let Ty::Con(_, args) = &rt else { unreachable!() };
                let fields = self.record_fields(r, &args.clone())?;
                let mut eff = Eff::default();
                for (f, ge) in given {
                    let Some((_, ft)) = fields.iter().find(|(g, _)| g == f) else {
                        return Err(format!("record {} has no field {}", r, f));
                    };
                    let ft = ft.clone();
                    let (gt, ge) = self.infer(env, ge)?;
                    self.st.unify(&gt, &ft).map_err(|m| format!("in field {} of {}: {}", f, r, m))?;
                    eff = eff.union(&ge);
                }
                for (f, _) in &fields {
                    if given.iter().filter(|(g, _)| g == f).count() != 1 {
                        return Err(format!("record {} needs field {} exactly once", r, f));
                    }
                }
                Ok((rt, eff))
            }
            Exp::Field(r, f) => {
                let (rt, eff) = self.infer(env, r)?;
                let ft = self.st.fresh();
                self.st.preds.push(Pred::Has(rt, f.clone(), ft.clone()));
                Ok((ft, eff))
            }
            Exp::If(c, a, b) => {
                let (ct, ce) = self.infer(env, c)?;
                self.st.unify(&ct, &con("Bool")).map_err(|m| format!("in the test of an if: {}", m))?;
                let (at, ae) = self.infer(env, a)?;
                let (bt, be) = self.infer(env, b)?;
                self.st.unify(&at, &bt).map_err(|m| format!("the two branches of an if differ: {}", m))?;
                Ok((at, ce.union(&ae).union(&be)))
            }
            Exp::Let(x, rhs, body) => {
                let mark = self.st.preds.len();
                let (rt, re) = self.infer(env, rhs)?;
                // Type variables generalize freely: µNorman has no mutable
                // references (design/12 §4.1). Name variables, which step 3
                // adds, will generalize only for syntactic values.
                let env_tvs = self.env_tvs(env);
                let env_evs = self.env_evs(env);
                let mut keep = BTreeSet::new();
                self.st.ftv(&rt, &mut keep);
                self.settle(mark, &env_tvs, &keep)?;
                let preds: Vec<Pred> = self.st.preds.drain(mark..).collect();
                let sc = self.generalize(&rt, &preds, &env_tvs, &env_evs);
                // Predicates the scheme didn't take stay pending outside it.
                let taken: Vec<Pred> = sc.preds.clone();
                for p in preds {
                    let z = self.st.zonk_pred(&p);
                    if !taken.contains(&z) {
                        self.st.preds.push(z);
                    }
                }
                env.push((x.clone(), sc));
                let result = self.infer(env, body);
                env.pop();
                let (bt, be) = result?;
                Ok((bt, re.union(&be)))
            }
            Exp::Lambda(l) => self.infer_lambda(env, l),
            Exp::Apply(f, args) => {
                // `list` and `string-append` take any number of arguments.
                if let Exp::Var(name) = &**f
                    && !env.iter().any(|(y, _)| y == name)
                    && let Some(Global::Prim(p @ ("list" | "string-append"))) = self.globals.get(name).cloned()
                {
                    let elem = if p == "list" { self.st.fresh() } else { con("Text") };
                    let mut eff = Eff::default();
                    for a in args {
                        let (at, ae) = self.infer(env, a)?;
                        self.st.unify(&at, &elem).map_err(|m| format!("in an argument of {}: {}", p, m))?;
                        eff = eff.union(&ae);
                    }
                    return Ok((if p == "list" { list_of(elem) } else { elem }, eff));
                }
                let (ft, fe) = self.infer(env, f)?;
                let mut eff = fe;
                let mut ats = vec![];
                for a in args {
                    let (at, ae) = self.infer(env, a)?;
                    ats.push(at);
                    eff = eff.union(&ae);
                }
                let res = self.st.fresh();
                let phi = self.st.fresh_ev();
                self.st
                    .unify(&ft, &Ty::Fun(ats, Box::new(res.clone()), phi))
                    .map_err(|m| format!("in an application: {}", m))?;
                Ok((res, eff.union(&Eff::var(phi))))
            }
            Exp::Case(scrutinee, branches) => {
                let (st_ty, mut eff) = self.infer(env, scrutinee)?;
                let result = self.st.fresh();
                for (pat, body) in branches {
                    let n = env.len();
                    let bound = self.pattern(pat, &st_ty)?;
                    env.extend(bound);
                    let r = self.infer(env, body);
                    env.truncate(n);
                    let (bt, be) = r?;
                    self.st.unify(&bt, &result).map_err(|m| format!("the branches of a case differ: {}", m))?;
                    eff = eff.union(&be);
                }
                Ok((result, eff))
            }
            Exp::Ask { .. } => Err("the checker doesn't handle ask yet (design/12 §9, step 3)".into()),
            Exp::Call { .. } => Err("the checker doesn't handle call yet (design/12 §9, step 3)".into()),
            Exp::Consult { .. } => Err("the checker doesn't handle consult yet (design/12 §9, step 3)".into()),
            Exp::Fail(x) => {
                let (xt, xe) = self.infer(env, x)?;
                self.st.unify(&xt, &con("Failure")).map_err(|m| format!("fail needs a Failure: {}", m))?;
                Ok((self.st.fresh(), xe))
            }
            Exp::Catch(body, x, handler) => {
                let (bt, be) = self.infer(env, body)?;
                env.push((x.clone(), Scheme::mono(con("Failure"))));
                let r = self.infer(env, handler);
                env.pop();
                let (ht, he) = r?;
                self.st.unify(&bt, &ht).map_err(|m| format!("a catch's handler differs from its body: {}", m))?;
                Ok((bt, be.union(&he)))
            }
            Exp::Budget { cost, time, body } => {
                let mut eff = Eff::default();
                if let Some(c) = cost {
                    let (ct, ce) = self.infer(env, c)?;
                    self.st.unify(&ct, &con("Money")).map_err(|m| format!("a budget's cost: {}", m))?;
                    eff = eff.union(&ce);
                }
                if let Some(t) = time {
                    let (tt, te) = self.infer(env, t)?;
                    self.st.unify(&tt, &con("Dur")).map_err(|m| format!("a budget's time: {}", m))?;
                    eff = eff.union(&te);
                }
                let (bt, be) = self.infer(env, body)?;
                Ok((bt, eff.union(&be)))
            }
            Exp::Workflow(w) => {
                // Each node sees the nodes it depends on; the body sees them
                // all. Ownership (design/12 §5) is step 4.
                let n = env.len();
                let mut types: Vec<Option<Ty>> = vec![None; w.nodes.len()];
                let mut eff = Eff::default();
                let mut remaining: Vec<usize> = (0..w.nodes.len()).collect();
                while !remaining.is_empty() {
                    let pos = remaining
                        .iter()
                        .position(|&i| w.nodes[i].deps.iter().all(|&d| types[d].is_some()))
                        .expect("the parser rejects cyclic workflows");
                    let i = remaining.remove(pos);
                    let node = &w.nodes[i];
                    for &d in &node.deps {
                        env.push((w.nodes[d].name.clone(), Scheme::mono(types[d].clone().unwrap())));
                    }
                    let r = self.infer(env, &node.exp);
                    env.truncate(n);
                    let (t, e) = r?;
                    types[i] = Some(t);
                    eff = eff.union(&e);
                }
                for (node, t) in w.nodes.iter().zip(&types) {
                    env.push((node.name.clone(), Scheme::mono(t.clone().unwrap())));
                }
                let r = self.infer(env, &w.body);
                env.truncate(n);
                let (bt, be) = r?;
                Ok((bt, eff.union(&be)))
            }
        }
    }

    /// The variables a pattern binds, matching a scrutinee of type `t`.
    fn pattern(&mut self, p: &Pattern, t: &Ty) -> R<Vec<(Name, Scheme)>> {
        match p {
            Pattern::Wild => Ok(vec![]),
            Pattern::Nil => {
                let e = self.st.fresh();
                self.st.unify(t, &list_of(e)).map_err(|m| format!("in a '() pattern: {}", m))?;
                Ok(vec![])
            }
            Pattern::Cons(h, tl) => {
                let e = self.st.fresh();
                self.st.unify(t, &list_of(e.clone())).map_err(|m| format!("in a cons pattern: {}", m))?;
                let mut out = vec![];
                if let Some(h) = h {
                    out.push((h.clone(), Scheme::mono(e.clone())));
                }
                if let Some(tl) = tl {
                    out.push((tl.clone(), Scheme::mono(list_of(e))));
                }
                Ok(out)
            }
            Pattern::Con(k, vars) => {
                let (ct, fields) = self.constructor(k)?;
                if fields.len() != vars.len() {
                    return Err(format!(
                        "constructor {} has {} field(s), but the pattern names {}",
                        k,
                        fields.len(),
                        vars.len()
                    ));
                }
                self.st.unify(t, &ct).map_err(|m| format!("in a pattern for {}: {}", k, m))?;
                Ok(vars
                    .iter()
                    .zip(fields)
                    .filter_map(|(v, ft)| v.as_ref().map(|v| (v.clone(), Scheme::mono(ft))))
                    .collect())
            }
        }
    }

    /// A unit test's expressions are checked on their own (T8).
    fn check_test(&mut self, t: &UnitTest) -> R<()> {
        let mark = self.st.preds.len();
        let result = (|| -> R<()> {
            let mut env = vec![];
            match &t.kind {
                TestKind::Expect(a, b) | TestKind::Equiv(a, b, _) => {
                    let (ta, _) = self.infer(&mut env, a)?;
                    let (tb, _) = self.infer(&mut env, b)?;
                    self.st.unify(&ta, &tb).map_err(|m| format!("the two sides of this test differ: {}", m))
                }
                TestKind::Assert(a) => {
                    let (ta, _) = self.infer(&mut env, a)?;
                    self.st.unify(&ta, &con("Bool"))
                }
                TestKind::Error(a) | TestKind::Within(a, _, _) => self.infer(&mut env, a).map(|_| ()),
                TestKind::Fail(a, p) => {
                    self.infer(&mut env, a)?;
                    self.pattern(p, &con("Failure")).map(|_| ())
                }
            }
        })();
        let settled = result.and_then(|_| self.settle(mark, &BTreeSet::new(), &BTreeSet::new()));
        self.st.preds.truncate(mark);
        settled
    }
}

fn mono_index(group: &[usize], items: &[(Loc, Item)], name: &Name) -> usize {
    group
        .iter()
        .position(|&i| match &items[i].1 {
            Item::Define(f, _) | Item::Val(f, _) => f == name,
            Item::Expr(_) => false,
        })
        .expect("every group member has a name")
}

fn subst_ty(t: &Ty, tmap: &HashMap<TV, Ty>, emap: &HashMap<EV, EV>) -> Ty {
    match t {
        Ty::Var(v) => tmap.get(v).cloned().unwrap_or(Ty::Var(*v)),
        Ty::Con(n, args) => Ty::Con(n.clone(), args.iter().map(|a| subst_ty(a, tmap, emap)).collect()),
        Ty::Fun(args, res, e) => Ty::Fun(
            args.iter().map(|a| subst_ty(a, tmap, emap)).collect(),
            Box::new(subst_ty(res, tmap, emap)),
            *emap.get(e).unwrap_or(e),
        ),
    }
}

fn subst_pred(p: &Pred, tm: &HashMap<TV, Ty>, em: &HashMap<EV, EV>) -> Pred {
    let s = |t: &Ty| subst_ty(t, tm, em);
    match p {
        Pred::Arith(t) => Pred::Arith(s(t)),
        Pred::Ord(t) => Pred::Ord(s(t)),
        Pred::Showable(t) => Pred::Showable(s(t)),
        Pred::Mul(a, b, c) => Pred::Mul(s(a), s(b), s(c)),
        Pred::Div(a, b, c) => Pred::Div(s(a), s(b), s(c)),
        Pred::Has(a, f, b) => Pred::Has(s(a), f.clone(), s(b)),
    }
}

/// Tarjan's algorithm over the definitions `nodes`: components in an order
/// where each comes after everything it depends on.
fn sccs(nodes: &[usize], graph: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct T<'a> {
        graph: &'a [Vec<usize>],
        index: HashMap<usize, usize>,
        low: HashMap<usize, usize>,
        stack: Vec<usize>,
        on: HashSet<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(t: &mut T, v: usize) {
        t.index.insert(v, t.next);
        t.low.insert(v, t.next);
        t.next += 1;
        t.stack.push(v);
        t.on.insert(v);
        for &w in &t.graph[v] {
            if !t.index.contains_key(&w) {
                visit(t, w);
                let lw = t.low[&w];
                let lv = t.low.get_mut(&v).unwrap();
                *lv = (*lv).min(lw);
            } else if t.on.contains(&w) {
                let iw = t.index[&w];
                let lv = t.low.get_mut(&v).unwrap();
                *lv = (*lv).min(iw);
            }
        }
        if t.low[&v] == t.index[&v] {
            let mut comp = vec![];
            loop {
                let w = t.stack.pop().unwrap();
                t.on.remove(&w);
                comp.push(w);
                if w == v {
                    break;
                }
            }
            comp.sort();
            t.out.push(comp);
        }
    }
    let mut t = T {
        graph,
        index: HashMap::new(),
        low: HashMap::new(),
        stack: vec![],
        on: HashSet::new(),
        next: 0,
        out: vec![],
    };
    for &v in nodes {
        if !t.index.contains_key(&v) {
            visit(&mut t, v);
        }
    }
    t.out
}
