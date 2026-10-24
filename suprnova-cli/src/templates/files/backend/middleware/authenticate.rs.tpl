//! Authentication middleware helpers

pub use suprnova::{AuthMiddleware, GuestMiddleware};
use suprnova::{Middleware, Next, Redirect, Request, Response, async_trait, url};

/// The query parameter a sign-in link names the address to return to in,
/// as the OAuth server and agent auth send a guest to `/login`.
const RETURN_TO: &str = "return_to";

/// Where a signed-in user goes from a guest page when no address is named.
const HOME: &str = "/dashboard";

/// Create auth middleware that redirects unauthenticated users to login
pub fn auth() -> AuthMiddleware {
    AuthMiddleware::redirect_to("/login")
}

/// The guard of the sign-in, registration and password reset pages: a
/// guest passes, and a signed-in user goes on (see
/// [`RedirectIfAuthenticated`]).
pub fn guest() -> RedirectIfAuthenticated {
    RedirectIfAuthenticated
}

/// Sends a signed-in user away from the guest pages, as Laravel's
/// middleware of the same name does.
///
/// A sign-in link from the OAuth server or agent auth names where the user
/// was going in `return_to`. A user who is already signed in goes straight
/// there when it is a path on this application, and to the dashboard
/// otherwise. The framework's guest middleware decides who is a guest and
/// how an Inertia visit is sent on.
pub struct RedirectIfAuthenticated;

#[async_trait]
impl Middleware for RedirectIfAuthenticated {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // `same_origin_path` refuses an address a browser would read as
        // another host, such as `//evil.example`, so a sign-in link cannot
        // send anyone off this application.
        let destination = request
            .query_param(RETURN_TO)
            .and_then(|candidate| url::same_origin_path(&candidate))
            .unwrap_or_else(|| HOME.to_owned());
        GuestMiddleware::redirect_to(destination)
            .handle(request, next)
            .await
    }
}

/// Keep a same-origin `return_to` as the session's intended address, so
/// the sign-in that follows continues there (`Redirect::intended`). An
/// address that fails the check changes nothing, and the sign-in continues
/// as it would without one.
pub fn remember_return_to(request: &Request) {
    if let Some(path) = request
        .query_param(RETURN_TO)
        .and_then(|candidate| url::same_origin_path(&candidate))
    {
        Redirect::set_intended_url(path);
    }
}
