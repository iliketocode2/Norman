//! Real tool hosts, for live mode.
//!
//! Scripted mode answers a `call` from a script. This module is the other
//! side: a capability backed by something that actually does the work.
//!
//! A host is **stateful and single-threaded by nature** — a Python kernel is
//! one process, and one process does one thing at a time. So each host runs on
//! its own worker thread behind a channel: calls to *different* hosts overlap,
//! calls to the *same* host queue. That is not a limitation to work around, it
//! is the reason `workflow` refuses to let two nodes share a stateful
//! capability in the first place (`04` §6.5).
//!
//! Hosts are created per world, not per program. Every unit test runs in a
//! fresh world with fresh capability state, so each one gets a fresh host —
//! the same rule scripted mode follows when it rewinds a script.

use std::collections::HashMap;

/// Something a capability can do. The result is the operation's answer as
/// text, which the machine then validates against the type the grant declared
/// (`04` §6.5). `Err` is a **tool error**: a failure the program can catch and
/// show to a model, not a reason to stop.
pub trait ToolHost: Send {
    fn call(&mut self, op: &str, args: &[String]) -> Result<String, String>;

    /// Shut down cleanly. Called when the world ends.
    fn close(&mut self) {}
}

/// Makes a host. One is called per world, so each test gets its own state.
pub type HostFactory = Box<dyn Fn() -> Result<Box<dyn ToolHost>, String> + Send + Sync>;

/// The hosts available to live mode, by grant name. A grant with no host here
/// is a checked run-time error when called, which is what happens today for
/// every tool.
#[derive(Default)]
pub struct HostRegistry(HashMap<String, HostFactory>);

impl HostRegistry {
    pub fn register(&mut self, key: &str, factory: HostFactory) {
        self.0.insert(key.to_string(), factory);
    }

    pub fn make(&self, key: &str) -> Option<Result<Box<dyn ToolHost>, String>> {
        self.0.get(key).map(|f| f())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The names a program could call, for an error message worth reading.
    pub fn keys(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.0.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }
}

// --------------------------------------------------------------- Python

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// A persistent Python interpreter: the `kernel` capability, for real.
///
/// The protocol is one JSON object per line in each direction, which avoids
/// every problem with driving `python -i`: no prompts to strip, no echo, no
/// guessing where one result ends. The driver below keeps a single `globals`
/// dict, so state persists between calls exactly as the semantics says a
/// stateful capability's does.
///
/// **What this deliberately does not do.** It does not implement `fork`.
/// Forking a live interpreter means either an OS-level fork, which is
/// Unix-only and fragile with an embedded runtime, or replaying the session's
/// history into a fresh process, which is only correct if everything the
/// program did was deterministic. CP-Agent never forks, so the cost is not
/// worth paying yet; `fork` on this host is a tool error that says so.
///
/// **It runs model-authored code with no sandbox.** The process has whatever
/// access the user running it has. That is a deliberate, stated assumption for
/// a research prototype on a developer's own machine, not an oversight — see
/// `design/09-live-oracle.md`.
pub struct PythonKernel {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// The driver. It reads a request per line, runs it, and answers with one
/// line, so output never has to be parsed out of a prompt.
const DRIVER: &str = r#"
import sys, json, io, traceback, contextlib
g = {"__name__": "__main__"}
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception as e:
        sys.stdout.write(json.dumps({"ok": False, "err": "bad request: %s" % e}) + "\n")
        sys.stdout.flush()
        continue
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = req.get("code", "")
            try:
                # An expression answers with its value, like a REPL; anything
                # else answers with whatever it printed.
                value = eval(compile(code, "<ask>", "eval"), g)
                if value is not None:
                    print(repr(value))
            except SyntaxError:
                exec(compile(code, "<ask>", "exec"), g)
        reply = {"ok": True, "out": out.getvalue()}
    except BaseException:
        lines = traceback.format_exc().strip().splitlines()
        reply = {"ok": False, "err": lines[-1] if lines else "error", "out": out.getvalue()}
    sys.stdout.write(json.dumps(reply) + "\n")
    sys.stdout.flush()
"#;

impl PythonKernel {
    /// Start one. `python` is the interpreter to run, e.g. `python3`.
    pub fn start(python: &str) -> Result<PythonKernel, String> {
        let mut child = Command::new(python)
            .args(["-c", DRIVER])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start {}: {}. Is Python on the PATH?", python, e))?;
        let stdin = child.stdin.take().ok_or("no stdin on the Python process")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("no stdout on the Python process")?);
        Ok(PythonKernel { child, stdin, stdout })
    }

    /// A factory for the registry, so each world gets its own interpreter.
    pub fn factory(python: &'static str) -> HostFactory {
        Box::new(move || PythonKernel::start(python).map(|k| Box::new(k) as Box<dyn ToolHost>))
    }

    fn exec(&mut self, code: &str) -> Result<String, String> {
        let request = serde_json::json!({ "code": code }).to_string();
        writeln!(self.stdin, "{}", request).map_err(|e| format!("the Python kernel has stopped: {}", e))?;
        self.stdin.flush().map_err(|e| format!("the Python kernel has stopped: {}", e))?;

        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).map_err(|e| format!("the Python kernel has stopped: {}", e))?;
        if n == 0 {
            return Err("the Python kernel closed its output".into());
        }
        let reply: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| format!("the Python kernel answered with nonsense: {}", e))?;

        let out = reply.get("out").and_then(|v| v.as_str()).unwrap_or("").trim_end().to_string();
        if reply.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(out)
        } else {
            // A Python error is a *tool error*: the program can catch it and
            // show it to the model, which is exactly what CP-Agent does.
            let err = reply.get("err").and_then(|v| v.as_str()).unwrap_or("error");
            Err(if out.is_empty() { err.to_string() } else { format!("{}\n{}", out, err) })
        }
    }
}

impl ToolHost for PythonKernel {
    fn call(&mut self, op: &str, args: &[String]) -> Result<String, String> {
        match op {
            "exec" => self.exec(args.first().map(String::as_str).unwrap_or("")),
            "fork" => Err("this Python kernel cannot fork; see src/tools.rs".into()),
            other => Err(format!("a Python kernel has no operation {}", other)),
        }
    }

    fn close(&mut self) {
        // Closing stdin ends the driver's loop; kill it if it will not go.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
