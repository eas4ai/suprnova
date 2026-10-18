//! What the signed-in user has let clients do, and taking it back
//! (AGENT-010, the client half).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use magnetar::authorization_server::{AuthorizationServer, ClientGrantSummary};
use serde::Serialize;

use crate::auth::Auth;
use crate::container::App;
use crate::error::FrameworkError;

/// One client a user approved to act as them: an MCP client holding
/// tokens of this grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientGrant {
    /// The grant's identifier, to revoke it by.
    pub id: String,
    /// The client's identifier: its metadata document URL, or the id
    /// dynamic registration gave it.
    pub client_id: String,
    /// The client's name when the user approved it. The client chose it,
    /// so a page shows it beside [`client_host`](Self::client_host).
    pub client_name: String,
    /// The host the approval page showed for the client, which the client
    /// cannot choose freely: its metadata document's host, or its redirect
    /// URI's.
    pub client_host: String,
    /// What the grant allows, with the descriptions the approval page
    /// showed.
    pub scopes: Vec<GrantScope>,
    /// When the user approved the client.
    pub created_at: DateTime<Utc>,
    /// When the client last used the grant, to the minute.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// One scope of a grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GrantScope {
    /// The scope's name, as a token carries it.
    pub name: String,
    /// What the scope allows, in words for the user.
    pub description: String,
}

impl From<ClientGrantSummary> for ClientGrant {
    fn from(summary: ClientGrantSummary) -> Self {
        Self {
            id: summary.grant_id,
            client_id: summary.client_id,
            client_name: summary.client_name,
            client_host: summary.client_host,
            scopes: summary
                .scopes
                .into_iter()
                .map(|scope| GrantScope {
                    name: scope.name,
                    description: scope.description,
                })
                .collect(),
            created_at: summary.created_at,
            last_used_at: summary.last_used_at,
        }
    }
}

/// The agents and clients bound to the signed-in user, for an
/// application's own connected-agents page or JSON routes.
///
/// Every method acts for the user the default guard signed in, and only on
/// that user's grants: an id of another user's grant is not found.
///
/// ```rust,ignore
/// use suprnova::agents::ConnectedAgents;
///
/// #[handler]
/// pub async fn index() -> Response {
///     let grants = ConnectedAgents::client_grants().await?;
///     Ok(HttpResponse::json(serde_json::json!({ "grants": grants })))
/// }
/// ```
pub struct ConnectedAgents;

impl ConnectedAgents {
    /// The client grants of the signed-in user that still work, newest
    /// first.
    ///
    /// # Errors
    ///
    /// 401 for a guest, 500 when no OAuth server is mounted, and 503 when
    /// the authorization server's store cannot answer.
    pub async fn client_grants() -> Result<Vec<ClientGrant>, FrameworkError> {
        let server = mounted()?;
        let user_id = signed_in_user_id().await?;
        list(&server, &user_id).await
    }

    /// Revoke the signed-in user's grant `grant_id` and every token of it.
    /// Returns `false` when the user has no such live grant, including one
    /// that belongs to another user.
    ///
    /// # Errors
    ///
    /// As [`client_grants`](Self::client_grants).
    pub async fn revoke_client_grant(grant_id: &str) -> Result<bool, FrameworkError> {
        let server = mounted()?;
        let user_id = signed_in_user_id().await?;
        revoke(&server, &user_id, grant_id).await
    }
}

/// The OAuth server the application's router mounted.
#[derive(Clone)]
struct MountedOAuthServer(Arc<AuthorizationServer>);

/// Make `server` the one [`ConnectedAgents`] reads. An application serves
/// one MCP endpoint, so it mounts one server; the latest mount wins.
pub(super) fn install(server: Arc<AuthorizationServer>) {
    App::singleton(MountedOAuthServer(server));
}

fn mounted() -> Result<Arc<AuthorizationServer>, FrameworkError> {
    App::get::<MountedOAuthServer>()
        .map(|mounted| mounted.0)
        .ok_or_else(|| {
            FrameworkError::internal(
                "no OAuth server is mounted: serve agents with AgentsConfig::oauth_server",
            )
        })
}

/// The default guard's signed-in user, or 401.
async fn signed_in_user_id() -> Result<String, FrameworkError> {
    let guard_name = Auth::default_guard_name();
    let user = match App::get::<crate::auth::AuthManager>() {
        Some(manager) => manager.guard(&guard_name)?.user().await?,
        None => Auth::user().await?,
    };
    user.map(|user| user.get_auth_identifier())
        .ok_or(FrameworkError::Unauthorized)
}

/// `user_id`'s live client grants, newest first.
pub(super) async fn list(
    server: &AuthorizationServer,
    user_id: &str,
) -> Result<Vec<ClientGrant>, FrameworkError> {
    let mut grants: Vec<ClientGrant> = server
        .list_grants(user_id)
        .await
        .map_err(store_unavailable)?
        .into_iter()
        // Agent auth keeps its grants on this server too; it lists them
        // with the registrations they belong to.
        .filter(|grant| grant.kind == magnetar::authorization_server::USER_GRANT_KIND)
        .map(ClientGrant::from)
        .collect();
    grants.sort_by_key(|grant| std::cmp::Reverse(grant.created_at));
    Ok(grants)
}

/// Revoke `user_id`'s client grant `grant_id`; `false` when they hold no
/// such live client grant.
pub(super) async fn revoke(
    server: &AuthorizationServer,
    user_id: &str,
    grant_id: &str,
) -> Result<bool, FrameworkError> {
    let owned = list(server, user_id)
        .await?
        .iter()
        .any(|grant| grant.id == grant_id);
    if !owned {
        return Ok(false);
    }
    server
        .revoke_grant(user_id, grant_id)
        .await
        .map_err(store_unavailable)
}

fn store_unavailable(error: magnetar::Error) -> FrameworkError {
    tracing::error!(%error, "the authorization server's store failed");
    FrameworkError::domain("the authorization server is unavailable", 503)
}
