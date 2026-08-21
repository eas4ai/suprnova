//! RFC 6749 §5 token-endpoint wire types.
//!
//! The success and error response shapes every grant (authorization code,
//! refresh, client credentials, JWT bearer, device authorization) parses.
//! I/O-free: this module only defines and parses JSON shapes. No HTTP
//! client, transport, or provider-name branch lives here -- later
//! iteration-003 tasks own the transport and the grant state machines.
//!
//! Adapted from `io-oauth` 0.3.0's `rfc6749::issue_access_token` module
//! (MIT OR Apache-2.0; see `THIRD_PARTY_NOTICES.md` and
//! `docs/provenance/source-ledger.md`). `io-oauth` was evaluated as a
//! dependency and rejected: its `io-http` dependency is mandatory (not
//! feature-gated), which would pull a second HTTP framework into every
//! build of this crate regardless of which Cargo features are enabled, and
//! its `base64 0.23`/`sha2 0.11` majors duplicate this crate's existing
//! `base64 0.22`/`sha2 0.10`. The types below are therefore adapted, kept
//! strictly I/O-free, and trimmed to what the request-shape surface needs
//! now (device-authorization and JWT-bearer specific codes belong to the
//! tasks that add those grants).

use secrecy::SecretString;
use serde::Deserialize;

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};

/// The token endpoint's successful response body.
///
/// Refs: <https://datatracker.ietf.org/doc/html/rfc6749#section-5.1>
#[derive(Clone, Debug, Deserialize)]
pub struct TokenSuccessResponse {
    /// The issued access token.
    pub access_token: SecretString,
    /// The token type, ordinarily `Bearer`.
    pub token_type: String,
    /// Access-token lifetime in seconds, when the provider states one.
    pub expires_in: Option<u64>,
    /// The refresh token, when this grant issues one.
    pub refresh_token: Option<SecretString>,
    /// The OpenID Connect ID token, when the provider returns one.
    ///
    /// This is optional because RFC 6749 token responses do not require
    /// OpenID Connect fields; providers without an `id_token` remain valid.
    pub id_token: Option<SecretString>,
    /// The granted scope, present when it differs from the request.
    pub scope: Option<String>,
}

/// The token endpoint's error response body.
///
/// Refs: <https://datatracker.ietf.org/doc/html/rfc6749#section-5.2>
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct TokenErrorResponse {
    /// A single ASCII error code.
    pub error: OAuthErrorCode,
    /// Human-readable text explaining the error.
    pub error_description: Option<String>,
    /// A URI to a human-readable page about the error.
    pub error_uri: Option<String>,
}

/// The RFC 6749 §5.2 token-endpoint error codes.
///
/// Refs: <https://datatracker.ietf.org/doc/html/rfc6749#section-5.2>
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OAuthErrorCode {
    /// The request is malformed or missing a required parameter.
    InvalidRequest,
    /// Client authentication failed.
    InvalidClient,
    /// The grant or refresh token is invalid, expired, revoked, does not
    /// match the redirection URI, or was issued to a different client.
    InvalidGrant,
    /// The authenticated client is not authorized to use this grant type.
    UnauthorizedClient,
    /// The authorization server does not support this grant type.
    UnsupportedGrantType,
    /// The requested scope is invalid, unknown, malformed, or exceeds the
    /// scope granted.
    InvalidScope,
    /// Any error code this crate does not yet model by name, carrying the
    /// raw wire string. Provider extensions and grant-specific codes
    /// (device authorization's `authorization_pending`/`slow_down`, JWT
    /// bearer's codes) fall back here until the task that adds those
    /// grants names them; the raw code is retained so Task 6's error-class
    /// mapping and operator tracing can still see exactly what arrived.
    Unknown(String),
}

impl<'de> serde::Deserialize<'de> for OAuthErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        if raw.trim().is_empty() {
            // An empty/whitespace `error` value is not a valid RFC 6749
            // error code. Rejecting it here (rather than falling back to
            // `Unknown`) lets `TokenResponseBody`'s untagged deserializer
            // fall through to the `Success` variant instead of discarding
            // an issued token behind a spurious empty error field.
            return Err(serde::de::Error::custom(
                "oauth error code must not be empty",
            ));
        }
        Ok(match raw.as_str() {
            "invalid_request" => Self::InvalidRequest,
            "invalid_client" => Self::InvalidClient,
            "invalid_grant" => Self::InvalidGrant,
            "unauthorized_client" => Self::UnauthorizedClient,
            "unsupported_grant_type" => Self::UnsupportedGrantType,
            "invalid_scope" => Self::InvalidScope,
            _ => Self::Unknown(raw),
        })
    }
}

impl OAuthErrorCode {
    /// The RFC 6749 §5.2 wire string this code was (de)serialized from --
    /// lets a caller (Task 4's grant executors) build
    /// [`OAuthProtocolError::ProviderReportedError`]'s `code` field without
    /// re-matching every named variant back to its wire spelling.
    #[must_use]
    pub fn wire_str(&self) -> &str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidClient => "invalid_client",
            Self::InvalidGrant => "invalid_grant",
            Self::UnauthorizedClient => "unauthorized_client",
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::InvalidScope => "invalid_scope",
            Self::Unknown(raw) => raw,
        }
    }
}

/// Either shape a token-endpoint response can take, resolved from the raw
/// response body alone.
///
/// Detection here never consults an HTTP status code: some providers
/// (TikTok, per `docs/specs/suprnova-magnetar/10-providers.md`) deliver
/// [`TokenErrorResponse`] bodies with an HTTP 200 status. A shape whose
/// [`crate::oauth::request_shape::TokenRequestShape::accept_http_success_error_body`]
/// is set tells a later transport task it must probe the body with
/// [`parse_token_response_body`] even after a 200 status, rather than
/// trusting the status code alone.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum TokenResponseBody {
    /// The provider reported an OAuth error.
    Error(TokenErrorResponse),
    /// The provider issued a token.
    Success(TokenSuccessResponse),
}

/// Parses a raw token-endpoint JSON response body into its success or error
/// shape, independent of the HTTP status the body arrived with.
///
/// # Errors
///
/// Returns [`OAuthProtocolError::MalformedTokenResponse`] when `body` is
/// neither valid JSON nor matches either wire shape.
pub fn parse_token_response_body(body: &str) -> OAuthResult<TokenResponseBody> {
    serde_json::from_str(body).map_err(|source| OAuthProtocolError::MalformedTokenResponse {
        message: source.to_string(),
    })
}
