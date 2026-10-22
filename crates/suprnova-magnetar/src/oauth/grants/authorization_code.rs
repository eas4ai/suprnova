//! RFC 6749 §4.1.3 authorization-code token exchange, PKCE-verifier aware.

use secrecy::SecretString;

use crate::oauth::authorization::OAuthCeremony;
use crate::oauth::errors::{OAuthProtocolError, OAuthResult};
use crate::oauth::protocol::TokenSuccessResponse;
use crate::oauth::provider::OAuthProvider;
use crate::oauth::request_shape::{PkcePosture, TokenRequestParams, render_token_request};
use crate::plugin::HttpTransport;

/// A successful authorization-code exchange together with its exact provider
/// response body.
///
/// `raw_body` is provider-sensitive: it can contain access, refresh, and ID
/// tokens, as well as provider-specific identity data. It is deliberately
/// omitted from [`Debug`]. Hosts that access [`Self::raw_body`] own redaction
/// and secure storage; they MUST NOT log or display its value.
pub struct AuthorizationCodeResult {
    /// The standardized token fields parsed from the provider response.
    pub response: TokenSuccessResponse,
    raw_body: SecretString,
}

impl AuthorizationCodeResult {
    /// Return the exact provider response body as a secret.
    ///
    /// The body is secret-bearing provider data. Callers MUST explicitly expose
    /// it only to parse or securely store provider-specific fields, and MUST
    /// redact it from logs and diagnostics.
    #[must_use]
    pub fn raw_body(&self) -> &SecretString {
        &self.raw_body
    }
}

impl std::fmt::Debug for AuthorizationCodeResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizationCodeResult")
            .field("response", &self.response)
            .field("raw_body", &"[REDACTED]")
            .finish()
    }
}

/// Exchange an authorization code for standardized token fields.
///
/// This backwards-compatible convenience API delegates to
/// [`execute_with_raw`] and discards the provider-sensitive raw response body.
///
/// # Errors
///
/// Returns [`OAuthProtocolError::InvalidRequestShape`] when `ceremony.provider`
/// does not match `provider.name()`, or when `provider`'s authorization
/// shape requires PKCE but `ceremony` carries no verifier (`begin` failed to
/// mint one, or a caller substituted the wrong ceremony) -- both are
/// caller-protocol faults, and neither reaches the network. Otherwise
/// propagates provider/network failure classes from the token exchange.
pub async fn execute(
    provider: &dyn OAuthProvider,
    transport: &dyn HttpTransport,
    ceremony: &OAuthCeremony,
    code: SecretString,
    redirect_uri: Option<String>,
    scopes: Vec<String>,
) -> OAuthResult<TokenSuccessResponse> {
    execute_with_raw(provider, transport, ceremony, code, redirect_uri, scopes)
        .await
        .map(|result| result.response)
}

/// Exchange an authorization code and retain the exact provider response body.
///
/// The request construction is identical to [`execute`]: it preserves the
/// provider's client authentication, request shape, and PKCE verifier. This
/// is the callback seam for hosts that need provider-specific response fields.
/// The returned [`AuthorizationCodeResult::raw_body`] is secret-bearing; hosts
/// MUST redact it from logs and diagnostics and own its secure storage.
///
/// # Errors
///
/// Returns [`OAuthProtocolError::InvalidRequestShape`] when `ceremony.provider`
/// does not match `provider.name()`, or when `provider`'s authorization
/// shape requires PKCE but `ceremony` carries no verifier. Otherwise
/// propagates provider/network failure classes from the token exchange.
pub async fn execute_with_raw(
    provider: &dyn OAuthProvider,
    transport: &dyn HttpTransport,
    ceremony: &OAuthCeremony,
    code: SecretString,
    redirect_uri: Option<String>,
    scopes: Vec<String>,
) -> OAuthResult<AuthorizationCodeResult> {
    if ceremony.provider != provider.name() {
        return Err(OAuthProtocolError::InvalidRequestShape {
            field: "provider".to_owned(),
            message: format!(
                "the consumed ceremony belongs to provider '{}', not '{}'",
                ceremony.provider,
                provider.name()
            ),
        });
    }
    if provider.authorization_shape().pkce == PkcePosture::Required && ceremony.verifier.is_none() {
        return Err(OAuthProtocolError::InvalidRequestShape {
            field: "code_verifier".to_owned(),
            message: format!(
                "provider '{}' requires pkce but the consumed ceremony carries no verifier",
                provider.name()
            ),
        });
    }

    let shape = provider.token_shape();
    let params = TokenRequestParams {
        client_id: provider.client_id().to_owned(),
        code,
        redirect_uri,
        code_verifier: ceremony.verifier.clone(),
        scopes,
    };
    let mut wire = render_token_request(&shape, &params);
    let auth = provider.client_authentication().await?;
    wire.extend(auth.params);

    super::execute_token_request_raw(
        transport,
        provider.name(),
        &provider.token_endpoint(),
        wire,
        auth.headers,
        shape.accept_http_success_error_body,
    )
    .await
    .map(|(response, raw_body)| AuthorizationCodeResult {
        response,
        raw_body: SecretString::from(raw_body),
    })
}
