//! A server tool's URL from its path pieces and the call's arguments.
//!
//! The server sends a tool's route as pieces it has already parsed, so no
//! client reads the router's placeholder grammar. The rules are the MCP
//! dispatcher's (`fill_path` and `query_of` in
//! `framework/src/agents/dispatch.rs`): each value is percent-encoded into
//! one segment, a segment of dots alone cannot climb, an optional parameter
//! without a value drops its segment, and a GET call sends its other
//! scalar arguments in the query.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Map, Value};

/// One piece of a tool's path.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum Piece {
    /// Literal text.
    Text(Text),
    /// A parameter filled from the argument of its name.
    Param(Param),
}

/// Literal text of a path.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Text {
    /// The text, already in URL form.
    pub(crate) text: String,
}

/// A parameter of a path.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Param {
    /// The argument that fills it.
    pub(crate) param: String,
    /// Whether the route matches without it.
    pub(crate) optional: bool,
    /// Whether it takes the rest of the path, slashes included.
    pub(crate) catch_all: bool,
}

/// What a path segment keeps as it is: the unreserved characters of RFC
/// 3986, as the framework's `PATH_SEGMENT_ENCODE` set leaves them.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The path of `pieces` with each parameter filled from `arguments`, or
/// the reason it cannot be.
pub(crate) fn fill(pieces: &[Piece], arguments: &Map<String, Value>) -> Result<String, String> {
    let mut out = String::new();
    let mut after_text = false;
    for piece in pieces {
        match piece {
            Piece::Text(text) => {
                out.push_str(&text.text);
                after_text = true;
            }
            Piece::Param(param) => {
                let name = &param.param;
                match scalar(arguments.get(name)) {
                    Err(()) => {
                        return Err(format!(
                            "the path parameter `{name}` must be a string, a number or a boolean"
                        ));
                    }
                    Ok(Some(value)) if !value.is_empty() => {
                        out.push_str(&encode(&value, param.catch_all));
                    }
                    Ok(_) if param.optional => {
                        // Drop the parameter and the `/` in front of it.
                        if after_text && out.ends_with('/') {
                            out.pop();
                        }
                    }
                    Ok(_) => return Err(format!("the path parameter `{name}` is missing")),
                }
                after_text = false;
            }
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    Ok(out)
}

/// For a GET call, the scalar arguments that fill no parameter of
/// `pieces`, as a query string in argument order. A structured argument is
/// refused, because a browser sends no body with a GET.
pub(crate) fn query(
    pieces: &[Piece],
    arguments: &Map<String, Value>,
) -> Result<Option<String>, String> {
    let in_path = |name: &str| {
        pieces
            .iter()
            .any(|piece| matches!(piece, Piece::Param(param) if param.param == name))
    };
    let mut query = form_urlencoded::Serializer::new(String::new());
    let mut any = false;
    for (name, value) in arguments {
        if in_path(name) {
            continue;
        }
        match scalar(Some(value)) {
            Ok(Some(text)) => {
                query.append_pair(name, &text);
                any = true;
            }
            Ok(None) => {}
            Err(()) => {
                return Err(format!(
                    "the argument `{name}` must be a string, a number or a boolean, because a GET \
                     tool sends its arguments in the query"
                ));
            }
        }
    }
    Ok(any.then(|| query.finish()))
}

/// `value` as URL text: `None` when it is absent or null, `Err` when it is
/// an array or an object.
fn scalar(value: Option<&Value>) -> Result<Option<String>, ()> {
    match value {
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        Some(Value::Bool(flag)) => Ok(Some(flag.to_string())),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(()),
    }
}

/// `value` as one path segment, or for a catch-all as segments each
/// encoded on its own. A segment of dots alone is encoded too, so it
/// cannot read as `.` or `..`.
fn encode(value: &str, catch_all: bool) -> String {
    let segment = |part: &str| -> String {
        if !part.is_empty() && part.bytes().all(|byte| byte == b'.') {
            "%2E".repeat(part.len())
        } else {
            utf8_percent_encode(part, SEGMENT).to_string()
        }
    };
    if catch_all {
        value.split('/').map(segment).collect::<Vec<_>>().join("/")
    } else {
        segment(value)
    }
}
