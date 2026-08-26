//! Framework error responses → the app's Inertia error page.
//!
//! The Inertia client treats a response without an `X-Inertia` header as
//! non-Inertia (`inertia-3.6.1/packages/core/src/response.ts:68,173-175`)
//! and hands it to `dialog.show(...)` - the full-screen "All Inertia
//! requests must receive a valid Inertia response, however a plain JSON
//! response was received" modal (`response.ts:168-169`). Every non-2xx
//! the framework produces takes that path: the `403` from
//! [`PermissionMiddleware`](crate::PermissionMiddleware) or
//! [`Gate::authorize`](crate::Gate), the `404` for an unrouted path, a
//! `429`, a `500`. A user with the wrong role clicks a nav link and gets
//! a crash screen instead of a page.
//!
//! Laravel + Inertia solve this in the exception handler: render an
//! app-defined `Error` component with the status as a prop, keeping the
//! status code. The conversion cannot live in
//! `From<FrameworkError> for HttpResponse` here, because that impl has no
//! request in scope and the answer depends entirely on who asked - an
//! Inertia visit wants a page object, a hard navigation wants the HTML
//! shell, and an API client wants the JSON it has always had. So it lives
//! in a middleware, the same way
//! [`InertiaValidationRedirectMiddleware`](crate::InertiaValidationRedirectMiddleware)
//! post-processes a `422`.
//!
//! [`Inertia::install`](crate::Inertia::install) registers this only when
//! [`InertiaConfig::error_page`](crate::InertiaConfig::error_page) names a
//! component, so an app that has not opted in runs exactly the code it
//! ran before.

use async_trait::async_trait;
use serde_json::Value;

use crate::http::{Request, Response};
use crate::middleware::{Middleware, Next};

use super::InertiaRequestExt;
use super::InertiaResponse;

/// Rewrites framework error responses into an Inertia page response for
/// the configured error component, keeping the original status code.
///
/// See the module documentation for why this is a middleware and not a
/// branch inside the error-to-response conversion.
pub struct InertiaErrorPageMiddleware {
    component: String,
}

impl InertiaErrorPageMiddleware {
    /// Build the middleware for a page component name (e.g. `"Error"`).
    ///
    /// Prefer [`InertiaConfig::error_page`](crate::InertiaConfig::error_page)
    /// plus [`Inertia::install`](crate::Inertia::install) - that wires this
    /// into the chain in the right place. Construct it directly only when
    /// assembling the Inertia layer by hand.
    pub fn new(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
        }
    }
}

#[async_trait]
impl Middleware for InertiaErrorPageMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Decide the audience before `next` consumes the request, and
        // capture the request only for an audience that could actually
        // receive a page. An API client - the common case for a service
        // that also serves an SPA - pays two header lookups and nothing
        // else.
        let captured = match audience(&request) {
            Audience::Neither => None,
            _ => Some(CapturedRequest::capture(&request)),
        };

        let response = next(request).await;
        let was_ok = response.is_ok();
        let http = response.unwrap_or_else(|e| e);
        let restore = |http| if was_ok { Ok(http) } else { Err(http) };

        let Some(captured) = captured else {
            return restore(http);
        };

        // Scoped so the borrow of `http` ends before it is either
        // returned untouched or dropped in favour of the page.
        let decision = {
            let facts = ErrorResponseFacts {
                status: http.status_code(),
                is_inertia_request: captured.is_inertia(),
                accept: captured.header("Accept"),
                response_is_inertia_page: http
                    .header_values("X-Inertia")
                    .any(|v| v.eq_ignore_ascii_case("true")),
                has_inertia_location: http.header_value("X-Inertia-Location").is_some(),
                is_streaming: http.is_streaming(),
                content_type: http.header_value("Content-Type"),
                body: http.body(),
            };
            decide(&facts)
        };

        let ErrorPageDecision::Render(props) = decision else {
            return restore(http);
        };

        // The replaced response's headers come along except the ones
        // that only described the body being replaced - see
        // `header_survives_rewrite` for the rule and why it is phrased as
        // a drop list. This is what keeps `Retry-After` on a `429` and
        // `WWW-Authenticate` on a `401` true after the body becomes a
        // page.
        let carried: Vec<(String, String)> = http
            .headers()
            .filter(|(name, _)| header_survives_rewrite(name))
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();

        let status = props.status;
        let mut page = InertiaResponse::new(self.component.clone())
            .with("status", props.status)
            .with("message", props.message);
        if let Some(request_id) = props.request_id {
            page = page.with("request_id", request_id);
        }

        match page.resolve(&captured).await {
            Ok(rendered) => restore(rendered.status(status).with_headers(carried)),
            Err(e) => {
                // The error page failing is not a reason to lose the
                // error. Returning the original response means the user
                // sees the modal again - bad, but recoverable and
                // truthful - rather than a second failure masking the
                // first.
                tracing::warn!(
                    component = %self.component,
                    status,
                    request_id = ?crate::logging::current_request_id(),
                    error = %e,
                    "Inertia error page failed to render; returning the original error response"
                );
                restore(http)
            }
        }
    }
}

/// Who the response is for. Only these two audiences can be handed a
/// page instead of the body they would otherwise get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Audience {
    /// An Inertia XHR visit (`X-Inertia: true`).
    InertiaVisit,
    /// A hard browser navigation: no `X-Inertia`, and an `Accept` that
    /// prefers HTML over JSON.
    BrowserNavigation,
    /// Everything else - an API client, a webhook, a health probe.
    Neither,
}

fn audience<R: InertiaRequestExt + ?Sized>(request: &R) -> Audience {
    if request.is_inertia() {
        Audience::InertiaVisit
    } else if prefers_html_over_json(request.header("Accept")) {
        Audience::BrowserNavigation
    } else {
        Audience::Neither
    }
}

/// The parts of the request the page render needs, captured before
/// `next` takes ownership.
///
/// The whole header map comes along rather than a hand-picked subset: an
/// app's [`InertiaSharedData`](crate::InertiaSharedData) provider is
/// handed this value and may read any header it likes, and a shared prop
/// that silently sees fewer headers on the error page than on every
/// other page would be a trap. Cloning a `HeaderMap` is one table
/// allocation plus refcount bumps on `Bytes`-backed values.
struct CapturedRequest {
    path: String,
    path_and_query: String,
    headers: hyper::HeaderMap,
}

impl CapturedRequest {
    fn capture(request: &Request) -> Self {
        Self {
            path: crate::http::Request::path(request).to_string(),
            path_and_query: InertiaRequestExt::path_and_query(request),
            headers: request.headers().clone(),
        }
    }
}

impl InertiaRequestExt for CapturedRequest {
    fn path(&self) -> &str {
        &self.path
    }

    fn path_and_query(&self) -> String {
        self.path_and_query.clone()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// Props the error component receives.
#[derive(Debug, PartialEq, Eq)]
struct ErrorPageProps {
    status: u16,
    message: String,
    request_id: Option<String>,
}

/// Outcome of the one decision this middleware makes.
#[derive(Debug, PartialEq, Eq)]
enum ErrorPageDecision {
    /// Leave the response exactly as the rest of the framework built it.
    PassThrough,
    /// Replace it with the error page carrying these props.
    Render(ErrorPageProps),
}

/// Everything the decision reads, lifted off `Request` and
/// `HttpResponse` so the rule set is a pure function.
struct ErrorResponseFacts<'a> {
    status: u16,
    is_inertia_request: bool,
    accept: Option<&'a str>,
    response_is_inertia_page: bool,
    has_inertia_location: bool,
    is_streaming: bool,
    content_type: Option<&'a str>,
    body: &'a [u8],
}

/// The whole rule set, in one place.
///
/// Every arm that returns [`ErrorPageDecision::PassThrough`] is a
/// contract somebody else already owns; taking the response from them
/// would break the thing they exist to do.
fn decide(facts: &ErrorResponseFacts<'_>) -> ErrorPageDecision {
    // Only client and server errors. This is also what leaves `302`
    // and every other redirect alone.
    if !(400..=599).contains(&facts.status) {
        return ErrorPageDecision::PassThrough;
    }
    // A streaming body reports an empty buffered slice because nothing
    // has been produced yet - there is no body here to judge, and
    // replacing a stream mid-flight would truncate it.
    if facts.is_streaming {
        return ErrorPageDecision::PassThrough;
    }
    // Already a valid Inertia response: a handler that rendered its own
    // page and gave it an error status.
    if facts.response_is_inertia_page {
        return ErrorPageDecision::PassThrough;
    }
    // `X-Inertia-Location` is a client instruction to do a full-page
    // visit - the version middleware's `409`, and the RBAC middlewares'
    // `redirect_to` denial. The client acts on the header, not the body.
    if facts.has_inertia_location {
        return ErrorPageDecision::PassThrough;
    }
    // `422` belongs to `InertiaValidationRedirectMiddleware`, which
    // bounces it back to the form with the errors flashed. A `422` that
    // survives that middleware did so deliberately (no `errors` object,
    // or a Precognition dry-run whose contract is that the client reads
    // the errors off this very response).
    if facts.status == 422 {
        return ErrorPageDecision::PassThrough;
    }
    // An API client keeps the JSON contract it has always had.
    if !facts.is_inertia_request && !prefers_html_over_json(facts.accept) {
        return ErrorPageDecision::PassThrough;
    }

    let Some(body) = replaceable_body(facts.content_type, facts.body) else {
        return ErrorPageDecision::PassThrough;
    };

    let (message, request_id) = match body {
        // The framework's own error bodies are `{ message, request_id }`,
        // with `message` already sanitized for `5xx` - the page must show
        // the same string the JSON path would have.
        ReplaceableBody::FrameworkError(fields) => (
            fields
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string),
            fields
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        ReplaceableBody::Empty | ReplaceableBody::RouterNotFound => (None, None),
    };

    ErrorPageDecision::Render(ErrorPageProps {
        status: facts.status,
        message: message.unwrap_or_else(|| reason_phrase(facts.status)),
        request_id,
    })
}

/// Whether a header on the replaced response carries over onto the page.
///
/// One rule, stated as what is **dropped** rather than what is kept, so a
/// header nobody here thought of survives instead of silently
/// disappearing. A field is dropped only when it describes the
/// representation being replaced - or how that representation was framed
/// on the wire - or when the page response is the authority on it:
///
/// - **Every `Content-*` field.** `Content-Length: 94` on a four-kilobyte
///   HTML shell is a framing bug, `Content-Type: application/json` on a
///   page object is a lie, and `Content-Encoding: gzip` claims a
///   compression that was never applied to the new body. The test is a
///   prefix rather than an enumeration on purpose: a representation field
///   added to HTTP after this was written is dropped by default, which is
///   the safe direction for metadata that describes bytes we threw away.
/// - **`Transfer-Encoding`**, the framing counterpart of
///   `Content-Length`, which shares none of that prefix.
/// - **`X-Inertia`.** Whether a response is an Inertia response is the
///   page response's own claim to make. Unreachable in practice - a
///   response already carrying it never reaches the rewrite, see
///   [`decide`] - and stated anyway so the rewrite cannot inherit a
///   contradictory claim.
///
/// `Content-Security-Policy` and `Content-Security-Policy-Report-Only`
/// are the one carve-out from the prefix. They are response policy, not
/// representation metadata - the shared prefix is a historical accident -
/// and dropping a CSP from an error page would be a security regression.
///
/// Everything else describes the request, the connection, or what the
/// client should do next: `Retry-After` on a `429`, `WWW-Authenticate` on
/// a `401`, `Cache-Control`, `Vary`, `Set-Cookie`, `X-Request-Id`. None of
/// that stopped being true because the body changed.
fn header_survives_rewrite(name: &str) -> bool {
    const CONTENT: &[u8] = b"Content-";
    let bytes = name.as_bytes();
    let content_prefixed =
        bytes.len() >= CONTENT.len() && bytes[..CONTENT.len()].eq_ignore_ascii_case(CONTENT);
    let security_policy = name.eq_ignore_ascii_case("Content-Security-Policy")
        || name.eq_ignore_ascii_case("Content-Security-Policy-Report-Only");

    !((content_prefixed && !security_policy)
        || name.eq_ignore_ascii_case("Transfer-Encoding")
        || name.eq_ignore_ascii_case("X-Inertia"))
}

/// Body shapes an error page may stand in for.
enum ReplaceableBody {
    /// No body at all.
    Empty,
    /// The framework's standard error envelope.
    FrameworkError(serde_json::Map<String, Value>),
    /// The fixed `404` the router and the static-file handler emit when
    /// nothing matched.
    RouterNotFound,
}

/// Classify the body, or refuse to touch it.
///
/// The gate is deliberately narrow. Anything a handler authored itself -
/// its own HTML error page, its own JSON envelope - is a considered
/// answer, and replacing it would be the framework overruling the app.
fn replaceable_body(content_type: Option<&str>, body: &[u8]) -> Option<ReplaceableBody> {
    if body.is_empty() {
        return Some(ReplaceableBody::Empty);
    }
    let content_type = content_type.unwrap_or("");
    if content_type.starts_with("application/json") {
        let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(body) else {
            return None;
        };
        // `message` is what makes it the framework's envelope rather
        // than some other JSON that happens to carry an error status.
        if !fields.get("message").is_some_and(Value::is_string) {
            return None;
        }
        return Some(ReplaceableBody::FrameworkError(fields));
    }
    if content_type.starts_with("text/plain") && body == crate::http::NOT_FOUND_BODY.as_bytes() {
        return Some(ReplaceableBody::RouterNotFound);
    }
    None
}

/// Whether the caller would rather have a page than a JSON document.
///
/// A hard navigation from a browser sends
/// `text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8`, so
/// HTML outranks JSON. `curl` sends `*/*`, which ranks them equally - and
/// equal is not "prefers", so an unopinionated client keeps its JSON.
/// An explicit `Accept: application/json` never reaches here at all.
fn prefers_html_over_json(accept: Option<&str>) -> bool {
    let Some(accept) = accept else {
        return false;
    };
    quality_for(accept, "text", "html") > quality_for(accept, "application", "json")
}

/// Quality assigned to one media type by an `Accept` header, as
/// hundredths so the result is an integer and orders exactly.
///
/// RFC 9110 §12.5.1 resolves a type against the **most specific**
/// matching range, not the highest-scoring one: given
/// `*/*;q=0.8, text/html;q=0.1`, `text/html` is 0.1. Returns `0` when
/// nothing matches.
fn quality_for(accept: &str, media_type: &str, subtype: &str) -> u16 {
    // Higher is more specific: exact type/subtype, then type/*, then */*.
    let mut best_specificity = -1i8;
    let mut best_quality = 0u16;

    for range in accept.split(',') {
        let mut parts = range.split(';');
        let Some(name) = parts.next().map(str::trim) else {
            continue;
        };
        let (range_type, range_subtype) = match name.split_once('/') {
            Some(pair) => pair,
            None => continue,
        };
        let specificity = if range_type.eq_ignore_ascii_case(media_type)
            && range_subtype.eq_ignore_ascii_case(subtype)
        {
            2
        } else if range_type.eq_ignore_ascii_case(media_type) && range_subtype == "*" {
            1
        } else if range_type == "*" && range_subtype == "*" {
            0
        } else {
            continue;
        };
        if specificity < best_specificity {
            continue;
        }
        let quality = parts
            .filter_map(|param| {
                let (key, value) = param.split_once('=')?;
                key.trim().eq_ignore_ascii_case("q").then_some(value.trim())
            })
            .next()
            .map_or(100, parse_quality);
        // A later range at the same specificity wins ties, which only
        // matters for a malformed header listing the same range twice.
        best_specificity = specificity;
        best_quality = quality;
    }

    best_quality
}

/// Parse an RFC 9110 qvalue (`0`..`1` with up to three decimals) into
/// hundredths. Anything unparseable reads as `0`, matching the spec's
/// "a sender that does not want the type" default for a broken value.
fn parse_quality(raw: &str) -> u16 {
    raw.parse::<f32>()
        .ok()
        .filter(|q| (0.0..=1.0).contains(q))
        .map_or(0, |q| (q * 100.0).round() as u16)
}

/// The status's reason phrase, for a body that carried no message.
fn reason_phrase(status: u16) -> String {
    hyper::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("Error")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(status: u16, content_type: &'a str, body: &'a [u8]) -> ErrorResponseFacts<'a> {
        ErrorResponseFacts {
            status,
            is_inertia_request: true,
            accept: None,
            response_is_inertia_page: false,
            has_inertia_location: false,
            is_streaming: false,
            content_type: Some(content_type),
            body,
        }
    }

    const UNAUTHORIZED: &[u8] =
        br#"{"message":"This action is unauthorized.","request_id":"b4020e5d"}"#;

    #[test]
    fn a_framework_denial_becomes_the_page_with_its_message_and_request_id() {
        assert_eq!(
            decide(&facts(403, "application/json", UNAUTHORIZED)),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 403,
                message: "This action is unauthorized.".to_string(),
                request_id: Some("b4020e5d".to_string()),
            })
        );
    }

    #[test]
    fn a_body_without_a_message_falls_back_to_the_reason_phrase() {
        // The router's own unrouted 404, and a body-less error.
        assert_eq!(
            decide(&facts(404, "text/plain", b"404 Not Found")),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 404,
                message: "Not Found".to_string(),
                request_id: None,
            })
        );
        assert_eq!(
            decide(&facts(429, "", b"")),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 429,
                message: "Too Many Requests".to_string(),
                request_id: None,
            })
        );
    }

    #[test]
    fn a_null_request_id_does_not_become_the_string_null() {
        // Outside a request scope the framework serializes `request_id`
        // as JSON null. The prop must be absent, not `"null"`.
        assert_eq!(
            decide(&facts(
                500,
                "application/json",
                br#"{"message":"Internal Server Error","request_id":null}"#
            )),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 500,
                message: "Internal Server Error".to_string(),
                request_id: None,
            })
        );
    }

    #[test]
    fn everything_another_contract_owns_passes_through() {
        // Success and redirects.
        assert_eq!(
            decide(&facts(200, "text/html", b"ok")),
            ErrorPageDecision::PassThrough
        );
        assert_eq!(decide(&facts(302, "", b"")), ErrorPageDecision::PassThrough);
        // Validation belongs to the redirect-back middleware.
        assert_eq!(
            decide(&facts(
                422,
                "application/json",
                br#"{"message":"The given data was invalid.","errors":{"email":["r"]}}"#
            )),
            ErrorPageDecision::PassThrough
        );

        // A response that already is an Inertia page.
        let mut already = facts(410, "application/json", br#"{"component":"Gone"}"#);
        already.response_is_inertia_page = true;
        assert_eq!(decide(&already), ErrorPageDecision::PassThrough);

        // A version-mismatch bounce, or an RBAC `redirect_to` denial.
        let mut bounce = facts(409, "", b"");
        bounce.has_inertia_location = true;
        assert_eq!(decide(&bounce), ErrorPageDecision::PassThrough);

        // A streaming body has produced nothing yet to inspect.
        let mut streaming = facts(500, "text/event-stream", b"");
        streaming.is_streaming = true;
        assert_eq!(decide(&streaming), ErrorPageDecision::PassThrough);
    }

    #[test]
    fn a_body_the_app_authored_is_never_overruled() {
        // A handler's own HTML error page.
        assert_eq!(
            decide(&facts(404, "text/html; charset=utf-8", b"<h1>gone</h1>")),
            ErrorPageDecision::PassThrough
        );
        // A handler's own JSON envelope, in some other shape.
        assert_eq!(
            decide(&facts(
                402,
                "application/json",
                br#"{"error":"payment_required"}"#
            )),
            ErrorPageDecision::PassThrough
        );
        // A plain-text body that is not the router's fixed 404.
        assert_eq!(
            decide(&facts(404, "text/plain", b"no such widget")),
            ErrorPageDecision::PassThrough
        );
        // JSON that does not parse.
        assert_eq!(
            decide(&facts(500, "application/json", b"{not json")),
            ErrorPageDecision::PassThrough
        );
    }

    #[test]
    fn an_api_client_keeps_its_json_and_a_browser_gets_the_page() {
        let json_client = ErrorResponseFacts {
            is_inertia_request: false,
            accept: Some("application/json"),
            ..facts(403, "application/json", UNAUTHORIZED)
        };
        assert_eq!(decide(&json_client), ErrorPageDecision::PassThrough);

        let browser = ErrorResponseFacts {
            is_inertia_request: false,
            accept: Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ..facts(403, "application/json", UNAUTHORIZED)
        };
        assert!(matches!(decide(&browser), ErrorPageDecision::Render(_)));

        // No `Accept` at all is not a browser navigation.
        let bare = ErrorResponseFacts {
            is_inertia_request: false,
            accept: None,
            ..facts(403, "application/json", UNAUTHORIZED)
        };
        assert_eq!(decide(&bare), ErrorPageDecision::PassThrough);
    }

    #[test]
    fn content_negotiation_follows_the_most_specific_range() {
        // A real browser navigation.
        assert!(prefers_html_over_json(Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        )));
        assert!(prefers_html_over_json(Some("text/html")));
        assert!(prefers_html_over_json(Some("text/*")));
        // `curl` and every other unopinionated client: equal, not
        // preferred.
        assert!(!prefers_html_over_json(Some("*/*")));
        assert!(!prefers_html_over_json(Some("application/json")));
        assert!(!prefers_html_over_json(Some("application/json, */*;q=0.1")));
        assert!(!prefers_html_over_json(None));
        // The specific range wins over the wildcard even when the
        // wildcard scores higher - RFC 9110 12.5.1.
        assert!(!prefers_html_over_json(Some("*/*;q=0.8, text/html;q=0.1")));
        // The Inertia client's own `Accept` prefers HTML, which is why
        // Laravel's `expectsJson()` is false for an Inertia visit.
        assert!(prefers_html_over_json(Some(
            "text/html, application/xhtml+xml"
        )));
        // Junk must not panic or accidentally prefer anything.
        assert!(!prefers_html_over_json(Some("")));
        assert!(!prefers_html_over_json(Some("garbage")));
        assert!(!prefers_html_over_json(Some("text/html;q=nope")));
    }

    #[test]
    fn only_what_described_the_replaced_body_is_dropped() {
        // Framing and representation metadata for a body that no longer
        // exists.
        for dropped in [
            "Content-Type",
            "content-length",
            "Content-Encoding",
            "Content-Language",
            "Content-Location",
            "Content-Range",
            "Content-Disposition",
            "Transfer-Encoding",
            "X-Inertia",
        ] {
            assert!(
                !header_survives_rewrite(dropped),
                "{dropped} describes the body being replaced and must not carry over"
            );
        }

        // Anything about the request, the connection, or what the client
        // does next is still true after the body changes.
        for kept in [
            "Retry-After",
            "WWW-Authenticate",
            "Cache-Control",
            "Vary",
            "Set-Cookie",
            "X-Request-Id",
            "Access-Control-Allow-Origin",
            "Strict-Transport-Security",
            "Precognition",
            "X-Inertia-Location",
        ] {
            assert!(
                header_survives_rewrite(kept),
                "{kept} says nothing about the body and must carry over"
            );
        }
    }

    #[test]
    fn the_security_policy_headers_are_not_content_headers() {
        // They share the prefix by historical accident. Dropping a CSP
        // from the error page would be a security regression, so the
        // prefix rule carves them out by name.
        assert!(header_survives_rewrite("Content-Security-Policy"));
        assert!(header_survives_rewrite(
            "content-security-policy-report-only"
        ));
        // A short name that merely starts with "Content" is not prefixed
        // by "Content-" and must not be swept up.
        assert!(header_survives_rewrite("Content"));
    }

    #[test]
    fn an_unassigned_status_still_names_something() {
        assert_eq!(reason_phrase(403), "Forbidden");
        assert_eq!(reason_phrase(499), "Error");
    }
}
