use suprnova::{get, group, post, routes, AuthMiddleware};

use crate::controllers;

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
    }).middleware(AuthMiddleware::new()),
}
