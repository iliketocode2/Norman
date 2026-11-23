//! Step 9 of Ramsey's process, "revisit tests": run the design's example
//! results and laws through the interpreter.

use munorman::driver::Interp;
use std::path::Path;

fn load(path: &str) -> munorman::driver::Summary {
    let mut interp = Interp::new();
    interp.load_file(Path::new(path)).expect("file loads")
}

#[test]
fn step5_example_results_all_pass() {
    let s = load("examples/step5-examples.nrm");
    assert!(s.total >= 35, "expected at least 35 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} Step 5 tests passed", s.passed, s.total);
}

#[test]
fn step6_laws_all_pass() {
    let s = load("examples/step6-laws.nrm");
    assert!(s.total >= 26, "expected at least 26 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} Step 6 tests passed", s.passed, s.total);
}

#[test]
fn step9_revisited_tests_all_pass() {
    let s = load("examples/step9-revisit.nrm");
    assert!(s.total >= 39, "expected at least 39 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} Step 9 tests passed", s.passed, s.total);
}

#[test]
fn saga_loop_tests_all_pass() {
    let s = load("examples/saga-loop.nrm");
    assert!(s.total >= 15, "expected at least 15 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} SAGA tests passed", s.passed, s.total);
}

#[test]
fn string_and_show_tests_all_pass() {
    let s = load("examples/strings.nrm");
    assert!(s.total >= 40, "expected at least 40 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} string tests passed", s.passed, s.total);
}

#[test]
fn typed_tool_tests_all_pass() {
    let s = load("examples/typed-tools.nrm");
    assert!(s.total >= 18, "expected at least 18 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} typed-tool tests passed", s.passed, s.total);
}

#[test]
fn cp_agent_tests_all_pass() {
    let s = load("examples/cp-agent.nrm");
    assert!(s.total >= 6, "expected at least 6 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} CP-Agent tests passed", s.passed, s.total);
}

#[test]
fn consult_tests_all_pass() {
    let s = load("examples/consult.nrm");
    assert!(s.total >= 52, "expected at least 52 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} consult tests passed", s.passed, s.total);
}

#[test]
fn type_parameter_tests_all_pass() {
    let s = load("examples/type-parameters.nrm");
    assert!(s.total >= 19, "expected at least 19 tests, found {}", s.total);
    assert!(s.clean(), "{} of {} type-parameter tests passed, {} errors", s.passed, s.total, s.errors);
}

#[test]
fn scripted_oracle_tests_all_pass() {
    let s = load("examples/oracle-scripted.nrm");
    assert!(s.total >= 10, "expected at least 10 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} oracle tests passed", s.passed, s.total);
}

/// Load `src` from a fresh file in its own temporary directory, alongside
/// `others` (name, source), and return the summary.
fn load_files(name: &str, src: &str, others: &[(&str, &str)]) -> munorman::driver::Summary {
    let dir = std::env::temp_dir().join(format!("norman-errors-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&dir).unwrap();
    for (n, s) in others {
        std::fs::write(dir.join(n), s).unwrap();
    }
    let path = dir.join(format!("{}.nrm", name));
    std::fs::write(&path, src).unwrap();
    let mut interp = Interp::new();
    interp.verbose = false;
    let s = interp.load_file(&path).expect("file loads");
    std::fs::remove_dir_all(&dir).ok();
    s
}

#[test]
fn errors_outside_the_tests_are_counted() {
    // Every test passes, yet each of these went wrong and must be counted.
    let s = load_files("failing-top-level", "(fail 'boom)\n(check-expect 1 1)", &[]);
    assert_eq!((s.passed, s.total, s.errors), (1, 1, 1));
    assert!(s.all_passed() && !s.clean());

    let s = load_files("stuck-top-level", "(car '())\n(if 1 2 3)", &[]);
    assert_eq!(s.errors, 2, "an unbound name and a non-Boolean test are both errors");

    let s = load_files("syntax", "(lambda)", &[]);
    assert_eq!(s.errors, 1);

    let s = load_files("missing-use", "(use no-such-file.nrm)", &[]);
    assert_eq!(s.errors, 1);

    // A used file's own errors and failed tests count against the file that
    // uses it: otherwise `use` would hide them from the exit code.
    let s = load_files(
        "uses-a-bad-file",
        "(use bad.nrm)\n(check-expect 1 1)",
        &[("bad.nrm", "(fail 'boom)\n(check-expect 1 2)")],
    );
    assert_eq!((s.passed, s.total, s.errors), (1, 1, 2));

    let s = load_files("all-good", "(val x 1)\n(check-expect x 1)", &[]);
    assert!(s.clean());
}

#[test]
fn the_command_fails_when_anything_goes_wrong() {
    let dir = std::env::temp_dir().join(format!("norman-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |name: &str, src: &str| {
        let path = dir.join(name);
        std::fs::write(&path, src).unwrap();
        std::process::Command::new(env!("CARGO_BIN_EXE_norman")).arg(&path).output().unwrap().status.code()
    };
    assert_eq!(run("good.nrm", "(check-expect 1 1)"), Some(0));
    assert_eq!(run("failing-test.nrm", "(check-expect 1 2)"), Some(1));
    // Before this, a top-level error exited 0, so a script couldn't tell a
    // refused live consult from success.
    assert_eq!(run("top-level-error.nrm", "(fail 'boom)\n(check-expect 1 1)"), Some(1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn deliberately_wrong_tests_all_fail() {
    for (file, total) in [("tests/must-fail.nrm", 10), ("tests/must-fail-consult.nrm", 4)] {
        // Each file must check, so each mutant fails for the reason its
        // comment gives, not because a name it uses failed to check.
        let report = munorman::check::Checker::new().check_file(Path::new(file));
        assert!(report.ok(), "{} doesn't type-check: {:?}", file, report.errors);
        let mut interp = Interp::new();
        interp.verbose = false;
        let s = interp.load_file(Path::new(file)).expect("file loads");
        assert_eq!(s.total, total, "{}", file);
        assert_eq!(s.passed, 0, "{}: a deliberately wrong test passed: the suite can't tell right from wrong", file);
        // The examples the mutants use still pass, with no errors.
        assert_eq!(s.errors, 0, "{}: something outside the mutants went wrong", file);
    }
}
