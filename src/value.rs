//! Values (04 §4) and environments.

use crate::ast::{Lambda, Name};
use crate::lexer::quote;
use std::fmt;
use std::rc::Rc;

/// A persistent environment: a linked list of bindings, shared by closures.
#[derive(Clone, Default)]
pub struct Env(Option<Rc<EnvNode>>);

struct EnvNode {
    name: Name,
    value: Value,
    next: Env,
}

impl Env {
    pub fn extend(&self, name: Name, value: Value) -> Env {
        Env(Some(Rc::new(EnvNode { name, value, next: self.clone() })))
    }

    pub fn lookup(&self, name: &str) -> Option<&Value> {
        let mut cur = &self.0;
        while let Some(node) = cur {
            if &*node.name == name {
                return Some(&node.value);
            }
            cur = &node.next.0;
        }
        None
    }
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "<env>")
    }
}

#[derive(Debug)]
pub struct Closure {
    pub lambda: Rc<Lambda>,
    pub env: Env,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Lt,
    Gt,
    Cons,
    List,
    StringAppend,
    StringLength,
    Println,
    Remaining,
}

impl Prim {
    pub fn name(self) -> &'static str {
        match self {
            Prim::Add => "+",
            Prim::Sub => "-",
            Prim::Mul => "*",
            Prim::Div => "/",
            Prim::Eq => "=",
            Prim::Lt => "<",
            Prim::Gt => ">",
            Prim::Cons => "cons",
            Prim::List => "list",
            Prim::StringAppend => "string-append",
            Prim::StringLength => "string-length",
            Prim::Println => "println",
            Prim::Remaining => "remaining",
        }
    }

    pub const ALL: [Prim; 13] = [
        Prim::Add,
        Prim::Sub,
        Prim::Mul,
        Prim::Div,
        Prim::Eq,
        Prim::Lt,
        Prim::Gt,
        Prim::Cons,
        Prim::List,
        Prim::StringAppend,
        Prim::StringLength,
        Prim::Println,
        Prim::Remaining,
    ];
}

#[derive(Debug)]
pub struct ModelSpec {
    pub name: Name,
    /// Micro-dollars per input token.
    pub in_price: i64,
    /// Micro-dollars per output token.
    pub out_price: i64,
    /// The model's maximum output, in tokens.
    pub ceiling: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapKind {
    Kernel,
    Filesystem,
    Http,
}

impl CapKind {
    pub fn stateful(self) -> bool {
        matches!(self, CapKind::Kernel | CapKind::Filesystem)
    }
}

#[derive(Debug, Clone)]
pub struct Cap {
    pub id: u64,
    pub kind: CapKind,
    /// The grant name; scripts key calls as `key/op`. Forked kernels share it.
    pub key: Name,
}

#[derive(Debug, Clone)]
pub enum Value {
    Num(i64),
    Str(Rc<str>),
    Bool(bool),
    Sym(Rc<str>),
    Nil,
    Cons(Rc<Value>, Rc<Value>),
    Con(Name, Rc<Vec<Value>>),
    Record(Name, Rc<Vec<(Name, Value)>>),
    Closure(Rc<Closure>),
    Prim(Prim),
    Model(Rc<ModelSpec>),
    Cap(Cap),
    /// Integer micro-dollars.
    Money(i64),
    /// Integer milliseconds.
    Dur(i64),
    /// An unlimited amount of money or time.
    Inf,
}

impl Value {
    pub fn con(name: &str, fields: Vec<Value>) -> Value {
        Value::Con(Rc::from(name), Rc::new(fields))
    }

    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }

    /// Build a proper list.
    pub fn list(items: Vec<Value>) -> Value {
        items.into_iter().rev().fold(Value::Nil, |tl, hd| Value::Cons(Rc::new(hd), Rc::new(tl)))
    }

    /// Collect a proper list, or `None` if the value isn't one.
    pub fn to_vec(&self) -> Option<Vec<Value>> {
        let mut out = vec![];
        let mut cur = self;
        loop {
            match cur {
                Value::Nil => return Some(out),
                Value::Cons(h, t) => {
                    out.push((**h).clone());
                    cur = t;
                }
                _ => return None,
            }
        }
    }

    /// Structural equality (`=` and `check-expect`). Functions, models and
    /// capabilities are equal only to themselves.
    pub fn equal(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Num(a), Num(b)) | (Money(a), Money(b)) | (Dur(a), Dur(b)) => a == b,
            (Str(a), Str(b)) | (Sym(a), Sym(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Nil, Nil) | (Inf, Inf) => true,
            (Cons(a, b), Cons(c, d)) => a.equal(c) && b.equal(d),
            (Con(k, xs), Con(j, ys)) => k == j && xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(x, y)| x.equal(y)),
            (Record(r, xs), Record(s, ys)) => {
                r == s && xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|((f, x), (g, y))| f == g && x.equal(y))
            }
            (Closure(a), Closure(b)) => Rc::ptr_eq(a, b),
            (Prim(a), Prim(b)) => a == b,
            (Model(a), Model(b)) => Rc::ptr_eq(a, b),
            (Cap(a), Cap(b)) => a.id == b.id,
            _ => false,
        }
    }
}

pub fn fmt_money(m: i64) -> String {
    let sign = if m < 0 { "-" } else { "" };
    let m = m.unsigned_abs();
    let whole = m / 1_000_000;
    let mut frac = format!("{:06}", m % 1_000_000);
    while frac.len() > 2 && frac.ends_with('0') {
        frac.pop();
    }
    format!("{}${}.{}", sign, whole, frac)
}

pub fn fmt_dur(ms: i64) -> String {
    if ms != 0 && ms % 60_000 == 0 {
        format!("{}min", ms / 60_000)
    } else if ms % 1000 == 0 {
        format!("{}s", ms / 1000)
    } else {
        format!("{}ms", ms)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Num(n) => write!(f, "{}", n),
            Value::Str(s) => write!(f, "{}", quote(s)),
            Value::Bool(true) => write!(f, "#t"),
            Value::Bool(false) => write!(f, "#f"),
            Value::Sym(s) => write!(f, "'{}", s),
            Value::Nil => write!(f, "'()"),
            Value::Cons(..) => match self.to_vec() {
                Some(items) => {
                    write!(f, "(list")?;
                    for x in items {
                        write!(f, " {}", x)?;
                    }
                    write!(f, ")")
                }
                None => {
                    if let Value::Cons(h, t) = self {
                        write!(f, "(cons {} {})", h, t)
                    } else {
                        unreachable!()
                    }
                }
            },
            Value::Con(k, xs) if xs.is_empty() => write!(f, "{}", k),
            Value::Con(k, xs) => {
                write!(f, "({}", k)?;
                for x in xs.iter() {
                    write!(f, " {}", x)?;
                }
                write!(f, ")")
            }
            Value::Record(r, fs) => {
                write!(f, "({}", r)?;
                for (name, v) in fs.iter() {
                    write!(f, " [{} {}]", name, v)?;
                }
                write!(f, ")")
            }
            Value::Closure(_) => write!(f, "<function>"),
            Value::Prim(p) => write!(f, "<primitive {}>", p.name()),
            Value::Model(m) => write!(f, "<model {}>", m.name),
            Value::Cap(c) => write!(f, "<capability {}#{}>", c.key, c.id),
            Value::Money(m) => write!(f, "{}", fmt_money(*m)),
            Value::Dur(d) => write!(f, "{}", fmt_dur(*d)),
            Value::Inf => write!(f, "∞"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_prints_like_literals() {
        assert_eq!(fmt_money(500_000), "$0.50");
        assert_eq!(fmt_money(1), "$0.000001");
        assert_eq!(fmt_money(1_000_000), "$1.00");
        assert_eq!(fmt_money(6_390), "$0.00639");
    }

    #[test]
    fn durations_print_like_literals() {
        assert_eq!(fmt_dur(40_000), "40s");
        assert_eq!(fmt_dur(600_000), "10min");
        assert_eq!(fmt_dur(250), "250ms");
    }
}
