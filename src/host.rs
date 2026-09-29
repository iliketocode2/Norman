//! The scripted oracle and capability hosts (05, "Conventions for scripted mode").
//!
//! A script maps each site to a sequence of replies. Every test starts from
//! a fresh `ScriptState`, so scripts rewind between tests.

use crate::ast::{Entry, Script};
use std::collections::HashMap;
use std::rc::Rc;

pub struct ScriptState {
    script: Option<Rc<Script>>,
    pos: HashMap<String, usize>,
}

impl ScriptState {
    pub fn new(script: Option<Rc<Script>>) -> ScriptState {
        ScriptState { script, pos: HashMap::new() }
    }

    /// The next reply for `site`, or `None` if the script has none left.
    /// Running out is a checked run-time error, reported by the caller.
    pub fn next(&mut self, site: &str) -> Option<Entry> {
        let entries = self.script.as_ref()?.sites.get(site)?;
        let i = self.pos.entry(site.to_string()).or_insert(0);
        let e = entries.get(*i)?.clone();
        *i += 1;
        Some(e)
    }
}
