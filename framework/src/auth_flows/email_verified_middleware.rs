//! [`EnsureEmailVerifiedMiddleware`] - gate routes on the authenticated
//! user's email-verification state.
//!
//! Mirrors Laravel's `Illuminate\Auth\Middleware\EnsureEmailIsVerified`
//! (the `verified` route alias). Composes naturally after
//! [`crate::AuthMiddleware`]: this middleware does not authenticate, it
//! only checks the verification flag on the user that auth already
//! resolved. That user is the route's: the user of the guard the last
//! `AuthMiddleware` checked, or of the default guard when none names one,
//! as Laravel's `verified` checks the user of the guard `auth:<guard>`
//! selected. The check goes through that guard's
//! [`UserProvider`](crate::auth::UserProvider), so it is backend-agnostic
//! (Eloquent today; any registered provider tomorrow) and carries no
//! coupling to a specific auth store. If the route's guard has no user,
//! it falls into the same response branch as "user authed but not
//! verified" - matching Laravel's `! $request->user() ||
//! ! hasVerifiedEmail()` shape.

use async_trait::async_trait;

use crate::auth::Auth;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Redirect, Request, Response};
use crate::middleware::{Middleware, Next};

/// The message of the `403` an unverified caller gets, Laravel's text.
const NOT_VERIFIED: &str = "Your email address is not verified.";

/// Middleware that 403s (or redirects) any request whose authenticated
/// user has not verified their email.
///
/// The answer depends on the request, as Laravel's `EnsureEmailIsVerified`
/// decides it. A request that expects JSON
/// ([`Request::expects_json`]) gets `403` with
/// `{"message": "Your email address is not verified."}`, whatever the
/// middleware names: a script cannot follow a redirect to a page. Any other
/// request gets the redirect the middleware names, sent through
/// [`Redirect::guest`], so the intended URL is stored by its rule (a `GET`
/// stores its own path and query) and [`Redirect::intended`] returns the
/// user there after verification. With no redirect named, every caller gets
/// the `403`.
///
/// # Example
///
/// ```rust,ignore
/// use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group};
///
/// // API routes - 403 JSON when unverified
/// group!("/api")
///     .middleware(AuthMiddleware::new())
///     .middleware(EnsureEmailVerifiedMiddleware::new())
///     .routes([/* ... */]);
///
/// // Web routes - 302 redirect to the "please verify your email" page,
/// // 403 JSON for a request that expects JSON
/// group!("/dashboard")
///     .middleware(AuthMiddleware::redirect_to("/login"))
///     .middleware(EnsureEmailVerifiedMiddleware::redirect_to_route("verification.notice"))
///     .routes([/* ... */]);
/// ```
///
/// # Inertia
///
/// When a redirect is named and the request is an Inertia visit, the
/// response is `409 Conflict` with an `X-Inertia-Location` header - the
/// Inertia adapter then performs a full-page visit to the target. The
/// intended URL is stored as for any other redirect. Plain HTML redirects
/// use `302 Found` with a `Location` header. Matches the pattern in
/// [`crate::AuthMiddleware`].
pub struct EnsureEmailVerifiedMiddleware {
    /// Where unverified users are sent. `None` → respond with `403` JSON
    /// instead.
    redirect: Option<RedirectTarget>,
}

/// The redirect an [`EnsureEmailVerifiedMiddleware`] names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RedirectTarget {
    /// A literal path, sent as given.
    Path(String),
    /// A route name, resolved on each request, so a route registered after
    /// the middleware was built is found.
    Route(String),
}

impl EnsureEmailVerifiedMiddleware {
    /// Create middleware that returns `403 Forbidden` with a JSON body
    /// `{"message": "Your email address is not verified."}` when the
    /// authenticated user has not verified their email (or when no
    /// user is authenticated at all), to every caller.
    ///
    /// Best for API routes - pair with [`crate::AuthMiddleware::new`].
    pub fn new() -> Self {
        Self { redirect: None }
    }

    /// Create middleware that redirects unverified users to `path`.
    ///
    /// Best for web routes - pair with
    /// [`crate::AuthMiddleware::redirect_to`]. Inertia requests
    /// receive `409 Conflict` + `X-Inertia-Location` instead of `302`, and
    /// a request that expects JSON receives the `403`.
    pub fn redirect_to(path: impl Into<String>) -> Self {
        Self {
            redirect: Some(RedirectTarget::Path(path.into())),
        }
    }

    /// Create middleware that redirects unverified users to the route named
    /// `name`, Laravel's `EnsureEmailIsVerified::redirectTo($route)`.
    ///
    /// The name is resolved through [`crate::routing::try_route`] on each
    /// request, as Laravel's `URL::route` is, so the redirect follows the
    /// route's path and public root. A name no route carries fails that
    /// request with a `500` whose error names the route, as `URL::route`
    /// throws; so does a route whose path needs parameters, rather than a
    /// `Location` with a raw `{placeholder}` in it. A request that expects
    /// JSON still gets the `403`, because it never needs the redirect.
    pub fn redirect_to_route(name: impl Into<String>) -> Self {
        Self {
            redirect: Some(RedirectTarget::Route(name.into())),
        }
    }

    /// Build the "not verified" response for `request`: the `403` JSON when
    /// no redirect is named or the request expects JSON, the redirect
    /// otherwise.
    fn unverified_response(&self, request: &Request) -> HttpResponse {
        let Some(redirect) = &self.redirect else {
            return not_verified();
        };
        if request.expects_json() {
            return not_verified();
        }
        let target = match redirect {
            RedirectTarget::Path(path) => path.clone(),
            RedirectTarget::Route(name) => match route_url(name) {
                Ok(url) => url,
                Err(error) => return error.into(),
            },
        };
        // `Redirect::guest` stores the intended URL by its one rule; the
        // Inertia answer stores it the same way and only changes the shape.
        let redirect = Redirect::guest(request, target.clone());
        if request.is_inertia() {
            return HttpResponse::text("")
                .status(409)
                .header("X-Inertia-Location", target);
        }
        let response: Response = redirect.into();
        response.unwrap_or_else(|response| response)
    }
}

/// The `403` an unverified caller gets when no redirect applies.
fn not_verified() -> HttpResponse {
    HttpResponse::json(serde_json::json!({ "message": NOT_VERIFIED })).status(403)
}

/// The URL of the route `name`, or the error that fails the request when no
/// route carries the name or the route needs parameters.
fn route_url(name: &str) -> Result<String, FrameworkError> {
    let no_parameters: &[(&str, &str)] = &[];
    crate::routing::try_route(name, no_parameters).map_err(|error| {
        FrameworkError::internal(format!(
            "EnsureEmailVerifiedMiddleware cannot redirect to route `{name}`: {error}"
        ))
    })
}

impl Default for EnsureEmailVerifiedMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for EnsureEmailVerifiedMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // 1. The id of the route's user, from the route's guard (no DB
        //    call). The default guard's user never stands in for the user
        //    of a named route guard.
        let Some(user_id) = Auth::route_user_id().await? else {
            // No authenticated user - same response branch as "authed
            // but unverified" (mirrors Laravel's `! user() || ! verified`).
            return Err(self.unverified_response(&request));
        };

        // 2. Ask the route guard's `UserProvider` whether this user has
        //    verified their email. A `?` here propagates a
        //    `FrameworkError` - e.g. the storage layer is down, or the
        //    active provider is token-only and doesn't support the check
        //    (its default impl returns an unsupported error) - as the
        //    framework's usual 500. That's the correct behaviour when the
        //    provider can't answer the question we were composed to ask:
        //    verification gating must not silently pass under outage or
        //    misconfiguration.
        //
        //    The Eloquent provider returns `Ok(false)` for an absent id
        //    (the user was deleted after auth resolved it), so a missing
        //    user collapses into the unverified branch below - preserving
        //    the prior "user since deleted → unverified" behaviour.
        let verified = Auth::route_user_provider()?
            .is_email_verified(&user_id)
            .await?;

        if verified {
            next(request).await
        } else {
            Err(self.unverified_response(&request))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_uses_no_redirect() {
        let mw = EnsureEmailVerifiedMiddleware::new();
        assert!(mw.redirect.is_none());
    }

    #[test]
    fn redirect_to_stores_path() {
        let mw = EnsureEmailVerifiedMiddleware::redirect_to("/verify");
        assert_eq!(mw.redirect, Some(RedirectTarget::Path("/verify".into())));
    }

    #[test]
    fn redirect_to_route_stores_the_name() {
        let mw = EnsureEmailVerifiedMiddleware::redirect_to_route("verification.notice");
        assert_eq!(
            mw.redirect,
            Some(RedirectTarget::Route("verification.notice".into()))
        );
    }

    #[test]
    fn default_is_no_redirect() {
        let mw = EnsureEmailVerifiedMiddleware::default();
        assert!(mw.redirect.is_none());
    }
}
