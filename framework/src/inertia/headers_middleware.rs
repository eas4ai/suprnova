//! The Inertia middleware's decisions about the response: `Vary: X-Inertia`
//! on every response, and on an Inertia visit Laravel's `onEmptyResponse`,
//! `onRedirectWithFragment` and `storeCurrentUrl`.
//!
//! Each needs to wrap the *entire* chain, so they share one pass:
//!
//! 1. **`Vary: X-Inertia` everywhere.** The same URL serves two different
//!    representations depending on one request header: an HTML shell to a
//!    hard navigation, a JSON page object to an Inertia XHR. A shared
//!    cache that doesn't know that will hand one to the other - raw JSON
//!    rendered in the browser, or an HTML shell the client rejects as
//!    non-Inertia. The Inertia responses themselves set the header; this
//!    middleware covers redirects, 404s, 422s, and static files too -
//!    exactly the responses a cache is most willing to store.
//!    Laravel sets it unconditionally (`Middleware.php:123`).
//!
//! 2. **Empty 200 on an Inertia visit → redirect back.** The Inertia
//!    client treats any response without `X-Inertia` as a non-Inertia
//!    response and surfaces an error modal, so a handler that falls
//!    through to a body-less 200 breaks the SPA rather than doing
//!    nothing. Laravel's `onEmptyResponse` redirects back instead: to the
//!    `Referer` when it is same-origin, else the previous URL, else `/`,
//!    with `302`, which the `302 → 303` rule makes `303` for `PUT`, `PATCH`
//!    and `DELETE`.
//!
//! 3. **A redirect with a `#fragment` → `409` + `X-Inertia-Redirect`.** The
//!    client follows a redirect inside its XHR, where the fragment of a
//!    `Location` is lost. Handing the target over as `X-Inertia-Redirect`
//!    lets the client visit it with the fragment intact. A prefetch keeps
//!    its redirect: it is never shown.
//!
//! 4. **The previous URL.** The session middleware records only full page
//!    loads, so an Inertia `GET` records its own URL here, unless it is a
//!    prefetch, a Precognition request or a partial reload of the page it
//!    rendered (see [`InertiaConfig::store_previous_url`]).
//!
//! Register it first (it is registered first by
//! [`Inertia::install`](crate::Inertia::install)) so it is the outermost
//! middleware and sees every response, including the `409` that
//! [`InertiaVersionMiddleware`](crate::InertiaVersionMiddleware) returns
//! without ever calling the handler. It also scopes the facts of the
//! request for [`Inertia::back`](crate::Inertia::back).

use std::sync::Arc;

use super::config::InertiaConfig;
use super::error_page_middleware::header_survives_rewrite;
use super::visit::{self, Visit};
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;

/// Sets `Vary: X-Inertia` on every response; on an Inertia visit, turns an
/// empty `200` into a redirect back and a redirect with a fragment into a
/// `409` the client follows, and records the visit as the previous URL.
pub struct InertiaHeadersMiddleware {
    store_previous_url: bool,
}

impl InertiaHeadersMiddleware {
    /// Build the middleware with the framework defaults: previous-URL
    /// recording on.
    pub fn new() -> Self {
        Self {
            store_previous_url: true,
        }
    }

    /// Build the middleware with the settings of `config` that it reads,
    /// [`InertiaConfig::store_previous_url`] so far. [`Inertia::install`]
    /// builds it this way.
    ///
    /// [`Inertia::install`]: crate::Inertia::install
    pub fn from_config(config: &InertiaConfig) -> Self {
        Self {
            store_previous_url: config.store_previous_url,
        }
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
/// comma list - and rewriting would risk dropping a `Vary` some other
/// middleware set for its own reasons.
///
/// Checks every `Vary` line via
/// [`header_values`](HttpResponse::header_values), not just the first:
/// `Vary: Precognition` followed by a separate `Vary: X-Inertia` already
/// advertises the token, and a first-line-only check would append a
/// redundant third line.
fn ensure_vary_x_inertia(response: HttpResponse) -> HttpResponse {
    let already = response.header_values("Vary").any(|v| {
        v.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("X-Inertia"))
    });
    if already {
        response
    } else {
        response.header("Vary", "X-Inertia")
    }
}

/// Symfony's `Response::isRedirect()`, which Laravel's middleware reads:
/// the status alone decides, `201` included.
fn is_redirect_status(status: u16) -> bool {
    matches!(status, 201 | 301 | 302 | 303 | 307 | 308)
}

/// The facts of the request the response decisions read, captured before
/// `next` consumes it.
struct RequestFacts {
    is_inertia: bool,
    is_get: bool,
    /// `PUT`, `PATCH` or `DELETE`: a `302` is answered as `303`.
    needs_303: bool,
    is_prefetch: bool,
    is_precognitive: bool,
    /// The router matched a route (Laravel's `$request->route()`).
    matched_route: bool,
    /// `X-Inertia-Partial-Component`, when the visit is a partial reload.
    partial_component: Option<String>,
    /// The URL the visit records as the previous URL, public root included.
    current_url: String,
}

impl RequestFacts {
    fn capture(request: &Request) -> Self {
        let method = request.method();
        let is_inertia = request.is_inertia();
        let is_get = *method == hyper::Method::GET;
        Self {
            is_inertia,
            is_get,
            needs_303: matches!(
                *method,
                hyper::Method::PUT | hyper::Method::PATCH | hyper::Method::DELETE
            ),
            is_prefetch: visit::is_prefetch(|name| request.header(name)),
            is_precognitive: request
                .header("Precognition")
                .is_some_and(|v| v.eq_ignore_ascii_case("true")),
            matched_route: request.route_pattern().is_some(),
            partial_component: request
                .header("X-Inertia-Partial-Component")
                .map(str::to_string),
            // Only an Inertia `GET` records it, so only one builds it.
            current_url: if is_inertia && is_get {
                crate::routing::url::current(request)
            } else {
                String::new()
            },
        }
    }
}

/// The response with its body replaced by `replacement`'s: everything the
/// handler decided besides the body - its cookies, security and CORS
/// headers, its error report - carries over, by the rule the error page
/// uses for the same substitution. `Location` is the replacement's own.
fn substitute(original: HttpResponse, replacement: HttpResponse) -> HttpResponse {
    let carried: Vec<(String, String)> = original
        .headers()
        .filter(|(name, _)| header_survives_rewrite(name) && !name.eq_ignore_ascii_case("Location"))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    replacement
        .with_headers(carried)
        .with_error_report_of(original)
}

/// Laravel's default `onEmptyResponse`: `Redirect::back()`, a `302` the
/// `302 → 303` rule turns into `303` for `PUT`, `PATCH` and `DELETE`.
fn redirect_back(visit: &Visit, response: HttpResponse) -> HttpResponse {
    let target = visit.back_target(None);
    substitute(
        response,
        HttpResponse::new().status(302).header("Location", target),
    )
}

/// Whether `response` is the body-less `200` Laravel's `onEmptyResponse`
/// replaces. `is_streaming` first: a streaming body reports an empty
/// buffered slice because nothing has been produced yet, and an SSE stream
/// is not an empty response.
fn is_empty_200(response: &HttpResponse) -> bool {
    response.status_code() == 200 && !response.is_streaming() && response.body().is_empty()
}

/// Laravel's default `onRedirectWithFragment`: `409` with the redirect's
/// target as `X-Inertia-Redirect`, which the client visits itself so the
/// fragment survives.
fn redirect_with_fragment(location: String, response: HttpResponse) -> HttpResponse {
    substitute(
        response,
        HttpResponse::new()
            .status(409)
            .header("X-Inertia-Redirect", location),
    )
}

impl InertiaHeadersMiddleware {
    /// Laravel's `storeCurrentUrl`: an Inertia `GET` that matched a route
    /// becomes the previous URL, unless it is a prefetch, a Precognition
    /// request or a partial reload of the component it rendered (deferred
    /// props, polling and infinite scroll reload the page the visitor is
    /// on). Only a page that rendered or redirected counts, as for a full
    /// page load in the session middleware.
    fn store_current_url(&self, facts: &RequestFacts, visit: &Visit, status: u16) {
        if !self.store_previous_url
            || !facts.is_get
            || !facts.matched_route
            || facts.is_prefetch
            || facts.is_precognitive
            || !(200..400).contains(&status)
        {
            return;
        }
        if let Some(partial) = facts.partial_component.as_deref()
            && visit.rendered_component().as_deref() == Some(partial)
        {
            return;
        }
        crate::session::middleware::record_previous_url(&facts.current_url);
    }
}

#[async_trait]
impl Middleware for InertiaHeadersMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Capture before `next` consumes the request.
        let facts = RequestFacts::capture(&request);
        let visit = Arc::new(Visit::capture(&request));
        let response = visit::scope(visit.clone(), next(request)).await;

        let was_ok = response.is_ok();
        let rewrap = |http| if was_ok { Ok(http) } else { Err(http) };
        let mut http = response.unwrap_or_else(|e| e);

        if !facts.is_inertia {
            return rewrap(ensure_vary_x_inertia(http));
        }

        // Read before the empty-response substitution: as in Laravel, a
        // redirect this middleware substitutes is not checked for a
        // fragment.
        let was_redirect = is_redirect_status(http.status_code());

        if is_empty_200(&http) {
            // Laravel `onEmptyResponse`. The visit records no previous URL:
            // an empty response is no page the visitor saw, and recording
            // it would make the redirect back point at itself.
            http = redirect_back(&visit, http);
            // The `302 → 303` rule for the redirect substituted here; one
            // the handler returned has already been converted by
            // `Inertia303Middleware` inside this one.
            if facts.needs_303 && http.status_code() == 302 {
                http = http.status(303);
            }
        } else {
            self.store_current_url(&facts, &visit, http.status_code());
        }

        if was_redirect
            && !facts.is_prefetch
            && let Some(location) = http
                .header_value("Location")
                .filter(|location| location.contains('#'))
                .map(str::to_string)
        {
            http = redirect_with_fragment(location, http);
        }

        rewrap(ensure_vary_x_inertia(http))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vary_is_not_duplicated_when_a_later_line_already_lists_x_inertia() {
        // `header_value` (singular) only ever sees the first `Vary` line.
        // A response that carries `Vary: Precognition` first and
        // `Vary: X-Inertia` as a separate, later line already advertises
        // the token - checking only the first line would miss it and
        // append a redundant third line.
        let response = HttpResponse::new()
            .header("Vary", "Precognition")
            .header("Vary", "X-Inertia");

        let result = ensure_vary_x_inertia(response);

        let vary_lines: Vec<&str> = result.header_values("Vary").collect();
        assert_eq!(
            vary_lines,
            vec!["Precognition", "X-Inertia"],
            "a token already present on a later Vary line must not be duplicated onto a third line"
        );
    }
}
