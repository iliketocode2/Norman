//! `norman [--live] [--trace] FILE…`: load each file, run its unit tests, and report.

use munorman::driver::Interp;
use munorman::live::LiveConfig;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: norman [--live] [--trace] FILE.nrm …

Loads each file, printing the value of each top-level expression, then runs
its unit tests (check-expect, check-fail, …).

  --live    answer `ask` with a real model through the Anthropic API
            (needs ANTHROPIC_API_KEY; ANTHROPIC_BASE_URL overrides the endpoint).
            Without it, models answer only from scripts.
  --trace   after each evaluation, print one line per ask or call:
            site, tokens, cost, timing and outcome";

fn main() -> ExitCode {
    let mut files = vec![];
    let mut live = false;
    let mut trace = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--live" => live = true,
            "--trace" => trace = true,
            "-h" | "--help" => {
                eprintln!("{}", USAGE);
                return ExitCode::SUCCESS;
            }
            f if f.starts_with("--") => {
                eprintln!("unknown option {}\n\n{}", f, USAGE);
                return ExitCode::from(2);
            }
            f => files.push(f.to_string()),
        }
    }
    if files.is_empty() {
        eprintln!("{}", USAGE);
        return ExitCode::from(2);
    }
    let mut interp = Interp::new();
    interp.trace = trace;
    if live {
        match LiveConfig::from_env() {
            Ok(cfg) => interp.live = Some(cfg),
            Err(msg) => {
                eprintln!("{}", msg);
                return ExitCode::from(2);
            }
        }
    }
    let mut ok = true;
    for f in &files {
        match interp.load_file(Path::new(f)) {
            Ok(summary) => ok &= summary.all_passed(),
            Err(msg) => {
                eprintln!("{}", msg);
                ok = false;
            }
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
