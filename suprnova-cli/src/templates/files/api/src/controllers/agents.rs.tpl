//! Connected agents over JSON: the MCP clients the signed-in user approved
//! and the agents they claimed, and taking either back. The framework's
//! page at `/oauth/connected` shows a person the same in a browser.
//!
//! These routes sit behind `AuthMiddleware`, as the user directory does,
//! and act for the user of the bearer token from `POST /api/auth/login`
//! only: another user's id is not found. They get no browser session
//! (`agents::ConsentSession`), and an MCP access token is not a sign-in to
//! this API, so either gets `401` here. Revoking answers `DELETE`, never
//! `POST`: a plain form on another site can send `POST`, and the JSON API
//! runs no CSRF check.

use suprnova::{ConnectedAgents, FrameworkError, HttpResponse, Request, Response, handler};

// ============================================================================
// GET /api/connected-agents
// ============================================================================

/// Every client grant and agent account of the signed-in user that still
/// works, each newest first.
#[handler]
pub async fn index(req: Request) -> Response {
    let clients = ConnectedAgents::client_grants(&req).await?;
    let agents = ConnectedAgents::agent_accounts(&req).await?;
    Ok(HttpResponse::json(serde_json::json!({
        "clients": clients,
        "agents": agents,
    })))
}

// ============================================================================
// DELETE /api/connected-agents/clients/:grant
// ============================================================================

/// Revoke a client grant and every token of it.
#[handler]
pub async fn revoke_client(req: Request) -> Response {
    let grant = req.param("grant").map_err(FrameworkError::from)?;
    if !ConnectedAgents::revoke_client_grant(&req, grant).await? {
        let gone = serde_json::json!({ "message": "No such connected app." });
        return Ok(HttpResponse::json(gone).status(404));
    }
    Ok(HttpResponse::new().status(204))
}

// ============================================================================
// DELETE /api/connected-agents/agents/:registration
// ============================================================================

/// Revoke an agent account and every token issued from it.
#[handler]
pub async fn revoke_agent(req: Request) -> Response {
    let registration = req.param("registration").map_err(FrameworkError::from)?;
    if !ConnectedAgents::revoke_agent_account(&req, registration).await? {
        let gone = serde_json::json!({ "message": "No such connected agent." });
        return Ok(HttpResponse::json(gone).status(404));
    }
    Ok(HttpResponse::new().status(204))
}
