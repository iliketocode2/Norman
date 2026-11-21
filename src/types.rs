//! Θ, the datatype and record environment, with the three type-directed
//! functions the semantics needs: `askable` (04 §3.1, rule 5), `bound(τ)`
//! (03 Q1) and `validate_Θ(j, τ)` (05 Step 4).

use crate::ast::{ConDef, Name, Type};
use crate::value::Value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Bytes added by the root wrapper `{"value":` … `}`.
pub const WRAP_OVERHEAD: u64 = 10;

fn arity(n: &str, want: usize, got: usize) -> String {
    match want {
        0 => format!("{} takes no type arguments, but was given {}", n, got),
        1 => format!("{} takes 1 type argument, but was given {}", n, got),
        _ => format!("{} takes {} type arguments, but was given {}", n, want, got),
    }
}

#[derive(Default)]
pub struct TypeEnv {
    pub datatypes: HashMap<Name, Rc<Vec<ConDef>>>,
    /// Each constructor's datatype, and its index within it.
    pub con_owner: HashMap<Name, (Name, usize)>,
    pub records: HashMap<Name, Rc<Vec<(Name, Type)>>>,
    /// The type parameters of each datatype and record; none for most.
    pub params: HashMap<Name, Rc<Vec<Name>>>,
}

/// A datatype or record at particular type arguments: its constructors or
/// fields with every parameter replaced.
enum Instance {
    Data(Vec<ConDef>),
    Record(Vec<(Name, Type)>),
}

/// Replace type parameters by their arguments.
fn subst(ty: &Type, sigma: &HashMap<Name, Type>) -> Type {
    match ty {
        Type::Var(v) => sigma.get(v).cloned().unwrap_or_else(|| ty.clone()),
        Type::List(t, b) => Type::List(Box::new(subst(t, sigma)), *b),
        Type::Named(n, args) => Type::Named(n.clone(), args.iter().map(|a| subst(a, sigma)).collect()),
        _ => ty.clone(),
    }
}

impl TypeEnv {
    pub fn is_type_name(&self, n: &str) -> bool {
        self.datatypes.contains_key(n) || self.records.contains_key(n)
    }

    pub fn is_record(&self, n: &str) -> bool {
        self.records.contains_key(n)
    }

    pub fn con_def(&self, k: &str) -> Option<&ConDef> {
        let (t, i) = self.con_owner.get(k)?;
        self.datatypes.get(t).map(|cons| &cons[*i])
    }

    /// The datatype that constructor `k` belongs to.
    pub fn con_type(&self, k: &str) -> Option<&Name> {
        self.con_owner.get(k).map(|(t, _)| t)
    }

    /// The type parameters of `n`, if `n` is a datatype or record.
    fn params_of(&self, n: &str) -> &[Name] {
        self.params.get(n).map(|p| &p[..]).unwrap_or(&[])
    }

    /// `n` at type arguments `args`, with the parameters substituted.
    fn instance(&self, n: &Name, args: &[Type]) -> Result<Instance, String> {
        let params = self.params_of(n);
        if !self.is_type_name(n) {
            return Err(format!("unknown type {}", n));
        }
        if params.len() != args.len() {
            return Err(arity(n, params.len(), args.len()));
        }
        let sigma: HashMap<Name, Type> = params.iter().cloned().zip(args.iter().cloned()).collect();
        if let Some(cons) = self.datatypes.get(n) {
            Ok(Instance::Data(
                cons.iter()
                    .map(|c| ConDef {
                        name: c.name.clone(),
                        fields: c.fields.iter().map(|(f, t)| (f.clone(), subst(t, &sigma))).collect(),
                    })
                    .collect(),
            ))
        } else {
            let fs = &self.records[n];
            Ok(Instance::Record(fs.iter().map(|(f, t)| (f.clone(), subst(t, &sigma))).collect()))
        }
    }

    /// A field's type is well formed: every name known and applied to the
    /// right number of arguments, and every type variable one of `params`.
    /// Inside its own definition a parameterized type must be applied to
    /// exactly its own parameters, in order: `(Tree a)`, never
    /// `(Tree (List a))`. Such *regular* recursion keeps every type-directed
    /// function finite, because unfolding a type never makes it bigger.
    fn check_type(&self, ty: &Type, defining: &str, params: &[Name]) -> Result<(), String> {
        match ty {
            Type::Named(n, args) if &**n == defining => {
                let own = args.len() == params.len()
                    && args.iter().zip(params).all(|(a, p)| matches!(a, Type::Var(v) if v == p));
                if own {
                    Ok(())
                } else if args.len() != params.len() {
                    Err(arity(n, params.len(), args.len()))
                } else {
                    let ps: Vec<&str> = params.iter().map(|p| &**p).collect();
                    Err(format!(
                        "inside its own definition, {} must be applied to its own parameters, ({} {})",
                        n,
                        n,
                        ps.join(" ")
                    ))
                }
            }
            Type::Named(n, args) if self.is_type_name(n) => {
                let want = self.params_of(n).len();
                if args.len() != want {
                    return Err(arity(n, want, args.len()));
                }
                args.iter().try_for_each(|a| self.check_type(a, defining, params))
            }
            Type::Named(n, _) => Err(format!("unknown type {}", n)),
            Type::Var(v) if params.contains(v) => Ok(()),
            Type::Var(v) => Err(format!("unknown type variable {}", v)),
            Type::List(t, _) => self.check_type(t, defining, params),
            _ => Ok(()),
        }
    }

    fn check_fields(&self, fields: &[(Name, Type)], defining: &str, params: &[Name], what: &str) -> Result<(), String> {
        let mut seen = HashSet::new();
        for (f, t) in fields {
            if !seen.insert(f.clone()) {
                return Err(format!("field {} appears twice in {}", f, what));
            }
            self.check_type(t, defining, params)?;
        }
        Ok(())
    }

    fn check_params(name: &Name, params: &[Name]) -> Result<(), String> {
        let mut seen = HashSet::new();
        for p in params {
            if !seen.insert(p.clone()) {
                return Err(format!("type parameter {} appears twice in {}", p, name));
            }
        }
        Ok(())
    }

    /// DEFINEDATATYPE: constructor names fresh, field types well formed.
    pub fn add_datatype(&mut self, name: Name, params: Vec<Name>, cons: Vec<ConDef>) -> Result<(), String> {
        if self.records.contains_key(&name) {
            return Err(format!("{} is already a record", name));
        }
        Self::check_params(&name, &params)?;
        let mut seen = HashSet::new();
        for c in &cons {
            if !seen.insert(c.name.clone()) {
                return Err(format!("constructor {} appears twice in {}", c.name, name));
            }
            if let Some((owner, _)) = self.con_owner.get(&c.name)
                && *owner != name
            {
                return Err(format!("constructor {} already belongs to {}", c.name, owner));
            }
            self.check_fields(&c.fields, &name, &params, &c.name)?;
        }
        if let Some(old) = self.datatypes.get(&name) {
            for c in old.iter() {
                self.con_owner.remove(&c.name);
            }
        }
        for (i, c) in cons.iter().enumerate() {
            self.con_owner.insert(c.name.clone(), (name.clone(), i));
        }
        self.params.insert(name.clone(), Rc::new(params));
        self.datatypes.insert(name, Rc::new(cons));
        Ok(())
    }

    pub fn add_record(&mut self, name: Name, params: Vec<Name>, fields: Vec<(Name, Type)>) -> Result<(), String> {
        if self.datatypes.contains_key(&name) {
            return Err(format!("{} is already a datatype", name));
        }
        Self::check_params(&name, &params)?;
        self.check_fields(&fields, &name, &params, &name)?;
        self.params.insert(name.clone(), Rc::new(params));
        self.records.insert(name, Rc::new(fields));
        Ok(())
    }

    /// Askable types are first order: no functions, capabilities, models or `Any`.
    /// So validation can never produce a capability (Theorem 2).
    pub fn askable(&self, ty: &Type) -> Result<(), String> {
        self.askable_in(ty, &mut HashSet::new())
    }

    // The three walks below guard against recursive types with a set of the
    // applied types being visited, keyed by the *whole* type, `(Option Num)`,
    // not the name `Option`. By name, `(Option (Option Num))` would look
    // recursive, and in `(Pair (Option Num) (Option Any))` the second Option
    // would be skipped as already seen. Regular recursion (`check_type`)
    // means a recursive type always recurs at the same key, so they end.

    fn askable_in(&self, ty: &Type, visiting: &mut HashSet<String>) -> Result<(), String> {
        match ty {
            Type::Text(_) | Type::Num | Type::Bool | Type::Sym => Ok(()),
            Type::List(t, _) => self.askable_in(t, visiting),
            Type::Any => Err("type Any is not askable".into()),
            Type::Money | Type::Dur => Err(format!("{} is not askable: JSON has no amounts of money or time", ty)),
            Type::Var(v) => Err(format!("the type variable {} is not askable", v)),
            Type::Named(n, args) => {
                if !visiting.insert(ty.to_string()) {
                    return Ok(());
                }
                match self.instance(n, args)? {
                    Instance::Data(cons) => {
                        for c in &cons {
                            for (_, t) in &c.fields {
                                self.askable_in(t, visiting)
                                    .map_err(|e| format!("{} is not askable: {} (in constructor {})", ty, e, c.name))?;
                            }
                        }
                    }
                    Instance::Record(fs) => {
                        for (f, t) in &fs {
                            self.askable_in(t, visiting)
                                .map_err(|e| format!("{} is not askable: {} (in field {})", ty, e, f))?;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// An upper bound, in bytes, on the JSON encoding of any value of `ty`,
    /// or `None` if unbounded. Each token covers at least one byte, so this is
    /// also a sound bound on output tokens.
    pub fn bound(&self, ty: &Type) -> Option<u64> {
        self.bound_in(ty, &mut HashSet::new())
    }

    fn bound_in(&self, ty: &Type, visiting: &mut HashSet<String>) -> Option<u64> {
        match ty {
            Type::Text(Some(n)) => Some(n + 2),
            Type::Text(None) | Type::Sym | Type::Any | Type::Money | Type::Dur | Type::Var(_) => None,
            Type::Num => Some(20),
            Type::Bool => Some(5),
            Type::List(t, Some(n)) => Some(2 + n * (self.bound_in(t, visiting)? + 1)),
            Type::List(_, None) => None,
            Type::Named(n, args) => {
                let key = ty.to_string();
                if !visiting.insert(key.clone()) {
                    return None; // recursive types are unbounded
                }
                let result = match self.instance(n, args).ok()? {
                    Instance::Data(cons) => {
                        let mut best = 0;
                        for c in &cons {
                            // {"tag":"K","f":…}
                            let mut size = 1 + 5 + 1 + (c.name.len() as u64 + 2);
                            for (f, t) in &c.fields {
                                size += 1 + (f.len() as u64 + 2) + 1 + self.bound_in(t, visiting)?;
                            }
                            best = best.max(size + 1);
                        }
                        Some(best)
                    }
                    Instance::Record(fs) => {
                        let mut size = 2;
                        for (f, t) in &fs {
                            size += (f.len() as u64 + 2) + 1 + self.bound_in(t, visiting)? + 1;
                        }
                        Some(size)
                    }
                };
                visiting.remove(&key);
                result
            }
        }
    }

    /// `S(τ)` (design/09 §2): the JSON Schema a live model's answer is decoded
    /// against. Bounds the API can't express (`(Text n)`, `(List τ n)`) are
    /// left out here and enforced by `validate`. Recursive datatypes are an error.
    pub fn json_schema(&self, ty: &Type) -> Result<serde_json::Value, String> {
        self.schema_in(ty, &mut HashSet::new())
    }

    fn schema_in(&self, ty: &Type, visiting: &mut HashSet<String>) -> Result<serde_json::Value, String> {
        use serde_json::json;
        let object = |props: Vec<(String, serde_json::Value)>| {
            let required: Vec<&String> = props.iter().map(|(k, _)| k).collect();
            json!({
                "type": "object",
                "properties": props.iter().cloned().collect::<serde_json::Map<_, _>>(),
                "required": required,
                "additionalProperties": false,
            })
        };
        Ok(match ty {
            Type::Text(_) | Type::Sym => json!({"type": "string"}),
            Type::Num => json!({"type": "integer"}),
            Type::Bool => json!({"type": "boolean"}),
            Type::List(t, _) => json!({"type": "array", "items": self.schema_in(t, visiting)?}),
            Type::Any => return Err("type Any is not askable".into()),
            Type::Money | Type::Dur | Type::Var(_) => return Err(format!("{} is not askable", ty)),
            Type::Named(n, args) => {
                let key = ty.to_string();
                if !visiting.insert(key.clone()) {
                    return Err(format!("{} is recursive, and a live model can't be asked for a recursive type", ty));
                }
                let schema = match self.instance(n, args)? {
                    Instance::Data(cons) => {
                        let mut alternatives = vec![];
                        for c in &cons {
                            let mut props = vec![("tag".to_string(), json!({"const": &*c.name}))];
                            for (f, t) in &c.fields {
                                props.push((f.to_string(), self.schema_in(t, visiting)?));
                            }
                            alternatives.push(object(props));
                        }
                        if alternatives.len() == 1 {
                            alternatives.pop().unwrap()
                        } else {
                            json!({"anyOf": alternatives})
                        }
                    }
                    Instance::Record(fs) => {
                        let mut props = vec![];
                        for (f, t) in &fs {
                            props.push((f.to_string(), self.schema_in(t, visiting)?));
                        }
                        object(props)
                    }
                };
                visiting.remove(&key);
                schema
            }
        })
    }

    /// Whether `S(τ)` is an object schema. The API requires an object at the
    /// root, so any other answer is asked for as `{"value": S(τ)}` (design/09 §2).
    /// This is decided by shape alone, so it also works for recursive types.
    pub fn root_is_object(&self, ty: &Type) -> bool {
        match ty {
            Type::Named(n, _) => {
                self.records.contains_key(n) || self.datatypes.get(n).is_some_and(|cons| cons.len() == 1)
            }
            _ => false,
        }
    }

    /// Extra output bytes for the `{"value": …}` wrapper, when one is needed.
    /// Scripted and live mode use the same arithmetic, so reservations agree.
    pub fn wrap_overhead(&self, ty: &Type) -> u64 {
        if self.root_is_object(ty) { 0 } else { WRAP_OVERHEAD }
    }

    /// `validate_Θ(j, τ)`: the µNorman value that JSON `j` encodes at type `ty`,
    /// or `None`. Datatype values are `{"tag": "K", field: …}`.
    /// How money, durations and ∞ are written. JSON has none of them, and
    /// none is askable, so the round-trip law does not reach them: `show`
    /// renders them in µNorman's own literal syntax instead.
    fn literal(v: &Value) -> Option<String> {
        match v {
            Value::Money(m) => Some(crate::value::fmt_money(*m)),
            Value::Dur(d) => Some(crate::value::fmt_dur(*d)),
            Value::Inf => Some("∞".into()),
            _ => None,
        }
    }

    /// The JSON encoding of a value: the inverse of `validate`.
    ///
    /// Keys come out in sorted order, so `show` is canonical — equal values
    /// always produce identical text.
    pub fn to_json(&self, v: &Value) -> Result<serde_json::Value, String> {
        use serde_json::Value as J;
        if let Some(s) = Self::literal(v) {
            return Ok(J::String(s));
        }
        Ok(match v {
            Value::Num(n) => J::from(*n),
            Value::Str(s) => J::String(s.to_string()),
            Value::Bool(b) => J::Bool(*b),
            Value::Sym(s) => J::String(s.to_string()),
            Value::Nil | Value::Cons(..) => {
                let mut items = vec![];
                let mut cur = v;
                while let Value::Cons(head, tail) = cur {
                    items.push(self.to_json(head)?);
                    cur = tail;
                }
                if !matches!(cur, Value::Nil) {
                    return Err("cannot show an improper list".into());
                }
                J::Array(items)
            }
            Value::Con(k, fields) => {
                // A constructor's fields are positional, so their names come
                // from the datatype definition, exactly as `validate` reads them.
                let def = self.con_def(k).ok_or_else(|| format!("unknown constructor {}", k))?;
                let mut o = serde_json::Map::new();
                o.insert("tag".into(), J::String(def.name.to_string()));
                for ((name, _), value) in def.fields.iter().zip(fields.iter()) {
                    if &**name == "tag" {
                        // The tagged encoding reserves this key. Overwriting
                        // the discriminator would produce JSON that silently
                        // means something else.
                        return Err(format!("{} has a field named `tag`, which the encoding reserves", k));
                    }
                    o.insert(name.to_string(), self.to_json(value)?);
                }
                J::Object(o)
            }
            Value::Record(_, fields) => {
                let mut o = serde_json::Map::new();
                for (name, value) in fields.iter() {
                    o.insert(name.to_string(), self.to_json(value)?);
                }
                J::Object(o)
            }
            // A capability must never reach a prompt, and a function has no
            // written form at all.
            Value::Cap(_) => return Err("cannot show a capability".into()),
            Value::Model(_) => return Err("cannot show a model".into()),
            Value::Human(_) => return Err("cannot show a human".into()),
            Value::Closure(_) | Value::Prim(_) => return Err("cannot show a function".into()),
            Value::Money(_) | Value::Dur(_) | Value::Inf => unreachable!("handled by literal"),
        })
    }

    /// `show v`: text that reads back. For every askable type this is the JSON
    /// that `ask` validates, so `validate (show v) τ = v`.
    pub fn show(&self, v: &Value) -> Result<String, String> {
        match Self::literal(v) {
            // Bare at the top level, so it reads naturally in a prompt;
            // quoted when nested, because there it is a JSON string.
            Some(s) => Ok(s),
            None => Ok(self.to_json(v)?.to_string()),
        }
    }

    pub fn validate(&self, j: &serde_json::Value, ty: &Type) -> Option<Value> {
        match ty {
            Type::Text(bound) => {
                let s = j.as_str()?;
                if bound.is_some_and(|n| s.len() as u64 > n) {
                    return None;
                }
                Some(Value::str(s))
            }
            Type::Num => j.as_i64().map(Value::Num),
            Type::Bool => j.as_bool().map(Value::Bool),
            Type::Sym => j.as_str().map(|s| Value::Sym(Rc::from(s))),
            Type::List(t, bound) => {
                let items = j.as_array()?;
                if bound.is_some_and(|n| items.len() as u64 > n) {
                    return None;
                }
                let vals = items.iter().map(|x| self.validate(x, t)).collect::<Option<Vec<_>>>()?;
                Some(Value::list(vals))
            }
            Type::Any | Type::Money | Type::Dur | Type::Var(_) => None,
            Type::Named(n, args) => {
                let obj = j.as_object()?;
                match self.instance(n, args).ok()? {
                    Instance::Data(cons) => {
                        let tag = obj.get("tag")?.as_str()?;
                        let c = cons.iter().find(|c| &*c.name == tag)?;
                        if obj.len() != c.fields.len() + 1 {
                            return None;
                        }
                        let vals = c
                            .fields
                            .iter()
                            .map(|(f, t)| self.validate(obj.get(&**f)?, t))
                            .collect::<Option<Vec<_>>>()?;
                        Some(Value::Con(c.name.clone(), Rc::new(vals)))
                    }
                    Instance::Record(fs) => {
                        if obj.len() != fs.len() {
                            return None;
                        }
                        let vals = fs
                            .iter()
                            .map(|(f, t)| Some((f.clone(), self.validate(obj.get(&**f)?, t)?)))
                            .collect::<Option<Vec<_>>>()?;
                        Some(Value::Record(n.clone(), Rc::new(vals)))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::ConDef;

    /// A Θ with one datatype and one record, enough to reach every form a
    /// value can take.
    fn theta() -> TypeEnv {
        let mut t = TypeEnv::default();
        let text = Type::Text(Some(400));
        t.add_datatype(
            Rc::from("Verdict"),
            vec![],
            vec![
                ConDef { name: Rc::from("Buy"), fields: vec![(Rc::from("reason"), text.clone())] },
                ConDef { name: Rc::from("Hold"), fields: vec![] },
            ],
        )
        .unwrap();
        t.add_record(Rc::from("Point"), vec![], vec![(Rc::from("x"), Type::Num), (Rc::from("y"), Type::Num)]).unwrap();
        t
    }

    fn text(s: &str) -> Value {
        Value::str(s)
    }

    /// **The law.** For every askable type τ and every value v of τ,
    /// `validate (show v) τ = v`. `show` is the inverse of `validate`, so a
    /// value can be written into a prompt and read back unchanged.
    ///
    /// One case per form of value, which is what Step 9 asks for.
    #[test]
    fn show_round_trips_through_validate() {
        let t = theta();
        let verdict = Type::named("Verdict");
        let point = Type::named("Point");

        let cases: Vec<(Value, Type)> = vec![
            (Value::Num(42), Type::Num),
            (Value::Num(-7), Type::Num),
            (Value::Bool(true), Type::Bool),
            (Value::Bool(false), Type::Bool),
            (text("hi"), Type::Text(None)),
            (text(""), Type::Text(None)),
            (text(r#"quotes " and \ backslash"#), Type::Text(None)),
            (text("a\nb\tc"), Type::Text(None)),
            (text("unicode: héllo ∞ 日本"), Type::Text(None)),
            (text("bounded"), Type::Text(Some(400))),
            (Value::Sym(Rc::from("declined")), Type::Sym),
            (Value::list(vec![]), Type::List(Box::new(Type::Num), None)),
            (Value::list(vec![Value::Num(1), Value::Num(2)]), Type::List(Box::new(Type::Num), None)),
            (Value::list(vec![text("a")]), Type::List(Box::new(Type::Text(None)), Some(8))),
            (Value::con("Buy", vec![text("Cheap on earnings.")]), verdict.clone()),
            (Value::con("Hold", vec![]), verdict.clone()),
            (
                Value::Record(
                    Rc::from("Point"),
                    Rc::new(vec![(Rc::from("x"), Value::Num(3)), (Rc::from("y"), Value::Num(4))]),
                ),
                point.clone(),
            ),
            // Nesting, in both directions.
            (
                Value::list(vec![Value::con("Hold", vec![]), Value::con("Buy", vec![text("why")])]),
                Type::List(Box::new(verdict.clone()), None),
            ),
        ];

        for (v, ty) in cases {
            let shown = t.show(&v).unwrap_or_else(|e| panic!("show failed on {v}: {e}"));
            let parsed: serde_json::Value = serde_json::from_str(&shown)
                .unwrap_or_else(|e| panic!("show produced invalid JSON for {v}: {e} ({shown})"));
            let back = t
                .validate(&parsed, &ty)
                .unwrap_or_else(|| panic!("validate rejected show's own output for {v}: {shown}"));
            assert!(back.equal(&v), "round trip changed {v} into {back} (via {shown})");
        }
    }

    /// `show` is canonical: keys are sorted, so equal values are equal text.
    #[test]
    fn show_is_canonical() {
        let t = theta();
        let point = |x, y| {
            Value::Record(
                Rc::from("Point"),
                Rc::new(vec![(Rc::from("x"), Value::Num(x)), (Rc::from("y"), Value::Num(y))]),
            )
        };
        assert_eq!(t.show(&point(3, 4)).unwrap(), t.show(&point(3, 4)).unwrap());
        assert_eq!(t.show(&point(3, 4)).unwrap(), r#"{"x":3,"y":4}"#);
        assert_eq!(t.show(&Value::con("Buy", vec![text("w")])).unwrap(), r#"{"reason":"w","tag":"Buy"}"#);
    }

    /// Money, durations and ∞ are not askable, so the law does not reach
    /// them. They are written in µNorman's own literal syntax: bare at the
    /// top level, and as strings when nested.
    #[test]
    fn money_and_durations_are_written_as_literals() {
        let t = theta();
        assert_eq!(t.show(&Value::Money(500_000)).unwrap(), "$0.50");
        assert_eq!(t.show(&Value::Dur(40_000)).unwrap(), "40s");
        assert_eq!(t.show(&Value::Inf).unwrap(), "∞");
        assert_eq!(t.show(&Value::list(vec![Value::Money(1)])).unwrap(), r#"["$0.000001"]"#);
    }

    /// Authority must never reach a prompt, and a function has no written form.
    #[test]
    fn capabilities_and_functions_cannot_be_shown() {
        let t = theta();
        let cap = Value::Cap(crate::value::Cap {
            id: 1,
            kind: crate::value::CapKind::Kernel,
            key: Rc::from("py"),
            ops: Rc::new(vec![]),
        });
        assert!(t.show(&cap).is_err());
        assert!(t.show(&Value::Prim(crate::value::Prim::Add)).is_err());
        assert!(t.show(&Value::Cons(Rc::new(Value::Num(1)), Rc::new(Value::Num(2)))).is_err(), "improper list");
    }

    /// The tagged encoding reserves the key `tag`; a field of that name would
    /// silently overwrite the discriminator.
    #[test]
    fn a_field_named_tag_is_refused() {
        let mut t = TypeEnv::default();
        t.add_datatype(
            Rc::from("Bad"),
            vec![],
            vec![ConDef { name: Rc::from("Oops"), fields: vec![(Rc::from("tag"), Type::Text(None))] }],
        )
        .unwrap();
        let err = t.show(&Value::con("Oops", vec![Value::str("x")])).unwrap_err();
        assert!(err.contains("reserves"), "{err}");
    }
}

/// Type parameters (design/12, T5 and step 0).
#[cfg(test)]
mod parameter_tests {
    use super::*;

    fn var(v: &str) -> Type {
        Type::Var(Rc::from(v))
    }
    fn app(n: &str, args: Vec<Type>) -> Type {
        Type::Named(Rc::from(n), args)
    }
    fn con(name: &str, fields: Vec<(&str, Type)>) -> ConDef {
        ConDef { name: Rc::from(name), fields: fields.into_iter().map(|(f, t)| (Rc::from(f), t)).collect() }
    }
    fn names(xs: &[&str]) -> Vec<Name> {
        xs.iter().map(|x| Rc::from(*x)).collect()
    }

    /// The basis's parameterized types, as the prelude declares them.
    fn theta() -> TypeEnv {
        let mut t = TypeEnv::default();
        t.add_datatype(
            Rc::from("Option"),
            names(&["a"]),
            vec![con("None", vec![]), con("Some", vec![("value", var("a"))])],
        )
        .unwrap();
        t.add_record(
            Rc::from("Pair"),
            names(&["a", "b"]),
            vec![(Rc::from("fst"), var("a")), (Rc::from("snd"), var("b"))],
        )
        .unwrap();
        t.add_record(
            Rc::from("Resources"),
            vec![],
            vec![(Rc::from("cost"), Type::Money), (Rc::from("time"), Type::Dur)],
        )
        .unwrap();
        // A regular recursive type.
        t.add_datatype(
            Rc::from("Tree"),
            names(&["a"]),
            vec![con("Leaf", vec![]), con("Node", vec![("left", app("Tree", vec![var("a")])), ("item", var("a"))])],
        )
        .unwrap();
        t
    }

    fn some(v: Value) -> Value {
        Value::con("Some", vec![v])
    }
    fn none() -> Value {
        Value::con("None", vec![])
    }
    fn pair(a: Value, b: Value) -> Value {
        Value::Record(Rc::from("Pair"), Rc::new(vec![(Rc::from("fst"), a), (Rc::from("snd"), b)]))
    }

    /// The round-trip law S1 holds at parameterized types, including nested
    /// applications of the same type to different arguments.
    #[test]
    fn show_round_trips_at_parameterized_types() {
        let t = theta();
        let opt = |a| app("Option", vec![a]);
        let cases: Vec<(Value, Type)> = vec![
            (some(Value::Num(3)), opt(Type::Num)),
            (none(), opt(Type::Num)),
            (some(some(Value::Num(3))), opt(opt(Type::Num))),
            (some(none()), opt(opt(Type::Num))),
            (pair(Value::str("q\"uote"), Value::Num(-1)), app("Pair", vec![Type::Text(Some(10)), Type::Num])),
            (Value::list(vec![some(Value::str("a")), none()]), Type::List(Box::new(opt(Type::Text(Some(5)))), Some(2))),
            (pair(some(Value::Sym(Rc::from("s"))), none()), app("Pair", vec![opt(Type::Sym), opt(Type::Bool)])),
        ];
        for (v, ty) in cases {
            let shown = t.show(&v).unwrap();
            let back = t.validate(&serde_json::from_str(&shown).unwrap(), &ty);
            assert!(back.as_ref().is_some_and(|b| b.equal(&v)), "{v} at {ty} came back as {back:?} via {shown}");
        }
    }

    /// Validation substitutes the arguments: the same JSON is a value of one
    /// instance and not another.
    #[test]
    fn validation_follows_the_type_arguments() {
        let t = theta();
        let j: serde_json::Value = serde_json::from_str(r#"{"tag":"Some","value":"x"}"#).unwrap();
        assert!(t.validate(&j, &app("Option", vec![Type::Text(None)])).is_some());
        assert!(t.validate(&j, &app("Option", vec![Type::Num])).is_none());
        assert!(t.validate(&j, &app("Option", vec![Type::Text(Some(0))])).is_none(), "the argument's bound applies");
    }

    #[test]
    fn askability_follows_the_type_arguments() {
        let t = theta();
        let opt = |a| app("Option", vec![a]);
        assert!(t.askable(&opt(Type::Num)).is_ok(), "(Option Num) is askable now that Option has a parameter");
        assert!(t.askable(&opt(Type::Any)).is_err());
        assert!(t.askable(&opt(Type::Money)).is_err());
        assert!(t.askable(&Type::named("Resources")).is_err(), "money and durations are not askable");
        // The visited set is keyed by the whole type. Keyed by name, the second
        // Option would be skipped as already seen, and this would be askable.
        assert!(t.askable(&app("Pair", vec![opt(Type::Num), opt(Type::Any)])).is_err());
        assert!(t.askable(&app("Tree", vec![Type::Num])).is_ok(), "regular recursion terminates");
    }

    #[test]
    fn a_type_must_get_exactly_its_arguments() {
        let t = theta();
        let err = t.askable(&Type::named("Option")).unwrap_err();
        assert!(err.contains("takes 1 type argument, but was given 0"), "{err}");
        let err = t.askable(&app("Option", vec![Type::Num, Type::Num])).unwrap_err();
        assert!(err.contains("given 2"), "{err}");
        assert!(t.askable(&app("Resources", vec![Type::Num])).unwrap_err().contains("takes no type arguments"));
        assert!(t.validate(&serde_json::json!({"tag": "None"}), &Type::named("Option")).is_none());
    }

    /// Bounds and schemas substitute too, and nesting the same type is not
    /// recursion.
    #[test]
    fn bound_and_schema_follow_the_type_arguments() {
        let mut t = theta();
        // The same shape with no parameter: its bound must be identical.
        t.add_datatype(
            Rc::from("OptNum"),
            vec![],
            vec![con("NoNum", vec![]), con("SomeNum", vec![("value", Type::Num)])],
        )
        .unwrap();
        let opt_num = app("Option", vec![Type::Num]);
        let shape = |b: Option<u64>| b.map(|n| n as i64);
        // Constructor names differ in length (None/NoNum, Some/SomeNum), by 3 bytes at most.
        assert_eq!(shape(t.bound(&opt_num)).unwrap() + 3, shape(t.bound(&Type::named("OptNum"))).unwrap());
        assert_eq!(t.bound(&app("Option", vec![Type::Text(None)])), None, "an unbounded argument is unbounded");
        assert_eq!(t.bound(&app("Tree", vec![Type::Num])), None, "recursive types are unbounded");

        let s = t.json_schema(&opt_num).unwrap().to_string();
        assert!(s.contains(r#""value":{"type":"integer"}"#), "{s}");
        assert!(t.json_schema(&app("Option", vec![opt_num.clone()])).is_ok(), "nesting is not recursion");
        assert!(t.json_schema(&app("Tree", vec![Type::Num])).unwrap_err().contains("recursive"));
    }

    #[test]
    fn definitions_are_checked() {
        let mut t = theta();
        let err = t.add_datatype(Rc::from("Bad"), names(&["a"]), vec![con("B", vec![("x", var("b"))])]).unwrap_err();
        assert!(err.contains("unknown type variable b"), "{err}");
        let err = t.add_record(Rc::from("Twice"), names(&["a", "a"]), vec![]).unwrap_err();
        assert!(err.contains("appears twice"), "{err}");
        // Non-regular recursion would make the type-directed walks endless.
        let nested = con("N", vec![("x", app("Nest", vec![Type::List(Box::new(var("a")), None)]))]);
        let err = t.add_datatype(Rc::from("Nest"), names(&["a"]), vec![nested]).unwrap_err();
        assert!(err.contains("its own parameters"), "{err}");
        let err = t.add_record(Rc::from("R"), vec![], vec![(Rc::from("o"), Type::named("Option"))]).unwrap_err();
        assert!(err.contains("takes 1 type argument"), "{err}");
    }
}
