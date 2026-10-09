//! Θ, the datatype and record environment, with the three type-directed
//! functions the semantics needs: `askable` (04 §3.1, rule 5), `bound(τ)`
//! (03 Q1) and `validate_Θ(j, τ)` (05 Step 4).

use crate::ast::{ConDef, Name, Type};
use crate::value::Value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Bytes added by the root wrapper `{"value":` … `}`.
pub const WRAP_OVERHEAD: u64 = 10;

#[derive(Default)]
pub struct TypeEnv {
    pub datatypes: HashMap<Name, Rc<Vec<ConDef>>>,
    /// Each constructor's datatype, and its index within it.
    pub con_owner: HashMap<Name, (Name, usize)>,
    pub records: HashMap<Name, Rc<Vec<(Name, Type)>>>,
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

    fn check_type(&self, ty: &Type, defining: &str) -> Result<(), String> {
        match ty {
            Type::Named(n) if &**n == defining || self.is_type_name(n) => Ok(()),
            Type::Named(n) => Err(format!("unknown type {}", n)),
            Type::List(t, _) => self.check_type(t, defining),
            _ => Ok(()),
        }
    }

    fn check_fields(&self, fields: &[(Name, Type)], defining: &str, what: &str) -> Result<(), String> {
        let mut seen = HashSet::new();
        for (f, t) in fields {
            if !seen.insert(f.clone()) {
                return Err(format!("field {} appears twice in {}", f, what));
            }
            self.check_type(t, defining)?;
        }
        Ok(())
    }

    /// DEFINEDATATYPE: constructor names fresh, field types well formed.
    pub fn add_datatype(&mut self, name: Name, cons: Vec<ConDef>) -> Result<(), String> {
        if self.records.contains_key(&name) {
            return Err(format!("{} is already a record", name));
        }
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
            self.check_fields(&c.fields, &name, &c.name)?;
        }
        if let Some(old) = self.datatypes.get(&name) {
            for c in old.iter() {
                self.con_owner.remove(&c.name);
            }
        }
        for (i, c) in cons.iter().enumerate() {
            self.con_owner.insert(c.name.clone(), (name.clone(), i));
        }
        self.datatypes.insert(name, Rc::new(cons));
        Ok(())
    }

    pub fn add_record(&mut self, name: Name, fields: Vec<(Name, Type)>) -> Result<(), String> {
        if self.datatypes.contains_key(&name) {
            return Err(format!("{} is already a datatype", name));
        }
        self.check_fields(&fields, &name, &name)?;
        self.records.insert(name, Rc::new(fields));
        Ok(())
    }

    /// Askable types are first order: no functions, capabilities, models or `Any`.
    /// So validation can never produce a capability (Theorem 2).
    pub fn askable(&self, ty: &Type) -> Result<(), String> {
        self.askable_in(ty, &mut HashSet::new())
    }

    fn askable_in(&self, ty: &Type, visiting: &mut HashSet<Name>) -> Result<(), String> {
        match ty {
            Type::Text(_) | Type::Num | Type::Bool | Type::Sym => Ok(()),
            Type::List(t, _) => self.askable_in(t, visiting),
            Type::Any => Err("type Any is not askable".into()),
            Type::Named(n) => {
                if !visiting.insert(n.clone()) {
                    return Ok(());
                }
                if let Some(cons) = self.datatypes.get(n).cloned() {
                    for c in cons.iter() {
                        for (_, t) in &c.fields {
                            self.askable_in(t, visiting)
                                .map_err(|e| format!("{} is not askable: {} (in constructor {})", n, e, c.name))?;
                        }
                    }
                    Ok(())
                } else if let Some(fs) = self.records.get(n).cloned() {
                    for (f, t) in fs.iter() {
                        self.askable_in(t, visiting)
                            .map_err(|e| format!("{} is not askable: {} (in field {})", n, e, f))?;
                    }
                    Ok(())
                } else {
                    Err(format!("unknown type {}", n))
                }
            }
        }
    }

    /// An upper bound, in bytes, on the JSON encoding of any value of `ty`,
    /// or `None` if unbounded. Each token covers at least one byte, so this is
    /// also a sound bound on output tokens.
    pub fn bound(&self, ty: &Type) -> Option<u64> {
        self.bound_in(ty, &mut HashSet::new())
    }

    fn bound_in(&self, ty: &Type, visiting: &mut HashSet<Name>) -> Option<u64> {
        match ty {
            Type::Text(Some(n)) => Some(n + 2),
            Type::Text(None) | Type::Sym | Type::Any => None,
            Type::Num => Some(20),
            Type::Bool => Some(5),
            Type::List(t, Some(n)) => Some(2 + n * (self.bound_in(t, visiting)? + 1)),
            Type::List(_, None) => None,
            Type::Named(n) => {
                if !visiting.insert(n.clone()) {
                    return None; // recursive types are unbounded
                }
                let result = if let Some(cons) = self.datatypes.get(n).cloned() {
                    let mut best = 0;
                    for c in cons.iter() {
                        // {"tag":"K","f":…}
                        let mut size = 1 + 5 + 1 + (c.name.len() as u64 + 2);
                        for (f, t) in &c.fields {
                            size += 1 + (f.len() as u64 + 2) + 1 + self.bound_in(t, visiting)?;
                        }
                        best = best.max(size + 1);
                    }
                    Some(best)
                } else if let Some(fs) = self.records.get(n).cloned() {
                    let mut size = 2;
                    for (f, t) in fs.iter() {
                        size += (f.len() as u64 + 2) + 1 + self.bound_in(t, visiting)? + 1;
                    }
                    Some(size)
                } else {
                    None
                };
                visiting.remove(n);
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

    fn schema_in(&self, ty: &Type, visiting: &mut HashSet<Name>) -> Result<serde_json::Value, String> {
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
            Type::Named(n) => {
                if !visiting.insert(n.clone()) {
                    return Err(format!("{} is recursive, and a live model can't be asked for a recursive type", n));
                }
                let schema = if let Some(cons) = self.datatypes.get(n).cloned() {
                    let mut alternatives = vec![];
                    for c in cons.iter() {
                        let mut props = vec![("tag".to_string(), json!({"const": &*c.name}))];
                        for (f, t) in &c.fields {
                            props.push((f.to_string(), self.schema_in(t, visiting)?));
                        }
                        alternatives.push(object(props));
                    }
                    if alternatives.len() == 1 { alternatives.pop().unwrap() } else { json!({"anyOf": alternatives}) }
                } else if let Some(fs) = self.records.get(n).cloned() {
                    let mut props = vec![];
                    for (f, t) in fs.iter() {
                        props.push((f.to_string(), self.schema_in(t, visiting)?));
                    }
                    object(props)
                } else {
                    return Err(format!("unknown type {}", n));
                };
                visiting.remove(n);
                schema
            }
        })
    }

    /// Whether `S(τ)` is an object schema. The API requires an object at the
    /// root, so any other answer is asked for as `{"value": S(τ)}` (design/09 §2).
    /// This is decided by shape alone, so it also works for recursive types.
    pub fn root_is_object(&self, ty: &Type) -> bool {
        match ty {
            Type::Named(n) => self.records.contains_key(n) || self.datatypes.get(n).is_some_and(|cons| cons.len() == 1),
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
            Type::Any => None,
            Type::Named(n) => {
                let obj = j.as_object()?;
                if let Some(cons) = self.datatypes.get(n) {
                    let tag = obj.get("tag")?.as_str()?;
                    let c = cons.iter().find(|c| &*c.name == tag)?;
                    if obj.len() != c.fields.len() + 1 {
                        return None;
                    }
                    let vals =
                        c.fields.iter().map(|(f, t)| self.validate(obj.get(&**f)?, t)).collect::<Option<Vec<_>>>()?;
                    Some(Value::Con(c.name.clone(), Rc::new(vals)))
                } else if let Some(fs) = self.records.get(n) {
                    if obj.len() != fs.len() {
                        return None;
                    }
                    let vals = fs
                        .iter()
                        .map(|(f, t)| Some((f.clone(), self.validate(obj.get(&**f)?, t)?)))
                        .collect::<Option<Vec<_>>>()?;
                    Some(Value::Record(n.clone(), Rc::new(vals)))
                } else {
                    None
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
            vec![
                ConDef { name: Rc::from("Buy"), fields: vec![(Rc::from("reason"), text.clone())] },
                ConDef { name: Rc::from("Hold"), fields: vec![] },
            ],
        )
        .unwrap();
        t.add_record(Rc::from("Point"), vec![(Rc::from("x"), Type::Num), (Rc::from("y"), Type::Num)]).unwrap();
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
        let verdict = Type::Named(Rc::from("Verdict"));
        let point = Type::Named(Rc::from("Point"));

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
        let cap = Value::Cap(crate::value::Cap { id: 1, kind: crate::value::CapKind::Kernel, key: Rc::from("py") });
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
            vec![ConDef { name: Rc::from("Oops"), fields: vec![(Rc::from("tag"), Type::Text(None))] }],
        )
        .unwrap();
        let err = t.show(&Value::con("Oops", vec![Value::str("x")])).unwrap_err();
        assert!(err.contains("reserves"), "{err}");
    }
}
