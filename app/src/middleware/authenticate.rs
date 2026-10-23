//! The sign-in page's guard and the return address a sign-in link carries,
//! as the Inertia kits have them.
//!
//! The OAuth server and agent auth send a guest to `/login` with the address
//! to come back to in `return_to` (`src/agents.rs`). The sign-in keeps it
//! as the session's intended address, and a user who is already signed in
//! goes straight there.

use suprnova::{GuestMiddleware, Middleware, Next, Redirect, Request, Response, async_trait, url};

/// The query parameter a sign-in link names the address to return to in.
const RETURN_TO: &str = "return_to";

/// Where a signed-in user goes from a guest page when no address is named.
const HOME: &str = "/";

/// Sends a signed-in user away from the sign-in and registration pages, as
/// Laravel's middleware of the same name does: to the `return_to` a
/// sign-in link names when it is a path on this application, and home
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
/// the sign-in that follows continues there (`Redirect::intended`). Any
/// other address changes nothing: an unchecked one stored here would make
/// every sign-in link an open redirect.
pub fn remember_return_to(request: &Request) {
    if let Some(path) = request
        .query_param(RETURN_TO)
        .and_then(|candidate| url::same_origin_path(&candidate))
    {
        Redirect::set_intended_url(path);
    }
}
