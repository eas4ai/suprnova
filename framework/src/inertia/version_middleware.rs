//! Asset-version mismatch middleware.
//!
//! Per the Inertia v3 protocol (see `core-concepts/the-protocol.mdx`),
//! Inertia GET requests carry an `X-Inertia-Version` header. The server
//! compares that to its configured version; on mismatch the server returns
//! `409 Conflict` with an `X-Inertia-Location` header holding the current
//! absolute URL and an `X-Inertia-Version` header holding the current
//! version. The client then performs a full-page visit to pick up the new
//! assets.
//!
//! Non-GET requests are exempt - the spec says version mismatch on
//! POST/PUT/PATCH/DELETE resolves naturally on the redirect that follows
//! the request (which IS a GET, and that GET will trigger the 409).
//!
//! ## Wiring
//!
//! This middleware is **opt-in**. Register globally from your app's
//! bootstrap so it runs on every request:
//!
//! ```rust,no_run
//! use suprnova::{global_middleware, InertiaConfig, InertiaVersionMiddleware};
//!
//! pub fn register() {
//!     let version = env!("CARGO_PKG_VERSION");
//!     let cfg = InertiaConfig::new().version(version);
//!     let _ = cfg;
//!     global_middleware!(InertiaVersionMiddleware::new(version));
//! }
//! ```
//!
//! Without this middleware, asset-version mismatch is silent - clients
//! continue to use the cached SPA bundle against a server emitting a
//! newer version.
//!
//! The bounce re-flashes the session first, so flashed errors and
//! messages survive the client's follow-up full-page GET. That requires
//! `SessionMiddleware` to be registered **ahead** of this middleware; it
//! is a no-op otherwise.

use crate::http::{HttpResponse, Request, Response};
use crate::inertia::config::VersionResolver;
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;

use super::InertiaRequestExt;

/// Asset-version mismatch detector. Compares the request's
/// `X-Inertia-Version` against the configured version and, on an Inertia
/// `GET` that does not match, returns `409` with `X-Inertia-Location`
/// holding the request's absolute URL and `X-Inertia-Version` holding the
/// current version, before the handler runs.
///
/// Accepts either a static version string or a dynamic resolver via
/// [`VersionResolver`]. The dynamic resolver runs on every request so
/// the middleware stays in sync with build-time changes (hot reloads,
/// rolling deploys).
pub struct InertiaVersionMiddleware {
    version: VersionResolver,
}

impl InertiaVersionMiddleware {
    /// Create a new middleware with a static asset version. Use
    /// [`with_resolver`](Self::with_resolver) for dynamic versions.
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: VersionResolver::Static(version.into()),
        }
    }

    /// Create a new middleware that resolves the asset version via the
    /// given closure on every request. Wrap any caching inside the
    /// closure.
    pub fn with_resolver<F>(f: F) -> Self
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        Self {
            version: VersionResolver::with(f),
        }
    }
}

#[async_trait]
impl Middleware for InertiaVersionMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Only act on Inertia XHR requests. Standard browser visits (no
        // X-Inertia header) reload the full HTML anyway.
        if !request.is_inertia() {
            return next(request).await;
        }

        // Per the protocol, only GETs return 409 for version mismatch.
        // Other methods (POST/PUT/PATCH/DELETE) flow through; their
        // redirect-after responses will trigger the 409 on the GET that
        // follows them.
        if request.method() != hyper::Method::GET {
            return next(request).await;
        }

        let server_version = self.version.resolve();
        let client_version = request.inertia_version().unwrap_or("");
        if client_version == server_version {
            return next(request).await;
        }

        // Mismatch - bounce the client to do a full-page visit at the
        // same URL so it picks up the new assets. Preserve the query
        // string: a 409 on `/search?q=rust` must redirect back to the
        // same search, not bare `/search` (which would silently drop
        // pagination cursors, filter state, and form-submitted GET
        // params on every asset-version mismatch).
        //
        // Reflash before bouncing. The client answers a 409 with a
        // full-page GET, and that GET is a new request: the session
        // middleware ages `_flash.old.*` away before the destination page
        // can read it. Without this, a validation error or success toast
        // flashed by the previous request disappears purely because the
        // asset version moved - the user submits a form, deploys race
        // them, and the error message is silently eaten. Laravel does the
        // same (`Middleware.php:171-175`). No-op outside a session scope,
        // which also means `SessionMiddleware` has to be registered ahead
        // of this one for it to bite.
        crate::session::session_mut(|session| session.reflash());

        // The current version goes back with the bounce: the client reads
        // `X-Inertia-Version` on a 409 so a poll or a background prop load
        // that lands on a deploy does not force a full reload (Laravel's
        // `onVersionChange`).
        Err(HttpResponse::new()
            .status(409)
            .header("X-Inertia-Location", absolute_location(&request))
            .header("X-Inertia-Version", server_version))
    }
}

/// The URL the 409 bounces the client to: the request's absolute URL, as
/// Laravel's `onVersionChange` names it with `$request->fullUrl()`.
///
/// Scheme and host come from [`Request::scheme_and_http_host`], so a
/// trusted proxy's forwarded host and scheme are honoured and nothing else
/// is. The path and query go through `InertiaRequestExt::path_and_query`,
/// the same derivation the page object's `url` uses, so the bounce and the
/// page it bounces to name the same path whenever the page object uses the
/// default derivation; a configured `url_resolver` changes only the page
/// object, because the bounce has to name a URL the browser can fetch. The
/// path carries the public root (PFX-005), and the query is normalised as
/// the page `url`'s is (PAR-056). A request that names no host keeps the
/// root-relative URL rather than inventing one.
fn absolute_location(request: &Request) -> String {
    let path = crate::routing::root::prefixed(&super::query_string::normalize_path_and_query(
        request.path_and_query(),
    ));
    match request.scheme_and_http_host() {
        Some(origin) => format!("{origin}{path}"),
        None => path,
    }
}
