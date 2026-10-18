//! The authorization server's access tokens as MCP callers, and its
//! metadata at the well-known locations.

use std::sync::Arc;

use async_trait::async_trait;
use magnetar::authorization_server::{
    ACCESS_TOKEN_KIND, ACCESS_TOKEN_PREFIX, AccessVerification, AuthorizationServer,
    InvalidAccessReason, USER_GRANT_KIND,
};

use crate::agents::{
    AuthorizationServerMetadata, CallerResolver, MetadataKind, ResolveContext, ResolveError,
    ToolCaller,
};
use crate::auth::Auth;
use crate::error::FrameworkError;

/// Turns an access token of a user grant into a `user` caller.
///
/// How each answer of
/// [`verify_access_token`](AuthorizationServer::verify_access_token) maps:
///
/// | Verification | Answer |
/// |---|---|
/// | no `mat_` prefix | [`ResolveError::NotMine`], without a lookup |
/// | `Unknown` | [`ResolveError::NotMine`] |
/// | valid, user grant | `ToolCaller::user` with the token's scopes and client |
/// | valid, any other grant (agent auth's) | [`ResolveError::NotMine`] |
/// | `Expired`, `Revoked`, `WrongResource`, `EpochChanged` | [`ResolveError::Invalid`] |
/// | storage failure | [`ResolveError::Unavailable`] |
///
/// `Unknown` covers a token the store has no digest of, a refresh token
/// and a token past the length limit alike: the store cannot tell a
/// token it never issued from one it forgot. Either way this resolver
/// accepts nothing for it, and the next resolver is asked; a token no
/// resolver accepts is refused, so `NotMine` never lets a token through.
pub(crate) struct UserTokens {
    server: Arc<AuthorizationServer>,
}

impl UserTokens {
    pub(crate) fn new(server: Arc<AuthorizationServer>) -> Self {
        Self { server }
    }
}

#[async_trait]
impl CallerResolver for UserTokens {
    async fn resolve(
        &self,
        token: &str,
        ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        if !token.starts_with(ACCESS_TOKEN_PREFIX) {
            return Err(ResolveError::NotMine);
        }
        let verified = self
            .server
            .verify_access_token(token, ctx.resource)
            .await
            .map_err(|error| {
                ResolveError::Unavailable(FrameworkError::internal(format!(
                    "the authorization server could not check a token: {error}"
                )))
            })?;
        let access = match verified {
            AccessVerification::Valid(access) => access,
            AccessVerification::Invalid(reason) => {
                return Err(match reason {
                    InvalidAccessReason::Unknown => ResolveError::NotMine,
                    InvalidAccessReason::Expired => ResolveError::Invalid("The token expired."),
                    InvalidAccessReason::Revoked => {
                        ResolveError::Invalid("The token was revoked.")
                    }
                    InvalidAccessReason::WrongResource => {
                        ResolveError::Invalid("The token was issued for another resource.")
                    }
                    InvalidAccessReason::EpochChanged => ResolveError::Invalid(
                        "The token ended with the user's sign-in: their password changed or \
                         they signed out everywhere.",
                    ),
                });
            }
        };
        // Agent auth issues its tokens on this server too, under a grant
        // kind of its own; its resolver decides those.
        if access.grant_kind != USER_GRANT_KIND || access.token_kind != ACCESS_TOKEN_KIND {
            return Err(ResolveError::NotMine);
        }
        let Some(user_id) = access.user_id.as_deref() else {
            return Err(ResolveError::Invalid("The token stands for no user."));
        };
        let user = load_user(user_id).await?;
        Ok(ToolCaller::user(user)
            .scopes(&access.scopes)
            .client(access.client_id.clone()))
    }

    fn authorization_servers(&self) -> Vec<String> {
        vec![self.server.issuer().to_owned()]
    }
}

/// The user `user_id` as the default guard's provider returns it, so a
/// token's caller is the same record a browser session of that user
/// resolves to.
pub(crate) async fn load_user(
    user_id: &str,
) -> Result<Arc<dyn crate::auth::Authenticatable>, ResolveError> {
    let guard_name = Auth::default_guard_name();
    let Some(provider) = crate::auth::epoch::guard_provider(&guard_name) else {
        return Err(ResolveError::Unavailable(FrameworkError::internal(format!(
            "the default guard '{guard_name}' has no user provider to load a token's user"
        ))));
    };
    match provider.retrieve_by_id(user_id).await {
        Ok(Some(user)) => Ok(user),
        Ok(None) => Err(ResolveError::Invalid("The token's user no longer exists.")),
        Err(error) => Err(ResolveError::Unavailable(error)),
    }
}

/// The authorization server's metadata document, served for both
/// well-known kinds: clients of the 2025-11-25 revision look under
/// `openid-configuration` as well.
pub(crate) struct ServerMetadata {
    server: Arc<AuthorizationServer>,
}

impl ServerMetadata {
    pub(crate) fn new(server: Arc<AuthorizationServer>) -> Self {
        Self { server }
    }
}

#[async_trait]
impl AuthorizationServerMetadata for ServerMetadata {
    async fn document(&self, _issuer: &str, _kind: MetadataKind) -> Option<serde_json::Value> {
        Some(self.server.metadata_document())
    }

    fn issuer(&self, _issuer: &str) -> String {
        self.server.issuer().to_owned()
    }
}
