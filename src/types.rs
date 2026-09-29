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
