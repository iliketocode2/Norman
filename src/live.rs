//! The live oracle (design/09 §5): `ask` answered by the Anthropic Messages
//! API, over raw HTTP, with a real clock.
//!
//! - `count` calls `/v1/messages/count_tokens` synchronously, before anything
//!   is reserved (M-ASK-COUNT).
//! - `issue_ask` sends `/v1/messages` on its own worker thread, so concurrent
//!   workflow nodes are really concurrent.
//! - `next` blocks until a reply arrives or the machine's deadline limit
//!   passes, whichever is first (M-TIME).
//!
//! There are no hidden retries (decision C): every HTTP failure is reported
//! once, as `ToolError` or as a checked run-time error.

use crate::anthropic::{self, API_VERSION, COUNT_TOKENS_PATH, MESSAGES_PATH};
use crate::host::{AskReq, Completion, Count, CountError, Oracle};
use crate::types::TypeEnv;
use crate::value::Value;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Debug, Clone)]
pub enum Auth {
    /// `x-api-key: …` (the `ANTHROPIC_API_KEY` environment variable).
    ApiKey(String),
    /// `Authorization: Bearer …` (the `ANTHROPIC_AUTH_TOKEN` environment variable).
    Bearer(String),
}

#[derive(Debug, Clone)]
pub struct LiveConfig {
    pub base_url: String,
    pub auth: Auth,
    /// Per-request HTTP timeout. Deadlines are enforced by the machine, not by this.
    pub timeout: Duration,
}

impl LiveConfig {
    /// Credentials from `ANTHROPIC_API_KEY`, or else `ANTHROPIC_AUTH_TOKEN`.
    /// `ANTHROPIC_BASE_URL` overrides the endpoint.
    pub fn from_env() -> Result<LiveConfig, String> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let auth = if let Some(k) = get("ANTHROPIC_API_KEY") {
            Auth::ApiKey(k)
        } else if let Some(t) = get("ANTHROPIC_AUTH_TOKEN") {
            Auth::Bearer(t)
        } else {
            return Err("live mode needs ANTHROPIC_API_KEY (or ANTHROPIC_AUTH_TOKEN) in the environment".into());
        };
        let base_url = get("ANTHROPIC_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.into());
        Ok(LiveConfig { base_url, auth, timeout: Duration::from_secs(600) })
    }
}

/// A reply, as a worker thread hands it back: `Ok((status, body))`, or a network error.
struct Arrival {
    rid: u64,
    at: Instant,
    result: Result<(u16, String), String>,
}

pub struct LiveOracle<'a> {
    theta: &'a TypeEnv,
    cfg: LiveConfig,
    agent: ureq::Agent,
    start: Instant,
    tx: Sender<Arrival>,
    rx: Receiver<Arrival>,
    /// In-flight asks, and whether each answer is wrapped as {"value": …}.
    inflight: HashMap<u64, bool>,
    /// A reply that arrived after the machine's limit, held for a later `next`.
    held: Vec<Arrival>,
}

impl<'a> LiveOracle<'a> {
    pub fn new(theta: &'a TypeEnv, cfg: LiveConfig) -> LiveOracle<'a> {
        let agent = ureq::AgentBuilder::new().timeout(cfg.timeout).build();
        let (tx, rx) = channel();
        LiveOracle { theta, cfg, agent, start: Instant::now(), tx, rx, inflight: HashMap::new(), held: vec![] }
    }

    fn ms(&self, t: Instant) -> i64 {
        t.saturating_duration_since(self.start).as_millis() as i64
    }

    /// Turn an arrival into a completion, or a checked run-time error for a fatal status.
    fn completion(&self, a: Arrival, wrapped: bool) -> Result<Completion, String> {
        match a.result {
            Ok((status, body)) => anthropic::parse_response(status, &body, wrapped),
            Err(e) => Ok(Completion::ProviderError { msg: format!("network error: {}", e), input_tokens: 0 }),
        }
    }
}

/// One POST, with the API's headers. HTTP error statuses are returned, not raised.
fn post(agent: &ureq::Agent, cfg: &LiveConfig, path: &str, body: &str) -> Result<(u16, String), String> {
    let mut req = agent
        .post(&format!("{}{}", cfg.base_url.trim_end_matches('/'), path))
        .set("content-type", "application/json")
        .set("anthropic-version", API_VERSION);
    req = match &cfg.auth {
        Auth::ApiKey(k) => req.set("x-api-key", k),
        Auth::Bearer(t) => req.set("authorization", &format!("Bearer {}", t)).set("anthropic-beta", "oauth-2025-04-20"),
    };
    match req.send_string(body) {
        Ok(resp) => {
            let status = resp.status();
            resp.into_string().map(|b| (status, b)).map_err(|e| e.to_string())
        }
        Err(ureq::Error::Status(status, resp)) => Ok((status, resp.into_string().unwrap_or_default())),
        Err(e) => Err(e.to_string()),
    }
}

impl<'a> Oracle for LiveOracle<'a> {
    fn clock(&self) -> Option<i64> {
        Some(self.ms(Instant::now()))
    }

    fn count(&mut self, req: &AskReq) -> Result<Count, CountError> {
        let body = anthropic::count_body(self.theta, req).map_err(CountError::Fatal)?;
        let (status, text) = post(&self.agent, &self.cfg, COUNT_TOKENS_PATH, &body.to_string())
            .map_err(|e| CountError::Transient(format!("network error while counting tokens: {}", e)))?;
        match anthropic::parse_count(status, &text) {
            Ok(tokens) => Ok(Count { tokens, exact: false }), // the docs call it an estimate (09 §3)
            Err(anthropic::Failure::Transient(m)) => Err(CountError::Transient(m)),
            Err(anthropic::Failure::Fatal(m)) => Err(CountError::Fatal(m)),
        }
    }

    fn issue_ask(&mut self, rid: u64, req: AskReq, _now: i64) -> Result<(), String> {
        let (body, wrapped) = anthropic::request_body(self.theta, &req)?;
        self.inflight.insert(rid, wrapped);
        let (agent, cfg, tx) = (self.agent.clone(), self.cfg.clone(), self.tx.clone());
        let body = body.to_string();
        std::thread::spawn(move || {
            let result = post(&agent, &cfg, MESSAGES_PATH, &body);
            // The receiver may be gone if the run already ended; that's fine.
            let _ = tx.send(Arrival { rid, at: Instant::now(), result });
        });
        Ok(())
    }

    fn issue_call(&mut self, _rid: u64, site: &str, _args: &[Value], _now: i64) -> Result<(), String> {
        Err(format!("live mode has no tool hosts yet, so call site '{}' can't be answered (09 §7)", site))
    }

    /// Abandon a request. Its worker thread finishes on its own; the reply is discarded.
    fn cancel(&mut self, rid: u64) {
        self.inflight.remove(&rid);
    }

    fn next(&mut self, limit: Option<i64>) -> Result<Option<(i64, Vec<(u64, Completion)>)>, String> {
        loop {
            if self.inflight.is_empty() {
                return Ok(None);
            }
            let arrival = if let Some(i) = self.held.iter().position(|a| limit.is_none_or(|l| self.ms(a.at) <= l)) {
                self.held.remove(i)
            } else {
                match limit {
                    None => self.rx.recv().map_err(|_| "internal error: the worker channel closed".to_string())?,
                    Some(l) => {
                        let now = self.ms(Instant::now());
                        if now >= l {
                            return Ok(None);
                        }
                        match self.rx.recv_timeout(Duration::from_millis((l - now) as u64)) {
                            Ok(a) => a,
                            Err(RecvTimeoutError::Timeout) => return Ok(None),
                            Err(RecvTimeoutError::Disconnected) => return Err("internal error: the worker channel closed".into()),
                        }
                    }
                }
            };
            if !self.inflight.contains_key(&arrival.rid) {
                continue; // cancelled earlier: discard
            }
            let t = self.ms(arrival.at);
            if limit.is_some_and(|l| t > l) {
                // It arrived after the deadline. Keep it; the machine will cut first.
                self.held.push(arrival);
                return Ok(None);
            }
            let wrapped = self.inflight.remove(&arrival.rid).unwrap();
            let rid = arrival.rid;
            let mut batch = vec![(rid, self.completion(arrival, wrapped)?)];
            // Whatever else has already arrived joins the batch, in issue order.
            while let Ok(more) = self.rx.try_recv() {
                match self.inflight.get(&more.rid).copied() {
                    Some(w) if limit.is_none_or(|l| self.ms(more.at) <= l) => {
                        self.inflight.remove(&more.rid);
                        let r = more.rid;
                        batch.push((r, self.completion(more, w)?));
                    }
                    Some(_) => self.held.push(more),
                    None => {}
                }
            }
            batch.sort_by_key(|(r, _)| *r);
            let latest = self.ms(Instant::now()).min(limit.unwrap_or(i64::MAX)).max(t);
            return Ok(Some((latest, batch)));
        }
    }
}
