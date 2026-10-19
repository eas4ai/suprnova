//! Tool results, and how a server tool's answer becomes one.
//!
//! The mapping is the framework's MCP mapping (`answer_result` in
//! `framework/src/agents/mcp/tools.rs`) without `resultType`, plus the rows
//! only a browser meets: an expired page session, a network failure, the
//! call timer and an answer over the size bound. The Live runtime and the
//! kits implement the same table, and the shared fixture
//! `framework/tests/agents/fixtures/browser-calls.json` holds all four to
//! it, so an agent reads the same words whichever front end it calls
//! through.

use std::fmt;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::WebMcpError;

/// What a WebMCP tool's `execute` resolves to: the shape MCP gives a tool
/// result, which WebMCP reuses.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolResult {
    /// What the agent reads.
    pub content: Vec<Content>,
    /// The same answer as JSON, when it is JSON, so the agent need not
    /// parse the text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    /// Whether the call failed. A failure is still a result, never a
    /// rejection, so the agent can read why and try again.
    pub is_error: bool,
}

/// One part of a tool result's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum Content {
    /// Text for the agent.
    Text {
        /// The text.
        text: String,
    },
}

impl ToolResult {
    /// A result carrying `value` as JSON text and as structured content,
    /// the shape a server tool's JSON answer takes.
    ///
    /// # Errors
    ///
    /// [`WebMcpError::Output`] when `value` cannot be serialized as JSON,
    /// a map with non-string keys for example.
    pub fn json<S: Serialize + ?Sized>(value: &S) -> Result<Self, WebMcpError> {
        let structured =
            serde_json::to_value(value).map_err(|error| WebMcpError::Output(error.to_string()))?;
        let text = serde_json::to_string(&structured)
            .map_err(|error| WebMcpError::Output(error.to_string()))?;
        Ok(Self {
            content: vec![Content::Text { text }],
            structured_content: Some(structured),
            is_error: false,
        })
    }

    /// A result carrying `text` alone.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text { text: text.into() }],
            structured_content: None,
            is_error: false,
        }
    }

    /// A result with no content, the shape a server tool's empty answer
    /// takes.
    pub fn empty() -> Self {
        Self {
            content: Vec::new(),
            structured_content: None,
            is_error: false,
        }
    }

    /// A failed call, with `message` for the agent.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text {
                text: message.into(),
            }],
            structured_content: None,
            is_error: true,
        }
    }
}

/// A page tool handler's failure: any error, shown to the agent as an
/// error result with its message on one line.
///
/// Any type that implements [`std::fmt::Display`] converts into it, so a
/// handler can use `?` on its own errors.
#[derive(Debug)]
pub struct ToolError {
    message: String,
}

impl ToolError {
    /// The message the agent reads.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl<E: fmt::Display> From<E> for ToolError {
    fn from(error: E) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

/// The result of a handler that failed with `error`.
pub(crate) fn handler_failure(error: &ToolError, max_bytes: usize) -> ToolResult {
    ToolResult::error(bounded(one_line(&error.message, usize::MAX), max_bytes))
}

/// How many fields a validation result names before it counts the rest,
/// as the framework's MCP endpoint does.
const MAX_REPORTED: usize = 20;

/// The longest field name a validation result shows.
const MAX_FIELD_CHARS: usize = 200;

/// The longest message a validation result shows for one field.
const MAX_MESSAGE_CHARS: usize = 500;

/// The answer of a server tool's route, as the browser received it.
pub(crate) struct Answer<'a> {
    /// The HTTP status. Zero for a redirect fetch did not follow.
    pub(crate) status: u16,
    /// Whether fetch reported an `opaqueredirect` response, which is what
    /// `redirect: "manual"` turns every redirect into.
    pub(crate) opaque_redirect: bool,
    /// The `Content-Type` header, when there was one.
    pub(crate) content_type: Option<&'a str>,
    /// The body, at most one byte over the bound.
    pub(crate) body: &'a [u8],
}

/// The tool result of a route's answer, with `max_bytes` the page's bound
/// on a result.
pub(crate) fn answer_result(answer: &Answer<'_>, max_bytes: usize) -> ToolResult {
    // The bound holds whatever the status, as the MCP dispatcher's bounded
    // read does.
    if answer.body.len() > max_bytes {
        return too_large(max_bytes);
    }
    if answer.opaque_redirect {
        return redirect();
    }
    let json_body = is_json(answer.content_type)
        .then(|| serde_json::from_slice::<Value>(answer.body).ok())
        .flatten();
    match answer.status {
        200..=299 => {
            if answer.body.is_empty() {
                return ToolResult::empty();
            }
            match json_body {
                Some(structured) => ToolResult {
                    content: vec![Content::Text {
                        text: String::from_utf8_lossy(answer.body).into_owned(),
                    }],
                    structured_content: Some(structured),
                    is_error: false,
                },
                None => ToolResult::error("The tool answered with a body that is not JSON."),
            }
        }
        300..=399 => redirect(),
        419 => ToolResult::error(
            "The page's session expired: reload the page, then call the tool again.",
        ),
        422 => match json_body.as_ref().and_then(|body| body.get("errors")) {
            Some(Value::Object(errors)) => route_validation(errors, max_bytes),
            _ => ToolResult::error(message_of(json_body.as_ref(), "The input is not valid.")),
        },
        429 => ToolResult::error("Too many calls: wait before calling the tool again."),
        status if status >= 500 => ToolResult::error("The tool failed."),
        401 => ToolResult::error(message_of(
            json_body.as_ref(),
            "The tool needs a signed-in user.",
        )),
        403 => ToolResult::error(message_of(
            json_body.as_ref(),
            "This action is unauthorized.",
        )),
        404 => ToolResult::error(message_of(json_body.as_ref(), "Not found.")),
        _ => ToolResult::error(message_of(json_body.as_ref(), "The tool refused the call.")),
    }
}

/// The result of a call whose request never got an answer.
pub(crate) fn unreachable() -> ToolResult {
    ToolResult::error("The tool could not be reached.")
}

/// The result of a call the timer stopped after `after_ms` milliseconds.
pub(crate) fn timed_out(after_ms: u64) -> ToolResult {
    // Millisecond counts stay exact in an f64 far past any timer, and the
    // seconds print as the framework prints them: 30, 1.5.
    let seconds = after_ms as f64 / 1000.0;
    ToolResult::error(format!(
        "The tool did not finish within {seconds} seconds and was stopped."
    ))
}

/// The result of an answer larger than the page's bound of `limit` bytes.
pub(crate) fn too_large(limit: usize) -> ToolResult {
    ToolResult::error(format!(
        "The tool's result is larger than the {limit} bytes the page returns."
    ))
}

/// The result of input the call could not send, for `reason`.
pub(crate) fn invalid_input(reason: &str) -> ToolResult {
    ToolResult::error(format!("Invalid input: {reason}."))
}

/// The result of a redirect, which a tool call never follows: following
/// one could carry the CSRF header to another origin.
fn redirect() -> ToolResult {
    ToolResult::error("The tool answered with a redirect, which a tool call cannot follow.")
}

/// A route's validation failure as a result naming each field.
fn route_validation(errors: &Map<String, Value>, max_bytes: usize) -> ToolResult {
    let fields = errors.iter().map(|(field, messages)| {
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
        (field.clone(), messages)
    });
    field_list(fields, errors.len(), max_bytes)
}

/// A page tool's input that broke its `validator` rules, as a result
/// naming each field the way a route's validation failure does.
///
/// Nested failures are named in the framework's dotted form
/// (`address.street`, `items.1.name`), and the fields are sorted, since
/// `validator` keeps them in a hash map.
pub(crate) fn input_validation(
    errors: &validator::ValidationErrors,
    max_bytes: usize,
) -> ToolResult {
    let mut fields = Vec::new();
    flatten(errors, "", &mut fields);
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    let total = fields.len();
    field_list(fields.into_iter(), total, max_bytes)
}

/// Every failing field of `errors` under `prefix`, with its messages.
fn flatten(
    errors: &validator::ValidationErrors,
    prefix: &str,
    out: &mut Vec<(String, Vec<String>)>,
) {
    for (field, kind) in errors.errors() {
        let key = if prefix.is_empty() {
            field.to_string()
        } else {
            format!("{prefix}.{field}")
        };
        match kind {
            validator::ValidationErrorsKind::Field(failures) => {
                let messages = failures
                    .iter()
                    .map(|failure| match &failure.message {
                        // The author's own message wins, as it does on the
                        // server.
                        Some(message) => message.to_string(),
                        None => format!("Validation failed for field '{key}'"),
                    })
                    .collect();
                out.push((key, messages));
            }
            validator::ValidationErrorsKind::Struct(inner) => flatten(inner, &key, out),
            validator::ValidationErrorsKind::List(items) => {
                for (index, inner) in items {
                    flatten(inner, &format!("{key}.{index}"), out);
                }
            }
        }
    }
}

/// "The input is not valid:" and one line per field, at most
/// [`MAX_REPORTED`] of `total`, then a count of the rest, the whole cut to
/// `max_bytes`.
fn field_list(
    fields: impl Iterator<Item = (String, Vec<String>)>,
    total: usize,
    max_bytes: usize,
) -> ToolResult {
    let mut text = String::from("The input is not valid:");
    for (field, messages) in fields.take(MAX_REPORTED) {
        text.push_str(&format!(
            "\n- {}: {}",
            one_line(&field, MAX_FIELD_CHARS),
            one_line(&messages.join(" "), MAX_MESSAGE_CHARS)
        ));
    }
    if total > MAX_REPORTED {
        text.push_str(&format!(
            "\n({} more fields are not valid.)",
            total - MAX_REPORTED
        ));
    }
    ToolResult::error(bounded(text, max_bytes))
}

/// `text` on one line, its control characters replaced by spaces, cut to
/// `max_chars` characters with an ellipsis, so a message cannot forge a
/// line of the result.
fn one_line(text: &str, max_chars: usize) -> String {
    let mut out: String = text
        .chars()
        .take(max_chars)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if max_chars < usize::MAX && text.chars().nth(max_chars).is_some() {
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
