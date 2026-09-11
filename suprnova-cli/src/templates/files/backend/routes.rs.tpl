use suprnova::{StaticFiles, fallback, get, group, post, routes};

use crate::controllers;
use crate::middleware;

routes! {
    // Public routes
    get!("/", controllers::home::index),

    // Guest-only routes (redirect to dashboard if logged in)
    group!("/", {
        get!("/login", controllers::auth::show_login),
        post!("/login", controllers::auth::login),
        get!("/register", controllers::auth::show_register),
        post!("/register", controllers::auth::register),
    }).middleware(middleware::authenticate::guest()),

    // Protected routes (require authentication)
    group!("/", {
        get!("/dashboard", controllers::dashboard::index),
        post!("/logout", controllers::auth::logout),
    }).middleware(middleware::authenticate::auth()),

    // Static files. Every route above wins first; a `GET` or `HEAD` that
    // none of them matched is looked up under `public/`. That is where
    // `npm run build` writes the hashed frontend bundle (`public/assets/`)
    // and where the production image copies it, so without this fallback
    // the HTML shell references `/assets/*` URLs that nothing serves once
    // Vite's dev server is out of the picture. Dotfiles are refused
    // (`public/assets/.vite/manifest.json` stays private), so is path
    // traversal, and a miss returns the same `404` the router produces,
    // which the Inertia error-page middleware renders as the `Error` page.
    fallback!(StaticFiles::public().handler()),
}
