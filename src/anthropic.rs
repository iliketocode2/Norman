//! The Messages API wire format (design/09 §1–4): pure translations between
//! µNorman's view of an `ask` and the JSON the Anthropic API sends and
//! receives. Nothing here touches the network; the live client (step 3)
//! sends what `request_body` builds and hands what comes back to
//! `parse_response`.
//!
//! There is no official Anthropic SDK for Rust, so this is raw HTTP:
//! `POST /v1/messages` and `POST /v1/messages/count_tokens`, with headers
//! `x-api-key` and `anthropic-version: 2023-06-01`.

use crate::host::{AskReq, Completion, Usage};
use crate::types::TypeEnv;
use serde_json::{Value as J, json};

pub const API_VERSION: &str = "2023-06-01";
pub const MESSAGES_PATH: &str = "/v1/messages";
pub const COUNT_TOKENS_PATH: &str = "/v1/messages/count_tokens";

/// Messages as the API wants them: a system prompt and alternating turns.
#[derive(Debug, PartialEq)]
pub struct Conversation {
    pub system: Option<String>,
    /// (role, text), where role is "user" or "assistant". Adjacent turns never share a role.
    pub turns: Vec<(&'static str, String)>,
}

/// Map µNorman messages to API turns (09 §1):
/// - `System` messages go to the top-level `system` field, joined in order.
/// - `User` messages are user turns; `Tool` messages are user turns marked as tool output.
/// - `Assistant` messages are assistant turns.
/// - Adjacent turns with the same API role are merged.
///
/// A context must contain a turn and must end with a user turn. Current models
/// reject assistant prefill, and a prefill isn't a question anyway.
pub fn conversation(req: &AskReq) -> Result<Conversation, String> {
    let mut system: Vec<String> = vec![];
    let mut turns: Vec<(&'static str, String)> = vec![];
    for (role, content) in &req.messages {
        let (api_role, text) = match &**role {
            "System" => {
                system.push(content.to_string());
                continue;
            }
            "User" => ("user", content.to_string()),
            "Tool" => ("user", format!("Tool output:\n{}", content)),
            "Assistant" => ("assistant", content.to_string()),
            other => return Err(format!("unknown message role {}", other)),
        };
        match turns.last_mut() {
            Some((last, prev)) if *last == api_role => {
                prev.push_str("\n\n");
                prev.push_str(&text);
            }
            _ => turns.push((api_role, text)),
        }
    }
    match turns.last() {
        None => Err(format!("the ask at site '{}' has no user message to send", req.site)),
        Some(("assistant", _)) => Err(format!(
            "the context of the ask at site '{}' ends with an Assistant message; a live model needs the last message to be from the User or a Tool",
            req.site
        )),
        Some(_) => Ok(Conversation { system: if system.is_empty() { None } else { Some(system.join("\n\n")) }, turns }),
    }
}

/// The schema to send for `ty`, and whether it wraps the answer as `{"value": …}`.
pub fn output_schema(theta: &TypeEnv, ty: &crate::ast::Type) -> Result<(J, bool), String> {
    let s = theta.json_schema(ty)?;
    if theta.root_is_object(ty) {
        Ok((s, false))
    } else {
        Ok((
            json!({
                "type": "object",
                "properties": {"value": s},
                "required": ["value"],
                "additionalProperties": false,
            }),
            true,
        ))
    }
}

/// The body shared by `/v1/messages` and `/v1/messages/count_tokens`:
/// everything that determines the input. Thinking is left at the model's
/// default; the grant's `think` allowance budgets for it (decision A).
fn common_body(theta: &TypeEnv, req: &AskReq) -> Result<(serde_json::Map<String, J>, bool), String> {
    let conv = conversation(req)?;
    let (schema, wrapped) = output_schema(theta, &req.ty)?;
    let mut body = serde_json::Map::new();
    body.insert("model".into(), json!(req.model.id));
    if let Some(s) = conv.system {
        body.insert("system".into(), json!(s));
    }
    let messages: Vec<J> = conv.turns.into_iter().map(|(role, text)| json!({"role": role, "content": text})).collect();
    body.insert("messages".into(), J::Array(messages));
    body.insert("output_config".into(), json!({"format": {"type": "json_schema", "schema": schema}}));
    Ok((body, wrapped))
}

/// The `/v1/messages` request for an ask, and whether its answer is wrapped.
///
/// A grant with `[cache …]` asks for prompt caching (design/09 §7a), with two
/// breakpoints: automatic caching, a top-level `cache_control` that the API
/// places on the last block and moves forward as a conversation grows, and an
/// explicit one on the system prompt, so asks that share a system prompt and
/// answer type but differ after it share that prefix. Both have the grant's
/// lifetime, which keeps them in the order the API requires.
pub fn request_body(theta: &TypeEnv, req: &AskReq) -> Result<(J, bool), String> {
    let (mut body, wrapped) = common_body(theta, req)?;
    body.insert("max_tokens".into(), json!(req.max_tokens));
    if let Some(ttl) = req.model.cache_ttl {
        let marker =
            if ttl > 300_000 { json!({"type": "ephemeral", "ttl": "1h"}) } else { json!({"type": "ephemeral"}) };
        if let Some(J::String(system)) = body.remove("system") {
            body.insert("system".into(), json!([{"type": "text", "text": system, "cache_control": marker.clone()}]));
        }
        body.insert("cache_control".into(), marker);
    }
    Ok((J::Object(body), wrapped))
}

/// The `/v1/messages/count_tokens` request: the same input, without `max_tokens`.
pub fn count_body(theta: &TypeEnv, req: &AskReq) -> Result<J, String> {
    Ok(J::Object(common_body(theta, req)?.0))
}

/// Read `{"input_tokens": n}`.
pub fn parse_count(status: u16, body: &str) -> Result<i64, Failure> {
    if !(200..300).contains(&status) {
        return Err(classify_error(status, body));
    }
    let j: J = serde_json::from_str(body)
        .map_err(|e| Failure::Fatal(format!("count_tokens returned malformed JSON: {}", e)))?;
    j.get("input_tokens")
        .and_then(J::as_i64)
        .ok_or_else(|| Failure::Fatal(format!("count_tokens response has no input_tokens: {}", body)))
}

/// How a request can go wrong before there's an answer to validate.
#[derive(Debug, PartialEq)]
pub enum Failure {
    /// 429, 5xx, and other transient failures: the program sees `ToolError` (decision C).
    Transient(String),
    /// 400, 401, 403, 404, 413: a bad key, model ID or request. A checked run-time error.
    Fatal(String),
}

/// Map an HTTP error status and body to a `Failure` (09 §4).
pub fn classify_error(status: u16, body: &str) -> Failure {
    let parsed: Option<J> = serde_json::from_str(body).ok();
    let kind = parsed.as_ref().and_then(|j| j.pointer("/error/type")).and_then(J::as_str).unwrap_or("error");
    let message = parsed.as_ref().and_then(|j| j.pointer("/error/message")).and_then(J::as_str).unwrap_or(body);
    let msg = format!("{} {}: {}", status, kind, message);
    match status {
        400 | 401 | 403 | 404 | 413 => Failure::Fatal(msg),
        _ => Failure::Transient(msg),
    }
}

/// Map a `/v1/messages` response to a completion (09 §4). `wrapped` says
/// whether the answer is `{"value": …}` to be unwrapped before `validate`;
/// `cache_ttl` is the grant's cache lifetime, which prices cache writes when
/// the response doesn't break them down by lifetime.
/// `Err` is a checked run-time error: a request the program or its grants got wrong.
pub fn parse_response(status: u16, body: &str, wrapped: bool, cache_ttl: Option<i64>) -> Result<Completion, String> {
    let none = Usage::default();
    if !(200..300).contains(&status) {
        return match classify_error(status, body) {
            Failure::Transient(msg) => Ok(Completion::ProviderError { msg, usage: none }),
            Failure::Fatal(msg) => Err(msg),
        };
    }
    let j: J = match serde_json::from_str(body) {
        Ok(j) => j,
        Err(e) => return Ok(Completion::ProviderError { msg: format!("malformed response: {}", e), usage: none }),
    };
    let usage = parse_usage(&j, cache_ttl);
    // The answer is the text blocks; thinking blocks are skipped.
    let text: String = j
        .get("content")
        .and_then(J::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(J::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(J::as_str))
                .collect()
        })
        .unwrap_or_default();
    let stop = j.get("stop_reason").and_then(J::as_str).unwrap_or("");
    Ok(match stop {
        "end_turn" | "stop_sequence" => {
            Completion::Answer { json: unwrap_answer(&text, wrapped), usage, truncated: false }
        }
        "max_tokens" => Completion::Answer { json: text, usage, truncated: true },
        "refusal" => {
            let category = j.pointer("/stop_details/category").and_then(J::as_str).unwrap_or("unspecified").to_string();
            Completion::Refusal { category, usage }
        }
        other => Completion::ProviderError { msg: format!("unexpected stop_reason {:?}", other), usage },
    })
}

/// A response's `usage`, split the way it is priced. `input_tokens` is only
/// the uncached remainder; cache writes come broken down by lifetime under
/// `cache_creation` when the API gives that, and otherwise all at the grant's.
fn parse_usage(j: &J, cache_ttl: Option<i64>) -> Usage {
    let n = |path: &str| j.pointer(path).and_then(J::as_i64);
    let written = n("/usage/cache_creation_input_tokens").unwrap_or(0);
    let (w5, w1) = match (
        n("/usage/cache_creation/ephemeral_5m_input_tokens"),
        n("/usage/cache_creation/ephemeral_1h_input_tokens"),
    ) {
        (None, None) if cache_ttl.is_some_and(|t| t > 300_000) => (0, written),
        (None, None) => (written, 0),
        (a, b) => (a.unwrap_or(0), b.unwrap_or(0)),
    };
    Usage {
        input: n("/usage/input_tokens").unwrap_or(0),
        cache_write_5m: w5,
        cache_write_1h: w1,
        cache_read: n("/usage/cache_read_input_tokens").unwrap_or(0),
        output: n("/usage/output_tokens").unwrap_or(0),
    }
}

/// `{"value": x}` → the JSON text of `x`. Anything else is returned as-is,
/// so `validate` reports it as `Invalid`.
fn unwrap_answer(text: &str, wrapped: bool) -> String {
    if !wrapped {
        return text.to_string();
    }
    match serde_json::from_str::<J>(text) {
        Ok(J::Object(mut o)) if o.len() == 1 && o.contains_key("value") => o.remove("value").unwrap().to_string(),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ConDef, Type};
    use crate::value::ModelSpec;
    use std::rc::Rc;

    fn theta() -> TypeEnv {
        let mut t = TypeEnv::default();
        let text400 = Type::Text(Some(400));
        let verdict = ["Buy", "Hold", "Sell"]
            .iter()
            .map(|k| ConDef { name: Rc::from(*k), fields: vec![(Rc::from("reason"), text400.clone())] })
            .collect();
        t.add_datatype(Rc::from("Verdict"), vec![], verdict).unwrap();
        t.add_record(Rc::from("Point"), vec![], vec![(Rc::from("x"), Type::Num), (Rc::from("y"), Type::Num)]).unwrap();
        let tree = vec![
            ConDef { name: Rc::from("Leaf"), fields: vec![] },
            ConDef {
                name: Rc::from("Node"),
                fields: vec![(Rc::from("left"), Type::named("Tree")), (Rc::from("right"), Type::named("Tree"))],
            },
        ];
        t.add_datatype(Rc::from("Tree"), vec![], tree).unwrap();
        t
    }

    /// A type whose answers are tiny but whose schema is large: twelve
    /// constructors, each carrying one one-byte field.
    fn wide(t: &mut TypeEnv) {
        let cons = ["Aa", "Bb", "Cc", "Dd", "Ee", "Ff", "Gg", "Hh", "Ii", "Jj", "Kk", "Ll"]
            .iter()
            .map(|k| ConDef { name: Rc::from(*k), fields: vec![(Rc::from("v"), Type::Text(Some(1)))] })
            .collect();
        t.add_datatype(Rc::from("Wide"), vec![], cons).unwrap();
    }

    fn req(messages: &[(&str, &str)], ty: Type) -> AskReq {
        AskReq {
            site: "analyst".into(),
            model: Rc::new(ModelSpec {
                name: Rc::from("m"),
                id: "claude-sonnet-5".into(),
                think: 2000,
                in_nano: 2000,
                out_nano: 10_000,
                read_nano: 200,
                cache_ttl: None,
                ceiling: 128_000,
            }),
            ty,
            messages: messages.iter().map(|(r, c)| (Rc::from(*r), Rc::from(*c))).collect(),
            max_tokens: 2436,
            schema: Rc::from(""),
        }
    }

    fn cached(mut r: AskReq, ttl: i64) -> AskReq {
        let mut m = ModelSpec { ..(*r.model).clone() };
        m.cache_ttl = Some(ttl);
        r.model = Rc::new(m);
        r
    }

    // ---------------------------------------------------------------- S(τ)

    #[test]
    fn datatype_schema_is_any_of_tagged_objects() {
        let s = theta().json_schema(&Type::named("Verdict")).unwrap();
        let alternatives = s["anyOf"].as_array().unwrap();
        assert_eq!(alternatives.len(), 3);
        assert_eq!(
            alternatives[2],
            json!({
                "type": "object",
                "properties": {"tag": {"const": "Sell"}, "reason": {"type": "string"}},
                "required": ["tag", "reason"],
                "additionalProperties": false,
            })
        );
    }

    #[test]
    fn records_are_object_roots_and_everything_else_is_wrapped() {
        let t = theta();
        let (point, wrapped) = output_schema(&t, &Type::named("Point")).unwrap();
        assert!(!wrapped);
        assert_eq!(point["required"], json!(["x", "y"]));
        let (text, wrapped) = output_schema(&t, &Type::Text(Some(20))).unwrap();
        assert!(wrapped);
        assert_eq!(text["properties"]["value"], json!({"type": "string"}));
        assert!(output_schema(&t, &Type::named("Verdict")).unwrap().1, "anyOf is not an object root");
        assert_eq!(t.wrap_overhead(&Type::Text(None)), 10);
        assert_eq!(t.wrap_overhead(&Type::named("Point")), 0);
    }

    #[test]
    fn bounds_are_left_to_validate() {
        let s = theta().json_schema(&Type::List(Box::new(Type::Text(Some(5))), Some(3))).unwrap();
        assert_eq!(s, json!({"type": "array", "items": {"type": "string"}}));
    }

    /// A parameterized type's schema is its definition with the arguments
    /// substituted, and the root rule looks at the definition's shape: a
    /// two-constructor `Option` is wrapped, a record `Pair` isn't.
    #[test]
    fn parameterized_types_are_asked_with_their_arguments() {
        let mut t = theta();
        let a = || Type::Var(Rc::from("a"));
        let b = || Type::Var(Rc::from("b"));
        let option = vec![
            ConDef { name: Rc::from("None"), fields: vec![] },
            ConDef { name: Rc::from("Some"), fields: vec![(Rc::from("value"), a())] },
        ];
        t.add_datatype(Rc::from("Option"), vec![Rc::from("a")], option).unwrap();
        t.add_record(
            Rc::from("Pair"),
            vec![Rc::from("a"), Rc::from("b")],
            vec![(Rc::from("fst"), a()), (Rc::from("snd"), b())],
        )
        .unwrap();

        let opt_num = Type::Named(Rc::from("Option"), vec![Type::Num]);
        let (schema, wrapped) = output_schema(&t, &opt_num).unwrap();
        assert!(wrapped, "anyOf is not an object root");
        let some = &schema["properties"]["value"]["anyOf"][1];
        assert_eq!(some["properties"]["value"], json!({"type": "integer"}), "{schema}");

        let pair = Type::Named(Rc::from("Pair"), vec![Type::Text(Some(9)), Type::Bool]);
        let (schema, wrapped) = output_schema(&t, &pair).unwrap();
        assert!(!wrapped);
        assert_eq!(schema["properties"]["fst"], json!({"type": "string"}));
        assert_eq!(schema["properties"]["snd"], json!({"type": "boolean"}));
    }

    #[test]
    fn recursive_types_cannot_be_asked_live() {
        let err = theta().json_schema(&Type::named("Tree")).unwrap_err();
        assert!(err.contains("recursive"), "{}", err);
        assert!(theta().json_schema(&Type::Money).is_err());
    }

    // ------------------------------------------------------ request bodies

    #[test]
    fn request_body_for_the_analyst() {
        let r = req(&[("System", "You are an analyst."), ("User", "FY2025 10-K")], Type::named("Verdict"));
        let (body, wrapped) = request_body(&theta(), &r).unwrap();
        assert!(wrapped);
        assert_eq!(body["model"], "claude-sonnet-5");
        assert_eq!(body["max_tokens"], 2436);
        assert_eq!(body["system"], "You are an analyst.");
        assert_eq!(body["messages"], json!([{"role": "user", "content": "FY2025 10-K"}]));
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert!(body["output_config"]["format"]["schema"]["properties"]["value"]["anyOf"].is_array());
        assert!(body.get("thinking").is_none(), "thinking is left at the model's default");
    }

    /// A grant without `[cache …]` sends no `cache_control`; with it, the
    /// request asks for automatic caching and marks the system prompt, both
    /// with the grant's lifetime (design/09 §7a). Counting doesn't change.
    #[test]
    fn a_caching_grant_marks_the_system_prompt_and_the_end() {
        let r = req(&[("System", "You are an analyst."), ("User", "FY2025 10-K")], Type::named("Verdict"));
        let (plain, _) = request_body(&theta(), &r).unwrap();
        assert!(plain.get("cache_control").is_none());
        assert_eq!(plain["system"], "You are an analyst.");

        let (five, _) = request_body(&theta(), &cached(r.clone(), 300_000)).unwrap();
        assert_eq!(five["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(
            five["system"],
            json!([{"type": "text", "text": "You are an analyst.", "cache_control": {"type": "ephemeral"}}])
        );
        let (hour, _) = request_body(&theta(), &cached(r.clone(), 3_600_000)).unwrap();
        assert_eq!(hour["cache_control"], json!({"type": "ephemeral", "ttl": "1h"}));
        assert_eq!(hour["system"][0]["cache_control"], json!({"type": "ephemeral", "ttl": "1h"}));
        assert_eq!(count_body(&theta(), &cached(r.clone(), 300_000)).unwrap(), count_body(&theta(), &r).unwrap());

        // With no system prompt, only the automatic breakpoint.
        let (bare, _) = request_body(&theta(), &cached(req(&[("User", "hi")], Type::Bool), 300_000)).unwrap();
        assert!(bare.get("system").is_none());
        assert_eq!(bare["cache_control"], json!({"type": "ephemeral"}));
    }

    /// Usage is split as it's priced: plain input, writes by lifetime, reads.
    #[test]
    fn usage_keeps_cache_reads_and_writes_apart() {
        let body = |usage: J| {
            json!({"content": [{"type": "text", "text": "true"}], "stop_reason": "end_turn", "usage": usage})
                .to_string()
        };
        let usage_of = |b: String, ttl| match parse_response(200, &b, false, ttl).unwrap() {
            Completion::Answer { usage, .. } => usage,
            other => panic!("{:?}", other),
        };
        let u = usage_of(
            body(json!({"input_tokens": 12, "cache_creation_input_tokens": 600, "cache_read_input_tokens": 900,
                        "cache_creation": {"ephemeral_5m_input_tokens": 100, "ephemeral_1h_input_tokens": 500},
                        "output_tokens": 7})),
            None,
        );
        assert_eq!(u, Usage { input: 12, cache_write_5m: 100, cache_write_1h: 500, cache_read: 900, output: 7 });
        assert_eq!(u.input_total(), 1512);
        // Without the breakdown, writes take the grant's lifetime.
        let flat = json!({"input_tokens": 1, "cache_creation_input_tokens": 600, "output_tokens": 2});
        assert_eq!(usage_of(body(flat.clone()), Some(300_000)).cache_write_5m, 600);
        assert_eq!(usage_of(body(flat), Some(3_600_000)).cache_write_1h, 600);
    }

    #[test]
    fn count_body_is_the_same_input_without_max_tokens() {
        let r = req(&[("User", "hi")], Type::Bool);
        let (full, _) = request_body(&theta(), &r).unwrap();
        let mut count = count_body(&theta(), &r).unwrap();
        assert!(count.get("max_tokens").is_none());
        count["max_tokens"] = full["max_tokens"].clone();
        assert_eq!(count, full);
    }

    #[test]
    fn tool_messages_are_user_turns_and_same_role_turns_merge() {
        let r = req(
            &[
                ("System", "a"),
                ("User", "task"),
                ("Assistant", "x = 1"),
                ("Tool", ""),
                ("User", "and?"),
                ("System", "b"),
            ],
            Type::Bool,
        );
        let conv = conversation(&r).unwrap();
        assert_eq!(conv.system.as_deref(), Some("a\n\nb"));
        assert_eq!(
            conv.turns,
            vec![("user", "task".into()), ("assistant", "x = 1".into()), ("user", "Tool output:\n\n\nand?".into())]
        );
    }

    #[test]
    fn contexts_must_end_with_a_user_turn() {
        assert!(conversation(&req(&[], Type::Bool)).is_err());
        assert!(conversation(&req(&[("System", "only a system prompt")], Type::Bool)).is_err());
        let err = conversation(&req(&[("User", "q"), ("Assistant", "prefill")], Type::Bool)).unwrap_err();
        assert!(err.contains("ends with an Assistant message"), "{}", err);
    }

    // ---------------------------------------------------------- responses

    fn ok(stop: &str, text: &str) -> String {
        json!({
            "content": [
                {"type": "thinking", "thinking": "", "signature": "…"},
                {"type": "text", "text": text}
            ],
            "stop_reason": stop,
            "usage": {"input_tokens": 120, "output_tokens": 45, "cache_read_input_tokens": 3}
        })
        .to_string()
    }

    #[test]
    fn a_wrapped_answer_is_unwrapped_and_thinking_is_skipped() {
        let c = parse_response(200, &ok("end_turn", r#"{"value": {"tag": "Sell", "reason": "Margins."}}"#), true, None)
            .unwrap();
        let Completion::Answer { json, usage, truncated } = c else { panic!("{:?}", c) };
        assert_eq!(serde_json::from_str::<J>(&json).unwrap(), json!({"tag": "Sell", "reason": "Margins."}));
        assert_eq!((usage.input_total(), usage.output, truncated), (123, 45, false));
    }

    #[test]
    fn an_unwrapped_answer_passes_through() {
        let c = parse_response(200, &ok("end_turn", r#"{"x": 1, "y": 2}"#), false, None).unwrap();
        assert!(matches!(c, Completion::Answer { ref json, .. } if json == r#"{"x": 1, "y": 2}"#));
    }

    #[test]
    fn max_tokens_is_truncation() {
        let c = parse_response(200, &ok("max_tokens", r#"{"value": {"tag": "Se"#), true, None).unwrap();
        assert!(matches!(c, Completion::Answer { truncated: true, .. }));
    }

    #[test]
    fn refusal_carries_its_category() {
        let body = json!({
            "content": [], "stop_reason": "refusal",
            "stop_details": {"type": "refusal", "category": "cyber", "explanation": "…"},
            "usage": {"input_tokens": 50, "output_tokens": 0}
        })
        .to_string();
        let c = parse_response(200, &body, true, None).unwrap();
        assert!(matches!(c, Completion::Refusal { ref category, usage } if category == "cyber" && usage.input == 50));
    }

    #[test]
    fn transient_errors_are_tool_errors_and_request_errors_are_fatal() {
        let err = |t: &str, m: &str| json!({"type": "error", "error": {"type": t, "message": m}}).to_string();
        let c = parse_response(429, &err("rate_limit_error", "slow down"), false, None).unwrap();
        assert!(matches!(c, Completion::ProviderError { ref msg, .. } if msg == "429 rate_limit_error: slow down"));
        assert!(matches!(
            parse_response(529, &err("overloaded_error", "busy"), false, None),
            Ok(Completion::ProviderError { .. })
        ));
        assert!(matches!(
            parse_response(503, "<html>gateway</html>", false, None),
            Ok(Completion::ProviderError { .. })
        ));
        let fatal = parse_response(404, &err("not_found_error", "model: claude-nope"), false, None).unwrap_err();
        assert!(fatal.contains("404 not_found_error"), "{}", fatal);
        assert!(parse_response(401, &err("authentication_error", "bad key"), false, None).is_err());
        assert!(parse_response(400, &err("invalid_request_error", "bad"), false, None).is_err());
    }

    #[test]
    fn counts_parse_and_count_errors_classify() {
        assert_eq!(parse_count(200, r#"{"input_tokens": 1234}"#), Ok(1234));
        assert!(matches!(parse_count(429, "{}"), Err(Failure::Transient(_))));
        assert!(matches!(parse_count(401, "{}"), Err(Failure::Fatal(_))));
    }
    /// Law A3 of `06` says a smaller output bound means a smaller reservation.
    /// That ignores the schema, which is sent as *input* on every ask: the
    /// first real call measured it at 478 of 538 input tokens (design/09 §3).
    /// Output bound and schema size are independent, so A3 is false. Here a
    /// type with a 60x smaller answer has a 3x larger schema.
    #[test]
    fn a_smaller_output_bound_can_cost_more_input() {
        let mut t = theta();
        wide(&mut t);
        let narrow = Type::named("Wide");
        let broad = Type::Text(Some(4000));

        let bound_narrow = t.bound(&narrow).expect("bounded");
        let bound_broad = t.bound(&broad).expect("bounded");
        let schema = |ty: &Type| output_schema(&t, ty).unwrap().0.to_string().len();

        assert!(bound_narrow < bound_broad, "{bound_narrow} is not below {bound_broad}");
        assert!(
            schema(&narrow) > schema(&broad),
            "Wide's schema is {} bytes, (Text 4000)'s is {}",
            schema(&narrow),
            schema(&broad)
        );
    }
}
