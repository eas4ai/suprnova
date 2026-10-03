//! TikTok Login Kit `OAuthProvider` plugin behind the `oauth-tiktok`
//! feature (`docs/specs/suprnova-magnetar/10-providers.md`'s TikTok
//! section).
//!
//! ## Dossier
//!
//! - **Endpoints**: authorize
//!   `https://www.tiktok.com/v2/auth/authorize/`, token
//!   `https://open.tiktokapis.com/v2/oauth/token/`, revoke
//!   `https://open.tiktokapis.com/v2/oauth/revoke/`. Evidence:
//!   `reference/arctic-oauth-master/src/providers/tiktok.rs`'s endpoint
//!   constants (including the trailing slash, which TikTok requires).
//! - **`client_key` instead of `client_id`**: both
//!   [`AuthorizationRequestShape::client_id_param`] and
//!   [`TokenRequestShape::client_id_param`] are `"client_key"`. Evidence:
//!   arctic `tiktok.rs`'s `authorization_url_uses_client_key`/
//!   `validate_authorization_code_uses_client_key_in_body` tests; the exact
//!   shape Task 1's `tests/oauth_request_shapes.rs` already fixture-tests.
//! - **Comma-delimited, always-emitted scopes**:
//!   `scope_delimiter = ","`, `always_send_scope = true` on both shapes.
//!   Evidence: arctic `tiktok.rs`'s authorization URL builder ("TikTok
//!   always sends scope, even when empty; comma-delimited").
//! - **Client authentication**: [`ClientAuthentication::RequestBody`] --
//!   `client_key`/`client_secret` in the POST body (arctic `tiktok.rs`'s
//!   `validate_authorization_code`/`revoke_token`, both put
//!   `client_secret` directly in the form body, never HTTP Basic).
//! - **PKCE posture**: [`PkcePosture::Required`] -- TikTok requires PKCE
//!   with `S256` on every authorization request (the 09 engine default
//!   already matches; arctic `tiktok.rs`'s module doc).
//! - **HTTP-200 error bodies**: TikTok's token *and* userinfo endpoints can
//!   report an OAuth-shaped error under an HTTP 200 status
//!   (`error.code != "ok"`), so [`TokenRequestShape::accept_http_success_error_body`]
//!   is `true`, and [`TikTokOAuthProvider::resolve_identity`] checks the
//!   response envelope's `error.code` itself rather than trusting the
//!   caller's HTTP status. Task 1's `protocol` module already detects this
//!   class for the token response; this provider applies the same
//!   discipline to the userinfo response, which is outside that module's
//!   scope.
//! - **Identity source**: a `GET` the host performs against
//!   `https://open.tiktokapis.com/v2/user/info/?fields=open_id,union_id,avatar_url,display_name`
//!   (TikTok's "Get user info" API reference,
//!   `developers.tiktok.com/doc/tiktok-api-v2-get-user-info/`, verified
//!   live 2026-08-19), wrapped in `{"data":{"user":{...}},"error":{...}}`.
//!   **TikTok's user-info schema has no email field at all** (not merely
//!   omitted -- it does not exist), so this provider always resolves
//!   `email: None`, `email_verified: false`.
//! - **Refresh**: supported; [`ClientAuthentication::RequestBody`], no
//!   extra required scopes (arctic `tiktok.rs`'s `refresh_access_token`).
//!   TikTok **rotates** refresh tokens on every refresh call: "the
//!   returned `refresh_token` may be different than the one passed in the
//!   payload" (TikTok's "Get user access token" API reference,
//!   `developers.tiktok.com/doc/oauth-user-access-token-management`,
//!   verified live 2026-08-19) -- so
//!   [`InvalidGrantMeaning::ReuseOrExternalRevocation`], not
//!   [`InvalidGrantMeaning::OrdinaryRevocation`], the four other
//!   first-party providers use.
//! - **Revocation**: supported; `token` + `client_key` + `client_secret` in
//!   the body, no `token_type_hint` (arctic `tiktok.rs`'s `revoke_token`).

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};
use crate::oauth::provider::{
    ClientAuthentication, ClientAuthenticationMaterial, EndpointOverrides, InvalidGrantMeaning,
    OAuthProvider, ParamPlacement, ProviderIdentity, ProviderResponse, RefreshPolicy,
    RevocationRequest, RevocationTransport, TokenHint,
};
use crate::oauth::request_shape::{AuthorizationRequestShape, PkcePosture, TokenRequestShape};

const AUTHORIZATION_ENDPOINT: &str = "https://www.tiktok.com/v2/auth/authorize/";
const TOKEN_ENDPOINT: &str = "https://open.tiktokapis.com/v2/oauth/token/";
const USERINFO_ENDPOINT: &str =
    "https://open.tiktokapis.com/v2/user/info/?fields=open_id,union_id,avatar_url,display_name";
const REVOCATION_ENDPOINT: &str = "https://open.tiktokapis.com/v2/oauth/revoke/";

/// Route-level configuration for the TikTok provider.
#[derive(Clone, Debug)]
pub struct TikTokProviderConfig {
    /// TikTok's `client_key` (wire name; config field kept as `client_id`
    /// for consistency with every other provider's config surface).
    pub client_id: String,
    /// The OAuth client secret.
    pub client_secret: SecretString,
    /// The registered callback URI, when the client sends one explicitly.
    pub redirect_uri: Option<String>,
    /// The requested scopes (`user.info.basic`, ...).
    pub scopes: Vec<String>,
    /// Endpoint URL overrides; defaults to TikTok's real dossier URLs.
    pub endpoints: EndpointOverrides,
}

/// The raw shape of TikTok's `/v2/user/info/` response envelope.
#[derive(Deserialize)]
struct TikTokUserInfoEnvelope {
    data: Option<TikTokUserInfoData>,
    error: Option<TikTokError>,
}

#[derive(Deserialize)]
struct TikTokUserInfoData {
    user: Option<TikTokUser>,
}

#[derive(Deserialize)]
struct TikTokUser {
    open_id: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct TikTokError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// The TikTok `OAuthProvider` plugin.
pub struct TikTokOAuthProvider {
    config: TikTokProviderConfig,
    transport: Arc<dyn RevocationTransport>,
}

impl TikTokOAuthProvider {
    /// Compose the provider from its configuration and a host-supplied
    /// revocation transport.
    #[must_use]
    pub fn new(config: TikTokProviderConfig, transport: Arc<dyn RevocationTransport>) -> Self {
        Self { config, transport }
    }
}

#[async_trait]
impl OAuthProvider for TikTokOAuthProvider {
    fn name(&self) -> &'static str {
        "tiktok"
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        AuthorizationRequestShape {
            client_id_param: "client_key".to_owned(),
            scope_delimiter: ",".to_owned(),
            always_send_scope: true,
            pkce: PkcePosture::Required,
            response_mode: None,
            requires_nonce: false,
        }
    }

    fn token_shape(&self) -> TokenRequestShape {
        TokenRequestShape {
            client_id_param: "client_key".to_owned(),
            scope_delimiter: ",".to_owned(),
            always_send_scope: true,
            accept_http_success_error_body: true,
        }
    }

    async fn resolve_identity(&self, response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        let ProviderResponse::UserInfo { body } = response else {
            return Err(OAuthProtocolError::MalformedProviderResponse {
                provider: "tiktok",
                message: "TikTok identity resolution requires a UserInfo response".to_owned(),
            });
        };
        let envelope: TikTokUserInfoEnvelope = serde_json::from_str(&body).map_err(|error| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "tiktok",
                message: format!("failed to parse user-info body: {error}"),
            }
        })?;
        if let Some(error) = envelope.error
            && !error.code.is_empty()
            && error.code != "ok"
        {
            return Err(OAuthProtocolError::ProviderReportedError {
                provider: "tiktok",
                code: error.code,
                message: if error.message.is_empty() {
                    None
                } else {
                    Some(error.message)
                },
            });
        }
        let user = envelope.data.and_then(|data| data.user).ok_or_else(|| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "tiktok",
                message: "user-info response missing `data.user`".to_owned(),
            }
        })?;
        let subject = user
            .open_id
            .filter(|open_id| !open_id.is_empty())
            .ok_or_else(|| OAuthProtocolError::MalformedProviderResponse {
                provider: "tiktok",
                message: "user-info response missing `data.user.open_id`".to_owned(),
            })?;
        // TikTok's user-info schema has no email field -- always absent by
        // design, not by omission; see this module's dossier doc.
        Ok(ProviderIdentity {
            provider: "tiktok".to_owned(),
            subject,
            email: None,
            email_verified: false,
            display_name: user.display_name,
        })
    }

    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()> {
        let _ = hint; // TikTok's revoke endpoint has no `token_type_hint`.
        let mut params = vec![
            ("token".to_owned(), token.to_owned()),
            ("client_key".to_owned(), self.config.client_id.clone()),
        ];
        params.extend(self.client_authentication().await?.params);
        let request = RevocationRequest {
            method: "POST",
            endpoint: self
                .config
                .endpoints
                .revocation_endpoint
                .clone()
                .unwrap_or_else(|| REVOCATION_ENDPOINT.to_owned()),
            placement: ParamPlacement::Body,
            params,
            headers: Vec::new(),
        };
        self.transport.send(request).await
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: true,
            token_client_authentication: ClientAuthentication::RequestBody,
            extra_authorization_params: Vec::new(),
            required_scopes: Vec::new(),
            requires_reconsent_for_reissue: false,
            // TikTok rotates refresh tokens on every refresh call (see
            // this module's dossier doc); an `invalid_grant` can mean a
            // stale, already-rotated-out token was presented.
            invalid_grant_meaning: InvalidGrantMeaning::ReuseOrExternalRevocation,
        }
    }

    fn client_id(&self) -> &str {
        &self.config.client_id
    }
    fn token_endpoint(&self) -> String {
        self.config
            .endpoints
            .token_endpoint
            .clone()
            .unwrap_or_else(|| TOKEN_ENDPOINT.to_owned())
    }
    fn authorization_endpoint(&self) -> String {
        self.config
            .endpoints
            .authorization_endpoint
            .clone()
            .unwrap_or_else(|| AUTHORIZATION_ENDPOINT.to_owned())
    }
    fn userinfo_endpoint(&self) -> Option<String> {
        Some(
            self.config
                .endpoints
                .userinfo_endpoint
                .clone()
                .unwrap_or_else(|| USERINFO_ENDPOINT.to_owned()),
        )
    }
    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial> {
        Ok(ClientAuthenticationMaterial {
            params: vec![(
                "client_secret".to_owned(),
                self.config.client_secret.expose_secret().to_owned(),
            )],
            headers: Vec::new(),
        })
    }
}
