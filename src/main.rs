//! `norman FILE…`: load each file, run its unit tests, and report.

use munorman::driver::Interp;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() || files.iter().any(|f| f == "-h" || f == "--help") {
        eprintln!("usage: norman FILE.nrm …\n\nLoads each file, then runs its unit tests (check-expect, check-fail, …).");
        return ExitCode::from(2);
    }
    let mut interp = Interp::new();
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
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
