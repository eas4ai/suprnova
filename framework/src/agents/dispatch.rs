//! Running a tool call through the tool's route (AGENT-001).
//!
//! The MCP endpoint does not call a tool's handler directly. It builds a
//! request for the tool's route from the call's arguments and serves it
//! through the server's routing and middleware, the path a request off the
//! wire takes, so the route's middleware, bindings, form request
//! validation, `#[authorize]` checks and throttles all run.
//!
//! The request carries:
//!
//! - the arguments as a JSON body, `Content-Type: application/json` and
//!   `Accept: application/json`, with each path parameter of the route
//!   filled from the argument of its name, percent-encoded so a value cannot
//!   add a path segment, and for a `GET` route the scalar arguments in the
//!   query string as well;
//! - the client's address and the proxy headers, so `Request::ip()` is the
//!   real client;
//! - no `Cookie` and no `Authorization`: the caller's token is never
//!   forwarded (AGENT-005);
//! - the call's mark, which the CSRF, session and bearer-token middleware
//!   step aside for.
//!
//! The caller's user, if any, is signed in on the guard before the
//! middleware runs, as a test client's `acting_as` does, so `Auth::user()`,
//! `AuthMiddleware` and `#[authorize]` see them. An agent account bound to
//! no user stays a guest.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use percent_encoding::utf8_percent_encode;
use serde_json::{Map, Value};

use super::caller::ToolCaller;
use super::mark::{AgentMark, CallContext, EndpointContext};
use super::tool::RegisteredTool;
use crate::error::FrameworkError;
use crate::http::Request;
use hyper::Method;

/// The headers of the endpoint request a tool call copies, so the call sees
/// the client the endpoint saw. The cookie and the `Authorization` header
/// are never among them.
const FORWARDED_HEADERS: [&str; 9] = [
    "host",
    "user-agent",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-port",
    "x-forwarded-proto",
    "x-forwarded-prefix",
    "x-real-ip",
    "forwarded",
];

/// What the endpoint request tells a tool call about its client.
#[derive(Clone, Default)]
pub(crate) struct ClientFacts {
    /// The headers of [`FORWARDED_HEADERS`] the endpoint request carried.
    headers: Vec<(hyper::header::HeaderName, hyper::header::HeaderValue)>,
    /// The request id of the endpoint request, so the call's logs carry it.
    request_id: Option<String>,
}

impl ClientFacts {
    /// The facts of `request`, read before the endpoint reads its body.
    pub(crate) fn of(request: &Request) -> Self {
        let mut headers = Vec::new();
        for name in FORWARDED_HEADERS {
            for value in request.headers().get_all(name) {
                if let Ok(header) = hyper::header::HeaderName::from_bytes(name.as_bytes()) {
                    headers.push((header, value.clone()));
                }
            }
        }
        Self {
            headers,
            request_id: crate::logging::current_request_id().map(|id| id.as_str().to_string()),
        }
    }
}

/// The response of a tool's route, read whole.
pub(crate) struct RouteAnswer {
    /// The status.
    pub(crate) status: u16,
    /// The `Content-Type`, when the route set one.
    pub(crate) content_type: Option<String>,
    /// The body.
    pub(crate) body: Bytes,
}

/// Why a tool call did not reach an answer from its route.
pub(crate) enum CallFailure {
    /// The arguments cannot address the tool's route: a path parameter is
    /// missing or not a scalar, or the path they fill matches another route.
    BadArguments(String),
    /// The call ran longer than the configured limit and was stopped.
    TimedOut(Duration),
    /// The route's answer was larger than the configured limit.
    TooLarge(usize),
}

/// Run `tool` for `caller` with `arguments`, through the tool's route.
pub(crate) async fn call(
    endpoint: &EndpointContext,
    tool: &RegisteredTool,
    arguments: Map<String, Value>,
    caller: ToolCaller,
    client: ClientFacts,
    timeout: Duration,
    max_result_bytes: usize,
) -> Result<RouteAnswer, CallFailure> {
    let path = fill_path(tool.pattern(), &arguments).map_err(CallFailure::BadArguments)?;
    // A literal route such as `/posts/new` would otherwise answer a call
    // meant for `/posts/{post}` with `post = "new"`.
    match endpoint.router.match_route(tool.method(), &path) {
        Some((pattern, _, _)) if pattern == tool.pattern() => {}
        _ => {
            return Err(CallFailure::BadArguments(format!(
                "the arguments do not address the tool's route {} {}",
                tool.method(),
                tool.pattern()
            )));
        }
    }
    let target = match query_of(tool.method(), tool.pattern(), &arguments) {
        Some(query) => format!("{path}?{query}"),
        None => path,
    };
    let body = Bytes::from(serde_json::to_vec(&Value::Object(arguments)).unwrap_or_default());
    let request = build_request(tool, &target, body, &client).map_err(CallFailure::BadArguments)?;
    let guard = tool.guard();
    let seeded = caller.clone();
    let served = crate::server::serve_internal(
        endpoint.router.clone(),
        endpoint.registry.clone(),
        request,
        endpoint.peer_ip,
        async move { sign_in(&seeded, guard).await },
    );
    let response = match tokio::time::timeout(timeout, caller.scope(served)).await {
        Ok(response) => response,
        Err(_) => return Err(CallFailure::TimedOut(timeout)),
    };
    let (parts, body) = response.into_parts();
    let body = match Limited::new(body, max_result_bytes).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Err(CallFailure::TooLarge(max_result_bytes)),
    };
    Ok(RouteAnswer {
        status: parts.status.as_u16(),
        content_type: parts
            .headers
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        body,
    })
}

/// The request for a call of `tool` at `target`.
fn build_request(
    tool: &RegisteredTool,
    target: &str,
    body: Bytes,
    client: &ClientFacts,
) -> Result<Request, String> {
    let mut builder = hyper::Request::builder()
        .method(tool.method().clone())
        .uri(target)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::ACCEPT, "application/json");
    for (name, value) in &client.headers {
        builder = builder.header(name, value);
    }
    if let Some(id) = &client.request_id {
        builder = builder.header("x-request-id", id.as_str());
    }
    let (parts, ()) = builder
        .body(())
        .map_err(|error| format!("the tool call's request is not valid: {error}"))?
        .into_parts();
    Ok(Request::for_tool_call(
        parts,
        body,
        AgentMark::Call(CallContext { tool: tool.name() }),
    ))
}

/// Sign the caller's user in for the call, on the tool's guard or the
/// default guard, as a test client's `acting_as` does.
async fn sign_in(caller: &ToolCaller, guard: Option<&'static str>) -> Result<(), FrameworkError> {
    let Some(user) = caller.acting_user().cloned() else {
        return Ok(());
    };
    let Some(manager) = crate::App::get::<crate::auth::AuthManager>() else {
        crate::Auth::set_user(user);
        return Ok(());
    };
    let name = match guard {
        Some(name) => name.to_string(),
        None => manager.default_guard_name().to_owned(),
    };
    let resolved = manager.guard(&name).map_err(|error| {
        FrameworkError::from_external_with(
            format!("a tool call cannot sign its user in through the guard '{name}'"),
            error,
        )
    })?;
    resolved.set_user(user).await;
    if guard.is_some() {
        crate::auth::request_state::set_used_guard(&name);
    }
    Ok(())
}

/// The path of `pattern` with each placeholder filled from the argument of
/// its name, percent-encoded so a value is one segment. An optional
/// placeholder without an argument drops its segment.
pub(crate) fn fill_path(pattern: &str, arguments: &Map<String, Value>) -> Result<String, String> {
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|close| open + close) else {
            break;
        };
        let (name, _field, optional) =
            crate::routing::binding::split_placeholder(&rest[open + 1..close]);
        let catch_all = rest[open + 1..].starts_with('*');
        let value = match arguments.get(name) {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            Some(Value::Bool(flag)) => Some(flag.to_string()),
            Some(Value::Null) | None => None,
            Some(_) => {
                return Err(format!(
                    "the path parameter `{name}` must be a string, a number or a boolean"
                ));
            }
        };
        match value {
            Some(value) if !value.is_empty() => {
                out.push_str(&rest[..open]);
                out.push_str(&encode_segment(&value, catch_all));
            }
            _ if optional => {
                // Drop the placeholder and the `/` in front of it.
                out.push_str(rest[..open].strip_suffix('/').unwrap_or(&rest[..open]));
            }
            _ => return Err(format!("the path parameter `{name}` is missing")),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    if out.is_empty() {
        out.push('/');
    }
    Ok(out)
}

/// `value` as one path segment, or for a catch-all parameter as segments
/// each encoded on its own. A segment of dots alone is encoded too, so it
/// cannot read as `.` or `..`.
fn encode_segment(value: &str, catch_all: bool) -> String {
    let encode = |segment: &str| -> String {
        if !segment.is_empty() && segment.bytes().all(|byte| byte == b'.') {
            "%2E".repeat(segment.len())
        } else {
            utf8_percent_encode(segment, crate::routing::PATH_SEGMENT_ENCODE).to_string()
        }
    };
    if catch_all {
        value.split('/').map(encode).collect::<Vec<_>>().join("/")
    } else {
        encode(value)
    }
}

/// For a `GET` route, the scalar arguments that fill no path parameter, as
/// a query string, so a handler that reads the query sees them.
fn query_of(method: &Method, pattern: &str, arguments: &Map<String, Value>) -> Option<String> {
    if *method != Method::GET {
        return None;
    }
    let in_path: Vec<String> = crate::routing::binding::placeholders(pattern)
        .into_iter()
        .map(|placeholder| placeholder.name)
        .collect();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    let mut any = false;
    for (name, value) in arguments {
        if in_path.contains(name) {
            continue;
        }
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            _ => continue,
        };
        query.append_pair(name, &text);
        any = true;
    }
    any.then(|| query.finish())
}

#[cfg(test)]
mod tests {
    use super::fill_path;
    use serde_json::{Map, Value, json};

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn agent_001_path_values_stay_one_segment() {
        assert_eq!(
            fill_path("/posts/{post}", &args(json!({ "post": "a/b" }))).as_deref(),
            Ok("/posts/a%2Fb")
        );
        assert_eq!(
            fill_path("/posts/{post}", &args(json!({ "post": "%2F" }))).as_deref(),
            Ok("/posts/%252F")
        );
        assert_eq!(
            fill_path("/posts/{post}", &args(json!({ "post": ".." }))).as_deref(),
            Ok("/posts/%2E%2E")
        );
        assert_eq!(
            fill_path(
                "/posts/{post}/comments/{comment}",
                &args(json!({ "post": 7, "comment": "x y" }))
            )
            .as_deref(),
            Ok("/posts/7/comments/x%20y")
        );
    }

    #[test]
    fn agent_001_missing_or_structured_path_values_are_refused() {
        assert!(fill_path("/posts/{post}", &args(json!({}))).is_err());
        assert!(fill_path("/posts/{post}", &args(json!({ "post": { "id": 1 } }))).is_err());
        assert_eq!(
            fill_path("/posts/{post?}", &args(json!({}))).as_deref(),
            Ok("/posts")
        );
    }
}
