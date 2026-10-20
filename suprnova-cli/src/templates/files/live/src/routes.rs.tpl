//! The application's routes.
//!
//! Every page is a Live document: its controller mounts the page's islands
//! from `live::mounts()` and renders a view under `templates/live/pages/`.
//! `live::routes()` seals the same mounts into the router at startup, so a
//! route here and its islands there must name the same path. A route that
//! has a name carries the one Laravel's starter kits give it: the Live
//! actions redirect by these names. The agent pages are the exception:
//! plain documents with native forms (`controllers::agents`).

use suprnova::{ConsentPageHeaders, StaticFiles, fallback, get, group, post, routes};

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
        // user's notes. The list offers an agent in the browser the two
        // notes tools, the only tools a page offers.
        get!("/notes", controllers::notes::index).name("notes.index")
            .with_tools(["search_notes", "create_note"]),
        // The notes tools (`controllers::note_tools`), in JSON, for the
        // notes page and for MCP clients and agents acting for the user.
        // `/notes/search` is a static path, so it wins over `/notes/{id}`.
        get!("/notes/search", controllers::note_tools::search).name("notes.search"),
        post!("/notes/tools", controllers::note_tools::create).name("notes.create"),
        get!("/notes/{id}", controllers::notes::show).name("notes.show"),
        post!("/logout", controllers::auth::logout).name("logout"),
        // Where the sign-in and registration forms continue to when the
        // session holds an intended address: a Live action redirects only
        // to a named route.
        get!("/login/continue", controllers::auth::continue_after_sign_in).name("login.continue"),
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

    // Agents (AGENT-017): the approval page an MCP client sends the user
    // to and the claim page an agent sends the user to. A guest who opens
    // either signs in and comes back. No page here is framed, cached or
    // named in a referrer, and none offers an agent a tool.
    group!("/", {
        get!("/oauth/approve", controllers::agents::approval).name("agents.approval"),
        post!("/oauth/approve", controllers::agents::decide).name("agents.decide"),
        get!("/agent/claim", controllers::agents::claim).name("agents.claim"),
        post!("/agent/claim", controllers::agents::confirm).name("agents.confirm"),
    }).middleware(ConsentPageHeaders),

    // The signed-in user's connected agents: every MCP client and agent that
    // acts for them, each with a revoke form.
    group!("/", {
        get!("/connected-agents", controllers::agents::connected).name("agents.connected"),
        post!("/connected-agents/clients/{grant}/revoke", controllers::agents::revoke_client).name("agents.clients.revoke"),
        post!("/connected-agents/agents/{registration}/revoke", controllers::agents::revoke_agent).name("agents.accounts.revoke"),
    }).middleware(middleware::authenticate::auth()).middleware(live::tenant()).middleware(ConsentPageHeaders),

    // Static files. Every route above wins first; a `GET` or `HEAD` that
    // none of them matched is looked up under `public/`, where the kit's
    // stylesheet, `public/css/app.css`, lives. Dotfiles are refused, so is
    // path traversal, and a miss returns the same `404` the router
    // produces, which the error page renders.
    fallback!(StaticFiles::public().handler()),
}
