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
use crate::ast::Type;
use crate::host::{AskReq, Completion, ConsultReq, Count, CountError, Oracle};
use crate::types::TypeEnv;
use crate::value::Value;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Shared by every `LiveOracle::new`, which has no hosts.
static EMPTY_REGISTRY: OnceLock<crate::tools::HostRegistry> = OnceLock::new();
/// Shared by every live oracle made without people.
static NO_PEOPLE: OnceLock<People> = OnceLock::new();

// ------------------------------------------------------------------ people
//
// A live person (design/11 §4.1). The operator binds each `human` grant on
// the command line, to the terminal or to `unattended`. A consult bound to
// the terminal is shown there and answered by a typed line; the line comes
// back through the same event loop as a model's reply, so a deadline can cut
// the person off and fail-fast can withdraw the question.

/// How a `human` grant is bound for a live run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// Asked at the terminal.
    Tty,
    /// No person: any consult is a checked run-time error, never a decline.
    Unattended,
}

impl std::str::FromStr for Binding {
    type Err = String;
    fn from_str(s: &str) -> Result<Binding, String> {
        match s {
            "tty" => Ok(Binding::Tty),
            "unattended" => Ok(Binding::Unattended),
            other => Err(format!("a person is bound to `tty` or `unattended`, not `{}`", other)),
        }
    }
}

/// A line the person typed, and when it was read.
pub type Line = (Instant, String);

/// Where questions are shown and answers typed. One per process in normal
/// use (stdin and stderr); tests make their own.
#[derive(Clone)]
pub struct Console {
    lines: Arc<Mutex<Receiver<Line>>>,
    out: Arc<Mutex<dyn Write + Send>>,
}

impl Console {
    /// A console reading `lines` and writing to `out`.
    pub fn new(lines: Receiver<Line>, out: Arc<Mutex<dyn Write + Send>>) -> Console {
        Console { lines: Arc::new(Mutex::new(lines)), out }
    }

    /// The process's terminal: lines from stdin, questions on stderr, so
    /// stdout stays the program's output. Stdin is read by one thread for
    /// the whole process, whichever run is asking.
    pub fn stdio() -> Console {
        static STDIN: OnceLock<Arc<Mutex<Receiver<Line>>>> = OnceLock::new();
        let lines = STDIN
            .get_or_init(|| {
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let stdin = std::io::stdin();
                    let mut buf = String::new();
                    loop {
                        buf.clear();
                        match stdin.read_line(&mut buf) {
                            Ok(0) | Err(_) => break, // end of input
                            Ok(_) => {
                                if tx.send((Instant::now(), buf.trim_end_matches(['\r', '\n']).to_string())).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
                Arc::new(Mutex::new(rx))
            })
            .clone();
        Console { lines, out: Arc::new(Mutex::new(std::io::stderr())) }
    }
}

/// The people a live run may consult: a binding for each `human` grant.
#[derive(Clone, Default)]
pub struct People {
    pub bindings: HashMap<String, Binding>,
    /// Required when any binding is `Tty`.
    pub console: Option<Console>,
}

fn closed(site: &str, human: &str) -> String {
    format!("the terminal's input ended, so nobody can answer consult site '{}' (human {})", site, human)
}

/// The question on screen, and when it appeared.
struct Showing {
    rid: u64,
    req: ConsultReq,
    since: Instant,
}

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

/// What a worker thread hands back.
enum Answer {
    /// An HTTP reply: `Ok((status, body))`, or a network error.
    Http(Result<(u16, String), String>),
    /// A tool's result, or a tool error the program can catch.
    Tool(Result<String, String>),
    /// A line typed at the console. It belongs to no request until the
    /// oracle matches it to the question on screen.
    Line(String),
    /// The console's input ended (a closed pipe, or Ctrl-D): nobody can
    /// answer any more.
    ConsoleClosed,
}

struct Arrival {
    rid: u64,
    at: Instant,
    answer: Answer,
}

/// What an in-flight request is waiting for.
enum Pending {
    /// An ask, whether its answer is wrapped as `{"value": …}`, and its
    /// grant's cache lifetime, which prices cache writes.
    Ask { wrapped: bool, cache_ttl: Option<i64> },
    /// A call, and the host it is running on.
    Call { key: String },
    /// A consult, shown or waiting its turn at the console.
    Consult,
}

/// A running host: its worker thread, and the queue feeding it. Calls to one
/// host are serialized, because a stateful host does one thing at a time.
struct Running {
    tx: Sender<(u64, String, Vec<String>)>,
    /// Stops the host's current call, if the host supports it.
    interrupt: Option<crate::tools::Interrupt>,
}

pub struct LiveOracle<'a> {
    theta: &'a TypeEnv,
    cfg: LiveConfig,
    agent: ureq::Agent,
    start: Instant,
    tx: Sender<Arrival>,
    rx: Receiver<Arrival>,
    /// In-flight requests, and what each is waiting for.
    inflight: HashMap<u64, Pending>,
    /// The hosts this run may use, and the ones it has already started.
    registry: &'a crate::tools::HostRegistry,
    hosts: HashMap<String, Running>,
    /// A reply that arrived after the machine's limit, held for a later `next`.
    held: Vec<Arrival>,
    /// Who may be consulted, and how.
    people: &'a People,
    /// The question on the console, and those waiting behind it. A person
    /// answers one thing at a time, so consults are shown in issue order.
    showing: Option<Showing>,
    waiting: VecDeque<(u64, ConsultReq)>,
    /// Set when the run ends, so the console's forwarding thread stops.
    stop: Arc<AtomicBool>,
    forwarding: bool,
    /// The console's input has ended, so no consult can be answered.
    console_closed: bool,
}

impl<'a> LiveOracle<'a> {
    pub fn new(theta: &'a TypeEnv, cfg: LiveConfig) -> LiveOracle<'a> {
        Self::with_hosts(theta, cfg, EMPTY_REGISTRY.get_or_init(Default::default))
    }

    /// A live oracle that can also reach real tools.
    pub fn with_hosts(theta: &'a TypeEnv, cfg: LiveConfig, registry: &'a crate::tools::HostRegistry) -> LiveOracle<'a> {
        Self::with_people(theta, cfg, registry, NO_PEOPLE.get_or_init(Default::default))
    }

    /// A live oracle that can reach real tools and consult real people.
    pub fn with_people(
        theta: &'a TypeEnv,
        cfg: LiveConfig,
        registry: &'a crate::tools::HostRegistry,
        people: &'a People,
    ) -> LiveOracle<'a> {
        let agent = ureq::AgentBuilder::new().timeout(cfg.timeout).build();
        let (tx, rx) = channel();
        LiveOracle {
            theta,
            cfg,
            agent,
            start: Instant::now(),
            tx,
            rx,
            inflight: HashMap::new(),
            held: vec![],
            registry,
            hosts: HashMap::new(),
            people,
            showing: None,
            waiting: VecDeque::new(),
            stop: Arc::new(AtomicBool::new(false)),
            forwarding: false,
            console_closed: false,
        }
    }

    // ------------------------------------------------------------ console

    fn say(&self, text: &str) {
        if let Some(c) = &self.people.console
            && let Ok(mut out) = c.out.lock()
        {
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
    }

    /// Forward the console's lines into this run's event loop, from the
    /// first consult until the run ends. Polling with a timeout lets the
    /// thread notice the end of the run and give the console back.
    fn forward_console(&mut self) {
        let Some(console) = self.people.console.clone() else { return };
        if self.forwarding {
            return;
        }
        self.forwarding = true;
        let (tx, stop) = (self.tx.clone(), self.stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok(lines) = console.lines.lock() else { return };
                match lines.recv_timeout(Duration::from_millis(50)) {
                    Ok((at, line)) => {
                        if tx.send(Arrival { rid: u64::MAX, at, answer: Answer::Line(line) }).is_err() {
                            return; // the run ended; nobody is asking, so the line is dropped
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        let _ = tx.send(Arrival { rid: u64::MAX, at: Instant::now(), answer: Answer::ConsoleClosed });
                        return;
                    }
                }
            }
        });
    }

    /// Put the next waiting question on the console, if nothing is showing.
    fn show_next(&mut self) {
        if self.showing.is_some() {
            return;
        }
        let Some((rid, req)) = self.waiting.pop_front() else { return };
        let mut text = format!("\n── consult '{} · human {} ──\n{}\n", req.site, req.human.name, req.prompt);
        if let Some(p) = &req.proposal {
            text.push_str(&format!("proposal: {}\n", p));
        }
        text.push_str(&format!("answer type: {}\n", req.ty));
        if req.proposal.is_some() {
            text.push_str("  a          approve the proposal as shown\n");
        }
        text.push_str("  d REASON   decline\n");
        if matches!(req.ty, Type::Text(_)) {
            text.push_str("  TEXT       anything else is your answer\n");
        } else {
            text.push_str("  JSON       answer with this value\n");
        }
        if !self.waiting.is_empty() {
            text.push_str(&format!("({} more waiting)\n", self.waiting.len()));
        }
        text.push_str("> ");
        self.say(&text);
        self.showing = Some(Showing { rid, req, since: Instant::now() });
    }

    /// Match a typed line to the question on screen. A line read before the
    /// question appeared never answers it: it was meant for something else.
    /// A reply that doesn't fit is asked for again, which is the console's
    /// form validation, not a second consult (design/11 §4.1).
    fn on_line(&mut self, at: Instant, line: &str) -> Option<(u64, Completion)> {
        let showing = self.showing.as_ref()?;
        if at < showing.since {
            return None;
        }
        let line = line.trim();
        let req = &showing.req;
        let completion = if line == "a" || line == "approve" {
            match &req.proposal {
                Some(p) => Completion::HumanAnswer { json: p.clone() },
                None => {
                    self.say("nothing was proposed, so there is nothing to approve; type an answer or `d REASON`\n> ");
                    return None;
                }
            }
        } else if line == "d" || line == "decline" {
            Completion::Declined { reason: String::new() }
        } else if let Some(reason) = line.strip_prefix("d ").or_else(|| line.strip_prefix("decline ")) {
            Completion::Declined { reason: reason.trim().to_string() }
        } else {
            let json = match serde_json::from_str::<serde_json::Value>(line) {
                Ok(_) => line.to_string(),
                // For a Text answer, plain text is the answer itself.
                Err(_) if matches!(req.ty, Type::Text(_)) => serde_json::Value::String(line.to_string()).to_string(),
                Err(_) => {
                    self.say("that isn't JSON; type `a`, `d REASON`, or a JSON value\n> ");
                    return None;
                }
            };
            let fits =
                serde_json::from_str::<serde_json::Value>(&json).ok().and_then(|j| self.theta.validate(&j, &req.ty));
            if fits.is_none() {
                self.say(&format!("that doesn't fit {}; try again\n> ", req.ty));
                return None;
            }
            Completion::HumanAnswer { json }
        };
        let rid = showing.rid;
        self.showing = None;
        self.show_next();
        Some((rid, completion))
    }

    /// Take a question back: from the screen if it's showing, or from the queue.
    fn withdraw(&mut self, rid: u64) {
        if self.showing.as_ref().is_some_and(|s| s.rid == rid) {
            self.showing = None;
            self.say("\n(withdrawn: the program stopped waiting for this answer)\n");
            self.show_next();
        } else {
            self.waiting.retain(|(r, _)| *r != rid);
        }
    }

    /// Start a host on its own thread, the first time its capability is used.
    fn start_host(&mut self, key: &str) -> Result<&Running, String> {
        if !self.hosts.contains_key(key) {
            let made = self.registry.make(key).ok_or_else(|| {
                if self.registry.is_empty() {
                    format!(
                        "live mode has no tool hosts, so '{}' cannot be called; \
                         `norman --live --python python3` backs kernel grants with Python",
                        key
                    )
                } else {
                    format!("live mode has no host for '{}'; it has {}", key, self.registry.keys().join(", "))
                }
            })?;
            let mut host = made?;
            let interrupt = host.interrupter();
            let (req_tx, req_rx) = channel::<(u64, String, Vec<String>)>();
            let out = self.tx.clone();
            std::thread::spawn(move || {
                // One host, one thread, one call at a time.
                while let Ok((rid, op, args)) = req_rx.recv() {
                    let result = host.call(&op, &args);
                    if out.send(Arrival { rid, at: Instant::now(), answer: Answer::Tool(result) }).is_err() {
                        break; // the run ended
                    }
                }
                host.close();
            });
            self.hosts.insert(key.to_string(), Running { tx: req_tx, interrupt });
        }
        Ok(&self.hosts[key])
    }

    fn interrupt(&self, key: &str) {
        if let Some(stop) = self.hosts.get(key).and_then(|h| h.interrupt.as_ref()) {
            stop();
        }
    }

    fn ms(&self, t: Instant) -> i64 {
        t.saturating_duration_since(self.start).as_millis() as i64
    }

    /// Turn an arrival into a completion, or a checked run-time error for a fatal status.
    fn completion(&self, a: Arrival, pending: Pending) -> Result<Completion, String> {
        match (a.answer, pending) {
            (Answer::Http(Ok((status, body))), Pending::Ask { wrapped, cache_ttl }) => {
                anthropic::parse_response(status, &body, wrapped, cache_ttl)
            }
            (Answer::Http(Err(e)), _) => {
                Ok(Completion::ProviderError { msg: format!("network error: {}", e), usage: Default::default() })
            }
            (Answer::Tool(Ok(text)), _) => Ok(Completion::ToolResult { text }),
            (Answer::Tool(Err(msg)), _) => Ok(Completion::ToolError { msg }),
            (Answer::Http(Ok(_)), _) => {
                Err("internal error: an HTTP reply completed something other than an ask".into())
            }
            (Answer::Line(_) | Answer::ConsoleClosed, _) => {
                Err("internal error: console input reached a request directly".into())
            }
        }
    }
}

/// When the run ends, stop any call still running, so a host's thread can
/// finish and close its process instead of outliving the run.
impl Drop for LiveOracle<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if self.showing.is_some() {
            self.say("\n(withdrawn: the run ended)\n");
        }
        let busy: Vec<String> = self
            .inflight
            .values()
            .filter_map(|p| if let Pending::Call { key } = p { Some(key.clone()) } else { None })
            .collect();
        for key in busy {
            self.interrupt(&key);
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
        self.inflight.insert(rid, Pending::Ask { wrapped, cache_ttl: req.model.cache_ttl });
        let (agent, cfg, tx) = (self.agent.clone(), self.cfg.clone(), self.tx.clone());
        let body = body.to_string();
        std::thread::spawn(move || {
            let result = post(&agent, &cfg, MESSAGES_PATH, &body);
            // The receiver may be gone if the run already ended; that's fine.
            let _ = tx.send(Arrival { rid, at: Instant::now(), answer: Answer::Http(result) });
        });
        Ok(())
    }

    fn issue_call(&mut self, rid: u64, site: &str, args: &[Value], _now: i64) -> Result<(), String> {
        let (key, op) = site.split_once('/').ok_or_else(|| format!("malformed call site '{}'", site))?;
        // Arguments cross a thread boundary, so they go as text: a string
        // verbatim, anything else as `show` would write it.
        let args: Vec<String> = args
            .iter()
            .map(|v| match v {
                Value::Str(s) => Ok(s.to_string()),
                other => self.theta.show(other),
            })
            .collect::<Result<_, _>>()?;
        let op = op.to_string();
        self.start_host(key)?.tx.send((rid, op, args)).map_err(|_| format!("the host for '{}' has stopped", key))?;
        self.inflight.insert(rid, Pending::Call { key: key.to_string() });
        Ok(())
    }

    /// A live host's state is a running process, which can't be copied
    /// faithfully (see `PythonKernel`). Forking in the machine alone would
    /// hand out a second handle to the *same* process, which two workflow
    /// nodes could then share, so it is refused instead.
    fn fork(&mut self, key: &str) -> Result<(), String> {
        Err(format!("'{}' is a live host, and live hosts cannot fork", key))
    }

    /// A consult goes to the person its grant is bound to. `Unattended`, or
    /// no binding at all, is a checked run-time error: a missing person must
    /// never look like a person saying no (design/11, H3).
    fn issue_consult(&mut self, rid: u64, req: ConsultReq, _now: i64) -> Result<(), String> {
        let name = req.human.name.to_string();
        match self.people.bindings.get(&name) {
            Some(Binding::Tty) if self.console_closed => Err(closed(&req.site, &name)),
            Some(Binding::Tty) if self.people.console.is_some() => {
                self.forward_console();
                self.inflight.insert(rid, Pending::Consult);
                self.waiting.push_back((rid, req));
                self.show_next();
                Ok(())
            }
            Some(Binding::Tty) => Err(format!("human {} is bound to the terminal, but this run has none", name)),
            Some(Binding::Unattended) => Err(format!(
                "consult site '{}' needs a person, but human {} is unattended (--human {}=unattended)",
                req.site, name, name
            )),
            None => Err(format!(
                "no person is attached to answer consult site '{}' (human {}); bind one with --human {}=tty",
                req.site, name, name
            )),
        }
    }

    /// Abandon a request. An ask's worker thread finishes on its own and the
    /// reply is discarded. A call's host is interrupted, so whatever it was
    /// running doesn't go on running after the program has moved on. A
    /// consult's question is withdrawn from the console, so nobody goes on
    /// answering a question the program has stopped waiting for.
    fn cancel(&mut self, rid: u64) {
        match self.inflight.remove(&rid) {
            Some(Pending::Call { key }) => self.interrupt(&key),
            Some(Pending::Consult) => self.withdraw(rid),
            _ => {}
        }
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
                            Err(RecvTimeoutError::Disconnected) => {
                                return Err("internal error: the worker channel closed".into());
                            }
                        }
                    }
                }
            };
            if let Answer::ConsoleClosed = arrival.answer {
                // H3: a person who can no longer answer is no person at all,
                // which is a checked run-time error, never a decline.
                self.console_closed = true;
                if let Some(s) = self.showing.as_ref().map(|s| &s.req).or(self.waiting.front().map(|(_, r)| r)) {
                    return Err(closed(&s.site, &s.human.name));
                }
                continue;
            }
            if let Answer::Line(line) = &arrival.answer {
                // A typed line answers the question on screen, if it fits.
                // One typed after the machine's limit is discarded: by then
                // the question has been, or is about to be, withdrawn.
                let t = self.ms(arrival.at);
                if limit.is_some_and(|l| t > l) {
                    continue;
                }
                let Some((rid, c)) = self.on_line(arrival.at, line) else { continue };
                self.inflight.remove(&rid);
                return Ok(Some((t.max(self.ms(Instant::now()).min(limit.unwrap_or(i64::MAX))), vec![(rid, c)])));
            }
            if !self.inflight.contains_key(&arrival.rid) {
                continue; // cancelled earlier: discard
            }
            let t = self.ms(arrival.at);
            if limit.is_some_and(|l| t > l) {
                // It arrived after the deadline. Keep it; the machine will cut first.
                self.held.push(arrival);
                return Ok(None);
            }
            let pending = self.inflight.remove(&arrival.rid).unwrap();
            let rid = arrival.rid;
            let mut batch = vec![(rid, self.completion(arrival, pending)?)];
            // Whatever else has already arrived joins the batch, in issue order.
            while let Ok(more) = self.rx.try_recv() {
                if matches!(more.answer, Answer::Line(_)) {
                    // Lines are matched one at a time, in the order typed.
                    self.held.push(more);
                    continue;
                }
                match self.inflight.contains_key(&more.rid) {
                    true if limit.is_none_or(|l| self.ms(more.at) <= l) => {
                        let p = self.inflight.remove(&more.rid).unwrap();
                        let r = more.rid;
                        batch.push((r, self.completion(more, p)?));
                    }
                    // In flight, but it arrived after the limit: hold it.
                    true => self.held.push(more),
                    // Cancelled earlier: discard it.
                    false => {}
                }
            }
            batch.sort_by_key(|(r, _)| *r);
            let latest = self.ms(Instant::now()).min(limit.unwrap_or(i64::MAX)).max(t);
            return Ok(Some((latest, batch)));
        }
    }
}
