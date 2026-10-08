//! What the Inertia middleware knows about the request it is handling,
//! kept for the code that runs inside it.
//!
//! Laravel's `Inertia::back()` and `Inertia::location()` read the current
//! request from the container. A Rust handler has no ambient request, so
//! [`InertiaHeadersMiddleware`](crate::InertiaHeadersMiddleware) scopes the
//! few facts those calls need - whether this is an Inertia visit, the
//! `Referer` and the origin it is checked against - into a task-local for
//! the rest of the chain. The page render writes back which component it
//! rendered, which the middleware reads to tell a partial reload of the same
//! page from a navigation.

use std::sync::{Arc, Mutex};

use crate::http::Request;

tokio::task_local! {
    static VISIT: Arc<Visit>;
}

/// The facts of one request, captured before the handler consumes it.
pub(crate) struct Visit {
    /// The request is an Inertia visit.
    pub(crate) is_inertia: bool,
    /// The `Referer` header as sent.
    referer: Option<String>,
    /// The host as the trust rule gives it, which a `Referer` must name.
    http_host: Option<String>,
    /// The public root a `Referer` path must sit under.
    public_root: String,
    /// The component the page render answered with, when one rendered.
    rendered_component: Mutex<Option<String>>,
}

impl Visit {
    /// Capture the facts of `request`.
    pub(crate) fn capture(request: &Request) -> Self {
        Self {
            is_inertia: request.is_inertia(),
            referer: request.header("Referer").map(str::to_string),
            http_host: request.http_host(),
            public_root: request.public_root().to_string(),
            rendered_component: Mutex::new(None),
        }
    }

    /// Where a redirect back from this request goes: Laravel's
    /// `UrlGenerator::previous($fallback)`. See [`back_target`].
    pub(crate) fn back_target(&self, fallback: Option<&str>) -> String {
        back_target(
            self.referer.as_deref(),
            self.http_host.as_deref(),
            &self.public_root,
            fallback,
        )
    }

    /// The component the page render recorded, if one rendered.
    pub(crate) fn rendered_component(&self) -> Option<String> {
        crate::lock::recover(&self.rendered_component).clone()
    }
}

/// Run `fut` with `visit` as the current visit.
pub(crate) async fn scope<F: std::future::Future>(visit: Arc<Visit>, fut: F) -> F::Output {
    VISIT.scope(visit, fut).await
}

/// The visit the Inertia middleware scoped, or `None` when the request did
/// not pass through it.
pub(crate) fn current() -> Option<Arc<Visit>> {
    VISIT.try_with(Arc::clone).ok()
}

/// Record the component a page render answered with, for the middleware's
/// partial-reload check. A no-op outside the middleware.
pub(crate) fn record_rendered_component(component: &str) {
    let _ = VISIT.try_with(|visit| {
        *crate::lock::recover(&visit.rendered_component) = Some(component.to_string());
    });
}

/// Where a redirect back goes, in Laravel's order: the `Referer` when it
/// passes the same-origin check the validation redirect applies, else the
/// session's previous URL, else `fallback`, else the application root.
///
/// The `Referer` is client-set and ends up in `Location`, so it is reduced
/// to a same-origin path under the public root or dropped; the previous URL
/// is checked when it is read. The final `/` is an application path, which
/// the response gives the public root when it is sent.
pub(crate) fn back_target(
    referer: Option<&str>,
    host: Option<&str>,
    root: &str,
    fallback: Option<&str>,
) -> String {
    if let Some(path) =
        referer.and_then(|r| super::validation_redirect_middleware::same_origin_path(r, host, root))
    {
        return path;
    }
    if let Some(previous) = crate::session::session().and_then(|s| s.previous_url()) {
        return previous;
    }
    match fallback {
        Some(fallback) => fallback.to_string(),
        None => "/".to_string(),
    }
}

/// Laravel's `Request::prefetch()`: `X-Moz`, `Purpose` or `Sec-Purpose`
/// equal to `prefetch`, ignoring case. A prefetch is fetched for later and
/// may never be shown, so it records no previous URL and its redirects are
/// left as they are.
pub(crate) fn is_prefetch<'a>(header: impl Fn(&str) -> Option<&'a str>) -> bool {
    ["X-Moz", "Purpose", "Sec-Purpose"]
        .iter()
        .any(|name| header(name).is_some_and(|value| value.eq_ignore_ascii_case("prefetch")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inp_back_target_prefers_a_same_origin_referer_then_the_fallback() {
        assert_eq!(
            back_target(Some("http://app.test/a?b=1"), Some("app.test"), "", None),
            "/a?b=1"
        );
        // No session in a unit test: a foreign Referer falls through to the
        // fallback, then to the root.
        assert_eq!(
            back_target(
                Some("https://evil.test/a"),
                Some("app.test"),
                "",
                Some("/f")
            ),
            "/f"
        );
        assert_eq!(back_target(None, Some("app.test"), "", None), "/");
    }

    #[test]
    fn inp_prefetch_reads_three_headers_case_insensitively() {
        let headers = [("Sec-Purpose", "Prefetch")];
        let lookup = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| *v)
        };
        assert!(is_prefetch(lookup));
        assert!(!is_prefetch(|_| None));
        assert!(!is_prefetch(|_| Some("prerender")));
    }
}
