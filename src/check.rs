//! The type and effect checker (design/12), steps 1 to 4: inference for
//! every form, and ownership: concurrent workflow nodes share no stateful
//! capability (§5).
//!
//! - Types are inferred as in Ramsey's µML (Hindley-Milner), with type
//!   parameters on datatypes and records (design/12, T5).
//! - Overloaded and restricted operations are **qualified types** (T4): a
//!   scheme carries predicates such as `Ord a`, checked when it is
//!   instantiated, and defaulted per binding group as SML does (§3).
//! - Effects follow Talpin and Jouvelot (§2): every arrow carries an effect
//!   *variable*, and the rules add lower bounds to it (`ε ⊆ φ`). Unifying two
//!   arrows unifies their effect variables. The atoms are `observe`, for
//!   `(remaining)`, and `ask μ`, `call κ` and `consult η`, each with the
//!   name of what it reaches (T3).
//! - A file is checked as a whole before it runs (T7): definitions are
//!   grouped into strongly connected components and generalized together,
//!   a name defined twice is an error (Q-2), and a form that runs at load
//!   time may depend only on definitions above it (§4.4).
//!
//! **Names in types** (§1). A grant's type carries the grant's name:
//! `(Model claude)`, `(Human reviewer)`, `(Kernel py)`. A name is a
//! [`Ty::Grant`], or a type variable standing for one. A capability's type
//! has *two* names: its own, which ownership is about, and the grant whose
//! operations it offers, which `Offers` is about. They are the same for a
//! granted capability. A fork gets a fresh name of its own (§5.2) but keeps
//! its parent's operations, so a fork of a closed kernel is closed too.
//! Capability names are the only variables kept from generalizing when the
//! bound expression isn't a syntactic value (§4.1): `(val k (call py fork))`
//! is one kernel, and every use of `k` must say so.

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
    /// The types of grants use constructors whose names start with `#`,
    /// which no program can write, so a user's `Model` datatype can't be
    /// confused with them: `(#Model μ)`, `(#Human η)`, and
    /// `(#Cap kind name offers)` with kind `#Kernel`, `#Filesystem` or `#Http`.
    Con(Name, Vec<Ty>),
    /// A function: arguments, result, and the variable for its latent effect.
    Fun(Vec<Ty>, Box<Ty>, EV),
    /// A grant's name, in a name position of a grant's type.
    Grant(Name),
}

fn con(n: &str) -> Ty {
    Ty::Con(Rc::from(n), vec![])
}
fn list_of(t: Ty) -> Ty {
    Ty::Con(Rc::from("List"), vec![t])
}
fn model_of(n: Ty) -> Ty {
    Ty::Con(Rc::from("#Model"), vec![n])
}
fn human_of(n: Ty) -> Ty {
    Ty::Con(Rc::from("#Human"), vec![n])
}
fn cap_of(kind: Ty, name: Ty, offers: Ty) -> Ty {
    Ty::Con(Rc::from("#Cap"), vec![kind, name, offers])
}
fn is_grant_type(n: &str) -> bool {
    matches!(n, "#Model" | "#Human" | "#Cap")
}

/// A name in an effect atom: a grant's, or a variable standing for one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Nm {
    Grant(Name),
    Var(TV),
}

/// An effect atom (design/12 §2).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Atom {
    /// Reading `(remaining)`.
    Observe,
    Ask(Nm),
    /// A call on the capability named by the first, of the kind named by the
    /// second (`#Kernel`, `#Filesystem`, `#Http`, or a variable). The kind is
    /// what tells ownership whether the call touches something stateful
    /// (design/12 §1, "Names have kinds"); it isn't printed.
    Call(Nm, Nm),
    Consult(Nm),
}

impl Atom {
    fn names(&self) -> Vec<&Nm> {
        match self {
            Atom::Observe => vec![],
            Atom::Ask(n) | Atom::Consult(n) => vec![n],
            Atom::Call(n, k) => vec![n, k],
        }
    }

    fn map(&self, f: impl Fn(&Nm) -> Nm) -> Atom {
        match self {
            Atom::Observe => Atom::Observe,
            Atom::Ask(n) => Atom::Ask(f(n)),
            Atom::Call(n, k) => Atom::Call(f(n), f(k)),
            Atom::Consult(n) => Atom::Consult(f(n)),
        }
    }
}

/// A workflow node's footprint (design/12 §5.1): the capabilities it can
/// use, each with its kind, and the effect variables whose calls it makes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foot {
    names: Vec<(Ty, Ty)>,
    evs: Vec<EV>,
}

/// One workflow node's atoms and effect variables, for the law warnings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    atoms: Vec<Atom>,
    evs: Vec<EV>,
}

/// A program that is legal but falls outside a law's premise gets a
/// warning, not an error (design/12, T2; law W1 in `06` §4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Warning {
    /// A node reads `(remaining)`: W1's premise (ii).
    Observe,
    /// Two concurrent nodes consult one person: W1's premise (iv).
    SamePerson,
}

/// Whether a capability of this kind is stateful, if the kind is known.
fn stateful(kind: &Ty) -> Option<bool> {
    match kind {
        Ty::Con(k, _) => Some(matches!(&**k, "#Kernel" | "#Filesystem")),
        _ => None,
    }
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
    fn atom(a: Atom) -> Eff {
        Eff { atoms: [a].into(), vars: BTreeSet::new() }
    }
    fn union(mut self, other: &Eff) -> Eff {
        self.atoms.extend(other.atoms.iter().cloned());
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
    /// `Offers(κ, op, τ)`: the grant named κ offers `op`, answering τ. The
    /// first argument is a capability's *offers* name (see the module notes).
    Offers(Ty, Name, Ty),
    /// `F₁ # F₂`: two concurrent workflow nodes share no stateful
    /// capability (§5.1). When both footprints include the same effect
    /// variable this is `Stateless(φ)`, and it prints that way.
    Disjoint(Foot, Foot),
    /// A law warning, checked like `#` and reported where it can be
    /// decided: at once, or where a generic function's scheme is used. The
    /// name is that function's, for the message.
    Warn(Warning, Side, Side, Option<Name>),
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
    /// The variables that stand for a capability's own name, which
    /// generalize only for syntactic values (design/12 §4.1).
    cap_names: HashSet<TV>,
}

type R<T> = Result<T, String>;

impl State {
    fn fresh(&mut self) -> Ty {
        self.tsub.push(None);
        Ty::Var((self.tsub.len() - 1) as TV)
    }

    /// A fresh variable for a capability's own name.
    fn fresh_cap_name(&mut self) -> Ty {
        let t = self.fresh();
        if let Ty::Var(v) = t {
            self.cap_names.insert(v);
        }
        t
    }

    /// The name a name-position type stands for, as an atom holds it.
    fn nm(&self, t: &Ty) -> Nm {
        match self.prune(t) {
            Ty::Grant(g) => Nm::Grant(g),
            Ty::Var(v) => Nm::Var(v),
            // A capability's kind, which a `call` atom carries too.
            Ty::Con(k, args) if args.is_empty() => Nm::Grant(k),
            other => unreachable!("a name position holds {:?}", other),
        }
    }

    /// The type a name in an atom stands for.
    fn nm_ty(&self, n: &Nm) -> Ty {
        match n {
            Nm::Grant(k) if k.starts_with('#') => con(k),
            Nm::Grant(g) => Ty::Grant(g.clone()),
            Nm::Var(v) => self.prune(&Ty::Var(*v)),
        }
    }

    /// A footprint with its effect variables opened up: the capabilities of
    /// every `call` they reach, and every variable they reach, by root.
    fn expand(&mut self, f: &Foot) -> Foot {
        let mut names: Vec<(Ty, Ty)> = f.names.iter().map(|(n, k)| (self.zonk(n), self.zonk(k))).collect();
        let mut evs = BTreeSet::new();
        for &e in &f.evs {
            let (atoms, reached) = self.closure(e);
            for a in atoms {
                if let Atom::Call(n, k) = a {
                    names.push((self.nm_ty(&n), self.nm_ty(&k)));
                }
            }
            evs.extend(reached);
        }
        names.dedup();
        Foot { names, evs: evs.into_iter().collect() }
    }

    /// A node's atoms with its effect variables opened up, and every
    /// variable they reach.
    fn expand_side(&mut self, s: &Side) -> Side {
        let mut atoms: Vec<Atom> = s.atoms.iter().map(|a| self.norm_atom(a)).collect();
        let mut evs = BTreeSet::new();
        for &e in &s.evs {
            let (found, reached) = self.closure(e);
            atoms.extend(found);
            evs.extend(reached);
        }
        atoms.sort();
        atoms.dedup();
        Side { atoms, evs: evs.into_iter().collect() }
    }

    /// `F₁ # F₂`, as far as it can be decided now: an error if both use one
    /// stateful capability, and otherwise whether it may still fail later.
    fn disjoint(&mut self, l: &Foot, r: &Foot) -> R<bool> {
        let (l, r) = (self.expand(l), self.expand(r));
        // An effect variable can still grow, and so conflict with anything
        // on the other side.
        let grows = |a: &Foot, b: &Foot| !a.evs.is_empty() && !(b.names.is_empty() && b.evs.is_empty());
        let mut open = grows(&l, &r) || grows(&r, &l);
        for (n1, k1) in &l.names {
            for (n2, k2) in &r.names {
                let (s1, s2) = (stateful(k1), stateful(k2));
                if s1 == Some(false) || s2 == Some(false) {
                    continue;
                }
                match (n1, n2) {
                    (Ty::Grant(a), Ty::Grant(b)) if a != b => {}
                    _ if n1 == n2 && (s1 == Some(true) || s2 == Some(true)) => {
                        let what = match n1 {
                            Ty::Grant(g) => format!("{}, a stateful capability", g),
                            _ => "the same stateful capability".to_string(),
                        };
                        return Err(format!(
                            "two concurrent workflow nodes would both use {}. Give each node its own: fork it \
                             before the workflow, with (call k fork) (design/12 §5)",
                            what
                        ));
                    }
                    // Two names that may yet turn out to be one.
                    _ => open = true,
                }
            }
        }
        Ok(open)
    }

    /// An atom with its name resolved as far as the substitution allows.
    fn norm_atom(&self, a: &Atom) -> Atom {
        a.map(|n| match n {
            Nm::Var(v) => self.nm(&Ty::Var(*v)),
            g => g.clone(),
        })
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
        low.atoms.extend(eff.atoms.iter().cloned());
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
            let found: Vec<Atom> = self.evars[v as usize].lower.atoms.iter().map(|a| self.norm_atom(a)).collect();
            atoms.extend(found);
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
            Ty::Grant(g) => Ty::Grant(g),
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
            Ty::Grant(_) => false,
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
                // A capability's name stays a capability's name.
                if self.cap_names.contains(x)
                    && let Ty::Var(y) = t
                {
                    self.cap_names.insert(*y);
                }
                self.tsub[*x as usize] = Some(t.clone());
                Ok(())
            }
            (Ty::Grant(g), Ty::Grant(h)) if g == h => Ok(()),
            (Ty::Grant(g), Ty::Grant(h)) => {
                Err(format!("{} and {} are different grants, and a type names exactly one (design/12, T3)", h, g))
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
            Ty::Grant(_) => {}
            Ty::Con(_, args) => args.iter().for_each(|a| self.ftv(a, out)),
            Ty::Fun(args, res, _) => {
                args.iter().for_each(|a| self.ftv(a, out));
                self.ftv(&res, out);
            }
        }
    }

    fn fev(&mut self, t: &Ty, out: &mut BTreeSet<EV>) {
        match self.prune(t) {
            Ty::Var(_) | Ty::Grant(_) => {}
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
            Pred::Has(a, _, b) | Pred::Offers(a, _, b) => {
                self.ftv(a, out);
                self.ftv(b, out);
            }
            Pred::Disjoint(l, r) => {
                for (n, k) in l.names.iter().chain(&r.names) {
                    self.ftv(n, out);
                    self.ftv(k, out);
                }
            }
            Pred::Warn(_, l, r, _) => {
                for a in l.atoms.iter().chain(&r.atoms) {
                    for n in a.names() {
                        if let Nm::Var(v) = self.norm_nm(n) {
                            out.insert(v);
                        }
                    }
                }
            }
        }
    }

    fn norm_nm(&self, n: &Nm) -> Nm {
        match n {
            Nm::Var(v) => self.nm(&Ty::Var(*v)),
            g => g.clone(),
        }
    }

    /// The name variables in the atoms an effect variable reaches.
    fn atom_tvs(&mut self, phi: EV, out: &mut BTreeSet<TV>) {
        let (atoms, _) = self.closure(phi);
        for a in atoms {
            for n in a.names() {
                if let Nm::Var(v) = n {
                    out.insert(*v);
                }
            }
        }
    }

    /// Whether a predicate may still matter to something that outlives the
    /// current group: it mentions a type variable in `tvs`, or (for `#`) it
    /// reaches an effect variable in `evs`.
    fn pred_live(&mut self, p: &Pred, tvs: &BTreeSet<TV>, evs: &BTreeSet<EV>) -> bool {
        let mut vs = BTreeSet::new();
        self.pred_ftv(p, &mut vs);
        if vs.iter().any(|v| tvs.contains(v)) {
            return true;
        }
        match p {
            Pred::Disjoint(l, r) => {
                let (l, r) = (self.expand(l), self.expand(r));
                l.evs.iter().chain(&r.evs).any(|e| evs.contains(e))
            }
            Pred::Warn(_, l, r, _) => {
                let (l, r) = (self.expand_side(l), self.expand_side(r));
                l.evs.iter().chain(&r.evs).any(|e| evs.contains(e))
            }
            _ => false,
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
            Pred::Offers(a, op, b) => Pred::Offers(self.zonk(a), op.clone(), self.zonk(b)),
            Pred::Disjoint(l, r) => {
                let mut z = |f: &Foot| Foot {
                    names: f.names.iter().map(|(n, k)| (self.zonk(n), self.zonk(k))).collect(),
                    evs: f.evs.iter().map(|&e| self.find(e)).collect(),
                };
                Pred::Disjoint(z(l), z(r))
            }
            Pred::Warn(w, l, r, o) => {
                let z = |st: &mut State, s: &Side| Side {
                    atoms: s.atoms.iter().map(|a| st.norm_atom(a)).collect(),
                    evs: s.evs.iter().map(|&e| st.find(e)).collect(),
                };
                let (l, r) = (z(self, l), z(self, r));
                Pred::Warn(*w, l, r, o.clone())
            }
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
            Ty::Grant(g) => g.to_string(),
            Ty::Con(n, args) if &*n == "#Cap" => {
                // `(Kernel py)`; `(Kernel a py)` for a capability named a that
                // offers py's operations, such as a fork; `(Cap k a b)` when
                // the kind isn't known.
                let kind = match st.prune(&args[0]) {
                    Ty::Con(k, _) => k.trim_start_matches('#').to_string(),
                    other => format!("Cap {}", self.ty(st, &other, false)),
                };
                let (name, offers) = (st.prune(&args[1]), st.prune(&args[2]));
                let shown = self.ty(st, &name, false);
                if name == offers {
                    format!("({} {})", kind, shown)
                } else {
                    format!("({} {} {})", kind, shown, self.ty(st, &offers, false))
                }
            }
            Ty::Con(n, args) if args.is_empty() => n.trim_start_matches('#').to_string(),
            Ty::Con(n, args) => {
                let args: Vec<String> = args.iter().map(|a| self.ty(st, a, false)).collect();
                format!("({} {})", n.trim_start_matches('#'), args.join(" "))
            }
            Ty::Fun(args, res, e) => {
                let args: Vec<String> = args.iter().map(|a| self.ty(st, a, false)).collect();
                let res = self.ty(st, &res, false);
                let (atoms, vars) = st.closure(e);
                let mut eff: Vec<String> = atoms.iter().map(|a| self.atom(a)).collect();
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

    fn atom(&mut self, a: &Atom) -> String {
        let mut nm = |n: &Nm| match n {
            Nm::Grant(g) => g.to_string(),
            Nm::Var(v) => self.tname(*v),
        };
        match a {
            Atom::Observe => "observe".into(),
            Atom::Ask(n) => format!("ask {}", nm(n)),
            Atom::Call(n, _) => format!("call {}", nm(n)),
            Atom::Consult(n) => format!("consult {}", nm(n)),
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
            Pred::Offers(a, op, b) => format!("Offers {} {} {}", t(st, a), op, t(st, b)),
            Pred::Warn(..) => String::new(),
            Pred::Disjoint(l, r) => {
                // Shown by what a reader can see: the capabilities, and the
                // effect variables named in the type.
                let mut side = |st: &mut State, f: &Foot| -> Vec<String> {
                    let f = st.expand(f);
                    let mut out: Vec<String> = f.names.iter().map(|(n, _)| self.ty(st, n, false)).collect();
                    for e in f.evs {
                        if self.shown.contains(&e) {
                            out.push(self.ename(e));
                        }
                    }
                    out.sort();
                    out.dedup();
                    out
                };
                let (l, r) = (side(st, l), side(st, r));
                if l == r && !l.is_empty() {
                    format!("Stateless {}", l.join(" "))
                } else {
                    format!("({}) # ({})", l.join(" "), r.join(" "))
                }
            }
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
    /// Unit tests whose expressions are ill-typed, and `check-type-error`s
    /// whose expression is well typed. Each fails on its own, without
    /// rejecting the file (design/12, T8).
    pub test_errors: Vec<String>,
    /// What checking said about each unit test, by its location: `Ok` if
    /// the checker has nothing against it (for a `check-type-error`, that
    /// it passes), and otherwise why it fails.
    pub verdicts: HashMap<String, Result<(), String>>,
    /// Programs that are legal but fall outside a law (design/12, T2), and
    /// `case`s that may not match.
    pub warnings: Vec<String>,
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
    /// The operations each capability grant declares; none means an open
    /// interface, every operation answering Text (`10` §2.1).
    ops: HashMap<Name, Vec<(Name, Type)>>,
    /// Warnings found while checking the current form.
    pending: Vec<String>,
    /// The positions in the environment of names bound to an answer from
    /// `ask` or `consult`, for Q-3.
    answers: Vec<usize>,
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
            ops: HashMap::new(),
            pending: vec![],
            answers: vec![],
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
        let preds: Vec<String> = preds.iter().map(|q| p.pred(&mut self.st, q)).filter(|q| !q.is_empty()).collect();
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
                    report.verdicts.extend(used.verdicts);
                    report.warnings.extend(used.warnings);
                }
                Top::Grant(x, spec) => {
                    // A grant's type carries its name (design/12 §1).
                    let me = Ty::Grant(x.clone());
                    let (ty, ops) = match spec {
                        HostSpec::Model { .. } => (model_of(me), None),
                        HostSpec::Human { .. } => (human_of(me), None),
                        HostSpec::Kernel(ops) => (cap_of(con("#Kernel"), me.clone(), me), Some(ops)),
                        HostSpec::Filesystem(ops) => (cap_of(con("#Filesystem"), me.clone(), me), Some(ops)),
                        HostSpec::Http(ops) => (cap_of(con("#Http"), me.clone(), me), Some(ops)),
                    };
                    let before = report.errors.len();
                    self.define_global(&x, Global::Scheme(Scheme::mono(ty)), &loc.to_string(), &mut report);
                    if report.errors.len() == before
                        && let Some(ops) = ops
                    {
                        self.ops.insert(x, (*ops).clone());
                    }
                }
                Top::Script(..) => {}
                Top::Under(_, ts) => tests.extend(ts),
                Top::Test(t) => tests.push(t),
            }
        }
        self.check_items(file, items, &mut report);
        for t in &tests {
            let mut verdict = self.check_test(t);
            self.flush_warnings(&format!("{}: ", t.loc), &mut report);
            // A type error proves nothing in a file that doesn't check: the
            // name the test uses may be the one whose definition was wrong.
            if matches!(t.kind, TestKind::TypeError(_))
                && verdict.is_ok()
                && let Some(first) = report.errors.first()
            {
                verdict = Err(format!(
                    "Check-type-error failed: this file doesn't type-check, so a type error here proves nothing. \
                     The first error: {}",
                    first
                ));
            }
            if let Err(e) = &verdict {
                report.test_errors.push(format!("{}: {}", t.loc, e));
            }
            report.verdicts.insert(t.loc.to_string(), verdict);
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
                self.flush_warnings(&format!("{}: ", loc), report);
            }
        }
    }

    /// Infer `e` with no local variables, then discharge its predicates.
    fn infer_closed(&mut self, e: &ExpRef, mark: usize) -> R<Ty> {
        let mut env = vec![];
        let result = self
            .infer(&mut env, e)
            .and_then(|(t, _)| self.settle(mark, &BTreeSet::new(), &BTreeSet::new(), &BTreeSet::new()).map(|_| t));
        // Nothing about a closed expression is owed to anything later.
        self.st.preds.truncate(mark);
        result
    }

    /// Check one binding group: monomorphic inside, generalized together.
    fn check_group(&mut self, items: &[(Loc, Item)], group: &[usize], report: &mut Report) {
        let mark = self.st.preds.len();
        // What earlier definitions left monomorphic, before this group's own
        // names are placed in the globals.
        let outer = self.global_tvs();
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
        let mut gen_evs = BTreeSet::new();
        for (_, t) in &mono {
            self.st.ftv(t, &mut gen_tvs);
            self.st.fev(t, &mut gen_evs);
        }
        if let Err(e) = self.settle(mark, &outer, &gen_tvs, &gen_evs) {
            let names: Vec<&str> = mono.iter().map(|(n, _)| &**n).collect();
            report.errors.push(format!("{}: in the definition of {}: {}", items[group[0]].0, names.join(" and "), e));
            failed = true;
        }
        let preds: Vec<Pred> = self.st.preds.drain(mark..).collect();
        // Capability names generalize only if every binding is a value.
        let values = group.iter().all(|&i| match &items[i].1 {
            Item::Define(..) => true,
            Item::Val(_, e) => is_value(e),
            Item::Expr(_) => unreachable!(),
        });
        let names: Vec<String> = mono.iter().map(|(n, _)| n.to_string()).collect();
        let prefix = format!("{}: in the definition of {}: ", items[group[0]].0, names.join(" and "));
        self.flush_warnings(&prefix, report);
        for (name, t) in mono {
            let loc = &items[group[mono_index(group, items, &name)]].0;
            let sc =
                if failed { Scheme::mono(t) } else { self.generalize(&t, &preds, &outer, &BTreeSet::new(), values).0 };
            self.globals.insert(name.clone(), Global::Scheme(sc));
            self.defined_in.insert(name, loc.to_string());
        }
    }

    /// `∀` over the variables of `t` not free in the environment. Unless
    /// the bound expression is a syntactic value (`value`), capability names
    /// stay monomorphic (design/12 §4.1).
    fn generalize(
        &mut self,
        t: &Ty,
        preds: &[Pred],
        env_tvs: &BTreeSet<TV>,
        env_evs: &BTreeSet<EV>,
        value: bool,
    ) -> (Scheme, Vec<Pred>) {
        let ty = self.st.zonk(t);
        let mut roots = BTreeSet::new();
        self.st.fev(&ty, &mut roots);
        let quantified: BTreeSet<EV> = roots.difference(env_evs).copied().collect();
        // The type's variables, and the names in the effects it quantifies:
        // a function that forks inside makes a new name at every call.
        let mut tvs = BTreeSet::new();
        self.st.ftv(&ty, &mut tvs);
        for &r in &quantified {
            self.st.atom_tvs(r, &mut tvs);
        }
        let tvs: Vec<TV> =
            tvs.difference(env_tvs).copied().filter(|v| value || !self.st.cap_names.contains(v)).collect();
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
        // The predicates on what the scheme quantifies go with it. A `#` is
        // flattened like the effect bounds, so it mentions only variables
        // instantiating will map, and capabilities.
        let (mut kept, mut rest) = (vec![], vec![]);
        let tv_set: BTreeSet<TV> = tvs.iter().copied().collect();
        for p in preds {
            let z = self.st.zonk_pred(p);
            if !self.st.pred_live(&z, &tv_set, &quantified) {
                rest.push(z);
                continue;
            }
            kept.push(match z {
                Pred::Disjoint(l, r) => {
                    let mut flat = |f: &Foot| {
                        let mut f = self.st.expand(f);
                        f.evs.retain(|e| quantified.contains(e) || env_evs.contains(e));
                        f
                    };
                    Pred::Disjoint(flat(&l), flat(&r))
                }
                Pred::Warn(w, l, r, o) => {
                    let mut flat = |s: &Side| {
                        let mut s = self.st.expand_side(s);
                        s.evs.retain(|e| quantified.contains(e) || env_evs.contains(e));
                        s
                    };
                    Pred::Warn(w, flat(&l), flat(&r), o)
                }
                other => other,
            });
        }
        (Scheme { tvs, evs, preds: kept, ty }, rest)
    }

    fn instantiate(&mut self, sc: &Scheme) -> Ty {
        let tmap: HashMap<TV, Ty> = sc
            .tvs
            .iter()
            .map(|&v| (v, if self.st.cap_names.contains(&v) { self.st.fresh_cap_name() } else { self.st.fresh() }))
            .collect();
        let emap: HashMap<EV, EV> = sc.evs.iter().map(|(e, _)| (*e, self.st.fresh_ev())).collect();
        for (e, lower) in &sc.evs {
            let mapped = Eff {
                atoms: lower.atoms.iter().map(|a| subst_atom(a, &tmap)).collect(),
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
    fn settle(&mut self, mark: usize, env_tvs: &BTreeSet<TV>, keep: &BTreeSet<TV>, live: &BTreeSet<EV>) -> R<()> {
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
            let held: BTreeSet<TV> = keep.union(env_tvs).copied().collect();
            if self.st.pred_live(&p, &held, live) {
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
                // Checked as far as it ever can be: nothing can extend it.
                Pred::Disjoint(..) | Pred::Warn(..) => {
                    self.st.preds.remove(i);
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
                Ty::Grant(_) => unreachable!("a name is never a value's type"),
                Ty::Con(n, args) => match &**n {
                    "Text" | "Num" | "Bool" | "Sym" | "Money" | "Dur" => Ok(Step::Done),
                    "List" => Ok(Step::Then(vec![Pred::Showable(args[0].clone())])),
                    n if is_grant_type(n) => {
                        let mut pr = Printer::default();
                        Err(format!(
                            "a model, a person or a capability can't be shown, and this is {}",
                            pr.ty(&mut self.st, t, false)
                        ))
                    }
                    _ => {
                        let fields = self.field_types(n, args)?;
                        Ok(Step::Then(fields.into_iter().map(Pred::Showable).collect()))
                    }
                },
            },
            Pred::Offers(k, op, res) => match k {
                Ty::Var(_) => Ok(Step::Keep),
                Ty::Grant(g) => {
                    let ops = self.ops.get(g).cloned().unwrap_or_default();
                    if ops.is_empty() {
                        return Ok(Step::Unify(vec![(res.clone(), con("Text"))]));
                    }
                    match ops.iter().find(|(o, _)| o == op) {
                        Some((_, t)) => {
                            let t = self.convert(t, &HashMap::new())?;
                            Ok(Step::Unify(vec![(res.clone(), t)]))
                        }
                        None => {
                            let known: Vec<&str> = ops.iter().map(|(o, _)| &**o).collect();
                            Err(format!("{} does not offer {}; it offers {}", g, op, known.join(", ")))
                        }
                    }
                }
                _ => unreachable!("Offers holds a name"),
            },
            Pred::Warn(w, l, r, origin) => Ok(self.law_warning(*w, l, r, origin)),
            Pred::Disjoint(l, r) => {
                if self.st.disjoint(l, r)? {
                    Ok(Step::Keep)
                } else {
                    Ok(Step::Done)
                }
            }
            Pred::Has(t, f, ft) => match t {
                Ty::Grant(_) => unreachable!("a name is never a value's type"),
                Ty::Con(r, _) if is_grant_type(r) => {
                    let mut pr = Printer::default();
                    Err(format!("{} is not a record, so it has no field {}", pr.ty(&mut self.st, t, false), f))
                }
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

    /// Decide a law warning if it can be decided, and report it once.
    fn law_warning(&mut self, w: Warning, l: &Side, r: &Side, origin: &Option<Name>) -> Step {
        let (l, r) = (self.st.expand_side(l), self.st.expand_side(r));
        let via = origin.as_ref().map(|o| format!(", through {}", o)).unwrap_or_default();
        let open = !l.evs.is_empty() || !r.evs.is_empty();
        match w {
            Warning::Observe => {
                if l.atoms.contains(&Atom::Observe) {
                    self.pending.push(format!(
                        "a workflow node reads (remaining){}, so the workflow may not behave like its nodes run \
                         one after another (law W1, premise ii)",
                        via
                    ));
                    return Step::Done;
                }
            }
            Warning::SamePerson => {
                let people = |s: &Side| -> Vec<Nm> {
                    s.atoms
                        .iter()
                        .filter_map(|a| if let Atom::Consult(n) = a { Some(n.clone()) } else { None })
                        .collect()
                };
                let (lp, rp) = (people(&l), people(&r));
                if let Some(n) = lp.iter().find(|n| rp.contains(n)) {
                    let who = match n {
                        Nm::Grant(g) => g.to_string(),
                        Nm::Var(_) => "the same person".to_string(),
                    };
                    self.pending.push(format!(
                        "two concurrent workflow nodes both consult {}{}: an answer may depend on which question \
                         the person saw first (law W1, premise iv)",
                        who, via
                    ));
                    return Step::Done;
                }
                if lp.iter().chain(&rp).any(|n| matches!(n, Nm::Var(_))) && !lp.is_empty() && !rp.is_empty() {
                    return Step::Keep;
                }
            }
        }
        if open { Step::Keep } else { Step::Done }
    }

    /// Report the warnings found while checking one form, once each.
    fn flush_warnings(&mut self, prefix: &str, report: &mut Report) {
        let mut seen = HashSet::new();
        for w in std::mem::take(&mut self.pending) {
            if seen.insert(w.clone()) {
                report.warnings.push(format!("{}warning: {}", prefix, w));
            }
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
            Some(Global::Scheme(sc)) => {
                let before = self.st.preds.len();
                let t = self.instantiate(&sc);
                for p in &mut self.st.preds[before..] {
                    if let Pred::Warn(_, _, _, origin @ None) = p {
                        *origin = Some(x.clone());
                    }
                }
                Ok(t)
            }
            Some(Global::Prim(p)) => match prim_type(p, &mut self.st) {
                Some(t) => Ok(t),
                None => Err(format!("{} takes any number of arguments, so it can only be applied", p)),
            },
            None => Err(format!("unbound name {}", x)),
        }
    }

    fn env_tvs(&self, env: &[(Name, Scheme)]) -> BTreeSet<TV> {
        let mut out = self.global_tvs();
        for (_, sc) in env {
            let mut vs = BTreeSet::new();
            self.st.ftv(&sc.ty, &mut vs);
            out.extend(vs.into_iter().filter(|v| !sc.tvs.contains(v)));
        }
        out
    }

    /// The variables free in a global's scheme: a `val` bound to a fork
    /// keeps its capability's name monomorphic (§4.1), and whatever is
    /// defined later must not generalize it. Without this, `(val k2 k1)`
    /// would give one kernel two names.
    fn global_tvs(&self) -> BTreeSet<TV> {
        let mut out = BTreeSet::new();
        for g in self.globals.values() {
            if let Global::Scheme(sc) = g {
                let mut vs = BTreeSet::new();
                self.st.ftv(&sc.ty, &mut vs);
                out.extend(vs.into_iter().filter(|v| !sc.tvs.contains(v)));
            }
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
                // references (design/12 §4.1). Capability names generalize
                // only for syntactic values.
                let env_evs = self.env_evs(env);
                let mut env_tvs = self.env_tvs(env);
                for &e in &env_evs {
                    self.st.atom_tvs(e, &mut env_tvs);
                }
                let mut keep = BTreeSet::new();
                self.st.ftv(&rt, &mut keep);
                let mut live = env_evs.clone();
                self.st.fev(&rt, &mut live);
                self.settle(mark, &env_tvs, &keep, &live)?;
                let preds: Vec<Pred> = self.st.preds.drain(mark..).collect();
                let (sc, rest) = self.generalize(&rt, &preds, &env_tvs, &env_evs, is_value(rhs));
                // Predicates the scheme didn't take stay pending outside it.
                self.st.preds.extend(rest);
                // Q-4: a consult whose answer is never used, as in a `begin`,
                // is almost always a mistake: an approval is carried out only
                // if the program uses the answer (`11` §8).
                if matches!(&**rhs, Exp::Consult { .. }) && !free_vars(body).contains(x) {
                    self.pending.push(
                        "the answer of this consult is never used, so whatever the person approves or edits \
                         is ignored (design/11 §8)"
                            .into(),
                    );
                }
                let answer = is_answer(rhs);
                if answer {
                    self.answers.push(env.len());
                }
                env.push((x.clone(), sc));
                let result = self.infer(env, body);
                env.pop();
                if answer {
                    self.answers.pop();
                }
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
                let missing = self.missing(&st_ty, branches);
                if !missing.is_empty() {
                    // Q-3: an error on an answer from a model or a person,
                    // which may be any constructor; a warning otherwise.
                    let answer = match &**scrutinee {
                        Exp::Var(x) => env.iter().rposition(|(y, _)| y == x).is_some_and(|i| self.answers.contains(&i)),
                        e => is_answer(e),
                    };
                    let what = format!("this case doesn't cover {}", missing.join(", "));
                    if answer {
                        return Err(format!(
                            "{}, and it's on an answer from ask or consult, which may be any of them (design/12, Q-3)",
                            what
                        ));
                    }
                    self.pending.push(format!("{}, so it is stuck if one of them arrives", what));
                }
                Ok((result, eff))
            }
            // The agentic forms (design/12 §4.2).
            Exp::Ask { model, ty, ctx, .. } => {
                let answer = self.answer_type(ty, "an ask")?;
                let (mt, me) = self.infer(env, model)?;
                let mu = self.st.fresh();
                self.st.unify(&mt, &model_of(mu.clone())).map_err(|m| format!("ask needs a model: {}", m))?;
                let (ct, ce) = self.infer(env, ctx)?;
                self.st
                    .unify(&ct, &list_of(con("Message")))
                    .map_err(|m| format!("an ask's context is a list of messages: {}", m))?;
                let asks = Eff::atom(Atom::Ask(self.st.nm(&mu)));
                Ok((answer, me.union(&ce).union(&asks)))
            }
            Exp::Call { cap, op, args } => {
                let (kt, mut eff) = self.infer(env, cap)?;
                // A live host receives each argument as `show` writes it.
                for a in args {
                    let (at, ae) = self.infer(env, a)?;
                    self.st.preds.push(Pred::Showable(at));
                    eff = eff.union(&ae);
                }
                let kappa = self.st.fresh_cap_name();
                let offers = self.st.fresh();
                if &**op == "fork" {
                    // A fork is a new kernel: a fresh name, the same
                    // operations (design/12 §5.2).
                    self.st
                        .unify(&kt, &cap_of(con("#Kernel"), kappa.clone(), offers.clone()))
                        .map_err(|m| format!("only a kernel can fork: {}", m))?;
                    let calls = Eff::atom(Atom::Call(self.st.nm(&kappa), Nm::Grant(Rc::from("#Kernel"))));
                    let forked = self.st.fresh_cap_name();
                    return Ok((cap_of(con("#Kernel"), forked, offers), eff.union(&calls)));
                }
                let kind = self.st.fresh();
                self.st
                    .unify(&kt, &cap_of(kind.clone(), kappa.clone(), offers.clone()))
                    .map_err(|m| format!("call needs a capability: {}", m))?;
                let res = self.st.fresh();
                self.st.preds.push(Pred::Offers(offers, op.clone(), res.clone()));
                let calls = Eff::atom(Atom::Call(self.st.nm(&kappa), self.st.nm(&kind)));
                Ok((res, eff.union(&calls)))
            }
            Exp::Consult { human, ty, prompt, proposal, .. } => {
                let answer = self.answer_type(ty, "a consult")?;
                let (ht, mut eff) = self.infer(env, human)?;
                let eta = self.st.fresh();
                self.st.unify(&ht, &human_of(eta.clone())).map_err(|m| format!("consult needs a person: {}", m))?;
                let (pt, pe) = self.infer(env, prompt)?;
                self.st.unify(&pt, &con("Text")).map_err(|m| format!("a consult's prompt is Text: {}", m))?;
                eff = eff.union(&pe);
                if let Some(p) = proposal {
                    // So `v ⊨ τ` can fail only on a size bound (T6).
                    let (vt, ve) = self.infer(env, p)?;
                    self.st.unify(&vt, &answer).map_err(|m| format!("a proposal has the answer's type: {}", m))?;
                    eff = eff.union(&ve);
                }
                let consults = Eff::atom(Atom::Consult(self.st.nm(&eta)));
                Ok((answer, eff.union(&consults)))
            }
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
                // all. Every two nodes may run at once, so their footprints
                // must share no stateful capability (design/12 §5.1).
                let n = env.len();
                let mut types: Vec<Option<Ty>> = vec![None; w.nodes.len()];
                let mut feet: Vec<Foot> = vec![Foot::default(); w.nodes.len()];
                let mut sides: Vec<Side> = vec![Side::default(); w.nodes.len()];
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
                    feet[i] = self.footprint(env, w, node, &e);
                    sides[i] = Side { atoms: e.atoms.iter().cloned().collect(), evs: e.vars.iter().copied().collect() };
                    types[i] = Some(t);
                    eff = eff.union(&e);
                }
                for i in 0..feet.len() {
                    for j in i + 1..feet.len() {
                        self.st.preds.push(Pred::Disjoint(feet[i].clone(), feet[j].clone()));
                        self.st.preds.push(Pred::Warn(Warning::SamePerson, sides[i].clone(), sides[j].clone(), None));
                    }
                    self.st.preds.push(Pred::Warn(Warning::Observe, sides[i].clone(), Side::default(), None));
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

    /// A node's footprint (§5.1): the calls its effect makes, and the
    /// capabilities its free variables hold, which it could use or hand on.
    /// Other nodes' results are not counted: a node that holds one runs after
    /// the node that made it. A function's type says what it takes and
    /// makes, not what it holds, so arrows aren't looked inside; what a
    /// function calls is in the node's effect if the node calls it.
    fn footprint(&mut self, env: &[(Name, Scheme)], w: &WorkflowDef, node: &Node, e: &Eff) -> Foot {
        let mut names = vec![];
        for a in &e.atoms {
            if let Atom::Call(n, k) = self.st.norm_atom(a) {
                names.push((self.st.nm_ty(&n), self.st.nm_ty(&k)));
            }
        }
        for x in &node.fv {
            if w.nodes.iter().any(|m| &m.name == x) {
                continue;
            }
            let sc = match env.iter().rev().find(|(y, _)| y == x) {
                Some((_, sc)) => sc.clone(),
                None => match self.globals.get(x) {
                    Some(Global::Scheme(sc)) => sc.clone(),
                    _ => continue,
                },
            };
            self.held(&sc.ty, &sc.tvs, &mut names);
        }
        Foot { names, evs: e.vars.iter().map(|&v| self.st.find(v)).collect() }
    }

    /// The capabilities a value of type `t` holds, outside any arrow, except
    /// those whose names a scheme quantifies (they are parameters, not held).
    fn held(&self, t: &Ty, quantified: &[TV], out: &mut Vec<(Ty, Ty)>) {
        match self.st.prune(t) {
            Ty::Con(c, args) if &*c == "#Cap" => {
                let name = self.st.prune(&args[1]);
                if !matches!(name, Ty::Var(v) if quantified.contains(&v)) {
                    out.push((name, self.st.prune(&args[0])));
                }
            }
            Ty::Con(_, args) => args.iter().for_each(|a| self.held(a, quantified, out)),
            Ty::Var(_) | Ty::Grant(_) | Ty::Fun(..) => {}
        }
    }

    /// The constructors of the scrutinee's type that no branch covers.
    fn missing(&mut self, t: &Ty, branches: &[(Pattern, ExpRef)]) -> Vec<String> {
        if branches.iter().any(|(p, _)| matches!(p, Pattern::Wild)) {
            return vec![];
        }
        match self.st.prune(t) {
            Ty::Con(n, _) if &*n == "List" => {
                let mut out = vec![];
                if !branches.iter().any(|(p, _)| matches!(p, Pattern::Nil)) {
                    out.push("'()".to_string());
                }
                if !branches.iter().any(|(p, _)| matches!(p, Pattern::Cons(..))) {
                    out.push("cons".to_string());
                }
                out
            }
            Ty::Con(n, _) => match self.theta.datatypes.get(&n) {
                Some(cons) => cons
                    .iter()
                    .filter(|c| !branches.iter().any(|(p, _)| matches!(p, Pattern::Con(k, _) if *k == c.name)))
                    .map(|c| c.name.to_string())
                    .collect(),
                None => vec![],
            },
            _ => vec![],
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

    /// The answer type written in an `ask` or `consult`: ground, and
    /// askable here rather than when the form runs (`08`, deviation 1).
    fn answer_type(&mut self, ty: &Type, form: &str) -> R<Ty> {
        self.theta.askable(ty).map_err(|e| format!("the answer type of {}: {}", form, e))?;
        self.convert(ty, &HashMap::new())
    }

    /// A unit test's expressions are checked on their own (T8). A
    /// `check-type-error` passes exactly when its expression is ill-typed.
    fn check_test(&mut self, t: &UnitTest) -> R<()> {
        if let TestKind::TypeError(e) = &t.kind {
            let mark = self.st.preds.len();
            let typed = self.infer(&mut vec![], e).and_then(|(ty, _)| {
                self.settle(mark, &BTreeSet::new(), &BTreeSet::new(), &BTreeSet::new())?;
                let mut p = Printer::default();
                Ok(p.ty(&mut self.st, &ty, true))
            });
            self.st.preds.truncate(mark);
            return match typed {
                Err(_) => Ok(()),
                Ok(ty) => Err(format!(
                    "Check-type-error failed: expected {} to be ill-typed, but it has type {}.",
                    t.texts.first().map(String::as_str).unwrap_or("the expression"),
                    ty
                )),
            };
        }
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
                TestKind::TypeError(_) => unreachable!("handled above"),
            }
        })();
        let settled = result.and_then(|_| self.settle(mark, &BTreeSet::new(), &BTreeSet::new(), &BTreeSet::new()));
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

/// An answer from a model or a person, possibly with a budget around it.
fn is_answer(e: &Exp) -> bool {
    match e {
        Exp::Ask { .. } | Exp::Consult { .. } => true,
        Exp::Budget { body, .. } => is_answer(body),
        _ => false,
    }
}

/// A syntactic value (design/12 §4.1): evaluating it creates no capability.
fn is_value(e: &Exp) -> bool {
    match e {
        Exp::Literal(_) | Exp::Var(_) | Exp::Lambda(_) => true,
        Exp::Con(_, args) => args.iter().all(|a| is_value(a)),
        Exp::Record(_, fields) => fields.iter().all(|(_, a)| is_value(a)),
        _ => false,
    }
}

fn subst_atom(a: &Atom, tmap: &HashMap<TV, Ty>) -> Atom {
    a.map(|n| match n {
        Nm::Var(v) => match tmap.get(v) {
            Some(Ty::Var(w)) => Nm::Var(*w),
            Some(Ty::Grant(g)) => Nm::Grant(g.clone()),
            _ => n.clone(),
        },
        g => g.clone(),
    })
}

fn subst_ty(t: &Ty, tmap: &HashMap<TV, Ty>, emap: &HashMap<EV, EV>) -> Ty {
    match t {
        Ty::Var(v) => tmap.get(v).cloned().unwrap_or(Ty::Var(*v)),
        Ty::Grant(g) => Ty::Grant(g.clone()),
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
        Pred::Offers(a, op, b) => Pred::Offers(s(a), op.clone(), s(b)),
        Pred::Disjoint(l, r) => {
            let f = |x: &Foot| Foot {
                names: x.names.iter().map(|(n, k)| (s(n), s(k))).collect(),
                evs: x.evs.iter().map(|e| *em.get(e).unwrap_or(e)).collect(),
            };
            Pred::Disjoint(f(l), f(r))
        }
        Pred::Warn(w, l, r, o) => {
            let f = |x: &Side| Side {
                atoms: x.atoms.iter().map(|a| subst_atom(a, tm)).collect(),
                evs: x.evs.iter().map(|e| *em.get(e).unwrap_or(e)).collect(),
            };
            Pred::Warn(*w, f(l), f(r), o.clone())
        }
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
