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

// ------------------------------------------------------------ live person
//
// The terminal host (design/11 §4.1), driven by a thread that plays the
// person: it waits for a prompt to appear on the console, then types. The
// model endpoint is never contacted, because these programs only consult.

mod person {
    use munorman::ast::Config;
    use munorman::driver::Interp;
    use munorman::live::{Auth, Binding, Console, Line, LiveConfig, People};
    use munorman::machine::Outcome;
    use std::path::Path;
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const PROGRAM: &str = r#"
(grant reviewer (human))
(grant paid     (human [price $2.00]))
(grant nobody   (human))
"#;

    /// Typed as a person's step, this closes the terminal instead (Ctrl-D).
    pub const HANG_UP: &str = "<hang up>";

    pub struct Desk {
        /// The keyboard. `None` once the person hangs up: with no sender
        /// left, the console's input has ended.
        typed: Arc<Mutex<Option<Sender<Line>>>>,
        pub screen: Arc<Mutex<Vec<u8>>>,
        interp: Interp,
    }

    pub fn desk() -> Desk {
        let (typed, lines) = channel();
        let typed = Arc::new(Mutex::new(Some(typed)));
        let screen = Arc::new(Mutex::new(Vec::new()));
        let out: Arc<Mutex<dyn std::io::Write + Send>> = screen.clone();
        let mut interp = Interp::new();
        interp.verbose = false;
        interp.load_str(PROGRAM, "people.nrm", Path::new(".")).expect("program loads");
        interp.live = Some(LiveConfig {
            base_url: "http://127.0.0.1:9".into(),
            auth: Auth::ApiKey("unused".into()),
            timeout: Duration::from_secs(1),
        });
        let mut people = People::default();
        people.bindings.insert("reviewer".into(), Binding::Tty);
        people.bindings.insert("paid".into(), Binding::Tty);
        people.bindings.insert("nobody".into(), Binding::Unattended);
        people.console = Some(Console::new(lines, out));
        interp.people = people;
        Desk { typed, screen, interp }
    }

    impl Desk {
        pub fn screen(&self) -> String {
            String::from_utf8_lossy(&self.screen.lock().unwrap()).to_string()
        }

        /// Type a line now, whether or not anything is being asked.
        pub fn type_line(&self, line: &str) {
            if let Some(tx) = self.typed.lock().unwrap().as_ref() {
                tx.send((Instant::now(), line.to_string())).unwrap();
            }
        }

        /// Play the person: for each (what to wait for, what to type), wait
        /// until it appears on the screen after the last thing waited for.
        pub fn person(&self, steps: &[(&str, &str)]) -> std::thread::JoinHandle<()> {
            let (typed, screen) = (self.typed.clone(), self.screen.clone());
            let steps: Vec<(String, String)> = steps.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
            std::thread::spawn(move || {
                let mut seen = 0;
                for (wait_for, line) in steps {
                    let deadline = Instant::now() + Duration::from_secs(10);
                    loop {
                        let text = String::from_utf8_lossy(&screen.lock().unwrap()).to_string();
                        if let Some(i) = text[seen..].find(&wait_for) {
                            seen += i + wait_for.len();
                            break;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "the person waited 10s for {:?}; screen:\n{}",
                            wait_for,
                            text
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    let mut keyboard = typed.lock().unwrap();
                    if line == HANG_UP {
                        *keyboard = None;
                        return;
                    }
                    match keyboard.as_ref() {
                        Some(tx) if tx.send((Instant::now(), line)).is_ok() => {}
                        _ => return,
                    }
                }
            })
        }

        pub fn run(&self, src: &str, time_ms: i64) -> Result<Outcome, String> {
            let cfg = Config { script: None, cost: Some(5_000_000), time: Some(time_ms) };
            self.interp.eval_source(src, &cfg)
        }
    }
}

use person::desk;

const PROPOSE: &str = r#"(consult reviewer (List Num 3) "Approve these?" (propose (list 1 2 3)) 'review)"#;

#[test]
fn a_person_approves_at_the_terminal_and_gets_back_exactly_the_proposal() {
    let d = desk();
    let p = d.person(&[("> ", "a")]);
    let o = d.run(PROPOSE, 60_000).unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "(list 1 2 3)");
    let screen = d.screen();
    assert!(screen.contains("consult 'review · human reviewer"), "{}", screen);
    assert!(screen.contains("proposal: [1,2,3]"), "{}", screen);
    assert!(screen.contains("answer type: (List Num 3)"), "{}", screen);
    assert_eq!(o.trace.len(), 1);
    assert_eq!(o.trace[0].consult.as_ref().unwrap().said.as_deref(), Some("[1,2,3]"));
}

#[test]
fn a_reply_that_does_not_fit_is_asked_for_again_within_one_consult() {
    let d = desk();
    let p = d.person(&[("> ", "not json"), ("isn't JSON", "[1,2,3,4]"), ("doesn't fit", "[7]")]);
    let o = d.run(PROPOSE, 60_000).unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "(list 7)", "an edit comes back edited");
    assert_eq!(o.trace.len(), 1, "re-asking is the console's form validation, not a second consult");
}

#[test]
fn a_text_answer_can_be_typed_plainly() {
    let d = desk();
    let p = d.person(&[("> ", "drop toxicity")]);
    let o = d.run(r#"(consult reviewer (Text 40) "Which objective should I drop?" 'triage)"#, 60_000).unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "\"drop toxicity\"");
}

#[test]
fn a_person_declines_with_a_reason() {
    let d = desk();
    let p = d.person(&[("> ", "d Too costly.")]);
    let o = d.run(PROPOSE, 60_000).unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "(fail (Declined \"Too costly.\"))");
}

#[test]
fn approving_when_nothing_was_proposed_asks_again() {
    let d = desk();
    let p = d.person(&[("> ", "a"), ("nothing to approve", "5")]);
    let o = d.run(r#"(consult reviewer Num "How many?" 'n)"#, 60_000).unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "5");
}

#[test]
fn a_line_typed_before_the_question_appeared_never_answers_it() {
    // Typed ahead, or meant for an earlier question: if it counted, this
    // consult would be declined. It must be ignored, and the real answer used.
    let d = desk();
    d.type_line("d typed too early");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let p = d.person(&[("> ", "a")]);
    let o = d.run(PROPOSE, 60_000).unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "(list 1 2 3)");
}

#[test]
fn the_deadline_cuts_a_person_off_withdraws_the_question_and_charges_nothing() {
    let d = desk();
    let o = d.run(r#"(consult paid Num "How many?" 'n)"#, 300).unwrap();
    assert_eq!(o.result.to_string(), "(fail PastDeadline)");
    assert_eq!(o.spent, 0, "a cut consult is not charged, even by a priced reviewer");
    assert!(d.screen().contains("withdrawn"), "{}", d.screen());
}

#[test]
fn after_a_question_is_withdrawn_the_next_one_is_shown_and_answered() {
    // The first question is cut by its own short budget. If it stayed on
    // screen, the second would wait behind it forever.
    let d = desk();
    let p = d.person(&[("withdrawn", ""), ("consult 'second", "2")]);
    let o = d
        .run(
            r#"(begin (catch (budget ([time 200ms]) (consult reviewer Num "First?" 'first)) e 0)
                      (consult reviewer Num "Second?" 'second))"#,
            60_000,
        )
        .unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "2");
}

#[test]
fn concurrent_consults_are_shown_one_at_a_time() {
    let d = desk();
    // The first question is on screen before the second consult is even
    // issued, so it can't announce it; the order is what matters.
    let p = d.person(&[("consult 'first", "1"), ("consult 'second", "2")]);
    let o = d
        .run(r#"(par (consult reviewer Num "First?" 'first) (consult reviewer Num "Second?" 'second))"#, 60_000)
        .unwrap();
    p.join().unwrap();
    assert_eq!(o.result.to_string(), "(Pair [fst 1] [snd 2])");
    let screen = d.screen();
    let (a, b) = (screen.find("consult 'first").unwrap(), screen.find("consult 'second").unwrap());
    assert!(a < b, "the second question appears only after the first is answered:\n{}", screen);
}

#[test]
fn fail_fast_withdraws_a_question_and_charges_nothing() {
    let d = desk();
    let o = d.run(r#"(par (consult paid Num "Still there?" 'q) (fail 'boom))"#, 60_000).unwrap();
    assert_eq!(o.result.to_string(), "(fail (Raised 'boom))");
    assert_eq!(o.spent, 0);
    assert!(d.screen().contains("withdrawn"), "{}", d.screen());
}

#[test]
fn a_closed_terminal_stops_the_run_rather_than_hanging_or_declining() {
    // Ctrl-D, or the end of a pipe, while a question is on screen. With no
    // deadline, waiting would never end; and a person who can't answer is no
    // person at all (H3), so this is a checked error, not `Declined`.
    let d = desk();
    let p = d.person(&[("> ", person::HANG_UP)]);
    let err = d.run(&format!("(catch {} e 0)", PROPOSE), i64::MAX / 4).unwrap_err();
    p.join().unwrap();
    assert!(err.contains("input ended"), "{}", err);
    // A later run stops too, rather than waiting on a closed terminal.
    let err = d.run(PROPOSE, i64::MAX / 4).unwrap_err();
    assert!(err.contains("input ended"), "{}", err);
}

#[test]
fn an_unattended_human_stops_the_run_rather_than_declining() {
    let d = desk();
    let err = d.run(r#"(catch (consult nobody Num "Anyone?" 'q) e 0)"#, 60_000).unwrap_err();
    assert!(err.contains("unattended"), "{}", err);
}

// --------------------------------------------------------- start-up check

#[test]
fn the_start_up_check_finds_human_grants_through_use_and_refuses_unbound_ones() {
    let dir = std::env::temp_dir().join(format!("norman-people-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.nrm");
    // `sub` uses `main` back: the reader must not loop.
    std::fs::write(&main, "(use sub.nrm)\n(grant boss (human))\n(println \"main ran\")\n").unwrap();
    std::fs::write(dir.join("sub.nrm"), "(use main.nrm)\n(grant reviewer (human [price $1.00]))\n").unwrap();
    let i = Interp::new();
    let both = ["boss".to_string(), "reviewer".to_string()];

    let err = i.check_people(&[main.as_path()], &[]).unwrap_err();
    assert!(err.contains("boss") && err.contains("reviewer"), "{}", err);
    let err = i.check_people(&[main.as_path()], &both[..1]).unwrap_err();
    assert!(err.contains("bound to reviewer."), "only the unbound grant is named: {}", err);
    assert!(i.check_people(&[main.as_path()], &both).is_ok());
    let typo = [both[0].clone(), both[1].clone(), "reveiwer".to_string()];
    let err = i.check_people(&[main.as_path()], &typo).unwrap_err();
    assert!(err.contains("reveiwer"), "a binding that names no grant is refused: {}", err);
    // The usual mistake is one typo: the real grant unbound AND a stray
    // binding. Both halves must be named, or the typo stays hidden.
    let err = i.check_people(&[main.as_path()], &[both[0].clone(), "reveiwer".to_string()]).unwrap_err();
    assert!(err.contains("bound to reviewer.") && err.contains("names reveiwer"), "{}", err);

    std::fs::remove_dir_all(&dir).ok();
}
