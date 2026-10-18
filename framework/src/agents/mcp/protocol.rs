//! The MCP wire rules the endpoint enforces (AGENT-003): protocol
//! versions, the request metadata headers, and JSON-RPC answers.
//!
//! Revision 2026-07-28 carries the protocol version on every request, in
//! `params._meta` and in the `MCP-Protocol-Version` header, and mirrors the
//! method and the tool's name into the `Mcp-Method` and `Mcp-Name` headers;
//! a header that is missing or disagrees with the body is refused with 400
//! and `HeaderMismatch` (-32020). Revisions 2025-06-18 and 2025-11-25 open
//! with `initialize` and send the version header afterwards. Any other
//! version is refused with 400 and `UnsupportedProtocolVersion` (-32022),
//! listing the supported ones.

use base64::Engine;
use serde_json::{Value, json};

use crate::http::HttpResponse;

/// The revision served statelessly, with per-request metadata.
pub(crate) const MODERN: &str = "2026-07-28";

/// The `initialize`-based revisions served without sessions.
pub(crate) const LEGACY: [&str; 2] = ["2025-11-25", "2025-06-18"];

/// Every supported revision, newest first.
pub(crate) const SUPPORTED: [&str; 3] = [MODERN, LEGACY[0], LEGACY[1]];

/// The header carrying the request's protocol version.
pub(crate) const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
/// The header mirroring the JSON-RPC method.
pub(crate) const METHOD_HEADER: &str = "mcp-method";
/// The header mirroring the tool, prompt or resource a request names.
pub(crate) const NAME_HEADER: &str = "mcp-name";

/// The `_meta` key of the request's protocol version.
pub(crate) const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// The `_meta` key of the client's capabilities.
pub(crate) const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// The `_meta` key of the server's name and version in a result.
pub(crate) const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// JSON-RPC: the body is not JSON.
pub(crate) const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: the body is not a request the endpoint takes.
pub(crate) const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: the method does not exist.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: the parameters are not valid, or the tool is unknown.
pub(crate) const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC: the server failed.
pub(crate) const INTERNAL_ERROR: i64 = -32603;
/// MCP `HeaderMismatch`.
pub(crate) const HEADER_MISMATCH: i64 = -32020;
/// MCP `UnsupportedProtocolVersion`.
pub(crate) const UNSUPPORTED_VERSION: i64 = -32022;

// Suprnova's own codes, outside the range JSON-RPC and MCP reserve, for the
// HTTP-level refusals whose status already carries the meaning.

/// The request needs a token it did not present, or its token is invalid
/// (HTTP 401).
pub(crate) const UNAUTHORIZED: i64 = -31401;
/// The request may not do this (HTTP 403).
pub(crate) const FORBIDDEN: i64 = -31403;
/// The caller made too many calls (HTTP 429).
pub(crate) const RATE_LIMITED: i64 = -31429;
/// A dependency the endpoint needs is unavailable (HTTP 503).
pub(crate) const UNAVAILABLE: i64 = -31503;

/// A JSON-RPC result answer.
pub(crate) fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC error answer. `id` is `None` when the request's id could not
/// be read.
pub(crate) fn error(id: Option<&Value>, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    match id {
        Some(id) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        None => json!({ "jsonrpc": "2.0", "error": error }),
    }
}

/// `answer` as an HTTP response with `status`.
pub(crate) fn respond(status: u16, answer: Value) -> HttpResponse {
    HttpResponse::json(answer).status(status)
}

/// A JSON-RPC error as an HTTP response.
pub(crate) fn refuse(
    status: u16,
    id: Option<&Value>,
    code: i64,
    message: &str,
    data: Option<Value>,
) -> HttpResponse {
    respond(status, error(id, code, message, data))
}

/// The `UnsupportedProtocolVersion` refusal for `requested`.
pub(crate) fn unsupported_version(id: Option<&Value>, requested: Option<&str>) -> HttpResponse {
    refuse(
        400,
        id,
        UNSUPPORTED_VERSION,
        "Unsupported protocol version",
        Some(json!({ "supported": SUPPORTED, "requested": requested })),
    )
}

/// A `HeaderMismatch` refusal.
pub(crate) fn header_mismatch(id: Option<&Value>, message: &str) -> HttpResponse {
    refuse(400, id, HEADER_MISMATCH, message, None)
}

/// Why a request metadata header cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderProblem {
    /// The header appears more than once, so which value the body must
    /// match is unclear.
    Repeated,
    /// The value holds characters a header value may not.
    Invalid,
}

impl HeaderProblem {
    /// The refusal's text for the header `name`.
    pub(crate) fn describe(self, name: &str) -> String {
        match self {
            Self::Repeated => format!("The {name} header appears more than once."),
            Self::Invalid => format!("The {name} header holds characters a header may not."),
        }
    }
}

/// A request header's value as text: `Ok(None)` when absent, and an error
/// when it appears more than once or is not a valid value.
pub(crate) fn header_text(
    headers: &hyper::HeaderMap,
    name: &str,
) -> Result<Option<String>, HeaderProblem> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(HeaderProblem::Repeated);
    }
    value
        .to_str()
        .map(|text| Some(text.to_string()))
        .map_err(|_| HeaderProblem::Invalid)
}

/// The value of an `Mcp-Name` header, decoded from the Base64 sentinel
/// `=?base64?...?=` a client uses for a value that is not plain ASCII.
/// `None` when the sentinel holds no valid Base64 of UTF-8 text.
pub(crate) fn decode_name(value: &str) -> Option<String> {
    match value
        .strip_prefix("=?base64?")
        .and_then(|inner| inner.strip_suffix("?="))
    {
        Some(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        None => Some(value.to_string()),
    }
}

/// The body field a method's `Mcp-Name` mirrors, for the methods that have
/// one.
pub(crate) fn name_field(method: &str) -> Option<&'static str> {
    match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => Some("uri"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_name, name_field};

    #[test]
    fn agent_003_mcp_name_decodes_the_base64_sentinel() {
        assert_eq!(decode_name("posts.create").as_deref(), Some("posts.create"));
        assert_eq!(
            decode_name("=?base64?SGVsbG8sIOS4lueVjA==?=").as_deref(),
            Some("Hello, \u{4e16}\u{754c}")
        );
        assert_eq!(decode_name("=?base64?!!!?="), None);
    }

    #[test]
    fn agent_003_only_named_methods_mirror_a_name() {
        assert_eq!(name_field("tools/call"), Some("name"));
        assert_eq!(name_field("resources/read"), Some("uri"));
        assert_eq!(name_field("tools/list"), None);
    }
}
