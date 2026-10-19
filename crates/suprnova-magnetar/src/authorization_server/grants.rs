//! The token endpoint's wire types and its grant extension point.
//!
//! The token endpoint dispatches on `grant_type`. It answers the
//! `authorization_code` and `refresh_token` grants itself; any other grant
//! type goes to a [`GrantHandler`] registered with
//! [`AuthorizationServer::with_grant_handler`](super::AuthorizationServer::with_grant_handler),
//! so another plugin (agent auth adds the JWT bearer and claim grants) adds
//! a grant without changing this one. A handler mints its tokens through
//! [`TokenIssuer`], so they are stored, bound and
//! verified exactly as the built-in grants' tokens are.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use super::TokenIssuer;

/// One form-encoded token endpoint request.
#[derive(Clone, Default)]
pub struct TokenRequest {
    params: BTreeMap<String, String>,
    client_address: Option<String>,
}

impl fmt::Debug for TokenRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Parameter values carry codes, verifiers, refresh tokens and
        // assertions; only their names are safe to print.
        formatter
            .debug_struct("TokenRequest")
            .field("params", &self.params.keys().collect::<Vec<_>>())
            .field("client_address", &self.client_address)
            .finish()
    }
}

impl TokenRequest {
    /// A request from its form parameters and the client's address, when
    /// the host knows it.
    #[must_use]
    pub fn new(params: BTreeMap<String, String>, client_address: Option<String>) -> Self {
        Self {
            params,
            client_address,
        }
    }

    /// A parameter's value, treating an empty value as absent.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// The `grant_type` parameter.
    #[must_use]
    pub fn grant_type(&self) -> Option<&str> {
        self.param("grant_type")
    }

    /// Every parameter.
    #[must_use]
    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    /// The client's address, for rate limits and audit records.
    #[must_use]
    pub fn client_address(&self) -> Option<&str> {
        self.client_address.as_deref()
    }
}

/// A successful token response (RFC 6749 section 5.1).
pub struct TokenResponse {
    /// The access token.
    pub access_token: SecretString,
    /// Always `Bearer`.
    pub token_type: String,
    /// Seconds until the access token expires.
    pub expires_in: u64,
    /// The rotated refresh token, when the grant has one.
    pub refresh_token: Option<SecretString>,
    /// The space-separated scopes the access token carries.
    pub scope: String,
    /// Further response members an extension grant defines.
    pub extra: Map<String, Value>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenResponse")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field("has_refresh_token", &self.refresh_token.is_some())
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl TokenResponse {
    /// The JSON body, the only place the tokens leave the server.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut body = self.extra.clone();
        body.insert(
            "access_token".to_owned(),
            Value::String(self.access_token.expose_secret().to_owned()),
        );
        body.insert(
            "token_type".to_owned(),
            Value::String(self.token_type.clone()),
        );
        body.insert("expires_in".to_owned(), Value::from(self.expires_in));
        if let Some(refresh_token) = &self.refresh_token {
            body.insert(
                "refresh_token".to_owned(),
                Value::String(refresh_token.expose_secret().to_owned()),
            );
        }
        body.insert("scope".to_owned(), Value::String(self.scope.clone()));
        Value::Object(body)
    }
}

/// An error response (RFC 6749 section 5.2, RFC 8707 section 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenError {
    status: u16,
    error: String,
    description: Option<String>,
}

impl TokenError {
    /// An error with any code and status, for codes an extension defines.
    #[must_use]
    pub fn new(status: u16, error: impl Into<String>, description: Option<String>) -> Self {
        Self {
            status,
            error: error.into(),
            description,
        }
    }

    fn bad_request(error: &str, description: impl Into<String>) -> Self {
        Self::new(400, error, Some(description.into()))
    }

    /// The request is missing a parameter or is malformed.
    #[must_use]
    pub fn invalid_request(description: impl Into<String>) -> Self {
        Self::bad_request("invalid_request", description)
    }

    /// The client is unknown or may not use this endpoint.
    #[must_use]
    pub fn invalid_client(description: impl Into<String>) -> Self {
        Self::new(401, "invalid_client", Some(description.into()))
    }

    /// The code, refresh token or assertion is invalid, expired, revoked,
    /// spent or issued to another client.
    #[must_use]
    pub fn invalid_grant(description: impl Into<String>) -> Self {
        Self::bad_request("invalid_grant", description)
    }

    /// The client may not use this grant type.
    #[must_use]
    pub fn unauthorized_client(description: impl Into<String>) -> Self {
        Self::bad_request("unauthorized_client", description)
    }

    /// The server does not support the grant type.
    #[must_use]
    pub fn unsupported_grant_type(description: impl Into<String>) -> Self {
        Self::bad_request("unsupported_grant_type", description)
    }

    /// A requested scope is unknown or exceeds what was granted.
    #[must_use]
    pub fn invalid_scope(description: impl Into<String>) -> Self {
        Self::bad_request("invalid_scope", description)
    }

    /// The `resource` is unknown or differs from the one authorized.
    #[must_use]
    pub fn invalid_target(description: impl Into<String>) -> Self {
        Self::bad_request("invalid_target", description)
    }

    /// The server failed; the client may retry later.
    #[must_use]
    pub fn server_error() -> Self {
        Self::new(500, "server_error", None)
    }

    /// The HTTP status to answer with.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The error code.
    #[must_use]
    pub fn error(&self) -> &str {
        &self.error
    }

    /// The human-readable description, if any.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The JSON body.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match &self.description {
            Some(description) => json!({"error": self.error, "error_description": description}),
            None => json!({"error": self.error}),
        }
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.description {
            Some(description) => write!(formatter, "{}: {description}", self.error),
            None => write!(formatter, "{}", self.error),
        }
    }
}

impl std::error::Error for TokenError {}

/// A grant an extension issues through [`TokenIssuer::issue`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewGrant {
    /// The grant family, for example `agent`. It names the grant in
    /// listings and in [`VerifiedAccess::grant_kind`](super::VerifiedAccess::grant_kind).
    pub kind: String,
    /// The user the grant acts for, if any.
    pub user_id: Option<String>,
    /// The client or agent registration the grant is issued to.
    pub client_id: String,
    /// The name listings show for the client.
    pub client_name: String,
    /// The host listings show for the client.
    pub client_host: String,
    /// The scopes, each declared by the application.
    pub scopes: Vec<String>,
    /// The resource the tokens are bound to, one the server accepts.
    pub resource: String,
    /// The user's authentication epoch, when the grant should end with it.
    pub auth_epoch: Option<u64>,
}

/// Which tokens [`TokenIssuer::issue`] mints for a new grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueTokens {
    /// The access token's kind, which the verification API reports so a
    /// resource server can tell the grants' tokens apart. Must not be
    /// `refresh`.
    pub access_kind: String,
    /// Whether to mint a rotating refresh token too.
    pub refresh: bool,
    /// The access token's lifetime, or the server's default.
    pub access_lifetime: Option<Duration>,
}

/// What [`TokenIssuer::issue_grant`] created: the grant's identifier, so an
/// extension can end the grant later through
/// [`TokenIssuer::revoke_grant`], and the response to answer with.
#[derive(Debug)]
pub struct IssuedGrant {
    /// The identifier of the grant the tokens belong to.
    pub grant_id: String,
    /// The token response for the client.
    pub response: TokenResponse,
}

/// A grant type the token endpoint does not answer itself.
#[async_trait]
pub trait GrantHandler: Send + Sync {
    /// The `grant_type` value this handler answers.
    fn grant_type(&self) -> &str;

    /// Answer one token request of this grant type.
    async fn exchange(
        &self,
        request: &TokenRequest,
        issuer: &TokenIssuer<'_>,
    ) -> Result<TokenResponse, TokenError>;
}
