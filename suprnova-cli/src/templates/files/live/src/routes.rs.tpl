//! The application's routes.
//!
//! Every page is a Live document: its controller mounts the page's islands
//! from `live::mounts()` and renders a view under `templates/live/pages/`.
//! `live::routes()` seals the same mounts into the router at startup, so a
//! route here and its islands there must name the same path. A route that
//! has a name carries the one Laravel's starter kits give it: the Live
//! actions redirect by these names.

use suprnova::{StaticFiles, fallback, get, group, post, routes};

use crate::controllers;
use crate::live;
use crate::middleware;

routes! {
    // Public routes
    get!("/", controllers::home::index).name("home"),

    // Guest-only routes (redirect to dashboard if logged in). Each form on
    // these pages is a Live island any visitor may use.
    group!("/", {
        get!("/login", controllers::auth::login).name("login"),
        get!("/register", controllers::auth::register).name("register"),
        // Password reset: a link is mailed to a verified address and lands
        // on the reset form. Both steps are for someone who cannot sign in.
        get!("/forgot-password", controllers::password_reset::forgot).name("password.request"),
        get!("/reset-password", controllers::password_reset::reset).name("password.reset"),
    }).middleware(middleware::authenticate::guest()),

    // Protected routes (require authentication). Their islands belong to
    // the signed-in user, so Live also needs the tenant decision.
    group!("/", {
        get!("/dashboard", controllers::dashboard::index).name("dashboard"),
        // The signed-in user's own notes: the list and the form that writes
        // one, and one note. No route here lists accounts or another
        // user's notes.
        get!("/notes", controllers::notes::index).name("notes.index"),
        get!("/notes/{id}", controllers::notes::show).name("notes.show"),
        post!("/logout", controllers::auth::logout).name("logout"),
        // The display name over JSON, the endpoint the Inertia kits ship;
        // the dashboard's island saves it through Live.
        post!("/profile/name", controllers::profile::update_name).name("profile.name"),
        // Email verification: the notice and the mailed link's landing. The
        // framework only consumes a token for the account that is signed
        // in, so the link itself lives behind `auth()`.
        get!("/verify-email", controllers::email_verification::notice)
            .name("verification.notice"),
        get!("/verify-email/verify", controllers::email_verification::verify)
            .name("verification.verify"),
    }).middleware(middleware::authenticate::auth()).middleware(live::tenant()),

    // Static files. Every route above wins first; a `GET` or `HEAD` that
    // none of them matched is looked up under `public/`, where the kit's
    // stylesheet, `public/css/app.css`, lives. Dotfiles are refused, so is
    // path traversal, and a miss returns the same `404` the router
    // produces, which the error page renders.
    fallback!(StaticFiles::public().handler()),
}
