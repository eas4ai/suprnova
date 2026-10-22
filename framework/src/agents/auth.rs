//! Turning a bearer token into a caller, and the authorization server's
//! discovery documents (AGENT-004, AGENT-005).
//!
//! The framework verifies no token itself. An application registers
//! [`CallerResolver`]s on [`AgentsConfig`](super::AgentsConfig), such as the
//! Magnetar authorization server or an agent auth plugin, and the endpoint
//! asks them in order. A token no resolver accepts is refused: the endpoint
//! never honours a token it did not issue.

use std::net::IpAddr;

use async_trait::async_trait;

use super::caller::ToolCaller;
use crate::error::FrameworkError;

/// What a resolver is told about the request a token came with.
#[derive(Debug, Clone, Copy)]
pub struct ResolveContext<'a> {
    /// The canonical URL of the MCP endpoint, the resource a token must be
    /// bound to (RFC 8707). A resolver refuses a token issued for any
    /// other resource.
    pub resource: &'a str,
    /// The client's address, when known.
    pub client_ip: Option<IpAddr>,
}

/// Why a resolver did not turn a token into a caller.
#[derive(Debug)]
pub enum ResolveError {
    /// The token is not one this resolver issues; the next resolver is
    /// asked.
    NotMine,
    /// The token is this resolver's and is not valid: unknown, expired,
    /// revoked, or issued for another resource. The request gets 401
    /// `invalid_token` with this description, and no other resolver is
    /// asked.
    Invalid(&'static str),
    /// The resolver could not decide, such as when its store is down. The
    /// request gets 503; it is never served as a guest.
    Unavailable(FrameworkError),
}

/// Verifies bearer tokens for the MCP endpoint.
///
/// A resolver returns the caller a token stands for: a
/// [`ToolCaller::user`] with the scopes the user approved, or a
/// [`ToolCaller::agent`]. A caller of kind `public` is refused as an
/// invalid token, since a token is a credential.
///
/// ```rust,ignore
/// struct MyTokens;
///
/// #[async_trait]
/// impl CallerResolver for MyTokens {
///     async fn resolve(&self, token: &str, ctx: &ResolveContext<'_>)
///         -> Result<ToolCaller, ResolveError>
///     {
///         let Some(grant) = lookup(token).await else { return Err(ResolveError::NotMine) };
///         if grant.resource != ctx.resource {
///             return Err(ResolveError::Invalid("the token was issued for another resource"));
///         }
///         Ok(ToolCaller::user(grant.user).scopes(grant.scopes).client(grant.client_id))
///     }
/// }
/// ```
#[async_trait]
pub trait CallerResolver: Send + Sync {
    /// The caller `token` stands for.
    ///
    /// # Errors
    ///
    /// [`ResolveError::NotMine`] for a token this resolver does not issue,
    /// [`ResolveError::Invalid`] for one of its own that is not valid, and
    /// [`ResolveError::Unavailable`] when it cannot decide.
    async fn resolve(
        &self,
        token: &str,
        ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError>;

    /// The issuers of the authorization servers whose tokens this resolver
    /// accepts, listed in the protected resource metadata.
    fn authorization_servers(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The two forms of authorization server metadata the framework serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataKind {
    /// OAuth 2.0 authorization server metadata (RFC 8414), at
    /// `/.well-known/oauth-authorization-server`.
    OAuthAuthorizationServer,
    /// OpenID Connect discovery, at `/.well-known/openid-configuration`.
    OpenIdConfiguration,
}

impl MetadataKind {
    /// The well-known path, relative to the issuer.
    pub const fn well_known_path(self) -> &'static str {
        match self {
            Self::OAuthAuthorizationServer => "/.well-known/oauth-authorization-server",
            Self::OpenIdConfiguration => "/.well-known/openid-configuration",
        }
    }
}

/// The source of the application's authorization server metadata: the
/// Magnetar authorization server plugin, or the application's own.
///
/// The framework routes the well-known paths under the application's public
/// root to it, so an application served under a path prefix is discoverable
/// without the host root.
#[async_trait]
pub trait AuthorizationServerMetadata: Send + Sync {
    /// The metadata document of the given kind for `issuer`, the
    /// application's public URL; `None` answers 404.
    async fn document(&self, issuer: &str, kind: MetadataKind) -> Option<serde_json::Value>;

    /// The issuer, listed in the protected resource metadata. Defaults to
    /// `issuer`, the application's public URL.
    fn issuer(&self, issuer: &str) -> String {
        issuer.to_string()
    }
}
