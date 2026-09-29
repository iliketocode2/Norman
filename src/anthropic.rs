//! The Messages API wire format (design/09 §1–4): pure translations between
//! µNorman's view of an `ask` and the JSON the Anthropic API sends and
//! receives. Nothing here touches the network; the live client (step 3)
//! sends what `request_body` builds and hands what comes back to
//! `parse_response`.
//!
//! There is no official Anthropic SDK for Rust, so this is raw HTTP:
//! `POST /v1/messages` and `POST /v1/messages/count_tokens`, with headers
//! `x-api-key` and `anthropic-version: 2023-06-01`.

use crate::host::{AskReq, Completion};
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
pub fn request_body(theta: &TypeEnv, req: &AskReq) -> Result<(J, bool), String> {
    let (mut body, wrapped) = common_body(theta, req)?;
    body.insert("max_tokens".into(), json!(req.max_tokens));
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
    let j: J = serde_json::from_str(body).map_err(|e| Failure::Fatal(format!("count_tokens returned malformed JSON: {}", e)))?;
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
/// whether the answer is `{"value": …}` to be unwrapped before `validate`.
/// `Err` is a checked run-time error: a request the program or its grants got wrong.
pub fn parse_response(status: u16, body: &str, wrapped: bool) -> Result<Completion, String> {
    if !(200..300).contains(&status) {
        return match classify_error(status, body) {
            Failure::Transient(msg) => Ok(Completion::ProviderError { msg, input_tokens: 0 }),
            Failure::Fatal(msg) => Err(msg),
        };
    }
    let j: J = match serde_json::from_str(body) {
        Ok(j) => j,
        Err(e) => return Ok(Completion::ProviderError { msg: format!("malformed response: {}", e), input_tokens: 0 }),
    };
    let usage = |field: &str| j.pointer(&format!("/usage/{}", field)).and_then(J::as_i64).unwrap_or(0);
    // Every input token is charged at the grant's input price. Cache writes and
    // reads are priced differently by the provider, but µNorman doesn't send
    // cache_control, so both are normally zero (09 §7).
    let input_tokens = usage("input_tokens") + usage("cache_creation_input_tokens") + usage("cache_read_input_tokens");
    let output_tokens = usage("output_tokens");
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
            Completion::Answer { json: unwrap_answer(&text, wrapped), input_tokens, output_tokens, truncated: false }
        }
        "max_tokens" => Completion::Answer { json: text, input_tokens, output_tokens, truncated: true },
        "refusal" => {
            let category = j.pointer("/stop_details/category").and_then(J::as_str).unwrap_or("unspecified").to_string();
            Completion::Refusal { category, input_tokens, output_tokens }
        }
        other => Completion::ProviderError { msg: format!("unexpected stop_reason {:?}", other), input_tokens },
    })
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
        t.add_datatype(Rc::from("Verdict"), verdict).unwrap();
        t.add_record(Rc::from("Point"), vec![(Rc::from("x"), Type::Num), (Rc::from("y"), Type::Num)]).unwrap();
        let tree = vec![
            ConDef { name: Rc::from("Leaf"), fields: vec![] },
            ConDef {
                name: Rc::from("Node"),
                fields: vec![(Rc::from("left"), Type::Named(Rc::from("Tree"))), (Rc::from("right"), Type::Named(Rc::from("Tree")))],
            },
        ];
        t.add_datatype(Rc::from("Tree"), tree).unwrap();
        t
    }

    fn req(messages: &[(&str, &str)], ty: Type) -> AskReq {
        AskReq {
            site: "analyst".into(),
            model: Rc::new(ModelSpec {
                name: Rc::from("m"),
                id: "claude-sonnet-5".into(),
                think: 2000,
                in_price: 2,
                out_price: 10,
                ceiling: 128_000,
            }),
            ty,
            messages: messages.iter().map(|(r, c)| (Rc::from(*r), Rc::from(*c))).collect(),
            max_tokens: 2436,
        }
    }

    // ---------------------------------------------------------------- S(τ)

    #[test]
    fn datatype_schema_is_any_of_tagged_objects() {
        let s = theta().json_schema(&Type::Named(Rc::from("Verdict"))).unwrap();
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
        let (point, wrapped) = output_schema(&t, &Type::Named(Rc::from("Point"))).unwrap();
        assert!(!wrapped);
        assert_eq!(point["required"], json!(["x", "y"]));
        let (text, wrapped) = output_schema(&t, &Type::Text(Some(20))).unwrap();
        assert!(wrapped);
        assert_eq!(text["properties"]["value"], json!({"type": "string"}));
        assert!(output_schema(&t, &Type::Named(Rc::from("Verdict"))).unwrap().1, "anyOf is not an object root");
        assert_eq!(t.wrap_overhead(&Type::Text(None)), 10);
        assert_eq!(t.wrap_overhead(&Type::Named(Rc::from("Point"))), 0);
    }

    #[test]
    fn bounds_are_left_to_validate() {
        let s = theta().json_schema(&Type::List(Box::new(Type::Text(Some(5))), Some(3))).unwrap();
        assert_eq!(s, json!({"type": "array", "items": {"type": "string"}}));
    }

    #[test]
    fn recursive_types_cannot_be_asked_live() {
        let err = theta().json_schema(&Type::Named(Rc::from("Tree"))).unwrap_err();
        assert!(err.contains("recursive"), "{}", err);
        assert!(theta().json_schema(&Type::Any).is_err());
    }

    // ------------------------------------------------------ request bodies

    #[test]
    fn request_body_for_the_analyst() {
        let r = req(&[("System", "You are an analyst."), ("User", "FY2025 10-K")], Type::Named(Rc::from("Verdict")));
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
            &[("System", "a"), ("User", "task"), ("Assistant", "x = 1"), ("Tool", ""), ("User", "and?"), ("System", "b")],
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
        let c = parse_response(200, &ok("end_turn", r#"{"value": {"tag": "Sell", "reason": "Margins."}}"#), true).unwrap();
        let Completion::Answer { json, input_tokens, output_tokens, truncated } = c else { panic!("{:?}", c) };
        assert_eq!(serde_json::from_str::<J>(&json).unwrap(), json!({"tag": "Sell", "reason": "Margins."}));
        assert_eq!((input_tokens, output_tokens, truncated), (123, 45, false));
    }

    #[test]
    fn an_unwrapped_answer_passes_through() {
        let c = parse_response(200, &ok("end_turn", r#"{"x": 1, "y": 2}"#), false).unwrap();
        assert!(matches!(c, Completion::Answer { ref json, .. } if json == r#"{"x": 1, "y": 2}"#));
    }

    #[test]
    fn max_tokens_is_truncation() {
        let c = parse_response(200, &ok("max_tokens", r#"{"value": {"tag": "Se"#), true).unwrap();
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
        let c = parse_response(200, &body, true).unwrap();
        assert!(matches!(c, Completion::Refusal { ref category, input_tokens: 50, .. } if category == "cyber"));
    }

    #[test]
    fn transient_errors_are_tool_errors_and_request_errors_are_fatal() {
        let err = |t: &str, m: &str| json!({"type": "error", "error": {"type": t, "message": m}}).to_string();
        let c = parse_response(429, &err("rate_limit_error", "slow down"), false).unwrap();
        assert!(matches!(c, Completion::ProviderError { ref msg, .. } if msg == "429 rate_limit_error: slow down"));
        assert!(matches!(parse_response(529, &err("overloaded_error", "busy"), false), Ok(Completion::ProviderError { .. })));
        assert!(matches!(parse_response(503, "<html>gateway</html>", false), Ok(Completion::ProviderError { .. })));
        let fatal = parse_response(404, &err("not_found_error", "model: claude-nope"), false).unwrap_err();
        assert!(fatal.contains("404 not_found_error"), "{}", fatal);
        assert!(parse_response(401, &err("authentication_error", "bad key"), false).is_err());
        assert!(parse_response(400, &err("invalid_request_error", "bad"), false).is_err());
    }

    #[test]
    fn counts_parse_and_count_errors_classify() {
        assert_eq!(parse_count(200, r#"{"input_tokens": 1234}"#), Ok(1234));
        assert!(matches!(parse_count(429, "{}"), Err(Failure::Transient(_))));
        assert!(matches!(parse_count(401, "{}"), Err(Failure::Fatal(_))));
    }
}
