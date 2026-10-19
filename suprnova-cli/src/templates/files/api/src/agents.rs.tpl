//! Agents: the MCP endpoint at `/mcp`, the OAuth server MCP clients sign
//! users in through, and agent accounts under auth.md.
//!
//! This application has no frontend, so the approval page, the claim page
//! and the connected-agents page are the framework's own: plain HTML with
//! no script. A guest who opens the approval or the claim page signs in at
//! the kit's sign-in page (`controllers::sign_in`) and comes back, as does a
//! guest who opens the email verification link. Those pages are the only
//! ones a person uses in a browser, so they are the only ones with a
//! session ([`ConsentSession`]).
//!
//! Agent auth signs the identity assertions it gives agents with the key in
//! `AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY`. `suprnova new` wrote one
//! into `.env`. Every other environment needs its own, which
//! `cargo run -- agents:key` prints: the application does not start without
//! it, because a key made at each start would make nodes and restarts
//! disagree about every assertion.

use suprnova::agents::{AgentAuth, AgentsConfig, OAuthServer};
use suprnova::{
    FrameworkError, Middleware, Next, Request, Response, Router, SessionConfig, SessionMiddleware,
    async_trait,
};

/// The kit's sign-in page, where the approval and claim pages send a guest.
pub const SIGN_IN_PAGE: &str = "/login";

/// The query parameter a sign-in link names the page to return to in.
pub const RETURN_TO: &str = "return_to";

/// The OAuth server's routes: the authorization endpoint at
/// `/oauth/authorize`, and a user's decision at `/oauth/authorizations/{id}`.
pub const OAUTH_ROUTES: &str = "/oauth";

/// The framework's approval page.
pub const APPROVAL_PAGE: &str = "/oauth/approve";

/// The framework's connected-agents page. Its revoke forms post to paths
/// under it.
pub const CONNECTED_PAGE: &str = "/oauth/connected";

/// The framework's claim page, which is also the address agent auth gives
/// an agent to send its user to.
pub const CLAIM_PAGE: &str = "/agent/claim";

/// The page the email verification link opens
/// (`controllers::email_verification`). A claim needs a verified address,
/// and the page verifies it for the user signed in there.
pub const VERIFY_EMAIL_PAGE: &str = "/email/verify";

/// What the application serves to agents. Anonymous and `service_auth`
/// registrations are on. An agent holds no scope until a user claims it,
/// and then every scope the application declares with `tool_scopes!`.
pub fn config() -> AgentsConfig {
    AgentsConfig::new()
        .oauth_server(
            OAuthServer::new()
                .route_prefix(OAUTH_ROUTES)
                .sign_in(SIGN_IN_PAGE, RETURN_TO)
                .approval_page(APPROVAL_PAGE)
                .connected_agents_page(CONNECTED_PAGE),
        )
        .agent_auth(AgentAuth::new().claim_page(CLAIM_PAGE))
}

/// `router` with the MCP endpoint, both plugins' routes and pages, and the
/// discovery documents. A missing signing key or database fails here, at
/// boot, rather than on an agent's first request.
pub fn serve(router: Router) -> Result<Router, FrameworkError> {
    router.try_agents(config())
}

/// The browser session, on the pages a person uses in a browser and nowhere
/// else: the sign-in page, the approval, connected-agents and claim pages,
/// with the routes their forms post to, and the email verification page.
///
/// Every other route serves a client of the JSON API, an MCP client or an
/// agent, which sign in with a bearer token. A session on every path would
/// let any of them create a session row per request, and would give the
/// JSON sign-in a browser cookie. The allow-list is built from the paths
/// `config()` gives the plugins, so the two cannot drift apart.
pub struct ConsentSession(SessionMiddleware);

impl ConsentSession {
    /// The session middleware with `config`, run on the consent pages.
    pub fn new(config: SessionConfig) -> Self {
        Self(SessionMiddleware::new(config))
    }

    /// Whether a request to `path` gets the session.
    pub fn covers(path: &str) -> bool {
        let oauth = path.strip_prefix(OAUTH_ROUTES).unwrap_or_default();
        [
            SIGN_IN_PAGE,
            APPROVAL_PAGE,
            CONNECTED_PAGE,
            CLAIM_PAGE,
            VERIFY_EMAIL_PAGE,
        ]
        .contains(&path)
            || oauth == "/authorize"
            || oauth.starts_with("/authorizations/")
            || path
                .strip_prefix(CONNECTED_PAGE)
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

#[async_trait]
impl Middleware for ConsentSession {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if Self::covers(request.path()) {
            self.0.handle(request, next).await
        } else {
            next(request).await
        }
    }
}
