use suprnova::{ConsentPageHeaders, StaticFiles, fallback, get, group, post, routes};

use crate::controllers;
use crate::middleware;

routes! {
    // Public routes
    get!("/", controllers::home::index),

    // Guest-only routes (redirect to dashboard if logged in). A route that
    // has a name carries the one Laravel's starter kits give it, so code
    // and generated route types that look routes up by name read the same
    // names a Laravel application does.
    group!("/", {
        get!("/login", controllers::auth::show_login).name("login"),
        post!("/login", controllers::auth::login)
            .middleware(suprnova::LoginThrottle::request_context()),
        get!("/register", controllers::auth::show_register).name("register"),
        post!("/register", controllers::auth::register),
        // Password reset: a link is mailed to a verified address and lands
        // on the reset form. Both steps are for someone who cannot sign in.
        get!("/forgot-password", controllers::password_reset::forgot).name("password.request"),
        post!("/forgot-password", controllers::password_reset::send_link).name("password.email"),
        get!("/reset-password", controllers::password_reset::reset_form).name("password.reset"),
        post!("/reset-password", controllers::password_reset::reset).name("password.update"),
    }).middleware(middleware::authenticate::guest()),

    // Protected routes (require authentication)
    group!("/", {
        get!("/dashboard", controllers::dashboard::index).name("dashboard"),
        // The signed-in user's own notes: the list, one note, and the form
        // that writes one. No route here lists accounts or another user's
        // notes. The list offers an agent in the browser the two notes
        // tools, the only tools a page offers.
        get!("/notes", controllers::notes::index).name("notes.index")
            .with_tools(["search_notes", "create_note"]),
        // The notes tools (`controllers::note_tools`), in JSON, for the
        // notes page and for MCP clients and agents acting for the user.
        // `/notes/search` is a static path, so it wins over `/notes/{id}`.
        get!("/notes/search", controllers::note_tools::search).name("notes.search"),
        get!("/notes/{id}", controllers::notes::show).name("notes.show"),
        post!("/notes", controllers::notes::store).name("notes.store"),
        post!("/notes/tools", controllers::note_tools::create).name("notes.create"),
        // JSON in and out, for the dashboard's display-name form.
        post!("/profile/name", controllers::profile::update_name).name("profile.name"),
        post!("/logout", controllers::auth::logout).name("logout"),
        // Email verification: the notice, a resend, and the mailed link's
        // landing. The framework only consumes a token for the account
        // that is signed in, so the link itself lives behind `auth()`.
        get!("/verify-email", controllers::email_verification::notice)
            .name("verification.notice"),
        post!("/email/verification-notification", controllers::email_verification::resend)
            .name("verification.send"),
        get!("/verify-email/verify", controllers::email_verification::verify)
            .name("verification.verify"),
    }).middleware(middleware::authenticate::auth()),

    // Agents: the approval page an MCP client sends the user to, the claim
    // page an agent sends the user to, and the user's connected agents
    // (`controllers::agents`, served with `src/agents.rs`). A guest who
    // opens the first two signs in and comes back. `ConsentPageHeaders`
    // keeps every answer here out of frames, caches and referrers. No route
    // here offers a tool: consent comes from the person.
    group!("/", {
        get!("/oauth/approve", controllers::agents::approval).name("agents.approval"),
        post!("/oauth/approve", controllers::agents::decide).name("agents.decide"),
        get!("/agent/claim", controllers::agents::claim).name("agents.claim"),
        post!("/agent/claim", controllers::agents::confirm).name("agents.confirm"),
    }).middleware(ConsentPageHeaders),
    group!("/", {
        get!("/connected-agents", controllers::agents::connected).name("agents.connected"),
        post!("/connected-agents/clients/{grant}/revoke", controllers::agents::revoke_client)
            .name("agents.clients.revoke"),
        post!("/connected-agents/agents/{registration}/revoke", controllers::agents::revoke_agent)
            .name("agents.accounts.revoke"),
    }).middleware(middleware::authenticate::auth()).middleware(ConsentPageHeaders),

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
