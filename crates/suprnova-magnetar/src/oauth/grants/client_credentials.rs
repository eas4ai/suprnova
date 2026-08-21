//! RFC 6749 §4.4 client-credentials grant: machine-to-machine token
//! acquisition with no end-user involved.

use crate::oauth::errors::OAuthResult;
use crate::oauth::protocol::TokenSuccessResponse;
use crate::oauth::provider::OAuthProvider;
use crate::plugin::HttpTransport;

/// Exchange the provider's own client credentials for a token.
///
/// # Errors
///
/// Propagates provider/network failure classes from
/// [`super::execute_token_request`].
async fn prepare(
    provider: &dyn OAuthProvider,
    scopes: &[String],
) -> OAuthResult<(Vec<(String, String)>, Vec<(String, String)>, bool)> {
    let shape = provider.token_shape();
    let mut wire = vec![
        (
            shape.client_id_param.clone(),
            provider.client_id().to_owned(),
        ),
        ("grant_type".to_owned(), "client_credentials".to_owned()),
    ];
    if shape.always_send_scope || !scopes.is_empty() {
        wire.push(("scope".to_owned(), scopes.join(&shape.scope_delimiter)));
    }
    let auth = provider.client_authentication().await?;
    wire.extend(auth.params);

    Ok((wire, auth.headers, shape.accept_http_success_error_body))
}

/// Exchange the provider's own client credentials for a token.
///
/// # Errors
///
/// Propagates provider/network failure classes from
/// [`super::execute_token_request`].
pub async fn execute(
    provider: &dyn OAuthProvider,
    transport: &dyn HttpTransport,
    scopes: &[String],
) -> OAuthResult<TokenSuccessResponse> {
    let (wire, headers, accept_http_success_error_body) = prepare(provider, scopes).await?;
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
    scopes: &[String],
) -> OAuthResult<(TokenSuccessResponse, String)> {
    let (wire, headers, accept_http_success_error_body) = prepare(provider, scopes).await?;
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
