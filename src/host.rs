//! Oracles: who answers an `ask` or a `call` (design/09 §5–6).
//!
//! The machine never talks to a model or a tool directly. It asks an `Oracle`
//! to count an ask's input, issue a request, cancel one, and deliver the next
//! batch of completions up to a time limit. The scripted oracle below answers
//! from scripts with a virtual clock (05, "Conventions for scripted mode"). A
//! live oracle answers from a real model API with a real clock.

use crate::ast::{Entry, Script, Type};
use crate::value::{ModelSpec, Value};
use std::collections::HashMap;
use std::rc::Rc;

/// One `ask`, as the oracle sees it.
#[derive(Debug, Clone)]
pub struct AskReq {
    pub site: String,
    pub model: Rc<ModelSpec>,
    pub ty: Type,
    /// The context, as (role, content) pairs; roles are System/User/Assistant/Tool.
    pub messages: Messages,
    pub max_tokens: i64,
}

/// An input-token count for an ask, before it's sent.
#[derive(Debug, Clone, Copy)]
pub struct Count {
    pub tokens: i64,
    /// Exact counts are used as-is. Estimates get the safety margin of 09 §3.
    pub exact: bool,
}

/// How a request ended, as reported by the oracle.
#[derive(Debug, Clone)]
pub enum Completion {
    /// A model reply. `truncated` means it stopped at `max_tokens`.
    Answer { json: String, input_tokens: i64, output_tokens: i64, truncated: bool },
    Refusal { category: String, input_tokens: i64, output_tokens: i64 },
    /// A retryable provider failure (429, 5xx, network).
    ProviderError { msg: String, input_tokens: i64 },
    ToolResult { text: String },
    ToolError { msg: String },
}

/// Why an ask couldn't be counted.
#[derive(Debug, Clone)]
pub enum CountError {
    /// A transient provider failure (429, 5xx, network): the ask fails with `ToolError`.
    Transient(String),
    /// A bad key, model ID or request: a checked run-time error.
    Fatal(String),
}

pub trait Oracle {
    /// The oracle's clock in ms since the run began, if it has one. A live
    /// oracle's time passes by itself, so the machine syncs to it. The scripted
    /// oracle's time is virtual and moves only through `next` (`None`).
    fn clock(&self) -> Option<i64> {
        None
    }
    /// Count an ask's input tokens (M-ASK-COUNT).
    fn count(&mut self, req: &AskReq) -> Result<Count, CountError>;
    /// Send an ask. It completes later, through `next`.
    fn issue_ask(&mut self, rid: u64, req: AskReq, now: i64) -> Result<(), String>;
    /// Send a call to capability site `key/op`.
    fn issue_call(&mut self, rid: u64, site: &str, args: &[Value], now: i64) -> Result<(), String>;
    /// Abandon an in-flight request (a deadline cut, or fail-fast cancellation).
    fn cancel(&mut self, rid: u64);
    /// The earliest batch of completions at or before `limit`: everything that
    /// completes at the same instant, in issue order. `None` means nothing
    /// completes by `limit`, and the machine advances its clock to `limit`.
    #[allow(clippy::type_complexity)]
    fn next(&mut self, limit: Option<i64>) -> Result<Option<(i64, Vec<(u64, Completion)>)>, String>;
}

/// Read an ask's context: a list of `Message` records.
/// A context as (role, content) pairs.
pub type Messages = Vec<(Rc<str>, Rc<str>)>;

pub fn messages(ctx: &Value) -> Result<Messages, String> {
    let items = ctx.to_vec().ok_or_else(|| format!("an ask's context must be a list of Message, got {}", ctx))?;
    items
        .iter()
        .map(|m| {
            let Value::Record(r, fs) = m else {
                return Err(format!("an ask's context must contain Message records, got {}", m));
            };
            if &**r != "Message" {
                return Err(format!("an ask's context must contain Message records, got {}", m));
            }
            let mut role = None;
            let mut content = None;
            for (f, v) in fs.iter() {
                match (&**f, v) {
                    ("role", Value::Con(k, _)) => role = Some(k.clone()),
                    ("content", Value::Str(s)) => content = Some(s.clone()),
                    _ => return Err(format!("malformed Message {}", m)),
                }
            }
            match (role, content) {
                (Some(r), Some(c)) => Ok((r, c)),
                _ => Err(format!("malformed Message {}", m)),
            }
        })
        .collect()
}

/// The test tokenizer (05): for each message, role + content + 4 bytes of
/// framing; tokens = ⌈bytes ÷ 4⌉.
pub fn test_tokens(messages: &[(Rc<str>, Rc<str>)]) -> i64 {
    let bytes: i64 = messages.iter().map(|(r, c)| r.len() as i64 + c.len() as i64 + 4).sum();
    (bytes + 3) / 4
}

/// Position within each site's script. Rewinds with every fresh world.
pub struct ScriptState {
    script: Option<Rc<Script>>,
    pos: HashMap<String, usize>,
}

impl ScriptState {
    pub fn new(script: Option<Rc<Script>>) -> ScriptState {
        ScriptState { script, pos: HashMap::new() }
    }

    /// The next entry for `site`, or `None` if the script has none left.
    pub fn next(&mut self, site: &str) -> Option<Entry> {
        let entries = self.script.as_ref()?.sites.get(site)?;
        let i = self.pos.entry(site.to_string()).or_insert(0);
        let e = entries.get(*i)?.clone();
        *i += 1;
        Some(e)
    }
}

/// The scripted oracle: replies come from a script, and time is virtual.
pub struct ScriptedOracle {
    state: ScriptState,
    /// (rid, due time, completion)
    inflight: Vec<(u64, i64, Completion)>,
}

impl ScriptedOracle {
    pub fn new(script: Option<Rc<Script>>) -> ScriptedOracle {
        ScriptedOracle { state: ScriptState::new(script), inflight: vec![] }
    }
}

impl Oracle for ScriptedOracle {
    /// The test tokenizer defines billing in scripted mode, so the count is exact.
    fn count(&mut self, req: &AskReq) -> Result<Count, CountError> {
        Ok(Count { tokens: test_tokens(&req.messages), exact: true })
    }

    fn issue_ask(&mut self, rid: u64, req: AskReq, now: i64) -> Result<(), String> {
        let entry = self
            .state
            .next(&req.site)
            .ok_or_else(|| format!("script exhausted: no reply left for ask site '{}'", req.site))?;
        let input_tokens = test_tokens(&req.messages);
        let out = |n: u64| (n as i64).min(req.max_tokens);
        let completion = match &entry {
            Entry::Reply { json, out: n, .. } => Completion::Answer {
                json: json.clone(),
                input_tokens,
                output_tokens: out(*n),
                truncated: (*n as i64) > req.max_tokens,
            },
            Entry::Refusal { category, out: n, .. } => {
                Completion::Refusal { category: category.clone(), input_tokens, output_tokens: out(*n) }
            }
            Entry::ProviderError { msg, .. } => Completion::ProviderError { msg: msg.clone(), input_tokens },
            Entry::Result { .. } | Entry::Error { .. } => {
                return Err(format!("the script entry for ask site '{}' is a tool result, not a model reply", req.site));
            }
        };
        self.inflight.push((rid, now + entry.latency(), completion));
        Ok(())
    }

    fn issue_call(&mut self, rid: u64, site: &str, _args: &[Value], now: i64) -> Result<(), String> {
        let entry = self.state.next(site).ok_or_else(|| format!("script exhausted: no result left for call site '{}'", site))?;
        let completion = match &entry {
            Entry::Result { text, .. } => Completion::ToolResult { text: text.clone() },
            Entry::Error { msg, .. } => Completion::ToolError { msg: msg.clone() },
            _ => return Err(format!("the script entry for call site '{}' is a model reply, not a tool result", site)),
        };
        self.inflight.push((rid, now + entry.latency(), completion));
        Ok(())
    }

    fn cancel(&mut self, rid: u64) {
        self.inflight.retain(|(r, _, _)| *r != rid);
    }

    fn next(&mut self, limit: Option<i64>) -> Result<Option<(i64, Vec<(u64, Completion)>)>, String> {
        let Some(t) = self.inflight.iter().map(|(_, due, _)| *due).min() else {
            return Ok(None);
        };
        if limit.is_some_and(|l| t > l) {
            return Ok(None);
        }
        let mut batch = vec![];
        let mut i = 0;
        while i < self.inflight.len() {
            if self.inflight[i].1 == t {
                let (rid, _, c) = self.inflight.remove(i);
                batch.push((rid, c));
            } else {
                i += 1;
            }
        }
        batch.sort_by_key(|(rid, _)| *rid);
        Ok(Some((t, batch)))
    }
}
