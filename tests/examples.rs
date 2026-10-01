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
fn scripted_oracle_tests_all_pass() {
    let s = load("examples/oracle-scripted.nrm");
    assert!(s.total >= 10, "expected at least 10 tests, found {}", s.total);
    assert!(s.all_passed(), "{} of {} oracle tests passed", s.passed, s.total);
}

#[test]
fn deliberately_wrong_tests_all_fail() {
    let mut interp = Interp::new();
    interp.verbose = false;
    let s = interp.load_file(Path::new("tests/must-fail.nrm")).expect("file loads");
    assert_eq!(s.total, 8);
    assert_eq!(s.passed, 0, "a deliberately wrong test passed: the suite can't tell right from wrong");
}
