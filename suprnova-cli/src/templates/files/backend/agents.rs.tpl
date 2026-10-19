//! Agents: the MCP endpoint at `/mcp`, the OAuth server MCP clients sign
//! users in through, and agent accounts under auth.md. The approval page,
//! the claim page and the connected-agents page are this application's own.
//!
//! Their paths are the framework's defaults, so a link a client or an agent
//! holds opens the same page with or without this kit. A guest who opens
//! the approval or the claim page signs in at `/login` and comes back.
//!
//! Agent auth signs the identity assertions it gives agents with the key in
//! `AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY`. `suprnova new` wrote one
//! into `.env`. Every other environment needs its own, which
//! `cargo run -- agents:key` prints: the application does not start without
//! it, because a key made at each start would make nodes and restarts
//! disagree about every assertion.

use suprnova::agents::{AgentAuth, AgentsConfig, OAuthServer};
use suprnova::{FrameworkError, Router};

suprnova::tool_scopes! {
    /// What a client or an agent may do for a user. A token holds the
    /// scopes the user approved, and each tool names the ones it needs.
    /// The approval page shows each description, and a claimed agent holds
    /// them all. The notes tools (`controllers::note_tools`) use these two.
    pub mod scope {
        NOTES_READ = "notes:read" => "Read your notes";
        NOTES_WRITE = "notes:write" => "Write notes for you";
    }
}

/// What the application serves to agents. Anonymous and `service_auth`
/// registrations are on. An agent holds no scope until a user claims it,
/// and then every scope the application declares with `tool_scopes!`.
pub fn config() -> AgentsConfig {
    AgentsConfig::new()
        .oauth_server(
            OAuthServer::new()
                .custom_approval_page("/oauth/approve")
                .custom_connected_agents_page(),
        )
        .agent_auth(AgentAuth::new().custom_claim_page("/agent/claim"))
}

/// `router` with the MCP endpoint, both plugins' routes and the discovery
/// documents. A missing signing key or database fails here, at boot, rather
/// than on an agent's first request.
pub fn serve(router: Router) -> Result<Router, FrameworkError> {
    router.try_agents(config())
}
