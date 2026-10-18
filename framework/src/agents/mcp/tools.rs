//! `tools/list` and the result of `tools/call` (AGENT-003, AGENT-004).
//!
//! A tool's route answers like any route. The endpoint turns that answer
//! into an MCP tool result: a JSON body becomes the structured content and
//! its text, and a failure becomes a result with `isError: true`, never a
//! protocol error, so the agent can read what went wrong and try again. A
//! validation failure names each field and its messages; a server failure
//! says only that the tool failed, so no internal detail leaks.

use serde_json::{Map, Value, json};

use super::super::caller::ToolCaller;
use super::super::dispatch::{CallFailure, RouteAnswer};
use super::super::input::{InputReport, MAX_PATH_CHARS, MAX_REPORTED};
use super::super::tool::ToolRegistry;

/// The tools `caller` may call, sorted by name: the tools that accept the
/// caller's kind and, for a token caller, whose scopes the token holds.
pub(crate) fn list(registry: &ToolRegistry, caller: &ToolCaller) -> Vec<Value> {
    registry
        .tools()
        .iter()
        .filter(|tool| tool.callable_by(caller))
        .map(|tool| tool.definition())
        .collect()
}

/// A tool result with `isError: true` and `message` as its text.
pub(crate) fn error_result(message: &str) -> Value {
    json!({
        "resultType": "complete",
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

/// The tool result of a call that did not reach its route's answer.
pub(crate) fn failure_result(failure: &CallFailure) -> Value {
    match failure {
        CallFailure::BadArguments(reason) => error_result(&format!("Invalid input: {reason}.")),
        CallFailure::TimedOut(limit) => error_result(&format!(
            "The tool did not finish within {} seconds and was stopped.",
            limit.as_secs_f64()
        )),
        CallFailure::TooLarge(limit) => error_result(&format!(
            "The tool's result is larger than the {limit} bytes the endpoint returns."
        )),
    }
}

/// The tool result of the route's answer.
pub(crate) fn answer_result(answer: &RouteAnswer, max_bytes: usize) -> Value {
    let json_body = is_json(answer.content_type.as_deref())
        .then(|| serde_json::from_slice::<Value>(&answer.body).ok())
        .flatten();
    match answer.status {
        200..=299 => {
            if answer.body.is_empty() {
                return json!({ "resultType": "complete", "content": [], "isError": false });
            }
            match json_body {
                Some(structured) => json!({
                    "resultType": "complete",
                    "content": [{
                        "type": "text",
                        "text": String::from_utf8_lossy(&answer.body),
                    }],
                    "structuredContent": structured,
                    "isError": false,
                }),
                None => error_result("The tool answered with a body that is not JSON."),
            }
        }
        300..=399 => {
            error_result("The tool answered with a redirect, which a tool call cannot follow.")
        }
        422 => match json_body.as_ref().and_then(|body| body.get("errors")) {
            Some(Value::Object(errors)) => validation_result(errors, max_bytes),
            _ => error_result(&message_of(json_body.as_ref(), "The input is not valid.")),
        },
        429 => error_result("Too many calls: wait before calling the tool again."),
        status if status >= 500 => error_result("The tool failed."),
        401 => error_result(&message_of(
            json_body.as_ref(),
            "The tool needs a signed-in user.",
        )),
        403 => error_result(&message_of(
            json_body.as_ref(),
            "This action is unauthorized.",
        )),
        404 => error_result(&message_of(json_body.as_ref(), "Not found.")),
        _ => error_result(&message_of(
            json_body.as_ref(),
            "The tool refused the call.",
        )),
    }
}

/// The answer of a headless Live tool, which marks it with its own media
/// type: `isError` and the structured content as the handler set them
/// (AGENT-013). A validation outcome is an error the agent must see and
/// still carries the new handle, since it consumed a revision. Read only for
/// a Live tool, whose route is the framework's own, so no application route
/// can shape a result this way.
pub(crate) fn live_result(answer: &RouteAnswer) -> Option<Value> {
    let media = answer
        .content_type
        .as_deref()?
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if media != crate::live::LIVE_TOOL_RESULT_MEDIA_TYPE || !(200..=299).contains(&answer.status) {
        return None;
    }
    let body = serde_json::from_slice::<Value>(&answer.body).ok()?;
    let structured = body.get("structuredContent")?.clone();
    let is_error = body
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(json!({
        "resultType": "complete",
        "content": [{ "type": "text", "text": structured.to_string() }],
        "structuredContent": structured,
        "isError": is_error,
    }))
}

/// A tool result naming each field of the input that breaks the tool's
/// schema.
///
/// The report already holds at most [`MAX_REPORTED`] fields, each path cut
/// and escaped; the text is cut to `max_bytes` as well.
pub(crate) fn invalid_input_result(report: &InputReport, max_bytes: usize) -> Value {
    let mut text = String::from("The input is not valid:");
    for failure in &report.failures {
        text.push_str(&format!("\n- {}: {}", failure.field, failure.message));
    }
    if report.more > 0 {
        text.push_str(&format!("\n({} more fields are not valid.)", report.more));
    }
    error_result(&bounded(text, max_bytes))
}

/// A validation failure from the route as a tool result naming each field,
/// bounded as [`invalid_input_result`] bounds its own.
fn validation_result(errors: &Map<String, Value>, max_bytes: usize) -> Value {
    let mut text = String::from("The input is not valid:");
    for (field, messages) in errors.iter().take(MAX_REPORTED) {
        let messages: Vec<String> = match messages {
            Value::Array(list) => list
                .iter()
                .map(|message| {
                    message
                        .as_str()
                        .map_or_else(|| message.to_string(), str::to_string)
                })
                .collect(),
            Value::String(message) => vec![message.clone()],
            other => vec![other.to_string()],
        };
        text.push_str(&format!(
            "\n- {}: {}",
            one_line(field, MAX_PATH_CHARS),
            one_line(&messages.join(" "), MAX_MESSAGE_CHARS)
        ));
    }
    if errors.len() > MAX_REPORTED {
        text.push_str(&format!(
            "\n({} more fields are not valid.)",
            errors.len() - MAX_REPORTED
        ));
    }
    error_result(&bounded(text, max_bytes))
}

/// The longest message a route's validation report shows for one field.
const MAX_MESSAGE_CHARS: usize = 500;

/// `text` on one line, its control characters replaced by spaces, cut to
/// `max_chars` characters with an ellipsis.
fn one_line(text: &str, max_chars: usize) -> String {
    let mut out: String = text
        .chars()
        .take(max_chars)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if text.chars().nth(max_chars).is_some() {
        out.push_str("...");
    }
    out
}

/// `text` cut to at most `max_bytes` bytes, at a character boundary.
fn bounded(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes.saturating_sub(3);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("...");
    text
}

/// The `message` of an error body, or `fallback`.
fn message_of(body: Option<&Value>, fallback: &str) -> String {
    body.and_then(|body| body.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Whether `content_type` is JSON: `application/json` or a `+json` type.
fn is_json(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|value| {
        let media = value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        media == "application/json" || media.ends_with("+json")
    })
}

#[cfg(test)]
mod tests {
    use super::answer_result;
    use crate::agents::dispatch::RouteAnswer;
    use bytes::Bytes;

    fn answer(status: u16, body: &str) -> RouteAnswer {
        RouteAnswer {
            status,
            content_type: Some("application/json".to_string()),
            body: Bytes::from(body.to_string()),
        }
    }

    #[test]
    fn agent_003_validation_errors_name_each_field() {
        let result = answer_result(
            &answer(
                422,
                r#"{"message":"invalid","errors":{"title":["The title is required."],"body":["Too long."]}}"#,
            ),
            usize::MAX,
        );
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text.contains("- title: The title is required."), "{text}");
        assert!(text.contains("- body: Too long."), "{text}");
    }

    #[test]
    fn agent_003_route_validation_reports_are_capped_and_one_line() {
        let mut errors = serde_json::Map::new();
        for index in 0..50 {
            errors.insert(
                format!("field{index}\n- forged"),
                serde_json::json!(["bad\nline"]),
            );
        }
        let body = serde_json::json!({ "message": "invalid", "errors": errors }).to_string();
        let result = answer_result(&answer(422, &body), 4096);
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text.len() <= 4096, "{}", text.len());
        assert!(
            !text.lines().any(|line| line.starts_with("- forged")),
            "{text}"
        );
        assert!(!text.contains("\nline"), "{text}");
    }

    #[test]
    fn agent_003_server_failures_leak_nothing() {
        let result = answer_result(
            &answer(500, r#"{"message":"db password wrong"}"#),
            usize::MAX,
        );
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "The tool failed.");
    }

    #[test]
    fn agent_003_json_success_is_structured_content() {
        let result = answer_result(&answer(201, r#"{"id":7}"#), usize::MAX);
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["id"], 7);
    }
}
