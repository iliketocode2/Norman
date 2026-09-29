//! Concrete syntax (04 §2) to abstract syntax (04 §3), including the syntactic
//! sugar of 04 §7.1 and the parse-time well-formedness checks of 04 §3.1.
//!
//! The parser consults Θ for one reason: `(R [f e] …)` is record construction
//! when `R` names a record, and constructor application otherwise. Definitions
//! are parsed one at a time, as the read-eval-print loop reaches them.

use crate::ast::*;
use crate::lexer::Sx;
use crate::types::TypeEnv;
use crate::value::Value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

type P<T> = Result<T, String>;

const KEYWORDS: &[&str] = &[
    "val", "define", "datatype", "record", "use", "grant", "script", "under", "check-expect", "check-assert",
    "check-error", "check-fail", "check-within", "check-equiv", "if", "let*", "lambda", "case", "ask", "call",
    "fail", "catch", "budget", "workflow", "begin", "par", "and", "or", "_",
];
// `cost` and `time` are not reserved: they occur only in fixed bracket positions
// ([cost e] in budget, under and check-within), and they are the field names of
// the predefined Resources record.

static FRESH: AtomicU64 = AtomicU64::new(0);

/// A fresh name. It contains a space, so no source program can mention it.
fn fresh(base: &str) -> Name {
    let n = FRESH.fetch_add(1, Ordering::Relaxed);
    Rc::from(format!("{} {}", base, n).as_str())
}

fn perr<T>(sx: &Sx, msg: impl AsRef<str>) -> P<T> {
    Err(format!("{}: {}", sx.loc(), msg.as_ref()))
}

/// The token classes of 04 §1.
#[derive(Debug, PartialEq)]
pub enum Atom {
    Num(i64),
    Money(i64),
    Dur(i64),
    Bool(bool),
    Sym(Name),
    Nil,
    Upper(Name),
    Lower(Name),
}

pub fn classify(a: &str) -> Result<Atom, String> {
    match a {
        "#t" => return Ok(Atom::Bool(true)),
        "#f" => return Ok(Atom::Bool(false)),
        "'()" => return Ok(Atom::Nil),
        _ => {}
    }
    if let Some(s) = a.strip_prefix('\'') {
        return Ok(Atom::Sym(Rc::from(s)));
    }
    if let Some(m) = a.strip_prefix('$') {
        return parse_money(m).map(Atom::Money).ok_or_else(|| format!("bad money literal {}", a));
    }
    let digits = a.strip_prefix(['+', '-']).unwrap_or(a);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return a.parse::<i64>().map(Atom::Num).map_err(|_| format!("numeral {} is too large", a));
    }
    if let Some(d) = parse_duration(a) {
        return Ok(Atom::Dur(d));
    }
    if a.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Ok(Atom::Upper(Rc::from(a)));
    }
    Ok(Atom::Lower(Rc::from(a)))
}

/// `$0.50` → 500 000 µ$. At most six fractional digits.
fn parse_money(m: &str) -> Option<i64> {
    let (whole, frac) = match m.split_once('.') {
        Some((w, f)) => (w, f),
        None => (m, ""),
    };
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) || frac.len() > 6 {
        return None;
    }
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let w: i64 = whole.parse().ok()?;
    let f: i64 = if frac.is_empty() { 0 } else { format!("{:0<6}", frac).parse().ok()? };
    w.checked_mul(1_000_000)?.checked_add(f)
}

/// `250ms`, `30s`, `10min`, `2h` → milliseconds.
fn parse_duration(a: &str) -> Option<i64> {
    let split = a.find(|c: char| !c.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let (n, unit) = a.split_at(split);
    let n: i64 = n.parse().ok()?;
    let scale = match unit {
        "ms" => 1,
        "s" => 1000,
        "min" => 60_000,
        "h" => 3_600_000,
        _ => return None,
    };
    n.checked_mul(scale)
}

fn lit(v: Value) -> ExpRef {
    Rc::new(Exp::Literal(v))
}

pub struct Parser<'a> {
    pub theta: &'a TypeEnv,
}

impl<'a> Parser<'a> {
    pub fn new(theta: &'a TypeEnv) -> Parser<'a> {
        Parser { theta }
    }

    // ---------------------------------------------------------------- names

    fn lower(&self, sx: &Sx, what: &str) -> P<Name> {
        match sx.atom().map(classify) {
            Some(Ok(Atom::Lower(x))) if !KEYWORDS.contains(&&*x) => Ok(x),
            _ => perr(sx, format!("expected {} (a lowercase name), found {}", what, sx)),
        }
    }

    fn upper(&self, sx: &Sx, what: &str) -> P<Name> {
        match sx.atom().map(classify) {
            Some(Ok(Atom::Upper(x))) => Ok(x),
            _ => perr(sx, format!("expected {} (a Capitalized name), found {}", what, sx)),
        }
    }

    /// A binding position: a variable, or `_` (which binds a fresh name).
    fn binder(&self, sx: &Sx) -> P<Name> {
        if sx.atom() == Some("_") {
            return Ok(fresh("_"));
        }
        self.lower(sx, "a variable")
    }

    fn formals(&self, sx: &Sx) -> P<Vec<Name>> {
        let xs = sx.list().ok_or_else(|| format!("{}: expected a list of formal parameters", sx.loc()))?;
        let names = xs.iter().map(|x| self.binder(x)).collect::<P<Vec<_>>>()?;
        let mut seen = HashSet::new();
        for (n, x) in names.iter().zip(xs) {
            if !seen.insert(n.clone()) {
                return perr(x, format!("formal parameter {} appears twice", n));
            }
        }
        Ok(names)
    }

    fn arity(&self, sx: &Sx, form: &str, args: &[Sx], n: usize) -> P<()> {
        if args.len() == n {
            Ok(())
        } else {
            perr(sx, format!("{} expects {} operand(s), found {}", form, n, args.len()))
        }
    }

    // ------------------------------------------------------------ top level

    pub fn top(&self, sx: &Sx) -> P<Top> {
        if let Sx::List(xs, _) = sx
            && let Some(head) = xs.first().and_then(Sx::atom) {
                let args = &xs[1..];
                match head {
                    "val" => {
                        self.arity(sx, "val", args, 2)?;
                        return Ok(Top::Def(Def::Val(self.lower(&args[0], "a variable")?, self.exp(&args[1])?)));
                    }
                    "define" => {
                        self.arity(sx, "define", args, 3)?;
                        let f = self.lower(&args[0], "a function name")?;
                        let formals = self.formals(&args[1])?;
                        let body = self.exp(&args[2])?;
                        return Ok(Top::Def(Def::Define(f, Rc::new(Lambda::new(formals, body)))));
                    }
                    "datatype" => return self.datatype(sx, args),
                    "record" => {
                        if args.is_empty() {
                            return perr(sx, "record needs a name");
                        }
                        let r = self.upper(&args[0], "a record name")?;
                        let fields = args[1..].iter().map(|f| self.field(f)).collect::<P<Vec<_>>>()?;
                        return Ok(Top::Def(Def::Record(r, fields)));
                    }
                    "use" => {
                        self.arity(sx, "use", args, 1)?;
                        return match &args[0] {
                            Sx::Atom(a, _) | Sx::Str(a, _) => Ok(Top::Use(a.to_string())),
                            other => perr(other, "use expects a file name"),
                        };
                    }
                    "grant" => return self.grant(sx, args),
                    "script" => return self.script(sx, args),
                    "under" => return self.under(sx, args),
                    h if h.starts_with("check-") => return Ok(Top::Test(self.test(sx)?)),
                    _ => {}
                }
            }
        Ok(Top::Def(Def::Exp(self.exp(sx)?)))
    }

    /// `(f τ)`: a named field, in a record or a constructor.
    fn field(&self, sx: &Sx) -> P<(Name, Type)> {
        match sx.list() {
            Some([f, t]) => Ok((self.lower(f, "a field name")?, self.ty(t)?)),
            _ => perr(sx, format!("expected a field (name type), found {}", sx)),
        }
    }

    fn datatype(&self, sx: &Sx, args: &[Sx]) -> P<Top> {
        if args.is_empty() {
            return perr(sx, "datatype needs a name");
        }
        let t = self.upper(&args[0], "a type name")?;
        let mut cons = vec![];
        for c in &args[1..] {
            let (k, fields) = match c {
                Sx::Atom(..) => (self.upper(c, "a constructor")?, vec![]),
                Sx::List(items, _) if !items.is_empty() => {
                    let k = self.upper(&items[0], "a constructor")?;
                    let fields = items[1..].iter().map(|f| self.field(f)).collect::<P<Vec<_>>>()?;
                    (k, fields)
                }
                _ => return perr(c, "expected [Constructor (field type) …]"),
            };
            cons.push(ConDef { name: k, fields });
        }
        Ok(Top::Def(Def::Datatype(t, cons)))
    }

    fn grant(&self, sx: &Sx, args: &[Sx]) -> P<Top> {
        self.arity(sx, "grant", args, 2)?;
        let x = self.lower(&args[0], "a variable")?;
        let spec = args[1].list().ok_or_else(|| format!("{}: expected a host specification", args[1].loc()))?;
        let kind = spec.first().and_then(Sx::atom).unwrap_or("");
        let spec = match kind {
            "model" => {
                // Every field is optional; omitted fields take the defaults in
                // crate::defaults, which describe Claude Sonnet 5 (design/09 §0).
                use crate::defaults as d;
                let (mut id, mut i, mut o, mut c, mut t) = (d::MODEL_ID.to_string(), d::IN_PRICE, d::OUT_PRICE, d::CEILING, d::THINK);
                for item in &spec[1..] {
                    match item.list() {
                        Some([k, Sx::Str(s, _)]) if k.atom() == Some("id") => id = s.to_string(),
                        Some([k, v]) => {
                            let n = match v.atom().map(classify) {
                                Some(Ok(Atom::Num(n))) if n >= 0 => n,
                                _ => return perr(v, "expected a non-negative numeral"),
                            };
                            match k.atom() {
                                Some("in") => i = n,
                                Some("out") => o = n,
                                Some("ceiling") => c = n as u64,
                                Some("think") => t = n as u64,
                                _ => return perr(k, "model options are [id \"…\"], [in n], [out n], [ceiling n] and [think n]"),
                            }
                        }
                        _ => return perr(item, "expected [option value]"),
                    }
                }
                HostSpec::Model { id, in_price: i, out_price: o, ceiling: c, think: t }
            }
            "kernel" => HostSpec::Kernel,
            "filesystem" => HostSpec::Filesystem,
            "http" => HostSpec::Http,
            _ => return perr(&args[1], "a host is (model …), (kernel), (filesystem) or (http)"),
        };
        Ok(Top::Grant(x, spec))
    }

    fn script(&self, sx: &Sx, args: &[Sx]) -> P<Top> {
        if args.is_empty() {
            return perr(sx, "script needs a name");
        }
        let name: Name = match args[0].atom() {
            Some(a) => Rc::from(a),
            None => return perr(&args[0], "script needs a name"),
        };
        let mut script = Script::default();
        for site in &args[1..] {
            let items = site.list().ok_or_else(|| format!("{}: expected [site entries…]", site.loc()))?;
            let key = match items.first().and_then(Sx::atom) {
                Some(a) => a.trim_start_matches('\'').to_string(),
                None => return perr(site, "a site entry starts with the site's name"),
            };
            let mut entries: Vec<Entry> = vec![];
            for item in &items[1..] {
                let parts = item.list().unwrap_or(&[]);
                let (head, arg) = match parts {
                    [h, a] => (h.atom().unwrap_or(""), a),
                    _ => return perr(item, "expected (reply …), (out …), (latency …) and so on"),
                };
                let text = |a: &Sx| match a {
                    Sx::Str(s, _) => Ok(s.to_string()),
                    _ => perr(a, "expected a string"),
                };
                match head {
                    "reply" => entries.push(Entry::Reply { json: text(arg)?, out: 0, latency: 0 }),
                    "refusal" => entries.push(Entry::Refusal { category: text(arg)?, out: 0, latency: 0 }),
                    "provider-error" => entries.push(Entry::ProviderError { msg: text(arg)?, latency: 0 }),
                    "result" => entries.push(Entry::Result { text: text(arg)?, latency: 0 }),
                    "error" => entries.push(Entry::Error { msg: text(arg)?, latency: 0 }),
                    "out" | "latency" => {
                        let last = entries.last_mut().ok_or_else(|| format!("{}: ({} …) must follow a reply", item.loc(), head))?;
                        match (head, arg.atom().map(classify)) {
                            ("out", Some(Ok(Atom::Num(n)))) if n >= 0 => match last {
                                Entry::Reply { out, .. } | Entry::Refusal { out, .. } => *out = n as u64,
                                _ => return perr(item, "(out …) applies only to a reply or a refusal"),
                            },
                            ("latency", Some(Ok(Atom::Dur(d)))) => match last {
                                Entry::Reply { latency, .. }
                                | Entry::Refusal { latency, .. }
                                | Entry::ProviderError { latency, .. }
                                | Entry::Result { latency, .. }
                                | Entry::Error { latency, .. } => *latency = d,
                            },
                            _ => return perr(arg, format!("bad value for ({} …)", head)),
                        }
                    }
                    _ => return perr(item, format!("unknown script item ({} …)", head)),
                }
            }
            script.sites.entry(key).or_default().extend(entries);
        }
        Ok(Top::Script(name, script))
    }

    fn under(&self, sx: &Sx, args: &[Sx]) -> P<Top> {
        if args.is_empty() {
            return perr(sx, "under needs a configuration");
        }
        let mut cfg = Config::default();
        let items = args[0].list().ok_or_else(|| format!("{}: expected a configuration list", args[0].loc()))?;
        for item in items {
            match item.list() {
                Some([k, v]) => match (k.atom(), v.atom().map(classify)) {
                    (Some("script"), _) => cfg.script = v.atom().map(Rc::from),
                    (Some("cost"), Some(Ok(Atom::Money(m)))) => cfg.cost = Some(m),
                    (Some("time"), Some(Ok(Atom::Dur(d)))) => cfg.time = Some(d),
                    _ => return perr(item, "configuration items are [script name], [cost $…] and [time …]"),
                },
                _ => return perr(item, "expected [key value]"),
            }
        }
        let tests = args[1..].iter().map(|t| self.test(t)).collect::<P<Vec<_>>>()?;
        Ok(Top::Under(cfg, tests))
    }

    fn test(&self, sx: &Sx) -> P<UnitTest> {
        let xs = sx.list().unwrap_or(&[]);
        let head = xs.first().and_then(Sx::atom).unwrap_or("");
        let args = if xs.is_empty() { &[][..] } else { &xs[1..] };
        let texts = args.iter().map(|a| a.to_string()).collect();
        let kind = match head {
            "check-expect" => {
                self.arity(sx, head, args, 2)?;
                TestKind::Expect(self.exp(&args[0])?, self.exp(&args[1])?)
            }
            "check-assert" => {
                self.arity(sx, head, args, 1)?;
                TestKind::Assert(self.exp(&args[0])?)
            }
            "check-error" => {
                self.arity(sx, head, args, 1)?;
                TestKind::Error(self.exp(&args[0])?)
            }
            "check-fail" => {
                self.arity(sx, head, args, 2)?;
                TestKind::Fail(self.exp(&args[0])?, self.pattern(&args[1])?)
            }
            "check-within" => {
                self.arity(sx, head, args, 2)?;
                let (mut c, mut t) = (None, None);
                for item in args[1].list().unwrap_or(&[]) {
                    match item.list() {
                        Some([k, v]) => match (k.atom(), v.atom().map(classify)) {
                            (Some("cost"), Some(Ok(Atom::Money(m)))) => c = Some(m),
                            (Some("time"), Some(Ok(Atom::Dur(d)))) => t = Some(d),
                            _ => return perr(item, "expected [cost $…] or [time …]"),
                        },
                        _ => return perr(item, "expected [cost $…] or [time …]"),
                    }
                }
                TestKind::Within(self.exp(&args[0])?, c, t)
            }
            "check-equiv" => {
                self.arity(sx, head, args, 3)?;
                let grade = match args[2].atom() {
                    Some("'exact") => Grade::Exact,
                    Some("'resource") => Grade::Resource,
                    Some("'value") => Grade::Value,
                    _ => return perr(&args[2], "the grade is 'exact, 'resource or 'value"),
                };
                TestKind::Equiv(self.exp(&args[0])?, self.exp(&args[1])?, grade)
            }
            _ => return perr(sx, "expected a unit test (check-expect, check-assert, check-error, check-fail, check-within, check-equiv)"),
        };
        Ok(UnitTest { kind, texts, loc: sx.loc().clone() })
    }

    // ------------------------------------------------------ types, patterns

    pub fn ty(&self, sx: &Sx) -> P<Type> {
        let bound = |n: &Sx| match n.atom().map(classify) {
            Some(Ok(Atom::Num(k))) if k >= 0 => Ok(k as u64),
            _ => perr(n, "expected a non-negative size bound"),
        };
        match sx {
            Sx::Atom(a, _) => match &**a {
                "Text" => Ok(Type::Text(None)),
                "Num" => Ok(Type::Num),
                "Bool" => Ok(Type::Bool),
                "Sym" => Ok(Type::Sym),
                "Any" => Ok(Type::Any),
                _ => Ok(Type::Named(self.upper(sx, "a type")?)),
            },
            Sx::List(xs, _) => match xs.first().and_then(Sx::atom) {
                Some("Text") if xs.len() == 2 => Ok(Type::Text(Some(bound(&xs[1])?))),
                Some("List") if xs.len() == 2 => Ok(Type::List(Box::new(self.ty(&xs[1])?), None)),
                Some("List") if xs.len() == 3 => Ok(Type::List(Box::new(self.ty(&xs[1])?), Some(bound(&xs[2])?))),
                _ => perr(sx, format!("not a type: {}", sx)),
            },
            Sx::Str(..) => perr(sx, "not a type"),
        }
    }

    fn pattern(&self, sx: &Sx) -> P<Pattern> {
        let var = |x: &Sx| -> P<Option<Name>> {
            if x.atom() == Some("_") {
                Ok(None)
            } else {
                Ok(Some(self.lower(x, "a pattern variable")?))
            }
        };
        let p = match sx {
            Sx::Atom(a, _) if &**a == "_" => Pattern::Wild,
            Sx::Atom(a, _) if &**a == "'()" => Pattern::Nil,
            Sx::Atom(..) => Pattern::Con(self.upper(sx, "a constructor, '() or _")?, vec![]),
            Sx::List(xs, _) if xs.first().and_then(Sx::atom) == Some("cons") && xs.len() == 3 => {
                Pattern::Cons(var(&xs[1])?, var(&xs[2])?)
            }
            Sx::List(xs, _) if !xs.is_empty() => {
                let k = self.upper(&xs[0], "a constructor")?;
                Pattern::Con(k, xs[1..].iter().map(var).collect::<P<Vec<_>>>()?)
            }
            _ => return perr(sx, format!("not a pattern: {}", sx)),
        };
        let vars = p.vars();
        let distinct: HashSet<&Name> = vars.iter().collect();
        if distinct.len() != vars.len() {
            return perr(sx, "a variable appears twice in one pattern");
        }
        Ok(p)
    }

    // ---------------------------------------------------------- expressions

    pub fn exp(&self, sx: &Sx) -> P<ExpRef> {
        match sx {
            Sx::Str(s, _) => Ok(lit(Value::Str(s.clone()))),
            Sx::Atom(a, _) => Ok(match classify(a).map_err(|m| format!("{}: {}", sx.loc(), m))? {
                Atom::Num(n) => lit(Value::Num(n)),
                Atom::Money(m) => lit(Value::Money(m)),
                Atom::Dur(d) => lit(Value::Dur(d)),
                Atom::Bool(b) => lit(Value::Bool(b)),
                Atom::Sym(s) => lit(Value::Sym(s)),
                Atom::Nil => lit(Value::Nil),
                Atom::Upper(k) => Rc::new(Exp::Con(k, vec![])),
                Atom::Lower(x) => {
                    if KEYWORDS.contains(&&*x) {
                        return perr(sx, format!("the keyword {} can't be used as a value", x));
                    }
                    Rc::new(Exp::Var(x))
                }
            }),
            Sx::List(xs, _) => {
                if xs.is_empty() {
                    return perr(sx, "() is not an expression (did you mean '()?)");
                }
                if let Some(h) = xs[0].atom()
                    && let Some(e) = self.special(sx, h, &xs[1..])? {
                        return Ok(e);
                    }
                let f = self.exp(&xs[0])?;
                let args = xs[1..].iter().map(|x| self.exp(x)).collect::<P<Vec<_>>>()?;
                Ok(Rc::new(Exp::Apply(f, args)))
            }
        }
    }

    fn exps(&self, xs: &[Sx]) -> P<Vec<ExpRef>> {
        xs.iter().map(|x| self.exp(x)).collect()
    }

    /// Keyword forms, field selection, sugar, and Capitalized heads.
    fn special(&self, sx: &Sx, head: &str, args: &[Sx]) -> P<Option<ExpRef>> {
        let e = match head {
            "if" => {
                self.arity(sx, "if", args, 3)?;
                Exp::If(self.exp(&args[0])?, self.exp(&args[1])?, self.exp(&args[2])?)
            }
            "let*" => {
                self.arity(sx, "let*", args, 2)?;
                let bindings = self.bindings(&args[0])?;
                let mut body = self.exp(&args[1])?;
                for (x, e) in bindings.into_iter().rev() {
                    body = Rc::new(Exp::Let(x, e, body));
                }
                return Ok(Some(body));
            }
            "lambda" => {
                self.arity(sx, "lambda", args, 2)?;
                Exp::Lambda(Rc::new(Lambda::new(self.formals(&args[0])?, self.exp(&args[1])?)))
            }
            "case" => {
                if args.is_empty() {
                    return perr(sx, "case needs an expression to examine");
                }
                let branches = args[1..]
                    .iter()
                    .map(|b| match b.list() {
                        Some([p, e]) => Ok((self.pattern(p)?, self.exp(e)?)),
                        _ => perr(b, "expected a branch [pattern expression]"),
                    })
                    .collect::<P<Vec<_>>>()?;
                Exp::Case(self.exp(&args[0])?, branches)
            }
            "ask" => {
                if args.len() != 3 && args.len() != 4 {
                    return perr(sx, "ask expects a model, a type, a context and an optional 'site");
                }
                let site: Name = match args.get(3) {
                    Some(s) => match s.atom().map(classify) {
                        Some(Ok(Atom::Sym(site))) => site,
                        _ => return perr(s, "the site of an ask is a quoted symbol"),
                    },
                    None => Rc::from(sx.loc().to_string().as_str()),
                };
                Exp::Ask { site, model: self.exp(&args[0])?, ty: self.ty(&args[1])?, ctx: self.exp(&args[2])? }
            }
            "call" => {
                if args.len() < 2 {
                    return perr(sx, "call expects a capability and an operation");
                }
                Exp::Call { cap: self.exp(&args[0])?, op: self.lower(&args[1], "an operation name")?, args: self.exps(&args[2..])? }
            }
            "fail" => {
                self.arity(sx, "fail", args, 1)?;
                // (fail 'sym) = (fail (Raised 'sym))
                if let Some(Ok(Atom::Sym(s))) = args[0].atom().map(classify) {
                    Exp::Fail(Rc::new(Exp::Con(Rc::from("Raised"), vec![lit(Value::Sym(s))])))
                } else {
                    Exp::Fail(self.exp(&args[0])?)
                }
            }
            "catch" => {
                self.arity(sx, "catch", args, 3)?;
                Exp::Catch(self.exp(&args[0])?, self.binder(&args[1])?, self.exp(&args[2])?)
            }
            "budget" => {
                self.arity(sx, "budget", args, 2)?;
                let (mut cost, mut time) = (None, None);
                for item in args[0].list().ok_or_else(|| format!("{}: expected ([cost e] [time e])", args[0].loc()))? {
                    match item.list() {
                        Some([k, v]) if k.atom() == Some("cost") && cost.is_none() => cost = Some(self.exp(v)?),
                        Some([k, v]) if k.atom() == Some("time") && time.is_none() => time = Some(self.exp(v)?),
                        _ => return perr(item, "expected [cost e] or [time e], each at most once"),
                    }
                }
                Exp::Budget { cost, time, body: self.exp(&args[1])? }
            }
            "workflow" => {
                self.arity(sx, "workflow", args, 2)?;
                let bindings = self.bindings(&args[0])?;
                return Ok(Some(self.workflow(sx, bindings, self.exp(&args[1])?)?));
            }
            "begin" => {
                // (begin) = #f;  (begin e₁ … eₙ) = (let* ([_ e₁] … ) eₙ)
                let es = self.exps(args)?;
                let Some((last, init)) = es.split_last() else {
                    return Ok(Some(lit(Value::Bool(false))));
                };
                let mut body = last.clone();
                for e in init.iter().rev() {
                    body = Rc::new(Exp::Let(fresh("_"), e.clone(), body));
                }
                return Ok(Some(body));
            }
            "par" => {
                // (par e₁ e₂) = (workflow ([a e₁] [b e₂]) (Pair [fst a] [snd b]))
                self.arity(sx, "par", args, 2)?;
                let (a, b) = (fresh("par-a"), fresh("par-b"));
                let body = Rc::new(Exp::Record(
                    Rc::from("Pair"),
                    vec![(Rc::from("fst"), Rc::new(Exp::Var(a.clone()))), (Rc::from("snd"), Rc::new(Exp::Var(b.clone())))],
                ));
                let bindings = vec![(a, self.exp(&args[0])?), (b, self.exp(&args[1])?)];
                return Ok(Some(self.workflow(sx, bindings, body)?));
            }
            "and" => {
                self.arity(sx, "and", args, 2)?;
                Exp::If(self.exp(&args[0])?, self.exp(&args[1])?, lit(Value::Bool(false)))
            }
            "or" => {
                self.arity(sx, "or", args, 2)?;
                Exp::If(self.exp(&args[0])?, lit(Value::Bool(true)), self.exp(&args[1])?)
            }
            "." => {
                self.arity(sx, ".", args, 2)?;
                Exp::Field(self.exp(&args[0])?, self.lower(&args[1], "a field name")?)
            }
            h if KEYWORDS.contains(&h) => {
                return perr(sx, format!("{} is not allowed inside an expression", h));
            }
            h if h.starts_with(|c: char| c.is_ascii_uppercase()) => {
                let name: Name = Rc::from(h);
                if self.theta.is_record(h) {
                    let fields = args
                        .iter()
                        .map(|f| match f.list() {
                            Some([k, v]) => Ok((self.lower(k, "a field name")?, self.exp(v)?)),
                            _ => perr(f, format!("record {} is built from fields [name value]", h)),
                        })
                        .collect::<P<Vec<_>>>()?;
                    Exp::Record(name, fields)
                } else {
                    Exp::Con(name, self.exps(args)?)
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(Rc::new(e)))
    }

    /// `([x e] …)`, for let* and workflow.
    fn bindings(&self, sx: &Sx) -> P<Vec<(Name, ExpRef)>> {
        sx.list()
            .ok_or_else(|| format!("{}: expected a list of bindings", sx.loc()))?
            .iter()
            .map(|b| match b.list() {
                Some([x, e]) => Ok((self.binder(x)?, self.exp(e)?)),
                _ => perr(b, "expected a binding [name expression]"),
            })
            .collect()
    }

    /// Build a workflow, checking 04 §3.1 rules 2 and 3: distinct node names,
    /// and an acyclic dependency graph read off the free variables.
    fn workflow(&self, sx: &Sx, bindings: Vec<(Name, ExpRef)>, body: ExpRef) -> P<ExpRef> {
        let index: HashMap<Name, usize> = bindings.iter().enumerate().map(|(i, (x, _))| (x.clone(), i)).collect();
        if index.len() != bindings.len() {
            return perr(sx, "two workflow nodes have the same name");
        }
        let nodes: Vec<Node> = bindings
            .into_iter()
            .map(|(name, exp)| {
                let fv = free_vars(&exp);
                let deps = fv.iter().filter_map(|x| index.get(x).copied()).collect();
                Node { name, exp, deps, fv: fv.into_iter().collect() }
            })
            .collect();
        // Depth-first search for a cycle: 0 unvisited, 1 on the stack, 2 finished.
        fn visit(i: usize, nodes: &[Node], color: &mut [u8]) -> Option<usize> {
            color[i] = 1;
            for &d in &nodes[i].deps {
                if color[d] == 1 {
                    return Some(d);
                }
                if color[d] == 0
                    && let Some(c) = visit(d, nodes, color) {
                        return Some(c);
                    }
            }
            color[i] = 2;
            None
        }
        let mut color = vec![0u8; nodes.len()];
        for i in 0..nodes.len() {
            if color[i] == 0
                && let Some(c) = visit(i, &nodes, &mut color) {
                    return perr(sx, format!("workflow node {} depends on itself (the dependency graph has a cycle)", nodes[c].name));
                }
        }
        Ok(Rc::new(Exp::Workflow(Rc::new(WorkflowDef { nodes, body }))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_classify() {
        assert_eq!(classify("$0.50"), Ok(Atom::Money(500_000)));
        assert_eq!(classify("$0.000001"), Ok(Atom::Money(1)));
        assert_eq!(classify("$20"), Ok(Atom::Money(20_000_000)));
        assert_eq!(classify("10min"), Ok(Atom::Dur(600_000)));
        assert_eq!(classify("250ms"), Ok(Atom::Dur(250)));
        assert_eq!(classify("-7"), Ok(Atom::Num(-7)));
        assert!(matches!(classify("-"), Ok(Atom::Lower(_))));
        assert!(matches!(classify("Verdict"), Ok(Atom::Upper(_))));
        assert!(matches!(classify("'declined"), Ok(Atom::Sym(_))));
    }
}
