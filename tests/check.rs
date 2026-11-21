//! The type and effect checker (design/12), steps 1 and 2: inference for
//! every form except `ask`, `call` and `consult`, which come with grants'
//! name-indexed types in step 3.

use munorman::check::{Checker, Report};
use std::path::Path;

/// Check `src` after the prelude, and return the checker and its report.
fn check(src: &str) -> (Checker, Report) {
    let mut c = Checker::new();
    let r = c.check_str(src, "test.nrm", Path::new("."));
    (c, r)
}

/// Check `src`, insist it's accepted, and return the scheme of each name.
fn schemes(src: &str, names: &[&str]) -> Vec<String> {
    let (mut c, r) = check(src);
    assert!(r.ok(), "rejected: {:?}", r.errors);
    names.iter().map(|n| c.scheme_of(n).unwrap_or_else(|| panic!("no scheme for {}", n))).collect()
}

/// Check `src`, insist it's rejected, and return the first error.
fn rejected(src: &str) -> String {
    let (_, r) = check(src);
    assert!(!r.ok(), "accepted, but should have been rejected:\n{}", src);
    r.errors[0].clone()
}

// ------------------------------------------------------------------ the prelude

/// The basis's inferred types, which design/12 predicts. Effects say which
/// thunks a function runs: `retry` has the effect of the thunk it retries.
#[test]
fn the_prelude_checks_and_its_types_are_as_designed() {
    let mut c = Checker::new();
    let expected = [
        ("not", "(Bool -> Bool)"),
        ("<=", "Ord a => (a a -> Bool)"),
        (">=", "Ord a => (a a -> Bool)"),
        ("append", "((List a) (List a) -> (List a))"),
        ("length", "((List a) -> Num)"),
        ("filter", "((a -> Bool ! e1) (List a) -> (List a) ! e1)"),
        ("retry", "(Num (-> a ! e1) -> a ! e1)"),
        ("correction", "(Text -> (List Message))"),
        ("repair", "(Num ((List Message) -> a ! e1) (List Message) -> a ! e1)"),
        ("attempt", "((-> a ! e1) (a -> Bool ! e2) -> (Option a) ! e1 e2)"),
        ("first-some", "((Option a) (Option a) -> (Option a))"),
        ("best-of*", "(Num (-> a ! e1) (a -> Bool ! e2) -> (Option a) ! e1 e2)"),
        ("best-of", "(Num (-> a ! e1) (a -> Bool ! e2) -> a ! e1 e2)"),
        ("system?", "(Message -> Bool)"),
        ("drop", "(Num (List a) -> (List a))"),
        ("last-n", "(Num (List a) -> (List a))"),
        ("window", "(Num (List Message) -> (List Message))"),
        // Primitives, with their constraints.
        ("+", "Arith a => (a a -> a)"),
        ("*", "Mul a b c => (a b -> c)"),
        ("show", "Showable a => (a -> Text)"),
        ("remaining", "(-> Resources ! observe)"),
    ];
    for (name, want) in expected {
        assert_eq!(c.scheme_of(name).unwrap(), want, "the type of {}", name);
    }
}

// ------------------------------------------------------------------ effects

/// An effect that reaches a function's type only through a variable not in
/// it must survive generalization. This was a bug in the first version:
/// `best-of` and anything calling `attempt` lost their effects entirely,
/// which would have made effect soundness (Theorem 8) false.
#[test]
fn effects_survive_generalization_through_intermediate_variables() {
    let got = schemes(
        r#"
(define calls-attempt (f check) (attempt f check))
(define calls-it-twice (f check) (calls-attempt f check))
(define rec-attempt (k f check)
  (if (<= k 0) None (first-some (attempt f check) (rec-attempt (- k 1) f check))))
"#,
        &["calls-attempt", "calls-it-twice", "rec-attempt"],
    );
    assert_eq!(got[0], "((-> a ! e1) (a -> Bool ! e2) -> (Option a) ! e1 e2)");
    assert_eq!(got[1], "((-> a ! e1) (a -> Bool ! e2) -> (Option a) ! e1 e2)");
    assert_eq!(got[2], "(Num (-> a ! e1) (a -> Bool ! e2) -> (Option a) ! e1 e2)");
}

#[test]
fn effects_flow_through_let_par_workflow_and_recursion() {
    let got = schemes(
        r#"
(define via-let (f) (let* ([p (f)]) p))
(define via-par (f) (. (par (f) (f)) fst))
(define via-node (f) (workflow ([x (f)]) x))
(define via-body (f) (workflow ([x 1]) (f)))
(define via-rec (k f) (if (<= k 0) 0 (begin (f) (via-rec (- k 1) f))))
(define via-catch (f g) (catch (f) e (g)))
"#,
        &["via-let", "via-par", "via-node", "via-body", "via-rec", "via-catch"],
    );
    for g in &got[..4] {
        assert_eq!(g, "((-> a ! e1) -> a ! e1)");
    }
    assert_eq!(got[4], "(Num (-> a ! e1) -> Num ! e1)");
    assert_eq!(got[5], "((-> a ! e1) (-> a ! e2) -> a ! e1 e2)", "a handler's effect counts too");
}

/// `observe` is what law W1's premise (ii) is about: whoever reads
/// `(remaining)`, directly or through a function, has it in their type.
#[test]
fn reading_remaining_is_an_effect_that_propagates() {
    let got = schemes(
        r#"
(define budget-left () (. (remaining) cost))
(define spent (thunk)
  (let* ([c0 (. (remaining) cost)] [_ (catch (thunk) e e)] [c1 (. (remaining) cost)]) (- c0 c1)))
(define pure-thing (x) (+ x 1))
(define calls-budget-left (x) (if (> (budget-left) $1.00) x 0))
"#,
        &["budget-left", "spent", "pure-thing", "calls-budget-left"],
    );
    assert_eq!(got[0], "(-> Money ! observe)");
    assert_eq!(got[1], "((-> Failure ! e1) -> Money ! observe e1)");
    assert_eq!(got[2], "(Num -> Num)", "no effect, and + defaulted to Num");
    assert_eq!(got[3], "(Num -> Num ! observe)");
}

// ------------------------------------------------------------------ rejection

#[test]
fn ill_typed_programs_are_rejected_before_they_run() {
    for (src, says) in [
        ("(define f (x) (if x 1 \"one\"))", "the two branches of an if differ"),
        ("(define f (x) (if 1 x x))", "in the test of an if"),
        ("(define f () (+ 1 \"two\"))", "expected"),
        ("(define f () (+ #t #f))", "+ and - works on Num, Money, Dur, not on Bool"),
        ("(define f () (< #t #f))", "< and > works on"),
        ("(define f () (* $1.00 $2.00))", "* can't take Money and Money"),
        ("(define f () (show (lambda (x) x)))", "a function can't be shown"),
        ("(define f () (undefined-thing 1))", "unbound name undefined-thing"),
        ("(define f () (fail 3))", "fail needs a Failure"),
        ("(define f () (not 1 2))", "argument"),
        ("(define f (x) (x x))", "contain itself"),
        ("(define f () (. (Some 1) value))", "Option is not a record"),
        ("(define f () (. (Message [role User] [content \"x\"]) colour))", "record Message has no field colour"),
        ("(define f (m) (. m colour))", "no record has a field colour"),
        ("(define f () (budget ([cost 5]) 1))", "a budget's cost"),
        ("(define f () (Some 1 2))", "expects 1 argument"),
        ("(define f (x) (case x [(Some v) v] ['() 0]))", "pattern"),
        ("(define f () (catch 1 e \"no\"))", "a catch's handler differs"),
    ] {
        let err = rejected(src);
        assert!(err.contains(says), "for {}\nexpected an error mentioning {:?}, got: {}", src, says, err);
    }
}

#[test]
fn any_has_no_static_meaning() {
    let err = rejected("(datatype Box [B (x Any)])\n(define f () (B 1))");
    assert!(err.contains("Any has no static meaning"), "{}", err);
}

#[test]
fn ask_call_and_consult_wait_for_step_3() {
    let err = rejected("(grant m (model))\n(define f () (ask m Num (list) 'q))");
    assert!(err.contains("step 3"), "{}", err);
}

// ------------------------------------------------------------------ the top level

/// Late binding still works for functions: a function may call one defined
/// below it (design/12, T7).
#[test]
fn functions_may_call_functions_defined_below_them() {
    let got = schemes(
        r#"
(define even? (n) (if (= n 0) #t (odd? (- n 1))))
(define odd? (n) (if (= n 0) #f (even? (- n 1))))
(define early (x) (late x))
(define late (x) (+ x 1))
"#,
        &["even?", "odd?", "early"],
    );
    assert_eq!(got, ["(Num -> Bool)", "(Num -> Bool)", "(Num -> Num)"]);
}

/// But a form that runs while the file loads may not (§4.4): it would type
/// check and then find the name unbound.
#[test]
fn a_form_that_runs_at_load_time_may_not_depend_on_a_later_definition() {
    let err = rejected("(val x (f 1))\n(define f (n) (+ n 1))");
    assert!(err.contains("defined below it"), "{}", err);
    // Transitively, through the body of a function it calls.
    let err = rejected("(define g (n) (f n))\n(val x (g 1))\n(define f (n) (+ n 1))");
    assert!(err.contains("needs f"), "{}", err);
    let err = rejected("(println (f 1))\n(define f (n) n)");
    assert!(err.contains("defined below it"), "{}", err);
    // A val bound to a lambda runs nothing, so it may refer ahead.
    assert!(check("(val g (lambda (n) (f n)))\n(define f (n) n)").1.ok());
    // And in the right order, all is well.
    assert!(check("(define f (n) (+ n 1))\n(val x (f 1))").1.ok());
}

#[test]
fn a_name_is_defined_once() {
    let err = rejected("(define f (x) x)\n(define f (x) (+ x 1))");
    assert!(err.contains("already defined") && err.contains("Q-2"), "{}", err);
    let err = rejected("(define retry (x) x)");
    assert!(err.contains("already defined"), "a prelude function can't be redefined either: {}", err);
    let err = rejected("(define cons (x) x)");
    assert!(err.contains("primitive"), "{}", err);
}

// ------------------------------------------------------------------ polymorphism

#[test]
fn let_bound_functions_are_polymorphic() {
    let got = schemes(
        r#"
(define pair-of-ids ()
  (let* ([id (lambda (x) x)]) (Pair [fst (id 1)] [snd (id "one")])))
"#,
        &["pair-of-ids"],
    );
    assert_eq!(got[0], "(-> (Pair Num Text))");
}

/// Overloaded operators stay generic through definitions, so the prelude's
/// `<=` works on money (design/12, T4), and default to Num only when nothing
/// decides them.
#[test]
fn overloading_stays_generic_and_defaults_per_group() {
    let got = schemes(
        r#"
(define cheap? (c) (<= c $0.10))
(define twice (x) (+ x x))
(define double-price (c) (* c 2))
(define half-time (t) (/ t 2))
(define nothing-decides () (+ 1 2))
"#,
        &["cheap?", "twice", "double-price", "half-time", "nothing-decides"],
    );
    assert_eq!(got[0], "(Money -> Bool)");
    assert_eq!(got[1], "Arith a => (a -> a)");
    assert_eq!(got[2], "Mul a Num b => (a -> b)");
    assert_eq!(got[3], "Div a Num b => (a -> b)");
    assert_eq!(got[4], "(-> Num)");
    // And using a generic one at Money is fine; at Bool it isn't.
    assert!(check("(define twice (x) (+ x x))\n(define m () (twice $1.00))").1.ok());
    let err = rejected("(define twice (x) (+ x x))\n(define m () (twice #t))");
    assert!(err.contains("not on Bool"), "{}", err);
}

/// A field only one record has decides the record. A field several records
/// share leaves a qualified type, resolved where the function is used. The
/// whole file is checked at once (T7), so `Labeled`, defined below `xof`,
/// still counts.
#[test]
fn field_selection_finds_its_record() {
    let src = r#"
(record Point (x Num) (y Num))
(define xof (p) (. p x))
(define yof (p) (. p y))
(record Labeled (name Text) (x Text))
(define name-of (p) (. p name))
"#;
    let got = schemes(src, &["xof", "yof", "name-of"]);
    assert_eq!(got[0], "Has a x b => (a -> b)", "Point and Labeled both have x");
    assert_eq!(got[1], "(Point -> Num)");
    assert_eq!(got[2], "(Labeled -> Text)");
    // Used at each record, it has each field's type.
    let uses = format!(
        "{}\n(define both () (Pair [fst (+ 1 (xof (Point [x 1] [y 2])))] [snd (xof (Labeled [name \"n\"] [x \"t\"]))]))",
        src
    );
    assert_eq!(schemes(&uses, &["both"])[0], "(-> (Pair Num Text))");
    let err = rejected(&format!("{}\n(define bad () (xof 5))", src));
    assert!(err.contains("Num is not a record"), "{}", err);
}

/// Only when nothing can ever decide the record is a shared field an error.
#[test]
fn a_shared_field_nothing_decides_is_an_error() {
    let err = rejected("(record A (n Num))\n(record B (n Text))\n(define k () (. (fail 'x) n))");
    assert!(err.contains("can't tell which record's field n"), "{}", err);
}

// ------------------------------------------------------------------ type parameters

#[test]
fn parameterized_types_are_inferred() {
    let got = schemes(
        r#"
(datatype (Outcome e a) [Got (value a)] [Unsure (why e)])
(define value-or (o d) (case o [(Got v) v] [(Unsure _) d]))
(define swap (p) (Pair [fst (. p snd)] [snd (. p fst)]))
(define maybe-first (xs) (case xs ['() None] [(cons x _) (Some x)]))
"#,
        &["value-or", "swap", "maybe-first"],
    );
    assert_eq!(got[0], "((Outcome a b) b -> b)");
    assert_eq!(got[1], "((Pair a b) -> (Pair b a))");
    assert_eq!(got[2], "((List a) -> (Option a))");
    let err = rejected("(define bad () (case (Some 1) [(Some v) (+ v \"x\")] [None 0]))");
    assert!(err.contains("expected"), "{}", err);
}

// ------------------------------------------------------------------ tests

/// A unit test's expressions are checked on their own: an ill-typed test
/// fails by itself, without rejecting the file (design/12, T8).
#[test]
fn an_ill_typed_test_fails_alone() {
    let (_, r) = check(
        r#"
(define f (x) (+ x 1))
(check-expect (f 1) 2)
(check-expect (f 1) "two")
(check-assert (f 1))
(check-fail (fail 'x) (Raised _))
"#,
    );
    assert!(r.ok(), "the file is fine: {:?}", r.errors);
    assert_eq!(r.test_errors.len(), 2, "{:?}", r.test_errors);
}
