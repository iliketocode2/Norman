//! `consult` (design/11) where `.nrm` tests can't reach: the trace's audit
//! record (§2.3), and live mode refusing a consult it has no person for (H3).

use munorman::ast::Config;
use munorman::driver::Interp;
use munorman::live::{Auth, LiveConfig};
use munorman::machine::{ConsultRecord, Outcome, Res};
use std::path::Path;
use std::time::Duration;

const PROGRAM: &str = r#"
(grant reviewer (human [price $2.00]))
(script s
  [ok   (approve)               (latency 1min)]
  [no   (decline "Too costly.") (latency 2min)]
  [late (approve)               (latency 2h)])
"#;

fn interp() -> Interp {
    let mut i = Interp::new();
    i.verbose = false;
    i.load_str(PROGRAM, "consult-trace.nrm", Path::new(".")).expect("program loads");
    i
}

fn run(src: &str) -> Outcome {
    let cfg = Config { script: Some("s".into()), cost: Some(5_000_000), time: Some(3_600_000) };
    interp().eval_source(src, &cfg).expect("runs")
}

fn record(o: &Outcome) -> &ConsultRecord {
    assert_eq!(o.trace.len(), 1, "one trace entry per consult: {:?}", o.trace);
    assert_eq!(o.trace[0].kind, "consult");
    o.trace[0].consult.as_ref().expect("a consult's entry carries its audit record")
}

#[test]
fn an_approval_records_the_prompt_the_proposal_and_what_was_submitted() {
    let o = run(r#"(consult reviewer Num "Approve the count?" (propose 7) 'ok)"#);
    assert!(matches!(&o.result, Res::Val(v) if v.to_string() == "7"), "{}", o.result);
    let r = record(&o);
    assert_eq!(r.prompt, "Approve the count?");
    assert_eq!(r.proposal.as_deref(), Some("7"));
    assert_eq!(r.said.as_deref(), Some("7"), "an unchanged approval submits the proposal's text");
    assert_eq!(o.trace[0].outcome, "ok");
    assert_eq!(o.trace[0].cost, 2_000_000, "a priced reviewer is charged their price");
    assert_eq!((o.trace[0].start, o.trace[0].end), (0, 60_000));
}

#[test]
fn a_decline_records_the_reason() {
    let o = run(r#"(consult reviewer Num "Approve?" 'no)"#);
    assert!(matches!(&o.result, Res::Fail(_)), "{}", o.result);
    let r = record(&o);
    assert_eq!(r.proposal, None, "nothing was proposed");
    assert_eq!(r.said.as_deref(), Some("Too costly."));
    assert_eq!(o.trace[0].outcome, "declined: Too costly.");
    assert_eq!(o.trace[0].cost, 2_000_000);
}

#[test]
fn a_cut_consult_records_that_the_person_said_nothing_and_charges_nothing() {
    let o = run(r#"(consult reviewer Num "Approve?" (propose 7) 'late)"#);
    assert_eq!(o.result.to_string(), "(fail PastDeadline)");
    let r = record(&o);
    assert_eq!(r.proposal.as_deref(), Some("7"), "what was shown is still recorded");
    assert_eq!(r.said, None);
    assert_eq!(o.trace[0].outcome, "past-deadline");
    assert_eq!(o.trace[0].cost, 0);
    assert_eq!(o.spent, 0);
}

#[test]
fn exact_equivalence_compares_the_audit_record() {
    // Same site, cost and timing, different prompts: the person was shown
    // different things, so the traces must not be 'exact-equivalent.
    let a = run(r#"(consult reviewer Num "Approve?" (propose 7) 'ok)"#);
    let b = run(r#"(consult reviewer Num "Approve this instead?" (propose 7) 'ok)"#);
    assert!(!a.trace[0].same(&b.trace[0]));
    let c = run(r#"(consult reviewer Num "Approve?" (propose 7) 'ok)"#);
    assert!(a.trace[0].same(&c.trace[0]));
}

#[test]
fn live_mode_refuses_a_consult_with_no_person_attached() {
    // H3: until a live human host exists, no person is attached, and a
    // missing person must never look like a person saying no. So this is a
    // checked run-time error, not a catchable `Declined`. No request is sent,
    // so the endpoint is never contacted.
    let mut i = interp();
    i.live = Some(LiveConfig {
        base_url: "http://127.0.0.1:9".into(),
        auth: Auth::ApiKey("unused".into()),
        timeout: Duration::from_secs(1),
    });
    let cfg = Config { script: None, cost: Some(5_000_000), time: Some(3_600_000) };
    let err = i.eval_source(r#"(catch (consult reviewer Num "Approve?" 'ok) e 0)"#, &cfg).unwrap_err();
    assert!(err.contains("no person is attached"), "{}", err);
}
