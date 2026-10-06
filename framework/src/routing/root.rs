//! The public root: the path the application is served under.
//!
//! A reverse proxy can serve the application under a path, such as
//! `/billing`, and strip that path before it forwards the request. The
//! application then matches its routes on the path it receives, but every
//! URL it hands the browser has to carry the stripped path again. That path
//! is the public root (PFX-002):
//!
//! - the value of `X-Forwarded-Prefix` from a trusted proxy (PFX-001),
//!   which replaces the path in `APP_URL`;
//! - otherwise the path in `APP_URL`, which is also the root outside a
//!   request (console commands, jobs, mail).
//!
//! The root is written without a trailing slash, and it is the empty string
//! at the host root, so the root followed by an application path such as
//! `/invoices/7` is a root-relative URL at every root.
//!
//! The server resolves the root once per request, before any middleware or
//! handler runs, and keeps it in a task-local for the life of the request.
//! A task spawned for the request does not inherit task-locals, so code that
//! spawns work which builds URLs for the request carries the root with
//! [`scope`].

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;

use hyper::HeaderMap;

/// The header a trusted proxy names the stripped path in.
pub(crate) const FORWARDED_PREFIX_HEADER: &str = "x-forwarded-prefix";

tokio::task_local! {
    /// The root of the request being handled. Set by the server before the
    /// middleware chain runs; never changed while the request runs.
    static REQUEST_ROOT: Arc<str>;
}

/// Run `future` with `root` as the public root of the request it serves.
///
/// The server calls this for every request. Code that spawns a task which
/// renders for the request, such as the RenderCache background rebuild,
/// calls it again inside the task with the root it read before the spawn,
/// because a spawned task starts with no task-locals.
///
/// It returns the task-local future itself rather than wrapping it in an
/// `async fn`, so a request future is not moved into one more state
/// machine on its way to the runtime.
pub(crate) fn scope<F: Future>(root: Arc<str>, future: F) -> impl Future<Output = F::Output> {
    REQUEST_ROOT.scope(root, future)
}

/// The root of the request being handled, or the root `APP_URL` gives
/// outside a request.
pub(crate) fn current() -> Arc<str> {
    REQUEST_ROOT
        .try_with(Arc::clone)
        .unwrap_or_else(|_| Arc::from(app_url_root()))
}

/// Resolve the root of a request from its headers.
///
/// `peer_trusted` says whether the TCP peer is a trusted proxy. A prefix
/// the request carries is read only then (PFX-001); otherwise, and when the
/// prefix breaks the value rule, the root is the path in `APP_URL`, which
/// `app_url_root` reads only when it is needed.
pub(crate) fn resolve(
    headers: &HeaderMap,
    peer_trusted: bool,
    app_url_root: impl FnOnce() -> String,
) -> String {
    if peer_trusted && let Some(prefix) = forwarded_prefix(headers) {
        return prefix;
    }
    app_url_root()
}

/// The root a trusted `X-Forwarded-Prefix` names, or `None` when the header
/// is absent or ignored (PFX-001).
///
/// The value `/` names the host root and gives the empty root. Any other
/// value is ignored unless every character is an ASCII letter or digit,
/// `-`, `.`, `_`, `~` or `/`, it starts with `/`, it does not end with `/`,
/// and no segment is empty, `.` or `..`. A header sent on two or more lines
/// is ignored whole: a proxy that adds its own line behind the client's
/// leaves the client's line first, so neither line can be believed. An
/// empty value is ignored.
///
/// Ignoring a value with a trailing slash, rather than trimming it, keeps
/// one spelling of each root, so two spellings never produce two cache keys
/// or two cookie paths for one deployment.
pub(crate) fn forwarded_prefix(headers: &HeaderMap) -> Option<String> {
    let mut lines = headers.get_all(FORWARDED_PREFIX_HEADER).iter();
    let value = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    let value = value.to_str().ok()?;
    if value == "/" {
        return Some(String::new());
    }
    valid_prefix(value).then(|| value.to_owned())
}

/// Whether `value` is a prefix the value rule of PFX-001 admits.
pub(crate) fn valid_prefix(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('/') else {
        return false;
    };
    value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
    }) && rest
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// The path in `APP_URL`, without a trailing slash: the root when no
/// trusted prefix arrives, and outside a request.
///
/// `APP_URL` is the operator's own setting, so its path is not held to the
/// value rule a forwarded header is. A path a browser would read as another
/// host - one that starts with `//`, or holds a `\` or a control character -
/// is the one exception: it is not used, and the root is the host root,
/// because every redirect would otherwise leave the application.
pub(crate) fn app_url_root() -> String {
    root_of_app_url(&crate::routing::url::app_url())
}

/// [`app_url_root`] for a given `APP_URL`.
pub(crate) fn root_of_app_url(app_url: &str) -> String {
    let path = split_app_url(app_url).1;
    if path.starts_with("//") || path.contains('\\') || path.bytes().any(|b| b.is_ascii_control()) {
        // Once per process: the root is read on every request.
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::warn!(
                "APP_URL has a path a browser would read as another host; the public root is \
                 the host root until it is corrected"
            );
        }
        return String::new();
    }
    path.to_owned()
}

/// Split an `APP_URL` into its origin (scheme, host and port) and its path.
///
/// The path stops at a query or a fragment and loses its trailing slashes,
/// so `https://example.org/billing/` gives `("https://example.org",
/// "/billing")` and `https://example.org` gives `("https://example.org",
/// "")`. A value with no `://` has no path the framework can tell apart, so
/// it is all origin.
pub(crate) fn split_app_url(app_url: &str) -> (&str, &str) {
    let Some(scheme_end) = app_url.find("://").map(|at| at + 3) else {
        return (app_url, "");
    };
    let rest = &app_url[scheme_end..];
    let Some(path_start) = rest.find(['/', '?', '#']) else {
        return (app_url, "");
    };
    let origin = &app_url[..scheme_end + path_start];
    let path = &rest[path_start..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    (origin, path.trim_end_matches('/'))
}

/// Whether `path` is under `root` (PFX-002): the root is the host root, or
/// the path starts with the root, byte for byte, followed by `/`, `?`, `#`
/// or its end.
pub(crate) fn is_under(root: &str, path: &str) -> bool {
    root.is_empty()
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// Give `target` the root `root` when it needs it (PFX-010).
///
/// A target that starts with exactly one `/` and is not under the root is
/// an application path, and gets the root. A target already under the root,
/// such as the output of `route()`, is left as it is, so the root is never
/// added twice. An absolute URL, a network-path reference (`//host/x`) and
/// a relative reference (`?q`, `#f`, `x`) are left as they are.
pub(crate) fn rooted_with<'a>(root: &str, target: &'a str) -> Cow<'a, str> {
    let application_path = target.starts_with('/') && !target.starts_with("//");
    if !application_path || is_under(root, target) {
        Cow::Borrowed(target)
    } else {
        Cow::Owned(format!("{root}{target}"))
    }
}

/// [`rooted_with`] under the current root.
pub(crate) fn rooted(target: &str) -> Cow<'_, str> {
    rooted_with(&current(), target)
}

/// The current root followed by `path`, an application path the framework
/// built itself (a route's path, the path a request arrived on).
///
/// Unlike [`rooted`] this always adds the root: the path is known to be an
/// application path, so a route whose own path happens to begin with the
/// root still gets it.
pub(crate) fn prefixed(path: &str) -> String {
    let root = current();
    let mut out = String::with_capacity(root.len() + path.len());
    out.push_str(&root);
    out.push_str(path);
    out
}

/// The `Path` a framework cookie takes when the application sets none
/// (PFX-007): the current root, or `/` at the host root.
pub(crate) fn cookie_path() -> String {
    let root = current();
    if root.is_empty() {
        "/".to_owned()
    } else {
        root.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(lines: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for line in lines {
            map.append(
                FORWARDED_PREFIX_HEADER,
                hyper::header::HeaderValue::from_str(line).expect("a header value"),
            );
        }
        map
    }

    #[test]
    fn a_valid_prefix_from_a_trusted_peer_is_the_root() {
        let root = resolve(&headers(&["/billing"]), true, || "/app".to_owned());
        assert_eq!(root, "/billing");
    }

    #[test]
    fn an_untrusted_peer_leaves_the_app_url_root() {
        let root = resolve(&headers(&["/billing"]), false, || "/app".to_owned());
        assert_eq!(root, "/app");
    }

    #[test]
    fn a_trusted_slash_is_the_host_root() {
        assert_eq!(resolve(&headers(&["/"]), true, || "/app".to_owned()), "");
    }

    #[test]
    fn values_that_break_the_rule_are_ignored() {
        for value in [
            "",
            "billing",
            "/billing/",
            "/a//b",
            "/a/./b",
            "/a/../b",
            "/%2e%2e",
            "/caf\u{e9}",
            "/a b",
            "/a,/b",
        ] {
            let mut map = HeaderMap::new();
            if let Ok(header) = hyper::header::HeaderValue::from_bytes(value.as_bytes()) {
                map.append(FORWARDED_PREFIX_HEADER, header);
            }
            assert_eq!(forwarded_prefix(&map), None, "{value:?}");
        }
    }

    #[test]
    fn a_header_on_two_lines_is_ignored_whole() {
        assert_eq!(forwarded_prefix(&headers(&["/billing", "/billing"])), None);
    }

    #[test]
    fn the_app_url_splits_into_origin_and_path() {
        assert_eq!(
            split_app_url("https://example.org/billing/"),
            ("https://example.org", "/billing")
        );
        assert_eq!(
            split_app_url("https://example.org"),
            ("https://example.org", "")
        );
        assert_eq!(
            split_app_url("http://localhost:8000/"),
            ("http://localhost:8000", "")
        );
        assert_eq!(
            split_app_url("https://example.org/a/b?x=1#y"),
            ("https://example.org", "/a/b")
        );
        assert_eq!(split_app_url("localhost"), ("localhost", ""));
        assert_eq!(
            split_app_url("https://example.org//evil.test/"),
            ("https://example.org", "//evil.test")
        );
    }

    #[test]
    fn an_app_url_path_that_names_another_host_is_not_a_root() {
        assert_eq!(root_of_app_url("https://example.org/billing/"), "/billing");
        assert_eq!(root_of_app_url("https://example.org//evil.test"), "");
        assert_eq!(root_of_app_url("https://example.org/a\\b"), "");
        assert_eq!(root_of_app_url("https://example.org"), "");
    }

    #[test]
    fn rooting_follows_pfx_010() {
        let root = "/billing";
        assert_eq!(rooted_with(root, "/invoices"), "/billing/invoices");
        assert_eq!(rooted_with(root, "/"), "/billing/");
        assert_eq!(rooted_with(root, "/billing"), "/billing");
        assert_eq!(rooted_with(root, "/billing/x"), "/billing/x");
        assert_eq!(rooted_with(root, "/billing?tab=2"), "/billing?tab=2");
        assert_eq!(rooted_with(root, "/billing#x"), "/billing#x");
        assert_eq!(rooted_with(root, "/billingx"), "/billing/billingx");
        assert_eq!(rooted_with(root, "?page=2"), "?page=2");
        assert_eq!(rooted_with(root, "#f"), "#f");
        assert_eq!(rooted_with(root, "x"), "x");
        assert_eq!(rooted_with(root, "//host/x"), "//host/x");
        assert_eq!(
            rooted_with(root, "https://example.org/x"),
            "https://example.org/x"
        );
        assert_eq!(rooted_with("", "/x"), "/x");
    }
}
