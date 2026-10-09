//! The live `call` path, proved with a host made of nothing.
//!
//! Until now `live.rs` refused every `call`, so a tool-using agent could not
//! run against a real model at all. These tests exercise the whole path —
//! starting a host, queueing a call on its thread, delivering the answer
//! through the same channel asks use, and validating the result against the
//! type the grant declared — using a counter rather than a Python process.
//!
//! That is the point: every one of these could fail for a reason that has
//! nothing to do with Python, and finding those failures here is much cheaper
//! than finding them inside a subprocess. They need no key and no network.

use munorman::ast::Config;
use munorman::driver::Interp;
use munorman::live::{Auth, LiveConfig};
use munorman::machine::{Outcome, Res};
use munorman::tools::{HostRegistry, ToolHost};
use munorman::value::Value;
use std::path::Path;
use std::time::Duration;

/// A host with state and no machinery.
#[derive(Default)]
struct Counter {
    n: i64,
}

impl ToolHost for Counter {
    fn call(&mut self, op: &str, args: &[String]) -> Result<String, String> {
        match op {
            "bump" => {
                self.n += args.first().and_then(|a| a.parse::<i64>().ok()).unwrap_or(1);
                Ok(self.n.to_string())
            }
            "echo" => Ok(args.join(" ")),
            "boom" => Err("the host refused".into()),
            other => Err(format!("no operation {}", other)),
        }
    }
}

const DEFS: &str = r#"
(grant counter (kernel [bump Num] [echo (Text 100)] [boom (Text 100)]))
(grant silent  (kernel [tick Num]))
"#;

fn registry() -> HostRegistry {
    let mut r = HostRegistry::default();
    r.register("counter", Box::new(|| Ok(Box::new(Counter::default()) as Box<dyn ToolHost>)));
    r
}

/// Run one expression in live mode. No model is ever asked, so the base URL
/// points nowhere: a request would fail loudly rather than quietly succeed.
fn run(src: &str, reg: &HostRegistry) -> Result<Outcome, String> {
    let mut interp = Interp::new();
    interp.load_str(DEFS, "defs.nrm", Path::new(".")).expect("definitions load");
    let cfg = LiveConfig {
        base_url: "http://127.0.0.1:1".into(),
        auth: Auth::ApiKey("unused".into()),
        timeout: Duration::from_secs(5),
    };
    let e = {
        let forms = munorman::lexer::read_all(src, "<test>")?;
        let [sx] = forms.as_slice() else { return Err("expected one expression".into()) };
        munorman::parser::Parser::new(&interp.theta).exp(sx)?
    };
    interp.run_live_with_hosts(&Config::default(), &e, cfg, reg)
}

fn value(o: &Outcome) -> String {
    match &o.result {
        Res::Val(v) => v.to_string(),
        Res::Fail(f) => format!("fail {}", f),
    }
}

#[test]
fn a_call_reaches_a_real_host_and_comes_back_typed() {
    let r = registry();
    let o = run(r#"(+ (call counter bump 2) 1)"#, &r).expect("no run-time error");
    // The grant declares `bump` answers with Num, so "2" came back as 2.
    assert_eq!(value(&o), "3", "{:?}", o.trace);
    assert_eq!(o.trace.len(), 1, "one trace entry per call");
    assert_eq!(o.trace[0].kind, "call");
    assert_eq!(o.trace[0].site, "counter/bump");
    assert_eq!(o.trace[0].outcome, "ok");
}

#[test]
fn a_hosts_state_persists_between_calls() {
    let r = registry();
    let o = run(r#"(begin (call counter bump 1) (call counter bump 1) (call counter bump 1))"#, &r)
        .expect("no run-time error");
    assert_eq!(value(&o), "3", "the host is one object across the whole run");
}

#[test]
fn each_world_gets_its_own_host() {
    let r = registry();
    // A fresh world means fresh capability state, exactly as a rewound script
    // does in scripted mode. Two separate runs must not see each other.
    for _ in 0..2 {
        let o = run(r#"(call counter bump 5)"#, &r).expect("no run-time error");
        assert_eq!(value(&o), "5", "state leaked between worlds");
    }
}

#[test]
fn a_host_error_is_a_catchable_tool_error() {
    let r = registry();
    let o = run(r#"(catch (call counter boom "x") e (case e [(ToolError m) m] [_ "other"]))"#, &r)
        .expect("no run-time error");
    assert_eq!(value(&o), "\"the host refused\"");
    assert_eq!(o.trace[0].outcome, "error: the host refused");
}

#[test]
fn a_result_that_does_not_fit_the_declared_type_is_invalid() {
    let r = registry();
    // `echo` is declared (Text 100) and answers verbatim, so this is fine…
    let o = run(r#"(call counter echo "hello")"#, &r).expect("no run-time error");
    assert_eq!(value(&o), "\"hello\"");

    // …but `bump` is declared Num, and a host that answered with prose would
    // fail the same way a model's malformed reply does.
    let mut bad = HostRegistry::default();
    struct Prose;
    impl ToolHost for Prose {
        fn call(&mut self, _op: &str, _args: &[String]) -> Result<String, String> {
            Ok("not a number".into())
        }
    }
    bad.register("counter", Box::new(|| Ok(Box::new(Prose) as Box<dyn ToolHost>)));
    let o = run(r#"(catch (call counter bump 1) e (case e [(Invalid raw) raw] [_ "other"]))"#, &bad)
        .expect("no run-time error");
    assert_eq!(value(&o), "\"not a number\"");
}

#[test]
fn an_operation_the_grant_never_declared_is_a_run_time_error() {
    let r = registry();
    // The host would have answered; the grant is what refuses.
    let err = run(r#"(call counter echo "x" )"#, &r).map(|o| value(&o));
    assert!(err.is_ok(), "echo is declared");
    let err = run(r#"(call counter undeclared "x")"#, &r).unwrap_err();
    assert!(err.contains("does not offer"), "{err}");
}

#[test]
fn a_capability_with_no_host_says_so_clearly() {
    let r = registry();
    let err = run(r#"(call silent tick 1)"#, &r).unwrap_err();
    assert!(err.contains("no host for 'silent'"), "{err}");
    assert!(err.contains("counter"), "it should say which hosts exist: {err}");
}

#[test]
fn calls_to_different_hosts_overlap_while_one_host_serializes() {
    // Two hosts that each sleep; run concurrently they take about as long as
    // one. This is the property that makes a stateful host safe to own: calls
    // to it queue, but it never blocks anything else.
    struct Slow;
    impl ToolHost for Slow {
        fn call(&mut self, _op: &str, _args: &[String]) -> Result<String, String> {
            std::thread::sleep(Duration::from_millis(300));
            Ok("1".into())
        }
    }
    let mut r = HostRegistry::default();
    r.register("counter", Box::new(|| Ok(Box::new(Slow) as Box<dyn ToolHost>)));
    r.register("other", Box::new(|| Ok(Box::new(Slow) as Box<dyn ToolHost>)));

    let mut interp = Interp::new();
    interp
        .load_str("(grant counter (kernel [bump Num]))\n(grant other (kernel [bump Num]))", "defs.nrm", Path::new("."))
        .expect("definitions load");
    let cfg = LiveConfig {
        base_url: "http://127.0.0.1:1".into(),
        auth: Auth::ApiKey("unused".into()),
        timeout: Duration::from_secs(5),
    };
    let src = "(workflow ([a (call counter bump 1)] [b (call other bump 1)]) (+ a b))";
    let forms = munorman::lexer::read_all(src, "<test>").unwrap();
    let e = munorman::parser::Parser::new(&interp.theta).exp(&forms[0]).unwrap();
    let started = std::time::Instant::now();
    let o = interp.run_live_with_hosts(&Config::default(), &e, cfg, &r).expect("no run-time error");
    let wall = started.elapsed();

    assert!(matches!(&o.result, Res::Val(Value::Num(2))), "{}", value(&o));
    assert!(wall < Duration::from_millis(550), "two hosts should overlap, took {:?}", wall);
}

// ------------------------------------------------------------- Python

use munorman::tools::PythonKernel;

/// The interpreter to drive. `python` on Windows, `python3` elsewhere is the
/// usual split; try both so the test runs wherever Python is installed.
fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|exe| {
        std::process::Command::new(exe)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn python_registry(exe: &'static str) -> HostRegistry {
    let mut r = HostRegistry::default();
    r.register("counter", Box::new(move || PythonKernel::start(exe).map(|k| Box::new(k) as Box<dyn ToolHost>)));
    r
}

/// The host alone, with no interpreter around it.
#[test]
fn a_python_kernel_runs_code_and_keeps_its_state() {
    let Some(exe) = python() else {
        eprintln!("SKIPPED: no Python on the PATH");
        return;
    };
    let mut k = PythonKernel::start(exe).expect("python starts");

    // An expression answers with its value, like a REPL.
    assert_eq!(k.call("exec", &["6 * 7".into()]).unwrap(), "42");
    // A statement answers with what it printed.
    assert_eq!(k.call("exec", &["print('hello')".into()]).unwrap(), "hello");
    // State persists, which is the whole point of a stateful capability.
    k.call("exec", &["x = 10".into()]).unwrap();
    assert_eq!(k.call("exec", &["x + 5".into()]).unwrap(), "15");
    // Imports persist too.
    k.call("exec", &["import math".into()]).unwrap();
    assert_eq!(k.call("exec", &["math.floor(3.7)".into()]).unwrap(), "3");
    // Multi-line code works.
    k.call("exec", &["def double(n):\n    return n * 2".into()]).unwrap();
    assert_eq!(k.call("exec", &["double(21)".into()]).unwrap(), "42");

    // An error is a tool error, carrying the message a model needs to fix it.
    let err = k.call("exec", &["undefined_name".into()]).unwrap_err();
    assert!(err.contains("NameError"), "{err}");
    // And the kernel survives it.
    assert_eq!(k.call("exec", &["x".into()]).unwrap(), "10");

    // `fork` says plainly that it is not implemented.
    assert!(k.call("fork", &[]).unwrap_err().contains("cannot fork"));
}

/// The kernel through the language: a µNorman program driving real Python.
#[test]
fn munorman_can_drive_a_real_python_kernel() {
    let Some(exe) = python() else {
        eprintln!("SKIPPED: no Python on the PATH");
        return;
    };
    let r = python_registry(exe);
    let mut interp = Interp::new();
    interp
        .load_str("(grant counter (kernel [exec (Text 4000)]))", "defs.nrm", Path::new("."))
        .expect("definitions load");
    let cfg = LiveConfig {
        base_url: "http://127.0.0.1:1".into(),
        auth: Auth::ApiKey("unused".into()),
        timeout: Duration::from_secs(10),
    };
    let src = r#"(begin (call counter exec "answer = 6 * 7")
                        (call counter exec "print(answer)"))"#;
    let forms = munorman::lexer::read_all(src, "<test>").unwrap();
    let e = munorman::parser::Parser::new(&interp.theta).exp(&forms[0]).unwrap();
    let o = interp.run_live_with_hosts(&Config::default(), &e, cfg, &r).expect("no run-time error");

    assert_eq!(value(&o), "\"42\"", "state carried from one call to the next");
    assert_eq!(o.trace.len(), 2, "one trace entry per call");
    assert!(o.trace.iter().all(|t| t.kind == "call" && t.outcome == "ok"));
}
