//! Property-based tests of the laws in design/06 (Seven Lessons, Lesson 2:
//! "Properties are used for testing. Substitute a permissible value for each
//! variable in the property, and check that equality holds.")
//!
//! The laws quantify over scripted worlds (06 §0), so a random script is a
//! random test case. Each case below generates a script, a budget and the
//! law's other variables from a fixed seed, writes them as a µNorman source
//! file, and runs the law as `check-equiv` or `check-expect`. A failing case
//! prints its seed and its source, which is a complete reproduction: save it
//! as a `.nrm` file and run it with `cargo run`.
//!
//! Theorem 1′ is checked inside the machine on every case (see
//! `budget_invariant` in src/machine.rs); a violation is a run-time error,
//! which fails the case.

use munorman::driver::Interp;
use std::fmt::Write;
use std::path::Path;

/// Cases per law. Fixed seeds keep every run identical.
const CASES: u64 = 150;

/// SplitMix64: small, fast, and good enough to pick test cases.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A number in `lo..=hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// Definitions every case shares: one model, the analyst's ask, two
/// workflow steps, and the analyst's `Verdict` type.
const PREAMBLE: &str = r#"
(grant claude (model [in 3] [out 15] [ceiling 8000]))
(datatype Verdict [Buy (reason (Text 400))] [Hold (reason (Text 400))] [Sell (reason (Text 400))])
(define ask-analyst () (ask claude Verdict (list (Message [role User] [content "FY2025 10-K"])) 'analyst))
(define ask-a () (ask claude (Text 20) (list (Message [role User] [content "Do step a"])) 'a))
(define ask-b () (ask claude (Text 20) (list (Message [role User] [content "Do step b"])) 'b))
(define accept-all (v) #t)
(define buy? (v) (case v [(Buy _) #t] [_ #f]))
(define swap (p) (Pair [fst (. p snd)] [snd (. p fst)]))
(define seq-ab () (let* ([x (ask-a)] [y (ask-b)]) (list x y)))
(define par-ab () (workflow ([x (ask-a)] [y (ask-b)]) (list x y)))
(define elapsed (thunk)
  (let* ([t0 (. (remaining) time)] [_ (thunk)] [t1 (. (remaining) time)]) (- t0 t1)))
"#;

/// Entries per site: more than any law here consumes in one world.
const ENTRIES: usize = 20;

/// One entry for the analyst's site: usually a verdict, sometimes garbage, an
/// outage or a refusal. Latencies are whole seconds, so ties happen.
fn analyst_entry(r: &mut Rng) -> String {
    let lat = r.range(0, 8);
    match r.range(0, 9) {
        0..=4 => {
            let tag = ["Buy", "Hold", "Sell"][r.range(0, 2) as usize];
            format!(
                r#"(reply "{{\"tag\":\"{}\",\"reason\":\"r{}\"}}") (out {}) (latency {}s)"#,
                tag,
                r.range(0, 99),
                r.range(5, 40),
                lat
            )
        }
        5 | 6 => format!(r#"(reply "not json") (out {}) (latency {}s)"#, r.range(1, 10), lat),
        7 | 8 => format!(r#"(provider-error "503 Service Unavailable") (latency {}s)"#, lat),
        _ => format!(r#"(refusal "cyber") (out {}) (latency {}s)"#, r.range(1, 5), lat),
    }
}

/// One entry for a workflow step: a short string, or garbage. Also returns
/// the entry's latency in seconds and whether it's a valid answer.
fn step_entry(r: &mut Rng, name: &str) -> (String, i64, bool) {
    let lat = r.range(0, 30);
    if r.chance(80) {
        (format!(r#"(reply "\"{}\"") (out 2) (latency {}s)"#, name, lat), lat, true)
    } else {
        (format!(r#"(reply "garbage") (out 2) (latency {}s)"#, lat), lat, false)
    }
}

/// The first entries of sites `a` and `b`: (latency in seconds, valid?).
/// A law that runs `ask-a` and `ask-b` once each sees exactly these.
#[derive(Clone, Copy)]
struct Firsts {
    a: (i64, bool),
    b: (i64, bool),
}

fn script(r: &mut Rng) -> (String, Firsts) {
    let mut s = String::from("(script s\n  [analyst");
    for _ in 0..ENTRIES {
        write!(s, "\n    {}", analyst_entry(r)).unwrap();
    }
    s.push_str("]\n");
    let mut firsts = vec![];
    for (site, name) in [("a", "A"), ("b", "B")] {
        write!(s, "  [{}", site).unwrap();
        for i in 0..ENTRIES {
            let (e, lat, ok) = step_entry(r, name);
            if i == 0 {
                firsts.push((lat, ok));
            }
            write!(s, "\n    {}", e).unwrap();
        }
        s.push_str("]\n");
    }
    s.push_str(")\n");
    (s, Firsts { a: firsts[0], b: firsts[1] })
}

/// A money literal for `micros` micro-dollars.
fn money(micros: i64) -> String {
    format!("${}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

/// Limits for `under`. Budgets range from "refuses one analyst ask" (about
/// $0.0066 reserved) to plenty; times from 1 s to 2 min.
fn limits(r: &mut Rng) -> String {
    format!("[cost {}] [time {}s]", money(r.range(1_000, 60_000)), r.range(1, 120))
}

/// Run one generated case; on failure, report the seed and the source.
/// Failing checks print their messages when `verbose` is set.
fn run_case(law: &str, seed: u64, src: &str, verbose: bool) -> Result<(), String> {
    let mut interp = Interp::new();
    interp.verbose = verbose;
    let summary = interp
        .load_str(src, &format!("{}-{}.nrm", law, seed), Path::new("."))
        .map_err(|e| format!("{} (seed {}): {}\n{}", law, seed, e, src))?;
    if summary.total == 0 || !summary.all_passed() {
        return Err(format!(
            "law {} fails for seed {} ({} of {} checks passed). Reproduce with this file:\n{}",
            law, seed, summary.passed, summary.total, src
        ));
    }
    Ok(())
}

/// The source of case `seed` of `law`: the preamble, a fresh script, and
/// the checks under their limits.
fn case_source(law: &str, seed: u64, case: &impl Fn(&mut Rng, Firsts) -> (String, String)) -> String {
    let mut r = Rng::new(seed ^ fnv(law));
    let (script, firsts) = script(&mut r);
    let (limits, checks) = case(&mut r, firsts);
    format!("{}{}(under ([script s] {})\n{})\n", PREAMBLE, script, limits, checks)
}

/// Check `law` on CASES generated cases. `case` returns the `under` limits
/// and the checks.
fn property(law: &str, case: impl Fn(&mut Rng, Firsts) -> (String, String)) {
    for seed in 0..CASES {
        if let Err(msg) = run_case(law, seed, &case_source(law, seed, &case), true) {
            panic!("{}", msg);
        }
    }
}

/// The opposite of `property`: a false law must fail on some case. This is
/// the must-fail suite's idea applied here: it shows the generator reaches
/// the counterexamples, so a passing `property` means something.
fn non_law(law: &str, case: impl Fn(&mut Rng, Firsts) -> (String, String)) {
    let found = (0..CASES).any(|seed| run_case(law, seed, &case_source(law, seed, &case), false).is_err());
    assert!(found, "no counterexample to the non-law {} in {} cases: the generator is too weak", law, CASES);
}

/// A stable hash of the law's name, so each law sees different cases.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// An expression that asks the analyst, possibly through the predefined
/// functions, so the laws are tested on more than a bare `ask`.
fn analyst_exp(r: &mut Rng) -> String {
    match r.range(0, 4) {
        0 => "(ask-analyst)".into(),
        1 => format!("(retry {} ask-analyst)", r.range(0, 3)),
        2 => format!("(best-of {} ask-analyst buy?)", r.range(0, 3)),
        3 => "(catch (ask-analyst) e (Hold \"fallback\"))".into(),
        _ => format!(
            "(repair {} (lambda (ctx) (ask claude Verdict ctx 'analyst)) (list (Message [role User] [content \"FY2025 10-K\"])))",
            r.range(0, 2)
        ),
    }
}

// ------------------------------------------------------------------ catch

#[test]
fn law_c3_rethrow_is_the_identity() {
    property("C3", |r, _| {
        let e = analyst_exp(r);
        (limits(r), format!("(check-equiv (catch {} x (fail x)) {} 'exact)", e, e))
    });
}

// ----------------------------------------------------------------- budget

#[test]
fn law_b1_nested_budgets_take_the_minimum() {
    property("B1", |r, _| {
        let (c1, c2) = (r.range(1_000, 60_000), r.range(1_000, 60_000));
        let (t1, t2) = (r.range(1, 60), r.range(1, 60));
        let e = analyst_exp(r);
        let lhs = format!(
            "(budget ([cost {}] [time {}s]) (budget ([cost {}] [time {}s]) {}))",
            money(c1),
            t1,
            money(c2),
            t2,
            e
        );
        let rhs = format!("(budget ([cost {}] [time {}s]) {})", money(c1.min(c2)), t1.min(t2), e);
        (limits(r), format!("(check-equiv {} {} 'exact)", lhs, rhs))
    });
}

#[test]
fn law_b4_a_scope_never_spends_more_than_its_limit() {
    property("B4", |r, _| {
        let c = r.range(1_000, 30_000);
        let e = analyst_exp(r);
        let checks = format!("(check-within (catch (budget ([cost {}]) {}) e 0) ([cost {}]))", money(c), e, money(c));
        (limits(r), checks)
    });
}

#[test]
fn law_b5_an_expired_scope_never_calls_the_oracle() {
    property("B5", |r, _| {
        // B5 is about an `ask`: e's first effect must be one, and e must not
        // catch PastDeadline (so not best-of 0, and not the catch fallback).
        let e = match r.range(0, 2) {
            0 => "(ask-analyst)".to_string(),
            1 => format!("(retry {} ask-analyst)", r.range(0, 3)),
            _ => format!("(best-of {} ask-analyst buy?)", r.range(1, 3)),
        };
        let checks = format!(
            "(check-equiv (budget ([time 0s]) {}) (fail PastDeadline) 'exact)\n\
             (check-within (catch (budget ([time 0s]) {}) e 0) ([cost $0] [time 0s]))",
            e, e
        );
        (limits(r), checks)
    });
}

// ------------------------------------------------------------------ retry

#[test]
fn law_retry_nested_retries_multiply_for_natural_counts() {
    property("retry-nested", |r, _| {
        let (m, n) = (r.range(0, 3), r.range(0, 3));
        let checks = format!(
            "(check-equiv (retry {} (lambda () (retry {} ask-analyst))) (retry {} ask-analyst) 'exact)",
            m,
            n,
            (m + 1) * (n + 1) - 1
        );
        (limits(r), checks)
    });
}

#[test]
fn law_retry_never_retries_budget_or_deadline_failures() {
    property("retry-budget", |r, _| {
        let n = r.range(0, 5);
        let checks = format!(
            "(check-equiv (retry {n} (lambda () (fail OverBudget))) (fail OverBudget) 'exact)\n\
             (check-equiv (retry {n} (lambda () (fail PastDeadline))) (fail PastDeadline) 'exact)\n\
             (check-equiv (budget ([cost $0.000001]) (retry {n} ask-analyst)) (fail OverBudget) 'exact)"
        );
        (limits(r), checks)
    });
}

// --------------------------------------------------------------- workflow

/// W1 needs premise (iii): no limit binds. Without it the law is false; see
/// `w1_is_false_when_a_deadline_binds` in examples/step6-laws.nrm.
#[test]
fn law_w1_workflow_sequentializes_when_no_limit_binds() {
    property("W1", |_, _| ("".into(), "(check-equiv (seq-ab) (par-ab) 'value)".into()));
}

#[test]
fn law_w2_par_commutes_up_to_swap() {
    property("W2", |r, _| {
        // ≡$ in general; ≡v when both branches may fail at the same instant.
        // Value grade is checked always, resource grade with no limits.
        let checks = "(check-equiv (par (ask-a) (ask-b)) (swap (par (ask-b) (ask-a))) 'value)";
        let lim = if r.chance(50) { limits(r) } else { "[time 100min]".into() };
        (lim, checks.into())
    });
}

#[test]
fn law_w5_parallelizing_is_never_slower_when_no_limit_binds() {
    property("W5", |_, f| {
        // Both sides see the first replies of a and b, so their times follow
        // from the script. Sequential: a, then b only if a succeeded.
        // Dataflow: both at once; fail-fast stops at the first failure.
        let ((la, oka), (lb, okb)) = (f.a, f.b);
        let seq = if oka { la + lb } else { la };
        let par = match (oka, okb) {
            (true, true) => la.max(lb),
            (false, true) => la,
            (true, false) => lb,
            (false, false) => la.min(lb),
        };
        assert!(par <= seq, "the law's arithmetic: {} > {}", par, seq);
        let checks = format!(
            "(check-equiv (seq-ab) (par-ab) 'value)
             (check-expect (elapsed (lambda () (catch (seq-ab) e 0))) {seq}s)
             (check-expect (elapsed (lambda () (catch (par-ab) e 0))) {par}s)"
        );
        ("[time 100min]".into(), checks)
    });
}

// ------------------------------------------------------------- best-of

#[test]
fn law_best_of_work_and_span() {
    // On success, best-of k spends the sum of its attempts and takes the
    // longest. Here: best-of k equals running k attempts as one workflow.
    property("best-of", |r, _| {
        let k = r.range(1, 3);
        let nodes: Vec<String> = (0..k).map(|i| format!("[x{} (attempt ask-analyst accept-all)]", i)).collect();
        let names: Vec<String> = (0..k).map(|i| format!("x{}", i)).collect();
        // first-some, folded from the right, exactly as best-of* does.
        let mut pick = "None".to_string();
        for n in names.iter().rev() {
            pick = format!("(first-some {} {})", n, pick);
        }
        let rhs = format!("(case (workflow ({}) {}) [(Some x) x] [None (fail 'none-accepted)])", nodes.join(" "), pick);
        let checks = format!("(check-equiv (best-of {} ask-analyst accept-all) {} 'value)", k, rhs);
        ("[time 100min]".into(), checks)
    });
}

// ------------------------------------------------------ context policies

/// A random context of System and User messages, as a µNorman literal, with
/// the number of each kind.
fn context(r: &mut Rng) -> (String, i64, i64) {
    let len = r.range(0, 7);
    let (mut sys, mut other) = (0, 0);
    let mut ms = vec![];
    for i in 0..len {
        if r.chance(30) {
            sys += 1;
            ms.push(format!("(Message [role System] [content \"s{}\"])", i));
        } else {
            other += 1;
            let role = ["User", "Assistant", "Tool"][r.range(0, 2) as usize];
            ms.push(format!("(Message [role {}] [content \"m{}\"])", role, i));
        }
    }
    (format!("(list {})", ms.join(" ")), sys, other)
}

#[test]
fn law_window_keeps_every_system_message_and_at_most_n_others() {
    property("window", |r, _| {
        let (ctx, sys, other) = context(r);
        let n = r.range(0, 6);
        let checks = format!(
            "(check-expect (filter system? (window {n} {ctx})) (filter system? {ctx}))\n\
             (check-expect (length (window {n} {ctx})) {})\n\
             (check-expect (window {n} {ctx})\n\
                           (append (filter system? {ctx}) (last-n {n} (filter (lambda (m) (not (system? m))) {ctx}))))",
            sys + other.min(n)
        );
        ("".into(), checks)
    });
}

#[test]
fn law_window_moves_system_messages_first_when_nothing_is_dropped() {
    property("window-reorders", |r, _| {
        let (ctx, _, other) = context(r);
        let n = other + r.range(0, 2); // nothing is dropped
        let checks = format!(
            "(check-expect (window {n} {ctx})\n\
                           (append (filter system? {ctx}) (filter (lambda (m) (not (system? m))) {ctx})))"
        );
        ("".into(), checks)
    });
}

#[test]
fn law_drop_and_filter() {
    property("drop-filter", |r, _| {
        let (xs, _, _) = context(r);
        let (ys, _, _) = context(r);
        let k = r.range(-2, 8);
        let checks = format!(
            "(check-expect (length (drop {k} {xs})) (let* ([d (- (length {xs}) {k})]) (if (< d 0) 0 (if (> d (length {xs})) (length {xs}) d))))\n\
             (check-expect (append (filter system? {xs}) (filter system? {ys})) (filter system? (append {xs} {ys})))\n\
             (check-expect (drop {k} (append {xs} {ys}))\n\
                           (if (<= {k} (length {xs})) (append (drop {k} {xs}) {ys}) (drop (- {k} (length {xs})) {ys})))"
        );
        ("".into(), checks)
    });
}

// ------------------------------------------------- false laws are caught

#[test]
fn non_law_the_old_window_law() {
    // 06 once stated (window n ctx) == ctx when (length ctx) ≤ n. False when a
    // System message follows another message: window moves it to the front.
    non_law("old-window", |r, _| {
        let (ctx, sys, other) = context(r);
        let n = sys + other + r.range(0, 2);
        ("".into(), format!("(check-expect (window {n} {ctx}) {ctx})"))
    });
}

#[test]
fn non_law_w1_when_limits_bind() {
    // Without premise (iii), the sequentialization can miss a deadline the
    // workflow meets.
    non_law("W1-limits", |r, _| (limits(r), "(check-equiv (seq-ab) (par-ab) 'value)".into()));
}

#[test]
fn non_law_nested_retry_with_negative_counts() {
    // Without the side condition m, n ≥ 0: (retry -2 (λ () (retry 1 f)))
    // makes up to 2 calls; (retry -3 f) makes 1.
    non_law("retry-negative", |r, _| {
        let (m, n) = (r.range(-3, -1), r.range(1, 3));
        let checks = format!(
            "(check-equiv (retry {} (lambda () (retry {} ask-analyst))) (retry {} ask-analyst) 'exact)",
            m,
            n,
            (m + 1) * (n + 1) - 1
        );
        ("[cost $1.00] [time 10min]".into(), checks)
    });
}
