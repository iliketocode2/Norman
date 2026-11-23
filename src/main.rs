//! `norman [--live] [--trace] FILE…`: load each file, run its unit tests, and report.

use munorman::driver::Interp;
use munorman::live::LiveConfig;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str =
    "usage: norman [--live [--python EXE] [--human NAME=tty|unattended]…] [--trace] [--no-check] FILE.nrm …

Type-checks each file, then loads it, printing the value of each top-level
expression, then runs its unit tests (check-expect, check-fail, …). A file
that doesn't type-check is not run.

  --live    answer `ask` with a real model through the Anthropic API
            (needs ANTHROPIC_API_KEY; ANTHROPIC_BASE_URL overrides the endpoint).
            Without it, models answer only from scripts.
  --python EXE
            with --live, back every `kernel` grant with a real Python
            process run by EXE (e.g. python3). WARNING: there is no
            sandbox; model-written code runs with your permissions.
  --human NAME=tty
            with --live, put each `consult` of the human granted as NAME
            to you at this terminal (questions on stderr, answers on stdin).
  --human NAME=unattended
            with --live, declare that nobody is there: a consult of NAME
            stops the run with an error. It is never treated as a refusal.
            A live run refuses to start while any human grant is unbound.
  --trace   after each evaluation, print one line per ask, call or consult:
            site, tokens, cost, timing and outcome
  --no-check
            run without the type checker, as before it existed: type
            mistakes are found when they happen, and check-type-error
            tests fail";

fn main() -> ExitCode {
    let mut files = vec![];
    let mut live = false;
    let mut trace = false;
    let mut check = true;
    let mut python = None;
    let mut people = munorman::live::People::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--live" => live = true,
            "--python" => match args.next() {
                Some(exe) => python = Some(exe),
                None => {
                    eprintln!("--python needs an interpreter, e.g. --python python3\n\n{}", USAGE);
                    return ExitCode::from(2);
                }
            },
            "--human" => {
                let parsed =
                    args.next().ok_or_else(|| "--human needs NAME=tty or NAME=unattended".to_string()).and_then(|b| {
                        let (name, how) = b
                            .split_once('=')
                            .ok_or_else(|| format!("--human {} should be NAME=tty or NAME=unattended", b))?;
                        Ok((name.to_string(), how.parse::<munorman::live::Binding>()?))
                    });
                match parsed {
                    Ok((name, how)) => {
                        people.bindings.insert(name, how);
                    }
                    Err(msg) => {
                        eprintln!("{}\n\n{}", msg, USAGE);
                        return ExitCode::from(2);
                    }
                }
            }
            "--trace" => trace = true,
            "--no-check" => check = false,
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
    if python.is_some() && !live {
        eprintln!("--python only applies with --live; scripted runs answer `call` from scripts\n\n{}", USAGE);
        return ExitCode::from(2);
    }
    if !people.bindings.is_empty() && !live {
        eprintln!("--human only applies with --live; scripted runs answer `consult` from scripts\n\n{}", USAGE);
        return ExitCode::from(2);
    }
    let mut interp = Interp::new();
    interp.trace = trace;
    interp.check = check;
    interp.python = python;
    if live {
        // H3: refuse to start, before anything runs or is paid for, while any
        // human grant is unbound.
        let bound: Vec<String> = people.bindings.keys().cloned().collect();
        let paths: Vec<&Path> = files.iter().map(Path::new).collect();
        if let Err(msg) = interp.check_people(&paths, &bound) {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
        if people.bindings.values().any(|b| *b == munorman::live::Binding::Tty) {
            people.console = Some(munorman::live::Console::stdio());
        }
        interp.people = people;
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
            // A failed test, or any error outside the tests, fails the run,
            // so a script can tell (say) a refused live consult from success.
            Ok(summary) => ok &= summary.clean(),
            Err(msg) => {
                eprintln!("{}", msg);
                ok = false;
            }
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
