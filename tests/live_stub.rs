//! The live oracle (design/09, step 3), tested against a local stub of the
//! Messages API. The stub answers `/v1/messages/count_tokens` with a fixed
//! count and replays canned `/v1/messages` responses (status, body, delay), so
//! the real HTTP path runs without spending money or needing a key.

use munorman::ast::Config;
use munorman::driver::Interp;
use munorman::live::{Auth, LiveConfig};
use munorman::machine::{Outcome, Res};
use munorman::value::Value;
use serde_json::{Value as J, json};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: String,
    body: J,
}

struct Stub {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

type Canned = (u16, String, u64);

/// Start a stub server. `count` answers every count_tokens request;
/// `replies` answer /v1/messages requests in order (then 500s).
fn stub(count: (u16, String), replies: Vec<Canned>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(vec![]));
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    let (seen2, count) = (seen.clone(), Arc::new(count));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (seen, queue, count) = (seen2.clone(), queue.clone(), count.clone());
            std::thread::spawn(move || serve(stream, &seen, &queue, &count));
        }
    });
    Stub { url, seen }
}

fn serve(mut s: TcpStream, seen: &Mutex<Vec<Seen>>, queue: &Mutex<VecDeque<Canned>>, count: &(u16, String)) {
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = s.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let len: usize = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
        .unwrap_or(0);
    while buf.len() < header_end + len {
        let n = s.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
    let body: J = serde_json::from_slice(&buf[header_end..]).unwrap_or(J::Null);
    seen.lock().unwrap().push(Seen { path: path.clone(), headers: head.to_ascii_lowercase(), body });
    let (status, text, delay) = if path.ends_with("/count_tokens") {
        (count.0, count.1.clone(), 0)
    } else {
        queue.lock().unwrap().pop_front().unwrap_or((
            500,
            r#"{"type":"error","error":{"type":"api_error","message":"stub is out of replies"}}"#.into(),
            0,
        ))
    };
    std::thread::sleep(Duration::from_millis(delay));
    let resp = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        text.len(),
        text
    );
    let _ = s.write_all(resp.as_bytes());
}

const DEFS: &str = r#"
(grant sonnet (model))
(datatype Verdict
  [Buy  (reason (Text 400))]
  [Hold (reason (Text 400))]
  [Sell (reason (Text 400))])
(define ask-v ()
  (ask sonnet Verdict (list (Message [role System] [content "You are an analyst."])
                            (Message [role User]   [content "FY2025 10-K"])) 'analyst))
"#;

fn interp(stub: &Stub) -> Interp {
    let mut i = Interp::new();
    i.load_str(DEFS, "defs.nrm", Path::new(".")).unwrap();
    i.live = Some(LiveConfig {
        base_url: stub.url.clone(),
        auth: Auth::ApiKey("test-key".into()),
        timeout: Duration::from_secs(10),
    });
    i
}

fn run(stub: &Stub, src: &str, cost: Option<i64>, time: Option<i64>) -> Result<Outcome, String> {
    interp(stub).eval_source(src, &Config { script: None, cost, time })
}

fn count100() -> (u16, String) {
    (200, r#"{"input_tokens": 100}"#.into())
}

fn answer(stop: &str, text: &str, input: i64, output: i64) -> String {
    json!({
        "content": [{"type": "text", "text": text}],
        "stop_reason": stop,
        "usage": {"input_tokens": input, "output_tokens": output}
    })
    .to_string()
}

fn sell() -> String {
    answer("end_turn", r#"{"value": {"tag": "Sell", "reason": "Margins are compressing."}}"#, 120, 45)
}

fn failure_name(o: &Outcome) -> String {
    match &o.result {
        Res::Fail(Value::Con(k, _)) => k.to_string(),
        other => format!("not a failure: {}", other),
    }
}

// Reservation for these asks: count 100 → ⌈100 × 1.05⌉ + 32 = 137 input tokens,
// max_tokens = 426 + 10 (wrapper) + 2000 (think) = 2436, so 137×2 + 2436×10 = 24 634 µ$.
const RESERVATION: i64 = 24_634;

#[test]
fn an_answer_is_validated_and_charged_from_reported_usage() {
    let s = stub(count100(), vec![(200, sell(), 0)]);
    let o = run(&s, "(ask-v)", Some(1_000_000), None).unwrap();
    assert!(matches!(&o.result, Res::Val(Value::Con(k, _)) if &**k == "Sell"), "{}", o.result);
    assert_eq!(o.spent, 120 * 2 + 45 * 10, "charged from usage, not the reservation");
    assert_eq!(o.trace.len(), 1);
    assert_eq!(o.trace[0].outcome, "ok");

    let seen = s.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "one count, then one message");
    assert_eq!(seen[0].path, "/v1/messages/count_tokens");
    assert_eq!(seen[1].path, "/v1/messages");
    let body = &seen[1].body;
    assert_eq!(body["model"], "claude-sonnet-5");
    assert_eq!(body["max_tokens"], 2436);
    assert_eq!(body["system"], "You are an analyst.");
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert!(seen[0].body.get("max_tokens").is_none());
    assert!(seen[1].headers.contains("x-api-key: test-key"));
    assert!(seen[1].headers.contains("anthropic-version: 2023-06-01"));
}

#[test]
fn a_429_is_a_tool_error_that_retry_handles_and_the_trace_shows() {
    let rate = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#.to_string();
    let s = stub(count100(), vec![(429, rate, 0), (200, sell(), 0)]);
    let o = run(&s, "(retry 1 ask-v)", Some(1_000_000), None).unwrap();
    assert!(matches!(&o.result, Res::Val(_)), "{}", o.result);
    let outcomes: Vec<&str> = o.trace.iter().map(|t| t.outcome.as_str()).collect();
    assert_eq!(outcomes, vec!["error: 429 rate_limit_error: slow down", "ok"]);
}

#[test]
fn a_refusal_is_refused_with_its_category() {
    let body = json!({
        "content": [], "stop_reason": "refusal",
        "stop_details": {"type": "refusal", "category": "cyber", "explanation": "…"},
        "usage": {"input_tokens": 100, "output_tokens": 0}
    });
    let s = stub(count100(), vec![(200, body.to_string(), 0)]);
    let o = run(&s, "(retry 3 ask-v)", Some(1_000_000), None).unwrap();
    assert_eq!(failure_name(&o), "Refused");
    assert_eq!(o.trace.len(), 1, "retry doesn't retry a refusal");
}

#[test]
fn stopping_at_max_tokens_is_invalid() {
    let s = stub(count100(), vec![(200, answer("max_tokens", r#"{"value": {"tag": "Se"#, 100, 2436), 0)]);
    let o = run(&s, "(ask-v)", Some(1_000_000), None).unwrap();
    assert_eq!(failure_name(&o), "Invalid");
    assert_eq!(o.spent, 100 * 2 + 2436 * 10);
}

#[test]
fn a_bad_key_or_model_is_a_run_time_error() {
    let auth = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.to_string();
    let s = stub(count100(), vec![(401, auth.clone(), 0)]);
    let err = run(&s, "(ask-v)", Some(1_000_000), None).unwrap_err();
    assert!(err.contains("401 authentication_error"), "{}", err);
    let s = stub((401, auth), vec![]);
    assert!(run(&s, "(ask-v)", Some(1_000_000), None).unwrap_err().contains("401"));
}

#[test]
fn a_transient_count_failure_is_a_tool_error_and_nothing_is_sent() {
    let s = stub(
        (529, r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#.into()),
        vec![(200, sell(), 0)],
    );
    let o = run(&s, "(ask-v)", Some(1_000_000), None).unwrap();
    assert_eq!(failure_name(&o), "ToolError");
    assert_eq!(o.spent, 0);
    assert_eq!(s.seen.lock().unwrap().len(), 1, "only the count was attempted");
}

#[test]
fn an_unaffordable_ask_is_refused_before_it_is_sent() {
    let s = stub(count100(), vec![(200, sell(), 0)]);
    let o = run(&s, "(ask-v)", Some(RESERVATION - 1), None).unwrap();
    assert_eq!(failure_name(&o), "OverBudget");
    assert_eq!(o.spent, 0);
    assert_eq!(s.seen.lock().unwrap().len(), 1, "counted, but never sent");
    // One micro-dollar more, and it's sent.
    let s = stub(count100(), vec![(200, sell(), 0)]);
    assert!(matches!(run(&s, "(ask-v)", Some(RESERVATION), None).unwrap().result, Res::Val(_)));
}

#[test]
fn a_late_reply_is_cut_at_the_deadline_and_charged_its_reservation() {
    let s = stub(count100(), vec![(200, sell(), 3000)]);
    let o = run(&s, "(ask-v)", Some(1_000_000), Some(300)).unwrap();
    assert_eq!(failure_name(&o), "PastDeadline");
    assert!(o.elapsed >= 300 && o.elapsed < 2000, "stopped waiting at the deadline, took {} ms", o.elapsed);
    assert_eq!(o.spent, RESERVATION);
    assert_eq!(o.trace[0].outcome, "past-deadline");
}

#[test]
fn concurrent_asks_really_run_concurrently() {
    let s = stub(count100(), vec![(200, sell(), 600), (200, sell(), 600)]);
    let o = run(&s, "(par (ask-v) (ask-v))", Some(1_000_000), None).unwrap();
    assert!(matches!(&o.result, Res::Val(Value::Record(r, _)) if &**r == "Pair"), "{}", o.result);
    assert!(o.elapsed >= 600 && o.elapsed < 1150, "two 600 ms asks in parallel took {} ms", o.elapsed);
    assert_eq!(o.spent, 2 * (120 * 2 + 45 * 10));
}
