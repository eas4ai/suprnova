//! The API kit (AGENT-017): the MCP endpoint and both plugins with the
//! framework's default approval, claim and connected-agents pages, the
//! browser sign-in those pages need, and its migrations.
//!
//! These read what `suprnova new --api` writes. The compiled proof is
//! `agent_017_a_generated_api_application_serves_agents` in
//! `scaffold_snapshot.rs`.

use crate::shared::{Project, contains, item};

/// AGENT-017: the API kit serves the MCP endpoint, the OAuth server and
/// agent auth with the framework's default pages, which exist for an
/// application without a frontend, at paths it names once as constants,
/// and its entry serves its routes through `agents::serve`. It ships no
/// browser helpers: it has no frontend to put them in (ruling 4).
#[test]
fn agent_017_api_kit_wires_the_endpoint_and_both_plugins() {
    let project = Project::new("api");
    let agents = project.read("src/agents.rs");
    for part in [
        "pub const SIGN_IN_PAGE: &str = \"/login\";",
        "pub const RETURN_TO: &str = \"return_to\";",
        "pub const OAUTH_ROUTES: &str = \"/oauth\";",
        "pub const APPROVAL_PAGE: &str = \"/oauth/approve\";",
        "pub const CONNECTED_PAGE: &str = \"/oauth/connected\";",
        "pub const CLAIM_PAGE: &str = \"/agent/claim\";",
        "AgentsConfig::new()",
        ".route_prefix(OAUTH_ROUTES)",
        ".sign_in(SIGN_IN_PAGE, RETURN_TO)",
        ".approval_page(APPROVAL_PAGE)",
        ".connected_agents_page(CONNECTED_PAGE)",
        ".agent_auth(AgentAuth::new().claim_page(CLAIM_PAGE))",
        "pub fn serve(router: Router) -> Result<Router, FrameworkError>",
        "router.try_agents(config())",
    ] {
        assert!(
            contains(&agents, part),
            "api: src/agents.rs lacks `{part}`:\n{agents}"
        );
    }
    // The framework's own pages, and no tool shared onto every page.
    for absent in [
        "custom_approval_page",
        "custom_claim_page",
        "custom_connected_agents_page",
        "share_tools",
    ] {
        assert!(
            !agents.contains(absent),
            "api: src/agents.rs must not call `{absent}`:\n{agents}"
        );
    }
    let lib = project.read("src/lib.rs");
    assert!(
        lib.lines().any(|line| line == "pub mod agents;"),
        "api: src/lib.rs must declare `pub mod agents;`:\n{lib}"
    );
    let main = project.read("src/main.rs");
    for part in [
        "::{agents, bootstrap, config, migrations, routes};",
        ".try_routes(|| agents::serve(routes::register()))",
    ] {
        assert!(
            contains(&main, part),
            "api: src/main.rs lacks `{part}`:\n{main}"
        );
    }
    assert!(
        !project.root.join("frontend").exists(),
        "api: the kit has no frontend, so no browser helpers"
    );
}

/// AGENT-017: the API kit's migrator creates the table the browser
/// sessions of the sign-in live in, and both plugins' tables, after the
/// users table.
#[test]
fn agent_017_api_kit_migrator_creates_the_sessions_and_both_plugins_tables() {
    let project = Project::new("api");
    let migrations = project.read("src/migrations/mod.rs");
    let squashed = migrations.split_whitespace().collect::<String>();
    let users = squashed
        .find("Box::new(m20240101_000001_create_users_table::Migration)")
        .unwrap_or_else(|| panic!("api: no users migration:\n{migrations}"));
    let sessions = squashed
        .find(
            "Box::new(suprnova::session::migrations::CreateSessionsTable::new(suprnova::session::migrations::SessionUserKey::Integer,))",
        )
        .unwrap_or_else(|| panic!("api: no sessions table:\n{migrations}"));
    let plugins = squashed
        .find("migrations.extend(suprnova::agents::migrations());")
        .unwrap_or_else(|| panic!("api: no plugins' tables:\n{migrations}"));
    assert!(
        users < sessions && sessions < plugins,
        "api: the users table comes first:\n{migrations}"
    );
}

/// AGENT-017: the default approval and claim pages act for a browser
/// session, so the kit runs a session on the consent pages only
/// (`agents::ConsentSession`), ahead of the bearer token, and serves a
/// sign-in page for them at the path `src/agents.rs` gives the OAuth
/// server. The page carries the consent headers, and its form posts
/// through route CSRF: the JSON API keeps taking no token. The gated
/// group stays first, as `template_drift.rs` reads it.
#[test]
fn agent_017_api_kit_signs_a_browser_in_for_the_consent_pages() {
    let project = Project::new("api");
    let bootstrap = project.read("src/bootstrap.rs");
    let session = bootstrap
        .find("global_middleware!(agents::ConsentSession::new(SessionConfig::from_env()));")
        .unwrap_or_else(|| panic!("api: no consent session:\n{bootstrap}"));
    let bearer = bootstrap
        .find("global_middleware!(BearerTokenMiddleware);")
        .expect("the bearer middleware");
    assert!(
        session < bearer,
        "api: the session runs before the bearer token:\n{bootstrap}"
    );
    assert!(
        !bootstrap.contains("global_middleware!(SessionMiddleware"),
        "api: no session on every path:\n{bootstrap}"
    );
    assert!(
        !bootstrap.contains("CsrfMiddleware"),
        "api: no global CSRF middleware, so the JSON API takes no token:\n{bootstrap}"
    );

    let routes = project.read("src/routes.rs");
    let gated = routes.find("group!(\"/api\"").expect("the gated group");
    let consent = routes
        .find("group!(\"/\"")
        .unwrap_or_else(|| panic!("api: no sign-in group:\n{routes}"));
    assert!(
        gated < consent,
        "api: the gated group stays first:\n{routes}"
    );
    let sign_in = &routes[consent..];
    for part in [
        "get!(agents::SIGN_IN_PAGE, controllers::sign_in::show),",
        "post!(agents::SIGN_IN_PAGE, controllers::sign_in::store).middleware(CsrfMiddleware::new().without_xsrf_cookie()),",
        "}).middleware(ConsentPageHeaders),",
    ] {
        assert!(
            contains(sign_in, part),
            "api: the sign-in routes lack `{part}`:\n{routes}"
        );
    }
    let controllers = project.read("src/controllers/mod.rs");
    assert!(
        controllers.lines().any(|line| line == "pub mod sign_in;"),
        "api: src/controllers/mod.rs must declare `pub mod sign_in;`:\n{controllers}"
    );
}

/// AGENT-017: the session runs only on the browser pages, by an allow-list
/// built from the paths `config()` gives the plugins, so the two cannot
/// drift apart. A session on every path would let any client or agent
/// create a session row per request.
#[test]
fn agent_017_api_kit_runs_the_session_on_the_consent_pages_only() {
    let project = Project::new("api");
    let agents = project.read("src/agents.rs");
    for part in [
        "pub struct ConsentSession(SessionMiddleware);",
        "pub fn covers(path: &str) -> bool {",
        "[SIGN_IN_PAGE, APPROVAL_PAGE, CONNECTED_PAGE, CLAIM_PAGE, VERIFY_EMAIL_PAGE,].contains(&path)",
        "path.strip_prefix(OAUTH_ROUTES)",
        "== \"/authorize\"",
        ".starts_with(\"/authorizations/\")",
        "path.strip_prefix(CONNECTED_PAGE)",
        "if Self::covers(request.path()) {",
        "self.0.handle(request, next).await",
        "next(request).await",
        "a session row per request",
    ] {
        assert!(
            contains(&agents, part),
            "api: src/agents.rs lacks `{part}`:\n{agents}"
        );
    }
}

/// AGENT-017: a claim needs a verified address, so the kit verifies one
/// through a mailed link. Registration mails it to a new account, a bearer
/// route mails a new one, and the link opens a page on the consent session
/// that verifies the address for the user signed in there, sending a guest
/// to sign in and back. Nothing in the kit writes `email_verified_at`.
#[test]
fn agent_017_api_kit_verifies_email_through_a_mailed_link() {
    let project = Project::new("api");
    let agents = project.read("src/agents.rs");
    assert!(
        contains(
            &agents,
            "pub const VERIFY_EMAIL_PAGE: &str = \"/email/verify\";"
        ),
        "{agents}"
    );
    let routes = project.read("src/routes.rs");
    let gated_start = routes.find("group!(\"/api\"").expect("the gated group");
    let gated_end = gated_start
        + routes[gated_start..]
            .find("}).middleware(AuthMiddleware::new()),")
            .expect("the gated group's end");
    assert!(
        contains(
            &routes[gated_start..gated_end],
            "post!(\"/email/verification-notification\", controllers::email_verification::notification).middleware(ThrottleRequestsMiddleware::with(6, 1, \"email-verification\")),"
        ),
        "api: the resend route is the signed-in user's, six a minute:\n{routes}"
    );
    let consent = routes.find("group!(\"/\"").expect("the consent group");
    assert!(
        contains(
            &routes[consent..],
            "get!(agents::VERIFY_EMAIL_PAGE, controllers::email_verification::verify),"
        ),
        "api: the link's page is a consent page:\n{routes}"
    );
    let controllers = project.read("src/controllers/mod.rs");
    assert!(
        controllers
            .lines()
            .any(|line| line == "pub mod email_verification;"),
        "{controllers}"
    );
    let users = project.read("src/controllers/users.rs");
    let register = item(&users, "register");
    for part in [
        "Registration::Created(user)",
        "Auth::email_verification().issue_for(&user)",
        "email_verification::mail_link(&link)",
    ] {
        assert!(
            contains(register, part),
            "api: registration lacks `{part}`:\n{register}"
        );
    }
    let verification = project.read("src/controllers/email_verification.rs");
    for part in [
        "Auth::email_verification().issue()",
        "EmailVerificationMail {",
        "Auth::email_verification().verify(&token)",
        "Redirect::to(SIGN_IN_PAGE)",
        ".query(RETURN_TO,",
    ] {
        assert!(
            contains(&verification, part),
            "api: email verification lacks `{part}`:\n{verification}"
        );
    }
    for entry in walkdir::WalkDir::new(project.root.join("src")) {
        let entry = entry.expect("walk the scaffold");
        if entry.file_type().is_file() {
            let source = std::fs::read_to_string(entry.path()).expect("read a source file");
            assert!(
                !source.contains("email_verified_at =")
                    && !source.contains("SET email_verified_at"),
                "api: {} writes email_verified_at itself",
                entry.path().display()
            );
        }
    }
}

/// AGENT-017: `Auth::user()` reads the `app_users` table Magnetar keeps, so
/// the framework's pages and the connected-agents routes find the
/// signed-in user, of a session or a bearer token.
#[test]
fn agent_017_api_kit_registers_a_user_provider_for_app_users() {
    let project = Project::new("api");
    let bootstrap = project.read("src/bootstrap.rs");
    let register = item(&bootstrap, "register");
    let magnetar = register.find("init_magnetar(").expect("Magnetar");
    let provider = register
        .find("bind!(")
        .unwrap_or_else(|| panic!("api: no user provider:\n{register}"));
    assert!(magnetar < provider, "{register}");
    assert!(
        contains(
            register,
            "bind!(dyn UserProvider, DatabaseUserProvider::new(\"app_users\").password_column(\"password_hash\"));"
        ),
        "api: the provider reads app_users:\n{register}"
    );
}

/// AGENT-017: the sign-in page is fixed markup with one value in it, the
/// session's CSRF token, escaped. `return_to` is read only through
/// `url::same_origin_path`, so a sign-in link cannot send anyone to
/// another site. An account with a second factor is told this page does
/// not offer one, and stays signed out.
#[test]
fn agent_017_api_sign_in_page_is_fixed_markup_and_keeps_return_to_on_this_origin() {
    let project = Project::new("api");
    let sign_in = project.read("src/controllers/sign_in.rs");
    assert!(
        !sign_in.contains("format!("),
        "api: the sign-in page formats nothing into its markup:\n{sign_in}"
    );
    let page = item(&sign_in, "page");
    assert!(
        contains(page, "&escape(&token)"),
        "api: the page interpolates the escaped CSRF token:\n{page}"
    );
    assert!(
        contains(&sign_in, "use crate::agents::{CONNECTED_PAGE, RETURN_TO};"),
        "api: the sign-in page takes its paths from src/agents.rs:\n{sign_in}"
    );

    // Every read of `return_to` goes through the check, before the address
    // is stored or followed.
    let reads = sign_in.matches(".query_param(RETURN_TO)").count();
    assert!(
        reads > 0,
        "api: the sign-in page reads return_to:\n{sign_in}"
    );
    let checked = sign_in
        .split_whitespace()
        .collect::<String>()
        .matches(".query_param(RETURN_TO).and_then(|candidate|url::same_origin_path(&candidate))")
        .count();
    assert_eq!(
        reads, checked,
        "api: every read of return_to goes through url::same_origin_path:\n{sign_in}"
    );
    let show = item(&sign_in, "show");
    let read = show.find(".query_param(RETURN_TO)").expect("read");
    for use_of_it in ["Redirect::set_intended_url(", "Redirect::to("] {
        let used = show
            .find(use_of_it)
            .unwrap_or_else(|| panic!("api: `show` lacks `{use_of_it}`:\n{show}"));
        assert!(read < used, "api: return_to is checked first:\n{show}");
    }

    let store = item(&sign_in, "store");
    for part in [
        "Auth::password()",
        ".authenticate_outcome(",
        "SignInOutcome::Authenticated",
        "Redirect::intended(CONNECTED_PAGE)",
        "SignInOutcome::FactorRequired",
        "page(422, Some(SECOND_FACTOR))",
        "page(422, Some(INVALID))",
    ] {
        assert!(
            contains(store, part),
            "api: `store` lacks `{part}`:\n{store}"
        );
    }
    assert!(
        contains(
            &sign_in,
            "\"This account signs in with a second factor, which this page does not offer.\""
        ),
        "{sign_in}"
    );
}

/// AGENT-017: a client of the JSON API lists and revokes the MCP clients
/// and agents that act for its user, behind the same gate as the user
/// directory, through `ConnectedAgents`. Revoking takes `DELETE`, never
/// `POST`: a plain form can post from another site, and the JSON API runs
/// no CSRF check.
#[test]
fn agent_017_api_kit_lists_and_revokes_over_json() {
    let project = Project::new("api");
    let routes = project.read("src/routes.rs");
    let gated_start = routes.find("group!(\"/api\"").expect("the gated group");
    let gated_end = gated_start
        + routes[gated_start..]
            .find("}).middleware(AuthMiddleware::new()),")
            .unwrap_or_else(|| {
                panic!("api: the gated group ends behind AuthMiddleware:\n{routes}")
            });
    let gated = &routes[gated_start..gated_end];
    for part in [
        "get!(\"/connected-agents\", controllers::agents::index),",
        "delete!(\"/connected-agents/clients/:grant\", controllers::agents::revoke_client),",
        "delete!(\"/connected-agents/agents/:registration\", controllers::agents::revoke_agent),",
    ] {
        assert!(
            contains(gated, part),
            "api: the gated group lacks `{part}`:\n{routes}"
        );
    }
    assert_eq!(
        routes.matches("connected-agents").count(),
        3,
        "api: no other route reaches connected agents, a POST least of all:\n{routes}"
    );

    let controllers = project.read("src/controllers/mod.rs");
    assert!(
        controllers.lines().any(|line| line == "pub mod agents;"),
        "api: src/controllers/mod.rs must declare `pub mod agents;`:\n{controllers}"
    );
    let agents = project.read("src/controllers/agents.rs");
    for (handler, parts) in [
        (
            "index",
            &[
                "ConnectedAgents::client_grants(&req)",
                "ConnectedAgents::agent_accounts(&req)",
                "\"clients\"",
                "\"agents\"",
            ][..],
        ),
        (
            "revoke_client",
            &[
                "req.param(\"grant\")",
                "ConnectedAgents::revoke_client_grant(&req,",
                "status(204)",
                "\"No such connected app.\"",
                "status(404)",
            ][..],
        ),
        (
            "revoke_agent",
            &[
                "req.param(\"registration\")",
                "ConnectedAgents::revoke_agent_account(&req,",
                "status(204)",
                "\"No such connected agent.\"",
                "status(404)",
            ][..],
        ),
    ] {
        let body = item(&agents, handler);
        for part in parts {
            assert!(
                contains(body, part),
                "api: `{handler}` lacks `{part}`:\n{body}"
            );
        }
    }
}

/// Items 3 and 4: registration folds the address before its rules run,
/// bounds it at 254 characters, and asks for a password of eight
/// characters, as the other kits do.
#[test]
fn kit_api_register_bounds_the_email_and_the_password() {
    let project = Project::new("api");
    let users = project.read("src/controllers/users.rs");
    let register = item(&users, "RegisterRequest");
    for part in [
        "email(message = \"Please enter a valid email address\")",
        "length(max = 254, message = \"Email must be at most 254 characters\")",
        "#[validate(length(min = 8, message = \"Password must be at least 8 characters\"))]",
    ] {
        assert!(
            contains(register, part),
            "api: `RegisterRequest` lacks `{part}`:\n{register}"
        );
    }
}
