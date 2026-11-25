//! Stage 0 of design/14: the authoring eval's corpus stays valid.
//!
//! Each task in `eval/tasks/` has hidden tests (`tests.nrm`), a solution a
//! person wrote (`reference.nrm`) and a plausible wrong one (`naive.nrm`).
//! The eval grades a model's program by putting it beside the tests as
//! `solution.nrm` and running `tests.nrm`. This file does the same with the
//! two solutions every task ships with, and requires:
//!
//! - the reference passes every hidden test, so a task is solvable as stated;
//! - the naive solution type-checks and runs, but fails at least one hidden
//!   test, so the tests catch the mistake the task is about.
//!
//! No model is involved, and nothing costs money.

use munorman::driver::{Interp, Summary};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const TASKS: &str = "eval/tasks";
const FILES: [&str; 5] = ["task.md", "interface.nrm", "tests.nrm", "reference.nrm", "naive.nrm"];

fn tasks() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(TASKS)
        .expect("eval/tasks exists")
        .map(|e| e.expect("readable entry").path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn name(dir: &Path) -> String {
    dir.file_name().unwrap().to_string_lossy().into_owned()
}

/// Grade `solution` against `dir`'s hidden tests, in a scratch directory laid
/// out exactly as the harness lays it out.
fn grade(dir: &Path, solution: &str) -> Summary {
    // Tests run in parallel and grade the same task, so each grading gets a
    // directory of its own.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let scratch =
        std::env::temp_dir().join(format!("munorman-eval-{}-{}-{}-{}", std::process::id(), n, name(dir), solution));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("scratch directory");
    for f in ["interface.nrm", "tests.nrm"] {
        fs::copy(dir.join(f), scratch.join(f)).expect("copy task file");
    }
    fs::copy(dir.join(format!("{}.nrm", solution)), scratch.join("solution.nrm")).expect("copy solution");
    let summary = Interp::new().load_file(&scratch.join("tests.nrm")).expect("tests.nrm loads");
    let _ = fs::remove_dir_all(&scratch);
    summary
}

#[test]
fn the_corpus_has_twenty_well_formed_tasks() {
    let dirs = tasks();
    assert!(dirs.len() >= 20, "expected at least 20 tasks, found {}", dirs.len());
    for dir in &dirs {
        let n = name(dir);
        for f in FILES {
            assert!(dir.join(f).is_file(), "{}: missing {}", n, f);
        }
        // `A01-word-freq` is titled `# A01 · …`.
        let id = n.split('-').next().unwrap();
        let task = fs::read_to_string(dir.join("task.md")).unwrap();
        assert!(task.starts_with(&format!("# {} ", id)), "{}: task.md should start with \"# {} \"", n, id);
        // The hidden tests load the interface and the solution themselves, so
        // the harness grades by running tests.nrm alone.
        let tests = fs::read_to_string(dir.join("tests.nrm")).unwrap();
        assert!(
            tests.starts_with("(use interface.nrm)\n(use solution.nrm)\n"),
            "{}: tests.nrm should start by using interface.nrm, then solution.nrm",
            n
        );
    }
}

#[test]
fn every_reference_solution_passes_its_hidden_tests() {
    let mut bad = vec![];
    for dir in tasks() {
        let s = grade(&dir, "reference");
        if s.total == 0 || !s.all_passed() || s.errors > 0 {
            bad.push(format!("{}: {} of {} passed, {} other errors", name(&dir), s.passed, s.total, s.errors));
        }
    }
    assert!(bad.is_empty(), "reference solutions that fail:\n{}", bad.join("\n"));
}

#[test]
fn every_naive_solution_runs_but_fails_a_hidden_test() {
    let mut bad = vec![];
    for dir in tasks() {
        let reference = grade(&dir, "reference");
        let naive = grade(&dir, "naive");
        // Every hidden test ran, so the naive solution type-checked: a
        // rejected program runs no tests at all.
        if naive.total != reference.total {
            bad.push(format!(
                "{}: naive ran {} of {} tests (did it type-check?)",
                name(&dir),
                naive.total,
                reference.total
            ));
        } else if naive.all_passed() {
            bad.push(format!(
                "{}: naive passed all {} tests, so they don't catch its mistake",
                name(&dir),
                naive.total
            ));
        }
    }
    assert!(bad.is_empty(), "naive solutions the tests don't separate:\n{}", bad.join("\n"));
}
