//! What a diagnostic tells the reader (design/13 §1.2, step 3 of its order):
//! where the mistake is, once, under the right label, without citations
//! unless asked, and as JSON when asked. An agent repairs its program from
//! these, so each one is a test.

use munorman::check::Checker;
use munorman::driver::Interp;
use std::path::Path;

fn errors(src: &str) -> Vec<String> {
    let mut c = Checker::new();
    c.check_str(src, "t.nrm", Path::new(".")).errors
}

/// Load `src` and return everything the interpreter reported.
fn transcript(src: &str, setup: impl FnOnce(&mut Interp)) -> String {
    let mut i = Interp::new();
    i.clear_transcript();
    setup(&mut i);
    i.load_str(src, "t.nrm", Path::new(".")).unwrap();
    i.transcript()
}

#[test]
fn a_type_error_points_at_the_expression_not_the_definition() {
    let errs = errors("(define shout (s)\n  (string-append s (+ 1 \"!\")))");
    assert_eq!(errs.len(), 1, "{:?}", errs);
    // (+ 1 "!") starts at line 2, column 20; the definition at 1:1.
    assert!(errs[0].starts_with("t.nrm:2:20: in the definition of shout:"), "{}", errs[0]);

    let errs = errors("(val n (length (list 1 2 \"three\")))");
    assert!(errs[0].starts_with("t.nrm:1:16:"), "the list, not the val: {}", errs[0]);

    // An ill-typed test says where inside it.
    let mut c = Checker::new();
    let r = c.check_str("(check-expect (+ 1 (length \"x\")) 2)", "t.nrm", Path::new("."));
    let e = &r.test_errors[0];
    assert!(e.contains("at t.nrm:1:20,"), "{}", e);
}

#[test]
fn a_definition_that_fails_is_reported_once() {
    // `broken` is used at two different types. Before, each use could
    // disagree with whatever half-solved type the failure left.
    let errs = errors(
        "(define broken (x) (+ x \"one\"))\n(define a () (broken 1))\n(define b () (string-append (broken \"s\") \"!\"))",
    );
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("in the definition of broken"), "{}", errs[0]);
}

#[test]
fn a_syntax_error_is_labelled_as_one() {
    let t = transcript("(define f (x) (case x [1 2 3]))", |_| {});
    assert!(t.contains("syntax error: t.nrm:1:"), "{}", t);
    assert!(!t.contains("type error: syntax error"), "{}", t);
    // The lexer's errors too.
    let t = transcript("(define f (x) x))", |_| {});
    assert!(t.contains("syntax error: t.nrm:1:17: unexpected ')'"), "{}", t);
}

/// The prelude is a file the reader has never seen, so a clash with it says
/// the name is predefined instead of pointing into it.
#[test]
fn redefining_a_predefined_name_says_so() {
    let errs = errors("(define attempt (x) x)");
    assert!(errs[0].contains("attempt is already defined: it's predefined, so give yours another name"), "{}", errs[0]);
    assert!(!errs[0].contains("prelude.nrm"), "{}", errs[0]);
    let errs = errors("(datatype (Option a) [Nothing] [Just (value a)])");
    assert!(errs[0].contains("the type Option is already defined: it's predefined"), "{}", errs[0]);
}

#[test]
fn citations_appear_only_when_asked_for() {
    let src = "(define f (x) x)\n(define f (x) x)";
    let plain = transcript(src, |_| {});
    assert!(plain.contains("f is already defined, at t.nrm:1:1"), "{}", plain);
    assert!(!plain.contains("design/"), "{}", plain);
    let explained = transcript(src, |i| i.explain = true);
    assert!(explained.contains("(a name is defined once: design/12, Q-2)"), "{}", explained);
}

#[test]
fn json_diagnostics_carry_the_location_and_both_types() {
    let t = transcript("(define shout (s)\n  (string-append s (+ 1 \"!\")))", |i| i.json = true);
    let lines: Vec<serde_json::Value> =
        t.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {} ({})", l, e))).collect();
    let d = &lines[0];
    assert_eq!(d["kind"], "type");
    assert_eq!(d["file"], "t.nrm");
    assert_eq!((d["line"].as_u64(), d["column"].as_u64()), (Some(2), Some(20)));
    assert_eq!((d["expected"].as_str(), d["found"].as_str()), (Some("Text"), Some("Num")));
    assert_eq!(lines.last().unwrap()["kind"], "note");

    // A test summary is a JSON object too.
    let t = transcript("(check-expect 1 1)", |i| i.json = true);
    let summary: serde_json::Value = serde_json::from_str(t.lines().last().unwrap()).unwrap();
    assert_eq!(summary, serde_json::json!({"kind": "tests", "file": "t.nrm", "passed": 1, "total": 1}));
}

const ASKS: &str = r#"
(grant claude (model))
(define wrong () (ask claude (Text 10) '() 'summary))
(define twice () (string-append (ask claude (Text 10) '() 'title) (ask claude (Text 10) '() 'title)))
(script s [title (reply "\"a\"") (out 1) (latency 1s)])
(under ([script s] [cost $1] [time 1min])
  (check-expect (wrong) "a")
  (check-expect (twice) "aa"))
"#;

#[test]
fn a_script_says_which_site_it_has_no_entry_for() {
    let t = transcript(ASKS, |_| {});
    // A site the script doesn't have: probably misspelled, so its sites are listed.
    assert!(t.contains("the script has no entry for ask site 'summary'; its sites are 'title"), "{}", t);
    // A site it has, asked once too often.
    assert!(t.contains("script exhausted: ask site 'title' had 1 reply, and all of them have been used"), "{}", t);
}
