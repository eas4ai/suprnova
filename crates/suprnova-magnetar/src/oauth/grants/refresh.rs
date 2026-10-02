//! RFC 6749 §6 refresh-token grant.

use secrecy::{ExposeSecret, SecretString};

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};
use crate::oauth::protocol::TokenSuccessResponse;
use crate::oauth::provider::OAuthProvider;
use crate::plugin::HttpTransport;

/// Exchange a refresh token for a new access token (and, when the provider
/// rotates them, a new refresh token).
///
/// # Errors
///
/// Returns [`OAuthProtocolError::ProviderConfiguration`] when
/// `provider.refresh_policy().supported` is `false` -- a caller-protocol
/// fault this function catches before making any network call, matching
/// dossier facts such as Facebook's non-RFC-6749 refresh shape
/// (`docs/specs/suprnova-magnetar/10-providers.md`). Otherwise propagates
/// provider/network failure classes from [`super::execute_token_request`].
async fn prepare(
    provider: &dyn OAuthProvider,
    refresh_token: SecretString,
    scopes: &[String],
) -> OAuthResult<(Vec<(String, String)>, Vec<(String, String)>, bool)> {
    if !provider.refresh_policy().supported {
        return Err(OAuthProtocolError::ProviderConfiguration {
            provider: provider.name(),
            message: "this provider does not support the rfc 6749 refresh_token grant".to_owned(),
        });
    }

    let shape = provider.token_shape();
    let mut wire = vec![
        (
            shape.client_id_param.clone(),
            provider.client_id().to_owned(),
        ),
        ("grant_type".to_owned(), "refresh_token".to_owned()),
        (
            "refresh_token".to_owned(),
            refresh_token.expose_secret().to_owned(),
        ),
    ];
    if shape.always_send_scope || !scopes.is_empty() {
        wire.push(("scope".to_owned(), scopes.join(&shape.scope_delimiter)));
    }
    let auth = provider.client_authentication().await?;
    wire.extend(auth.params);

    Ok((wire, auth.headers, shape.accept_http_success_error_body))
}

/// Exchange a refresh token for a new access token (and, when the provider
/// rotates them, a new refresh token).
///
/// # Errors
///
/// Returns [`OAuthProtocolError::ProviderConfiguration`] when
/// `provider.refresh_policy().supported` is `false` -- a caller-protocol
/// fault this function catches before making any network call, matching
/// dossier facts such as Facebook's non-RFC-6749 refresh shape
/// (`docs/specs/suprnova-magnetar/10-providers.md`). Otherwise propagates
/// provider/network failure classes from [`super::execute_token_request`].
pub async fn execute(
    provider: &dyn OAuthProvider,
    transport: &dyn HttpTransport,
    refresh_token: SecretString,
    scopes: &[String],
) -> OAuthResult<TokenSuccessResponse> {
    let (wire, headers, accept_http_success_error_body) =
        prepare(provider, refresh_token, scopes).await?;
    super::execute_token_request(
        transport,
        provider.name(),
        &provider.token_endpoint(),
        wire,
        headers,
        accept_http_success_error_body,
    )
    .await
}

/// [`execute`], additionally returning the exact response body text -- the
/// token broker's byte-faithful raw-payload storage
/// (`docs/specs/suprnova-magnetar/11-token-broker.md`) needs it.
pub(crate) async fn execute_with_raw(
    provider: &dyn OAuthProvider,
    transport: &dyn HttpTransport,
    refresh_token: SecretString,
    scopes: &[String],
) -> OAuthResult<(TokenSuccessResponse, String)> {
    let (wire, headers, accept_http_success_error_body) =
        prepare(provider, refresh_token, scopes).await?;
    super::execute_token_request_raw(
        transport,
        provider.name(),
        &provider.token_endpoint(),
        wire,
        headers,
        accept_http_success_error_body,
    )
    .await
}
