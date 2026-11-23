//! Oracles: who answers an `ask` or a `call` (design/09 §5–6).
//!
//! The machine never talks to a model or a tool directly. It asks an `Oracle`
//! to count an ask's input, issue a request, cancel one, and deliver the next
//! batch of completions up to a time limit. The scripted oracle below answers
//! from scripts with a virtual clock (05, "Conventions for scripted mode"). A
//! live oracle answers from a real model API with a real clock.

use crate::ast::{Entry, Script, Type};
use crate::value::{HumanSpec, ModelSpec, Value};
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
    /// The answer type's JSON schema, as sent (design/09 §2), or empty for a
    /// type that can't be asked live (a recursive one). It is input too.
    pub schema: Rc<str>,
}

/// One `consult`, as the oracle sees it (design/11 §2.1): what the person is
/// shown, and the shape their answer must take.
#[derive(Debug, Clone)]
pub struct ConsultReq {
    pub site: String,
    pub human: Rc<HumanSpec>,
    pub ty: Type,
    pub prompt: String,
    /// `show v` of the proposal, if there is one. Approving it unchanged
    /// submits exactly this text.
    pub proposal: Option<String>,
}

/// An input-token count for an ask, before it's sent.
#[derive(Debug, Clone, Copy)]
pub struct Count {
    pub tokens: i64,
    /// Exact counts are used as-is. Estimates get the safety margin of 09 §3.
    pub exact: bool,
}

/// What a model request billed, in tokens. Input is split three ways,
/// because the provider prices them differently (design/09 §7a): read from
/// the prompt cache (cheap), written to it (dearer than plain input), and
/// neither.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
    pub cache_read: i64,
    pub output: i64,
}

impl Usage {
    /// Input billed at the plain input price.
    pub fn uncached(input: i64) -> Usage {
        Usage { input, ..Usage::default() }
    }
    /// Every input token, however it was priced.
    pub fn input_total(&self) -> i64 {
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }
    pub fn cache_write(&self) -> i64 {
        self.cache_write_5m + self.cache_write_1h
    }
}

/// What `usage` costs at `model`'s prices, in µ$, rounded up. Writes cost
/// 1.25× the input price with the 5-minute lifetime and 2× with the hour's;
/// reads cost the grant's read price. The sum is taken in quarter
/// nano-dollars, where all of these are whole, so nothing is lost to rounding
/// before the one rounding at the end.
pub fn charge(model: &ModelSpec, u: &Usage) -> i64 {
    let q = u.input as i128 * model.in_nano as i128 * 4
        + u.cache_write_5m as i128 * model.in_nano as i128 * 5
        + u.cache_write_1h as i128 * model.in_nano as i128 * 8
        + u.cache_read as i128 * model.read_nano as i128 * 4
        + u.output as i128 * model.out_nano as i128 * 4;
    micro(q)
}

/// The most an ask with `n_in` input tokens and `max_out` output tokens can
/// be charged (Theorem 1′). For a grant that caches, every input token might
/// be a cache write, so input is reserved at the write price.
pub fn reservation(model: &ModelSpec, n_in: i64, max_out: i64) -> i64 {
    let per_in: i128 = match model.cache_ttl {
        None => 4,
        Some(ttl) if ttl > 300_000 => 8,
        Some(_) => 5,
    };
    micro(n_in as i128 * model.in_nano as i128 * per_in + max_out as i128 * model.out_nano as i128 * 4)
}

/// Quarter nano-dollars to µ$, rounding up.
fn micro(q: i128) -> i64 {
    ((q + 3999) / 4000) as i64
}

/// The scripted tokenizer's count for an answer type's schema (design/09 §6a).
pub fn schema_tokens(schema: &str) -> i64 {
    use crate::defaults::{SCHEMA_BYTES, SCHEMA_TOKENS};
    (schema.len() as i64 * SCHEMA_TOKENS + SCHEMA_BYTES - 1) / SCHEMA_BYTES
}

/// How a request ended, as reported by the oracle.
#[derive(Debug, Clone)]
pub enum Completion {
    /// A model reply. `truncated` means it stopped at `max_tokens`.
    Answer {
        json: String,
        usage: Usage,
        truncated: bool,
    },
    Refusal {
        category: String,
        usage: Usage,
    },
    /// A retryable provider failure (429, 5xx, network).
    ProviderError {
        msg: String,
        usage: Usage,
    },
    ToolResult {
        text: String,
    },
    ToolError {
        msg: String,
    },
    /// A person submitted JSON text, to be validated against the consult's type.
    HumanAnswer {
        json: String,
    },
    /// A person declined, with a reason.
    Declined {
        reason: String,
    },
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
    /// Put a question to a person. An oracle with no person attached refuses,
    /// which is a checked run-time error: a missing person must never look
    /// like a person saying no (design/11, H3 and §4.1).
    fn issue_consult(&mut self, _rid: u64, req: ConsultReq, _now: i64) -> Result<(), String> {
        Err(format!("no person is attached to answer consult site '{}' (human {})", req.site, req.human.name))
    }
    /// May the kernel granted as `key` be forked? A scripted kernel has no
    /// state outside the script, so it forks for free. `Err` is a tool error:
    /// a host that can't copy its state must say so, because handing out a
    /// second handle to the *same* state would defeat the ownership check.
    fn fork(&mut self, _key: &str) -> Result<(), String> {
        Ok(())
    }
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
    cache: PromptCache,
}

impl ScriptedOracle {
    pub fn new(script: Option<Rc<Script>>) -> ScriptedOracle {
        ScriptedOracle { state: ScriptState::new(script), inflight: vec![], cache: PromptCache::default() }
    }
}

/// An ask's input as the API renders it, cut where a cache entry can end:
/// the schema and system prompt first, then one cut per turn. Each cut is
/// (the prefix's text, its tokens, whether a breakpoint is there). The
/// breakpoints are the two `request_body` sends: an explicit one ending the
/// system prompt, and the automatic one ending the prompt. Tokens are the
/// scripted tokenizer's, so the last cut's count is exactly `count`'s.
fn prefix_cuts(req: &AskReq) -> Vec<(String, i64, bool)> {
    let schema = schema_tokens(&req.schema);
    let (system, turns): (Vec<_>, Vec<_>) = req.messages.iter().partition(|(r, _)| &**r == "System");
    let mut text = format!("{}\u{0}", req.schema);
    let mut bytes = 0i64;
    let mut cuts = vec![];
    for (i, (role, content)) in system.iter().chain(&turns).enumerate() {
        text.push_str(role);
        text.push('\u{0}');
        text.push_str(content);
        text.push('\u{0}');
        bytes += role.len() as i64 + content.len() as i64 + 4;
        let ends_system = i + 1 == system.len();
        cuts.push((text.clone(), schema + (bytes + 3) / 4, ends_system));
    }
    if let Some(last) = cuts.last_mut() {
        last.2 = true;
    }
    cuts
}

/// The prompt cache, simulated (design/09 §7a), so a scripted run is charged
/// what a live run would be. Entries are per model, keyed by a prefix's
/// text, and last their TTL on the scripted clock, measured from the start
/// of the request that wrote or last read them. As in the API: a prompt
/// below the model's minimum isn't cached at all; otherwise a request reads
/// the longest live cached prefix, refreshing it, and writes everything past
/// it; and an entry can be read only by a request that starts after the one
/// that wrote it, since it exists only once that one's reply begins.
#[derive(Default)]
struct PromptCache {
    /// (model, prefix) → (written at, expires at)
    entries: HashMap<(String, String), (i64, i64)>,
}

impl PromptCache {
    fn usage(&mut self, req: &AskReq, now: i64) -> Usage {
        let cuts = prefix_cuts(req);
        let total = cuts.last().map_or(schema_tokens(&req.schema), |c| c.1);
        let Some(ttl) = req.model.cache_ttl else {
            return Usage::uncached(total);
        };
        let model = req.model.id.clone();
        let min = crate::defaults::min_cacheable(&model);
        if total < min {
            return Usage::uncached(total);
        }
        let live = |e: &(i64, i64)| e.0 < now && e.1 > now;
        let mut read = 0;
        for (text, n, _) in cuts.iter().rev() {
            if let Some(e) = self.entries.get_mut(&(model.clone(), text.clone()))
                && live(e)
            {
                e.1 = now + ttl;
                read = *n;
                break;
            }
        }
        for (text, n, breakpoint) in &cuts {
            if *breakpoint && *n >= min {
                let e = self.entries.entry((model.clone(), text.clone())).or_insert((now, now + ttl));
                if e.1 <= now {
                    *e = (now, now + ttl); // expired: this is a new entry
                } else {
                    e.1 = e.1.max(now + ttl);
                }
            }
        }
        let write = total - read;
        if ttl > 300_000 {
            Usage { cache_write_1h: write, cache_read: read, ..Usage::default() }
        } else {
            Usage { cache_write_5m: write, cache_read: read, ..Usage::default() }
        }
    }
}

impl Oracle for ScriptedOracle {
    /// The test tokenizer defines billing in scripted mode, so the count is
    /// exact. It counts the schema as well as the messages: the schema is
    /// input, and usually most of it (design/09 §6a).
    fn count(&mut self, req: &AskReq) -> Result<Count, CountError> {
        Ok(Count { tokens: schema_tokens(&req.schema) + test_tokens(&req.messages), exact: true })
    }

    fn issue_ask(&mut self, rid: u64, req: AskReq, now: i64) -> Result<(), String> {
        let entry = self
            .state
            .next(&req.site)
            .ok_or_else(|| format!("script exhausted: no reply left for ask site '{}'", req.site))?;
        let usage = self.cache.usage(&req, now);
        let out = |n: u64| Usage { output: (n as i64).min(req.max_tokens), ..usage };
        let completion = match &entry {
            Entry::Reply { json, out: n, .. } => {
                Completion::Answer { json: json.clone(), usage: out(*n), truncated: (*n as i64) > req.max_tokens }
            }
            Entry::Refusal { category, out: n, .. } => {
                Completion::Refusal { category: category.clone(), usage: out(*n) }
            }
            Entry::ProviderError { msg, .. } => Completion::ProviderError { msg: msg.clone(), usage },
            _ => return Err(format!("the script entry for ask site '{}' is not a model reply", req.site)),
        };
        self.inflight.push((rid, now + entry.latency(), completion));
        Ok(())
    }

    fn issue_call(&mut self, rid: u64, site: &str, _args: &[Value], now: i64) -> Result<(), String> {
        let entry = self
            .state
            .next(site)
            .ok_or_else(|| format!("script exhausted: no result left for call site '{}'", site))?;
        let completion = match &entry {
            Entry::Result { text, .. } => Completion::ToolResult { text: text.clone() },
            Entry::Error { msg, .. } => Completion::ToolError { msg: msg.clone() },
            _ => return Err(format!("the script entry for call site '{}' is not a tool result", site)),
        };
        self.inflight.push((rid, now + entry.latency(), completion));
        Ok(())
    }

    /// Exhaustion, a non-person entry, or `(approve)` with nothing to approve
    /// are all mistakes in the test, so all are checked run-time errors.
    fn issue_consult(&mut self, rid: u64, req: ConsultReq, now: i64) -> Result<(), String> {
        let site = &req.site;
        let entry = self
            .state
            .next(site)
            .ok_or_else(|| format!("script exhausted: no answer left for consult site '{}'", site))?;
        let completion = match &entry {
            Entry::Answer { json, .. } => Completion::HumanAnswer { json: json.clone() },
            Entry::Approve { .. } => match &req.proposal {
                Some(shown) => Completion::HumanAnswer { json: shown.clone() },
                None => return Err(format!("(approve) at consult site '{}', which proposed nothing", site)),
            },
            Entry::Decline { reason, .. } => Completion::Declined { reason: reason.clone() },
            _ => return Err(format!("the script entry for consult site '{}' is not a person's answer", site)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn model(in_nano: i64, read_nano: i64, cache_ttl: Option<i64>) -> ModelSpec {
        ModelSpec {
            name: Rc::from("m"),
            id: "claude-sonnet-5".into(),
            think: 0,
            in_nano,
            out_nano: 10_000,
            read_nano,
            cache_ttl,
            ceiling: 1000,
        }
    }

    /// Writes cost 1.25× or 2× input, reads the grant's read price, and the
    /// charge is rounded up once, at the end.
    #[test]
    fn charges_price_cache_reads_and_writes() {
        let m = model(2000, 200, Some(300_000));
        let u = |input, w5, w1, read, output| Usage {
            input,
            cache_write_5m: w5,
            cache_write_1h: w1,
            cache_read: read,
            output,
        };
        assert_eq!(charge(&m, &u(100, 0, 0, 0, 10)), 300); // 100 × 2 + 10 × 10
        assert_eq!(charge(&m, &u(0, 100, 0, 0, 0)), 250); // × 2.5
        assert_eq!(charge(&m, &u(0, 0, 100, 0, 0)), 400); // × 4
        assert_eq!(charge(&m, &u(0, 0, 0, 1000, 0)), 200); // × 0.2
        assert_eq!(charge(&m, &u(0, 0, 0, 1, 0)), 1, "0.2 µ$ rounds up to 1");
        assert_eq!(charge(&m, &u(0, 1, 0, 1, 0)), 3, "2.5 + 0.2 = 2.7, rounded once");
    }

    /// Theorem 1′ needs every charge within its reservation: a caching grant
    /// reserves input at the write price, whatever mix of reads and writes
    /// the provider then bills.
    #[test]
    fn a_charge_never_exceeds_its_reservation() {
        for ttl in [None, Some(300_000), Some(3_600_000)] {
            let m = model(1999, 133, ttl);
            let r = reservation(&m, 1000, 50);
            for written in [0, 1, 500, 1000] {
                for read in [0, 1, 1000 - written] {
                    let input = 1000 - written - read.min(1000 - written);
                    let read = read.min(1000 - written);
                    let u = match ttl {
                        Some(t) if t > 300_000 => {
                            Usage { input, cache_write_1h: written, cache_read: read, output: 50, ..Usage::default() }
                        }
                        Some(_) => {
                            Usage { input, cache_write_5m: written, cache_read: read, output: 50, ..Usage::default() }
                        }
                        None => Usage { input: 1000, output: 50, ..Usage::default() },
                    };
                    assert!(charge(&m, &u) <= r, "{:?}: {} > {} for {:?}", ttl, charge(&m, &u), r, u);
                }
            }
        }
        // And without caching, nothing changes: the reservation is plain input.
        assert_eq!(reservation(&model(2000, 200, None), 25, 436), 25 * 2 + 436 * 10);
    }
}
