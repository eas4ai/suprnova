//! A server tool's URL from its path pieces and the call's arguments.
//!
//! The server sends a tool's route as pieces it has already parsed, so no
//! client reads the router's placeholder grammar. The rules are the MCP
//! dispatcher's (`fill_path` and `query_of` in
//! `framework/src/agents/dispatch.rs`): each value is percent-encoded into
//! one segment, an optional parameter without a value drops its segment,
//! and a GET call sends its scalar arguments in the query. Unlike the
//! dispatcher, a GET call sends its path arguments in the query too, since
//! a form request on GET reads its whole input from there.
//!
//! One rule is the browser's own. The dispatcher encodes a `.` or `..`
//! value as `%2E` so it cannot climb, but a browser's URL parser resolves
//! those segments, percent-encoded or not, before the request leaves, and
//! the call would reach another route. So a value that is `.` or `..`, or
//! a catch-all with such a part, is refused. Longer runs of dots are not
//! steps of a path, and are encoded as the dispatcher encodes them.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

/// One piece of a tool's path.
#[derive(Debug, Clone)]
pub(crate) enum Piece {
    /// Literal text.
    Text(Text),
    /// A parameter filled from the argument of its name.
    Param(Param),
}

/// A piece as the page writes it, an object: `{text}` or
/// `{param, optional, catchAll}`.
/// A key this reader does not know is ignored, so a later release can add
/// one without breaking a front end compiled earlier; a change an older
/// reader must not ignore bumps the payload's `v`. A known key of the wrong
/// type, a piece that is both or neither, and a parameter without its two
/// flags are refused.
impl<'de> Deserialize<'de> for Piece {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            #[serde(default, deserialize_with = "present")]
            text: Option<String>,
            #[serde(default, deserialize_with = "present")]
            param: Option<String>,
            #[serde(default, deserialize_with = "present")]
            optional: Option<bool>,
            #[serde(default, deserialize_with = "present")]
            catch_all: Option<bool>,
        }
        let raw: Raw = crate::server::object(deserializer)?;
        match (raw.text, raw.param) {
            (Some(text), None) => Ok(Self::Text(Text { text })),
            (None, Some(param)) => match (raw.optional, raw.catch_all) {
                (Some(optional), Some(catch_all)) => Ok(Self::Param(Param {
                    param,
                    optional,
                    catch_all,
                })),
                _ => Err(D::Error::custom(
                    "a parameter piece needs `optional` and `catchAll`",
                )),
            },
            _ => Err(D::Error::custom(
                "a path piece is either text or a parameter",
            )),
        }
    }
}

/// A key that is present, so `null` is the wrong type rather than missing.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// Literal text of a path.
#[derive(Debug, Clone)]
pub(crate) struct Text {
    /// The text, already in URL form.
    pub(crate) text: String,
}

/// A parameter of a path.
#[derive(Debug, Clone)]
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
                        if steps(&value, param.catch_all) {
                            return Err(format!(
                                "the path parameter `{name}` cannot be `.` or `..`, or hold one \
                                 as a part, because a browser resolves it as a step of the path"
                            ));
                        }
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

/// For a GET call, every scalar argument as a query string in argument
/// order, path arguments included: a form request on GET reads its whole
/// input from the query, and a tool's input holds its path parameters. A
/// path argument that filled no segment, empty or null, stays out, as a
/// null argument does. A structured argument is refused, because a browser
/// sends no body with a GET.
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
        match scalar(Some(value)) {
            Ok(Some(text)) if text.is_empty() && in_path(name) => {}
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

/// Whether `value`, or for a catch-all one of its parts, is `.` or `..`,
/// which a browser resolves as a step of the path.
fn steps(value: &str, catch_all: bool) -> bool {
    let step = |part: &str| part == "." || part == "..";
    if catch_all {
        value.split('/').any(step)
    } else {
        step(value)
    }
}

/// `value` as one path segment, or for a catch-all as segments each
/// encoded on its own. A segment of dots alone, three or more once `.`
/// and `..` are refused, is encoded too, as the dispatcher encodes it.
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
