//! CP-Agent, end to end: a real model writing real Python.
//!
//! This is the project's flagship example doing the thing it was written for.
//! It spends real money (a few cents) and runs model-authored code on this
//! machine, so it is `#[ignore]`d and `cargo test` skips it.
//!
//! ```text
//! cargo test --test live_cp_agent -- --ignored --nocapture
//! ```
//!
//! with `ANTHROPIC_API_KEY` set and Python on the PATH. `--nocapture` matters:
//! **the trace is the output.** µNorman records every ask and every call with
//! its cost and timing, so what this prints is a complete, auditable account
//! of what the agent did and what it cost — the artifact the language exists
//! to produce.
//!
//! See LIVE-SETUP.md. Safety: the kernel has no sandbox, and runs whatever the
//! model writes with this user's permissions.

use munorman::ast::Config;
use munorman::driver::Interp;
use munorman::live::LiveConfig;
use munorman::machine::{Outcome, Res};
use munorman::tools::{HostRegistry, PythonKernel, ToolHost};
use munorman::value::{fmt_dur, fmt_money};
use std::path::Path;

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

/// Load `examples/cp-agent.nrm` and run one task against the real world.
fn solve(task: &str) -> Option<Outcome> {
    let live = match LiveConfig::from_env() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("SKIPPED: {msg}\nSee LIVE-SETUP.md.");
            return None;
        }
    };
    let Some(exe) = python() else {
        eprintln!("SKIPPED: no Python on the PATH.");
        return None;
    };

    let mut interp = Interp::new();
    interp.verbose = true;
    // The same file the scripted tests run; only the host differs.
    interp.load_file(Path::new("examples/cp-agent.nrm")).expect("cp-agent.nrm loads");

    let mut hosts = HostRegistry::default();
    hosts.register("py", Box::new(move || PythonKernel::start(exe).map(|k| Box::new(k) as Box<dyn ToolHost>)));

    let src = format!(r#"(cp-agent claude py "{}")"#, task.replace('"', "\\\""));
    let forms = munorman::lexer::read_all(&src, "<task>").expect("the task parses");
    let e = munorman::parser::Parser::new(&interp.theta).exp(&forms[0]).expect("the task parses");

    let outcome = interp
        .run_live_with_hosts(&Config::default(), &e, live, &hosts)
        .expect("no run-time error");
    Some(outcome)
}

/// Print the trace: what the agent did, what it cost, how long it took.
fn report(task: &str, o: &Outcome) {
    println!("\n┌─ {task}");
    for t in &o.trace {
        println!(
            "│  {:<5} {:<6} {:>5} in {:>5} out {:>11} {:>8} → {:<8} {}",
            t.kind,
            t.site,
            t.in_tokens,
            t.out_tokens,
            fmt_money(t.cost),
            fmt_dur(t.start),
            fmt_dur(t.end),
            t.outcome
        );
    }
    let asks = o.trace.iter().filter(|t| t.kind == "ask").count();
    let calls = o.trace.iter().filter(|t| t.kind == "call").count();
    println!("│");
    println!("│  {asks} model calls, {calls} kernel calls");
    println!("│  spent {} of the $0.50 budget in {}", fmt_money(o.spent), fmt_dur(o.elapsed));
    match &o.result {
        Res::Val(v) => println!("└─ answer: {v}"),
        Res::Fail(f) => println!("└─ failed: {f}"),
    }
}

#[test]
#[ignore = "spends real money and runs model-authored Python; run with --ignored"]
fn cp_agent_solves_a_problem_with_a_real_model_and_a_real_kernel() {
    let Some(o) = solve("What is the sum of all multiples of 3 or 5 below 1000?") else { return };
    report("sum of multiples of 3 or 5 below 1000", &o);

    match &o.result {
        Res::Val(v) => {
            // The agent is free to phrase the answer; it must contain the number.
            assert!(v.to_string().contains("233168"), "expected 233168 somewhere in {v}");
        }
        Res::Fail(f) => panic!("the agent failed: {f}"),
    }

    // The point of the language: the account of what happened is complete.
    assert!(o.trace.iter().any(|t| t.kind == "ask"), "it must have asked a model");
    assert!(o.trace.iter().any(|t| t.kind == "call"), "it must have run Python");
    assert!(o.spent > 0 && o.spent <= 500_000, "spent {}", fmt_money(o.spent));
}

/// A second, harder task, to see the loop take more than one turn.
#[test]
#[ignore = "spends real money and runs model-authored Python; run with --ignored"]
fn cp_agent_solves_a_problem_that_needs_more_than_one_step() {
    let Some(o) = solve(
        "A farmer has chickens and cows, 30 heads and 74 legs in total. \
         How many of each? Work it out with code and check your answer.",
    ) else {
        return;
    };
    report("chickens and cows", &o);

    match &o.result {
        Res::Val(v) => {
            let text = v.to_string();
            assert!(text.contains("23") && text.contains('7'), "expected 23 chickens and 7 cows in {text}");
        }
        Res::Fail(f) => panic!("the agent failed: {f}"),
    }
}
