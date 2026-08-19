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
use crate::routing::url::{has_control_byte, root_relative_or_none};
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
/// just submitted. The final fallback is the failing request's own URL —
/// exactly right for the common `GET /login` + `POST /login` pair — run
/// through the same [`root_relative_or_none`] guard as the `Referer` leg
/// and falling back to `/` if even that somehow fails it (an origin-form
/// HTTP request-target is technically free to start with `//`, so this
/// is not purely defensive), which is what makes "never worse than
/// dropping the user on `/`" true rather than aspirational.
fn back_target(referer: Option<&str>, host: Option<&str>, current: &str) -> String {
    if let Some(from_referer) = referer.and_then(|r| same_origin_path(r, host)) {
        return from_referer;
    }
    if let Some(previous) = crate::session::session().and_then(|s| s.previous_url()) {
        return previous;
    }
    root_relative_or_none(current).unwrap_or_else(|| "/".to_string())
}

/// Reduce a `Referer` to a root-relative, same-origin path, or reject it.
///
/// The value lands in a `Location` header and `Referer` is client-set, so
/// an unchecked pass-through is an open redirect. Two forms are accepted:
/// a path already rooted at `/` and clear of any leading `//` or `/\` or
/// ASCII control byte (see [`root_relative_or_none`] for what that
/// guards against), and an absolute URL whose authority equals the
/// request's `Host` and which itself carries no control byte. Everything
/// else falls through.
fn same_origin_path(referer: &str, host: Option<&str>) -> Option<String> {
    let referer = referer.trim();
    if referer.starts_with('/') {
        return root_relative_or_none(referer);
    }
    if referer.is_empty() || has_control_byte(referer) {
        return None;
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
    fn a_control_byte_anywhere_in_the_referer_is_rejected() {
        // The URL parser strips ASCII tab/newline from its whole input
        // before comparing origins, so `/<TAB>/evil.test` is `//evil.test`
        // by the time a browser navigates it — confirmed working bypass
        // against a version of this guard that only inspected the single
        // character right after the leading `/`.
        assert_eq!(same_origin_path("/\t/evil.test", Some("app.test")), None);
        // `\n` / `\r` can't arrive through a real HTTP/1.1 `Referer`
        // header (CR/LF terminate the field), but they're the same code
        // defect and this pins the contract regardless of reachability.
        assert_eq!(same_origin_path("/\n/evil.test", Some("app.test")), None);
        assert_eq!(same_origin_path("/\r/evil.test", Some("app.test")), None);
        // Not just right after the leading slash — anywhere in the
        // candidate, since the parser strips the whole input, not a
        // prefix of it.
        assert_eq!(
            same_origin_path("/register/step\t2", Some("app.test")),
            None
        );
        // The same guard applies to the absolute-URL branch.
        assert_eq!(
            same_origin_path("https://app.test/reg\tister", Some("app.test")),
            None
        );
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
    fn an_unsanitary_current_url_falls_back_to_root_instead_of_being_trusted() {
        // `current` is normally the router's own matched path, but an
        // origin-form HTTP request-target is syntactically free to start
        // with `//` — a raw client or a non-normalizing proxy can hand
        // the framework `path() == "//evil.test/register"`. The final
        // fallback must not trust it verbatim.
        assert_eq!(
            back_target(None, Some("app.test"), "//evil.test/register"),
            "/"
        );
        assert_eq!(back_target(None, Some("app.test"), "/\t/evil.test"), "/");
        // A genuinely safe current URL still passes through unchanged.
        assert_eq!(
            back_target(None, Some("app.test"), "/register?step=2"),
            "/register?step=2"
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
