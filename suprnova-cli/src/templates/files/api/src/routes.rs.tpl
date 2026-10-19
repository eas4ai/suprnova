use suprnova::{
    AuthMiddleware, ConsentPageHeaders, CsrfMiddleware, ThrottleRequestsMiddleware, delete, get,
    group, post, routes,
};

use crate::{agents, controllers};

routes! {
    // Authentication - public by necessity: these are how a client gets a
    // token in the first place.
    post!("/api/auth/register", controllers::users::register),
    post!("/api/auth/login",    controllers::users::login),

    // User directory - requires a valid bearer token.
    //
    // `BearerTokenMiddleware`, registered globally in `bootstrap.rs`, only
    // *populates* the authenticated user when a valid token is present. It
    // never rejects a request. The `AuthMiddleware` attached below is what
    // turns a missing or invalid token into a 401, so these routes must
    // carry it explicitly - without it `GET /api/users` returns every row
    // of the users table, email addresses included, to anonymous callers.
    group!("/api", {
        get!("/users",     controllers::users::list_users),
        get!("/users/:id", controllers::users::show_user),

        // The MCP clients and agents that act for the signed-in user, and
        // taking either back (`controllers::agents`).
        get!("/connected-agents", controllers::agents::index),
        delete!("/connected-agents/clients/:grant", controllers::agents::revoke_client),
        delete!("/connected-agents/agents/:registration", controllers::agents::revoke_agent),

        // A new link that verifies the signed-in user's address, at most
        // six a minute (`controllers::email_verification`).
        post!("/email/verification-notification", controllers::email_verification::notification)
            .middleware(ThrottleRequestsMiddleware::with(6, 1, "email-verification")),
    }).middleware(AuthMiddleware::new()),

    // The sign-in page the framework's approval and claim pages send a
    // guest to (`controllers::sign_in`), at the path `src/agents.rs` gives
    // the OAuth server. It is never framed, cached or sent as a referrer,
    // and its form posts with the session's CSRF token.
    group!("/", {
        get!(agents::SIGN_IN_PAGE, controllers::sign_in::show),
        post!(agents::SIGN_IN_PAGE, controllers::sign_in::store)
            .middleware(CsrfMiddleware::new().without_xsrf_cookie()),
        // The page the email verification link opens, in the same session.
        get!(agents::VERIFY_EMAIL_PAGE, controllers::email_verification::verify),
    }).middleware(ConsentPageHeaders),
}
