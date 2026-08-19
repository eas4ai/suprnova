//! Validation failure → `303` redirect-back bridge for Inertia visits.
//!
//! The Inertia client treats a response with no `X-Inertia` header as
//! non-Inertia (`inertia-3.6.1/packages/core/src/response.ts:68,173-175`)
//! and hands it to `dialog.show(...)` — the full-screen error modal
//! (`response.ts:168-169`). A `422` body therefore never reaches
//! `form.errors`, so a failed `useForm().post()` shows a crash screen
//! unless the handler redirects by hand.
//!
//! Laravel gets this right by content negotiation: Inertia sends
//! `Accept: text/html, application/xhtml+xml` (`request.ts:168`), so
//! `$request->expectsJson()` is false and a `ValidationException` takes
//! the `redirect()->back()->withErrors()` branch. This is that branch,
//! made explicit.

use crate::http::{Redirect, Request, Response};
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;

/// Middleware that turns a validation `422` on an Inertia visit into a
/// `303 See Other` back to the form page, with the errors flashed for
/// the destination's `errors` prop.
pub struct InertiaValidationRedirectMiddleware;

impl InertiaValidationRedirectMiddleware {
    /// Build a new `InertiaValidationRedirectMiddleware`. Stateless — no
    /// arguments needed.
    pub fn new() -> Self {
        Self
    }
}

impl Default for InertiaValidationRedirectMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for InertiaValidationRedirectMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Capture everything needed before `next` consumes the request.
        let is_inertia = request.is_inertia();
        let bag = request
            .header("X-Inertia-Error-Bag")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("default")
            .to_string();
        let target = back_target(
            request.header("Referer"),
            request.header("Host"),
            &current_path_and_query(&request),
        );

        let response = next(request).await;
        if !is_inertia {
            return response;
        }

        let was_ok = response.is_ok();
        let http = response.unwrap_or_else(|e| e);
        let passthrough = |http| if was_ok { Ok(http) } else { Err(http) };

        // `is_streaming` first: a stream reports an empty buffered body,
        // so there is nothing to parse. `Precognition` marks a dry-run
        // whose whole contract is that the client reads these very
        // errors off this response.
        if http.status_code() != 422
            || http.is_streaming()
            || http.header_value("Precognition").is_some()
        {
            return passthrough(http);
        }
        let Some(errors) = validation_errors_from_body(http.body()) else {
            return passthrough(http);
        };

        // `errors.<bag>` is the key `Redirect::with_errors_bag` writes and
        // the only one `SessionData::pull_errors_flash` drains. The parsed
        // JSON goes straight through rather than back via the
        // `(field, message)` string-pair API, which would flatten it.
        Redirect::to(target)
            .status(303)
            .with(format!("errors.{bag}"), errors)
            .into()
    }
}

/// The request's own path plus query — the last-resort redirect target.
fn current_path_and_query(request: &Request) -> String {
    match request.query() {
        Some(q) if !q.is_empty() => format!("{}?{}", request.path(), q),
        _ => request.path().to_string(),
    }
}

/// Resolve where the `303` points.
///
/// Mirrors Laravel's `UrlGenerator::previous()`: `Referer` first, the
/// session's recorded previous URL second. [`Redirect::back`] cannot be
/// used — it consults only the session, and `SessionMiddleware` writes
/// `_previous.url` for non-Inertia GETs only, so in a pure Inertia SPA
/// that value is whatever page was last hard-loaded rather than the form
/// just submitted. The final fallback is the failing request's own URL,
/// which for the common `GET /login` + `POST /login` pair is exactly
/// right and never worse than dropping the user on `/`.
fn back_target(referer: Option<&str>, host: Option<&str>, current: &str) -> String {
    if let Some(from_referer) = referer.and_then(|r| same_origin_path(r, host)) {
        return from_referer;
    }
    crate::session::session()
        .and_then(|s| s.previous_url())
        .unwrap_or_else(|| current.to_string())
}

/// Reduce a `Referer` to a root-relative, same-origin path, or reject it.
///
/// The value lands in a `Location` header and `Referer` is client-set, so
/// an unchecked pass-through is an open redirect. Two forms are accepted:
/// a path already rooted at `/` (never the protocol-relative `//host`
/// form, which a browser reads as absolute, and never a leading `/\`
/// either — the WHATWG URL parser treats `\` the same as `/` for special
/// schemes, so `/\evil.test` normalizes to `//evil.test` in exactly the
/// browsers this redirect targets), and an absolute URL whose authority
/// equals the request's `Host`. Everything else falls through.
fn same_origin_path(referer: &str, host: Option<&str>) -> Option<String> {
    let referer = referer.trim();
    if referer.is_empty() {
        return None;
    }
    if let Some(rest) = referer.strip_prefix('/') {
        return if rest.starts_with('/') || rest.starts_with('\\') {
            None
        } else {
            Some(referer.to_string())
        };
    }
    let uri: hyper::Uri = referer.parse().ok()?;
    if !uri.authority()?.as_str().eq_ignore_ascii_case(host?) {
        return None;
    }
    let path = uri.path();
    if !path.starts_with('/') {
        return None;
    }
    Some(match uri.query() {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    })
}

/// Pull a populated `errors` object out of a `422` body.
///
/// The gate is the body SHAPE: a domain error someone gave status 422 has
/// no per-field detail to flash and no form to bounce to, so it keeps its
/// JSON.
fn validation_errors_from_body(body: &[u8]) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_slice(body).ok()?;
    let errors = parsed.get("errors")?;
    if !errors.is_object() || errors.as_object()?.is_empty() {
        return None;
    }
    Some(errors.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_same_origin_referer_survives_sanitisation() {
        assert_eq!(
            same_origin_path("https://app.test/register?step=2", Some("app.test")),
            Some("/register?step=2".to_string()),
        );
        assert_eq!(
            same_origin_path("/posts/create", Some("app.test")),
            Some("/posts/create".to_string()),
        );
        // Otherwise any site could steer our `Location` by linking here.
        assert_eq!(
            same_origin_path("https://evil.test/phish", Some("app.test")),
            None
        );
        // `//evil.test/x` is a `Location` a browser reads as absolute.
        assert_eq!(same_origin_path("//evil.test/x", Some("app.test")), None);
        // `/\evil.test` is the same bypass in disguise: the WHATWG URL
        // parser folds a backslash into a slash for special schemes, so
        // a browser normalizes this to `//evil.test` before navigating.
        assert_eq!(same_origin_path("/\\evil.test", Some("app.test")), None);
        assert_eq!(same_origin_path("   ", Some("app.test")), None);
    }

    #[test]
    fn no_usable_referer_falls_back_to_the_current_url() {
        // No session scope in a unit test, so this exercises the last leg.
        assert_eq!(back_target(None, Some("app.test"), "/login"), "/login");
        assert_eq!(
            back_target(Some("garbage"), Some("app.test"), "/login"),
            "/login"
        );
    }

    #[test]
    fn a_422_without_a_populated_errors_object_is_not_a_validation_failure() {
        assert!(validation_errors_from_body(br#"{"message":"nope"}"#).is_none());
        assert!(validation_errors_from_body(br#"{"errors":{}}"#).is_none());
        assert!(validation_errors_from_body(b"not json").is_none());
        assert!(validation_errors_from_body(br#"{"errors":{"a":["b"]}}"#).is_some());
    }
}
