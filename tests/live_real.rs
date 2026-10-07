//! Step 4 of design/09: **one real call** to the Anthropic API.
//!
//! Every other test in this repository is free and offline. These are not:
//! they send real requests and cost real money (well under a cent each). So
//! they are `#[ignore]`d, and `cargo test` skips them. To run them:
//!
//! ```text
//! cargo test --test live_real -- --ignored --nocapture
//! ```
//!
//! with `ANTHROPIC_API_KEY` set. `--nocapture` matters: what these tests
//! *print* is the point. See LIVE-SETUP.md.
//!
//! What has never been checked against anything but our own stub, and what
//! each test here settles:
//!
//! 1. Whether the API accepts the schema a sum type compiles to: an `anyOf`
//!    of tagged objects, wrapped in `{"value": …}` because the root isn't an
//!    object (`09` §2). `a_sum_type_answer_comes_back_typed`.
//! 2. Whether the 2000-token thinking allowance leaves room for the answer
//!    (`09`, decision A). Same test: a truncated reply fails `Invalid`.
//! 3. Whether the reservation really bounds the bill — the one assumption
//!    Theorem 1 rests on in live mode (`09` §3). Same test, and the margin is
//!    reported as a percentage so a near miss is visible, not just a pass.
//! 4. Whether an unaffordable ask is refused *before* the call, against a real
//!    provider rather than a stub. `an_unaffordable_ask_is_refused_for_free`.

use munorman::ast::Config;
use munorman::driver::Interp;
use munorman::live::LiveConfig;
use munorman::machine::{Outcome, Res};
use munorman::value::Value;
use std::path::Path;

/// The default grant: Claude Sonnet 5 at the prices in `src/defaults.rs`.
///
/// Deliberately *not* the `claude` grant from the Step 5 examples, whose
/// `[in 3] [out 15]` are invented numbers for scripted arithmetic. Against a
/// real model those would make every charge in this file wrong.
const DEFS: &str = r#"
(grant sonnet (model))
(datatype Verdict
  [Buy  (reason (Text 400))]
  [Hold (reason (Text 400))]
  [Sell (reason (Text 400))])
(define analyst-ctx ()
  (list (Message [role System]
                 [content "You are an equity analyst. Give a verdict and a one-sentence reason."])
        (Message [role User]
                 [content "Revenue fell 8% year over year and gross margin fell from 61% to 44%."])))
(define ask-verdict () (ask sonnet Verdict (analyst-ctx) 'analyst))
"#;

/// An interpreter in live mode, or `None` when no credential is set, so the
/// test can skip loudly instead of failing confusingly.
fn live() -> Option<Interp> {
    let cfg = match LiveConfig::from_env() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("SKIPPED: {msg}\nSee LIVE-SETUP.md.");
            return None;
        }
    };
    let mut i = Interp::new();
    i.load_str(DEFS, "live-defs.nrm", Path::new(".")).expect("definitions load");
    i.live = Some(cfg);
    Some(i)
}

fn run(i: &Interp, src: &str, cost: Option<i64>) -> Outcome {
    i.eval_source(src, &Config { script: None, cost, time: Some(120_000) }).expect("no run-time error")
}

fn money(micros: i64) -> String {
    format!("${}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

/// Print the trace, which is the whole point of running these.
fn report(title: &str, o: &Outcome) {
    println!("\n=== {title} ===");
    println!("result:  {}", o.result);
    println!("spent:   {}", money(o.spent));
    println!("elapsed: {} ms", o.elapsed);
    for t in &o.trace {
        println!(
            "  {} {}  in {} tok, out {} tok, cost {}, {}..{} ms, {}",
            t.kind,
            t.site,
            t.in_tokens,
            t.out_tokens,
            money(t.cost),
            t.start,
            t.end,
            t.outcome
        );
    }
}

/// The headline test: one ask, one real answer, and the three things it settles.
#[test]
#[ignore = "sends a real request and spends real money; run with --ignored"]
fn a_sum_type_answer_comes_back_typed_and_within_its_reservation() {
    let Some(i) = live() else { return };

    // A generous ceiling: this test is about whether the call works, not about
    // whether the budget binds. The next test covers refusal.
    let o = run(&i, "(ask-verdict)", Some(1_000_000));
    report("one real ask for a Verdict", &o);

    match &o.result {
        Res::Val(Value::Con(k, fields)) => {
            // (1) and (2): the API accepted our schema and the answer fits the type.
            assert!(matches!(&**k, "Buy" | "Hold" | "Sell"), "a Verdict must be Buy, Hold or Sell, got {k}");
            assert_eq!(fields.len(), 1, "every Verdict constructor has one field");
            let Value::Str(reason) = &fields[0] else {
                panic!("reason must be text, got {}", fields[0]);
            };
            assert!(!reason.is_empty(), "the model returned an empty reason");
            assert!(reason.len() <= 400, "reason is {} bytes, over the type's 400", reason.len());
            println!("verdict: {k}, reason {} bytes", reason.len());
        }
        Res::Fail(phi) => panic!(
            "the ask failed: {phi}\n\
             Invalid means the reply did not fit Verdict, or it hit max_tokens and was \
             truncated, which would mean the thinking allowance is too small (09, decision A). \
             The trace above says which."
        ),
        other => panic!("expected a Verdict, got {other}"),
    }

    // (3) The reservation bounded the bill. This is the assumption Theorem 1
    // rests on in live mode, compared against a real charge for the first time.
    let entry = o.trace.first().expect("one trace entry per ask");
    assert_eq!(entry.outcome, "ok", "trace outcome");
    assert!(
        !entry.outcome.contains("over-reservation"),
        "the provider billed more than we reserved: the count_tokens margin in \
         src/defaults.rs (5% + 32 tokens) is too small"
    );

    // Report the margin, so a near miss is visible rather than silently passing.
    let billed_input = entry.in_tokens;
    println!(
        "input tokens billed: {billed_input}. The reservation added 5% + 32 to our own count; \
         see the margin in src/defaults.rs."
    );
    assert!(o.spent > 0, "a real call must cost something");
    assert!(o.spent < 50_000, "one small ask should not cost {}", money(o.spent));
}

/// (4) A reservation too large for the budget is refused before anything is
/// sent. Scripted mode proves this by construction; this proves the live path
/// takes the same branch, and that a refusal really is free.
#[test]
#[ignore = "contacts the API to count tokens; run with --ignored"]
fn an_unaffordable_ask_is_refused_for_free() {
    let Some(i) = live() else { return };

    // One micro-dollar: nothing is affordable.
    let o = run(&i, "(catch (ask-verdict) e e)", Some(1));
    report("an ask under a 1 µ$ budget", &o);

    match &o.result {
        Res::Val(Value::Con(k, _)) => assert_eq!(&**k, "OverBudget", "expected OverBudget, got {k}"),
        other => panic!("expected the failure OverBudget as a value, got {other}"),
    }
    assert_eq!(o.spent, 0, "a refused ask must cost nothing");
    assert!(o.trace.is_empty(), "a refused ask makes no trace entry: the model was never called");
}

/// Where do the input tokens actually go? The first real call billed 538 input
/// tokens for a context of ~175 characters. This asks `count_tokens` (free) for
/// the same request with and without `output_config`, so the schema's share is
/// measured rather than guessed. See design/09 §3.
#[test]
#[ignore = "contacts the API's free count_tokens endpoint; run with --ignored"]
fn the_schema_is_most_of_the_input() {
    use munorman::anthropic;
    use munorman::ast::Type;
    use munorman::host::AskReq;
    use std::rc::Rc;

    let Some(i) = live() else { return };
    let cfg = i.live.clone().expect("live config");

    let Some(munorman::value::Value::Model(model)) = i.globals.get("sonnet").cloned() else {
        panic!("the `sonnet` grant should be a model");
    };
    let messages: munorman::host::Messages = vec![
        (Rc::from("System"), Rc::from("You are an equity analyst. Give a verdict and a one-sentence reason.")),
        (Rc::from("User"), Rc::from("Revenue fell 8% year over year and gross margin fell from 61% to 44%.")),
    ];
    let req = AskReq {
        site: "analyst".into(),
        model: Rc::clone(&model),
        ty: Type::Named(Rc::from("Verdict")),
        messages,
        max_tokens: 2436,
    };

    let full = anthropic::count_body(&i.theta, &req).expect("count body");
    let mut bare = full.clone();
    bare.as_object_mut().unwrap().remove("output_config");

    let count = |body: &serde_json::Value| -> i64 {
        let resp = ureq::post(&format!("{}/v1/messages/count_tokens", cfg.base_url.trim_end_matches('/')))
            .set("anthropic-version", munorman::anthropic::API_VERSION)
            .set(
                "x-api-key",
                match &cfg.auth {
                    munorman::live::Auth::ApiKey(k) => k,
                    munorman::live::Auth::Bearer(_) => panic!("this diagnostic needs an API key"),
                },
            )
            .send_string(&body.to_string());
        let (status, text) = match resp {
            Ok(r) => (r.status(), r.into_string().unwrap_or_default()),
            Err(ureq::Error::Status(s, r)) => (s, r.into_string().unwrap_or_default()),
            Err(e) => panic!("count_tokens failed: {e}"),
        };
        anthropic::parse_count(status, &text).expect("a count")
    };

    let with_schema = count(&full);
    let without_schema = count(&bare);
    let schema_bytes = full["output_config"]["format"]["schema"].to_string().len();

    println!("\n=== where the input tokens go ===");
    println!("messages only:     {without_schema} tokens");
    println!("messages + schema: {with_schema} tokens");
    println!("the schema costs:  {} tokens ({schema_bytes} bytes of JSON)", with_schema - without_schema);
    println!("the test tokenizer (05, convention 4) would say: {} tokens", munorman::host::test_tokens(&req.messages));

    assert!(with_schema > without_schema, "the schema must cost something");
}
