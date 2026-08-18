//! Protocol-hygiene middleware: `Vary: X-Inertia` on every response, and
//! Laravel's `onEmptyResponse` handling.
//!
//! Two responsibilities, one pass over the response, because both need to
//! wrap the *entire* chain:
//!
//! 1. **`Vary: X-Inertia` everywhere.** The same URL serves two different
//!    representations depending on one request header: an HTML shell to a
//!    hard navigation, a JSON page object to an Inertia XHR. A shared
//!    cache that doesn't know that will hand one to the other — raw JSON
//!    rendered in the browser, or an HTML shell the client rejects as
//!    non-Inertia. The Inertia responses themselves already set the
//!    header; redirects, 404s, 422s, and static files did not, and those
//!    are exactly the responses a cache is most willing to store.
//!    Laravel sets it unconditionally (`Middleware.php:123`).
//!
//! 2. **Empty 200 on an Inertia visit → redirect back.** The Inertia
//!    client treats any response without `X-Inertia` as a non-Inertia
//!    response and surfaces an error modal, so a handler that falls
//!    through to a body-less 200 breaks the SPA rather than doing
//!    nothing. Laravel's `onEmptyResponse` (`Middleware.php:137-139`)
//!    redirects back instead.
//!
//! Register it first (it is registered first by
//! [`Inertia::install`](crate::Inertia::install)) so it is the outermost
//! middleware and sees every response, including the `409` that
//! [`InertiaVersionMiddleware`](crate::InertiaVersionMiddleware) returns
//! without ever calling the handler.

use crate::http::{HttpResponse, Redirect, Request, Response};
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;

/// Sets `Vary: X-Inertia` on every response and converts an empty `200`
/// on an Inertia visit into a `303` redirect back.
pub struct InertiaHeadersMiddleware;

impl InertiaHeadersMiddleware {
    /// Build a new `InertiaHeadersMiddleware`. Stateless — no arguments needed.
    pub fn new() -> Self {
        Self
    }
}

impl Default for InertiaHeadersMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

/// Add `Vary: X-Inertia` unless the response already advertises it.
///
/// Appends a separate `Vary` header line rather than rewriting an
/// existing one: RFC 9110 §5.3 says repeated field lines combine, so
/// `Vary: Precognition` + `Vary: X-Inertia` means the same thing as the
/// comma list — and rewriting would risk dropping a `Vary` some other
/// middleware set for its own reasons.
fn ensure_vary_x_inertia(response: HttpResponse) -> HttpResponse {
    let already = response.header_value("Vary").is_some_and(|v| {
        v.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("X-Inertia"))
    });
    if already {
        response
    } else {
        response.header("Vary", "X-Inertia")
    }
}

#[async_trait]
impl Middleware for InertiaHeadersMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Capture before `next` consumes the request.
        let is_inertia = request.is_inertia();
        let response = next(request).await;

        let was_ok = response.is_ok();
        let http = response.unwrap_or_else(|e| e);

        // Laravel `onEmptyResponse`. `is_streaming` first: a streaming
        // body reports an empty buffered slice because nothing has been
        // produced yet, and an SSE stream is not an empty response.
        if is_inertia && http.status_code() == 200 && !http.is_streaming() && http.body().is_empty()
        {
            // `303`, not the `302` Laravel emits here, because Laravel
            // leans on its own later `302 → 303` conversion for
            // PUT/PATCH/DELETE and leaves GET at 302. A substituted
            // redirect is never a continuation of the original method —
            // the client must issue a GET — so we say so directly.
            let redirect: Response = Redirect::back("/").into();
            let substituted = redirect.unwrap_or_else(|e| e).status(303);
            return Ok(ensure_vary_x_inertia(substituted));
        }

        let http = ensure_vary_x_inertia(http);
        if was_ok { Ok(http) } else { Err(http) }
    }
}
