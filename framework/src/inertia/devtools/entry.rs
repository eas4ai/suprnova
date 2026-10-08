//! One recorded request as the extension reads it: Laravel's
//! `IncomingEntryBuilder` and `IncomingEntry::toArray`, key for key.

use std::time::Instant;

use bytes::Bytes;
use serde_json::{Map, Value, json};

use super::recorder::RenderPayload;
use super::redact::UNSERIALIZABLE;
use crate::http::{BodyRead, HttpResponse, Request};
use crate::inertia::prop::header_is_truthy;

/// The largest textual response body an entry keeps, in bytes. A request
/// body has no limit of its own: it is read up to the framework's request
/// body cap, the most the handler could read.
pub(crate) const RESPONSE_BODY_LIMIT: usize = 256_000;

/// The request header carrying the extension's id of the browser tab.
pub(crate) const TAB_HEADER: &str = "X-Inertia-Devtools-Tab";
/// The request header carrying the id of the entry a follow-up belongs to.
pub(crate) const PARENT_HEADER: &str = "X-Inertia-Devtools-Parent";
/// The request header carrying the client's visit id.
pub(crate) const VISIT_HEADER: &str = "X-Inertia-Devtools-Visit";
/// The request header the extension sets on a polling visit.
pub(crate) const POLL_HEADER: &str = "X-Inertia-Devtools-Poll";
/// The request header the extension sets on the visit that loads deferred
/// props, which on the wire is a partial reload like any other.
pub(crate) const DEFERRED_HEADER: &str = "X-Inertia-Devtools-Deferred";

/// The facts of a request, read before the handler takes it.
#[derive(Debug)]
pub(crate) struct RequestFacts {
    started: Instant,
    method: String,
    url: String,
    headers: Map<String, Value>,
    pub(crate) is_inertia: bool,
    precognition: bool,
    deferred: bool,
    poll: bool,
    partial: bool,
    pub(crate) prefetch: bool,
    tab: Option<String>,
    parent: Option<String>,
    visit: Option<String>,
    pub(crate) base_path: String,
    route_name: Option<String>,
    route_uri: Option<String>,
    route_action: Option<String>,
    body: RequestBody,
}

/// The request body as it could be read before the handler.
#[derive(Debug)]
enum RequestBody {
    /// A write that is not an Inertia visit: its body is not kept.
    NonInertiaWrite,
    /// A body longer than the request body cap, which the handler would
    /// refuse with 413 unless it has a larger cap of its own; the request
    /// keeps it for the handler.
    TooLarge,
    /// A body that failed to arrive; the handler meets the failure when
    /// it reads the body.
    Unreadable,
    /// The body, with the content type and query it is read with.
    Read {
        content_type: Option<String>,
        bytes: Bytes,
        query: Option<String>,
    },
}

/// Whether `method` writes, which Laravel keeps the body of only for an
/// Inertia visit.
fn is_write(method: &hyper::Method) -> bool {
    matches!(
        *method,
        hyper::Method::POST | hyper::Method::PUT | hyper::Method::PATCH | hyper::Method::DELETE
    )
}

/// A request header, `None` when it is absent or empty.
fn header(request: &Request, name: &str) -> Option<String> {
    request
        .header(name)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Whether a request header holds a value PHP reads as true.
fn truthy(request: &Request, name: &str) -> bool {
    request
        .headers()
        .get(name)
        .is_some_and(|value| header_is_truthy(value.as_bytes()))
}

impl RequestFacts {
    /// Read the facts of `request`, and its body when it is kept.
    ///
    /// The handler reads a body that was read here from the copy kept on
    /// the request, as it reads one the CSRF middleware read. A body over
    /// the request body cap, and one that fails to arrive, stay on the
    /// request for the handler, which answers as it would unrecorded.
    pub(crate) async fn capture(mut request: Request) -> (Request, Self) {
        let started = Instant::now();
        let is_inertia = request.is_inertia();
        let mut headers = Map::new();
        for name in request.headers().keys() {
            let mut values = Vec::new();
            let mut text = true;
            for value in request.headers().get_all(name) {
                match std::str::from_utf8(value.as_bytes()) {
                    Ok(value) => values.push(value.to_string()),
                    Err(_) => text = false,
                }
            }
            let value = if text {
                values.join(", ")
            } else {
                UNSERIALIZABLE.to_string()
            };
            headers.insert(name.as_str().to_string(), Value::String(value));
        }
        let route_uri = request
            .route_pattern()
            .map(|pattern| format!("/{}", pattern.trim_start_matches('/')));
        let facts = Self {
            started,
            method: request.method().as_str().to_string(),
            url: request.full_url(),
            headers,
            is_inertia,
            precognition: truthy(&request, "Precognition"),
            deferred: truthy(&request, DEFERRED_HEADER),
            poll: truthy(&request, POLL_HEADER),
            partial: truthy(&request, "X-Inertia-Partial-Component"),
            prefetch: crate::inertia::visit::is_prefetch(|name| request.header(name)),
            tab: header(&request, TAB_HEADER),
            parent: header(&request, PARENT_HEADER),
            visit: header(&request, VISIT_HEADER),
            base_path: request.public_root().to_string(),
            route_name: request.route_name(),
            route_uri,
            route_action: request.route_action().map(str::to_string),
            body: RequestBody::NonInertiaWrite,
        };
        if is_write(request.method()) && !is_inertia {
            return (request, facts);
        }
        let body = read_body(&mut request).await;
        (request, Self { body, ..facts })
    }

    /// The tab the extension recorded this request in.
    pub(crate) fn tab(&self) -> Option<&str> {
        self.tab.as_deref()
    }

    /// The entry this request belongs to: the incoming parent of an
    /// Inertia visit, Laravel's `resolveLineage`.
    pub(crate) fn batch_id(&self) -> Option<&str> {
        if self.is_inertia {
            self.parent.as_deref()
        } else {
            None
        }
    }
}

/// The body of `request`, read now up to the framework's request body
/// cap, the limit the extractors enforce, whether or not its length is
/// declared; a body already read by an earlier middleware is taken from
/// the request.
async fn read_body(request: &mut Request) -> RequestBody {
    let content_type = request.content_type().map(str::to_string);
    let query = request.query().map(str::to_string);
    match request
        .read_body_up_to(crate::http::body::global_max_request_body_bytes())
        .await
    {
        BodyRead::Whole(bytes) => RequestBody::Read {
            content_type,
            bytes,
            query,
        },
        BodyRead::TooLarge => RequestBody::TooLarge,
        BodyRead::Failed => RequestBody::Unreadable,
    }
}

/// `{"status": "present", "value": ...}`.
fn present(value: Value) -> Value {
    json!({"status": "present", "value": value})
}

/// `{"status": "omitted", "reason": ...}`.
fn omitted(reason: &str) -> Value {
    json!({"status": "omitted", "reason": reason})
}

/// `{"status": "empty"}`.
fn empty() -> Value {
    json!({"status": "empty"})
}

/// A raw body as text, Laravel's `captureBodyString`.
fn body_string(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return empty();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => present(Value::String(text.to_string())),
        Err(_) => omitted("binary"),
    }
}

/// Whether `content_type` names JSON (`/json` or `+json`).
fn is_json(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| {
        let ct = ct.to_ascii_lowercase();
        ct.contains("/json") || ct.contains("+json")
    })
}

/// Merge the members of `from` into `into`, `from` winning.
fn merge_into(into: &mut Map<String, Value>, from: Value) {
    if let Value::Object(map) = from {
        for (key, value) in map {
            into.insert(key, value);
        }
    }
}

/// The parts of a multipart body: text parts as text, a part that is not
/// text as `[UNSERIALIZABLE]`, and a file part as its name, size and MIME
/// type, never its bytes. Names are kept as sent.
async fn multipart_input(content_type: &str, bytes: Bytes) -> Option<Map<String, Value>> {
    let boundary = multer::parse_boundary(content_type).ok()?;
    let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(bytes) });
    let mut multipart = multer::Multipart::new(stream, boundary);
    let mut input = Map::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        let file_name = field.file_name().map(str::to_string);
        let mime = field.content_type().map(ToString::to_string);
        let data = field.bytes().await.ok()?;
        let value = match file_name {
            Some(file_name) => json!({"name": file_name, "size": data.len(), "mimeType": mime}),
            None => match String::from_utf8(data.to_vec()) {
                Ok(text) => Value::String(text),
                Err(_) => Value::String(UNSERIALIZABLE.to_string()),
            },
        };
        input.insert(name, value);
    }
    Some(input)
}

/// The request body an entry carries, Laravel's `captureRequestBody`: the
/// JSON body; else the query and form input, uploads summarized; else the
/// raw text. Redaction runs on the whole entry before it is stored.
async fn request_body(body: RequestBody) -> Value {
    let (content_type, bytes, query) = match body {
        RequestBody::NonInertiaWrite => return omitted("non-inertia-request"),
        RequestBody::TooLarge => return omitted("too-large"),
        RequestBody::Unreadable => return omitted("unreadable"),
        RequestBody::Read {
            content_type,
            bytes,
            query,
        } => (content_type, bytes, query),
    };
    // A JSON body is present whatever it parses to, `[]` and `null`
    // included; one that does not parse is kept as its text, and only a
    // body of no bytes is `empty`.
    if is_json(content_type.as_deref()) {
        return match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => present(value),
            Err(_) => body_string(&bytes),
        };
    }
    let mut input = Map::new();
    if let Some(query) = query.as_deref().filter(|query| !query.is_empty())
        && let Ok(parsed) =
            crate::http::parse_form::<Value>(&Bytes::copy_from_slice(query.as_bytes()))
    {
        merge_into(&mut input, parsed);
    }
    let lowered = content_type.as_deref().map(str::to_ascii_lowercase);
    match lowered.as_deref() {
        Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
            if let Ok(parsed) = crate::http::parse_form::<Value>(&bytes) {
                merge_into(&mut input, parsed);
            }
        }
        Some(ct) if ct.starts_with("multipart/form-data") => {
            if let Some(parts) =
                multipart_input(content_type.as_deref().unwrap_or(ct), bytes.clone()).await
            {
                input.extend(parts);
            }
        }
        _ => {}
    }
    if !input.is_empty() {
        return present(Value::Object(input));
    }
    body_string(&bytes)
}

/// Whether `content_type` is text the entry can keep.
fn is_textual(content_type: &str) -> bool {
    ["json", "text/", "xml", "javascript"]
        .iter()
        .any(|needle| content_type.contains(needle))
}

/// The response body an entry carries, Laravel's `captureResponseBody`:
/// the page object of a rendered page; else a textual body, decoded when
/// it is JSON, of at most [`RESPONSE_BODY_LIMIT`] bytes.
fn response_body(payload: Option<&RenderPayload>, response: &HttpResponse) -> Value {
    if let Some(payload) = payload {
        return match &payload.page {
            Value::Null => empty(),
            page => present(page.clone()),
        };
    }
    let content_type = response
        .header_value("Content-Type")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !is_textual(&content_type) {
        return omitted("non-textual");
    }
    if response.is_streaming() {
        return omitted("streamed");
    }
    let body = response.body();
    if body.is_empty() {
        return empty();
    }
    if body.len() > RESPONSE_BODY_LIMIT {
        return omitted("too-large");
    }
    if content_type.contains("json")
        && let Ok(value @ (Value::Object(_) | Value::Array(_))) =
            serde_json::from_slice::<Value>(body)
    {
        return present(value);
    }
    body_string(body)
}

/// The response headers, by lower-cased name, the values of a repeated
/// header joined with `, `, as Symfony's header bag lists them.
fn response_headers(response: &HttpResponse) -> Map<String, Value> {
    let mut headers: Map<String, Value> = Map::new();
    for (name, value) in response.headers() {
        let name = name.to_ascii_lowercase();
        match headers.get_mut(&name) {
            Some(Value::String(joined)) => {
                joined.push_str(", ");
                joined.push_str(value);
            }
            _ => {
                headers.insert(name, Value::String(value.to_string()));
            }
        }
    }
    headers
}

/// Where a response redirects, Laravel's `resolveRedirectLocation`:
/// `X-Inertia-Location`, else `Location` on a `3xx`.
fn redirect_location(response: &HttpResponse) -> Option<String> {
    if let Some(location) = response
        .header_value("X-Inertia-Location")
        .filter(|location| !location.is_empty())
    {
        return Some(location.to_string());
    }
    if !(300..400).contains(&response.status_code()) {
        return None;
    }
    response
        .header_value("Location")
        .filter(|location| !location.is_empty())
        .map(str::to_string)
}

/// The kind of request, Laravel's `resolveRequestType`: a Precognition
/// request first; a request that is not an Inertia visit is `initial`
/// when it rendered a page and `http` when it did not; then the
/// extension's deferred and poll headers, a partial reload, a prefetch,
/// and a navigation.
fn request_type(facts: &RequestFacts, rendered: bool) -> &'static str {
    if facts.precognition {
        "precognition"
    } else if !facts.is_inertia {
        if rendered { "initial" } else { "http" }
    } else if facts.deferred {
        "deferred"
    } else if facts.poll {
        "poll"
    } else if facts.partial {
        "partial"
    } else if facts.prefetch {
        "prefetch"
    } else {
        "navigate"
    }
}

/// The ids an entry is stamped with and the moment it was recorded.
#[derive(Debug)]
pub(crate) struct Stamp<'a> {
    /// The entry's id.
    pub(crate) id: &'a str,
    /// The moment the response was recorded.
    pub(crate) at: chrono::DateTime<chrono::Utc>,
}

/// The entry of `facts` answered by `response`, with what the page render
/// recorded, if a page rendered.
pub(crate) async fn build(
    facts: RequestFacts,
    payload: Option<RenderPayload>,
    response: &HttpResponse,
    stamp: Stamp<'_>,
) -> Value {
    let rendered = payload
        .as_ref()
        .is_some_and(|payload| !payload.component.is_empty());
    let utime = stamp.at.timestamp_millis() as f64 / 1000.0;
    let server_timing_ms = facts.started.elapsed().as_secs_f64() * 1000.0;
    let request_type = request_type(&facts, rendered);
    let response_body = response_body(payload.as_ref(), response);
    let mut route = Map::new();
    route.insert(
        "name".to_string(),
        facts.route_name.clone().map_or(Value::Null, Value::String),
    );
    route.insert(
        "uri".to_string(),
        Value::String(facts.route_uri.clone().unwrap_or_default()),
    );
    if rendered && facts.route_uri.is_some() {
        route.insert("method".to_string(), Value::String(facts.method.clone()));
    }
    route.insert(
        "action".to_string(),
        facts
            .route_action
            .clone()
            .map_or(Value::Null, Value::String),
    );
    let batch_id = facts.batch_id().map(str::to_string);
    let RequestFacts {
        method,
        url,
        headers,
        tab,
        visit,
        body,
        ..
    } = facts;
    let request_body = request_body(body).await;
    let (component, props, prop_values, render_source, component_path) = match payload {
        Some(payload) => (
            Value::String(payload.component),
            Value::Object(payload.props),
            Value::Object(payload.prop_values),
            payload
                .render_source
                .map_or(Value::Null, super::source::SourceLocation::to_json),
            payload.component_path.map_or(Value::Null, Value::String),
        ),
        None => (
            Value::Null,
            Value::Object(Map::new()),
            Value::Object(Map::new()),
            Value::Null,
            Value::Null,
        ),
    };
    json!({
        "__meta": {
            "id": stamp.id,
            "tabUuid": tab,
            "batchId": batch_id,
            "timestamp": stamp.at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            "utime": utime,
            "method": method,
            "url": url,
            "component": component,
            "requestType": request_type,
            "status": response.status_code(),
            "redirectLocation": redirect_location(response),
            "serverTimingMs": server_timing_ms,
            "visitId": visit,
        },
        "http": {
            "requestHeaders": headers,
            "responseHeaders": response_headers(response),
            "requestBody": request_body,
            "responseBody": response_body,
        },
        "props": props,
        "propValues": prop_values,
        "route": route,
        "renderSource": render_source,
        "componentPath": component_path,
    })
}
