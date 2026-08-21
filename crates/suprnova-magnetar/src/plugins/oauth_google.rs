//! Google `OAuthProvider` plugin behind the `oauth-google` feature
//! (`docs/specs/suprnova-magnetar/10-providers.md`'s Google section: "Pure
//! config on the engine's OIDC-shaped defaults").
//!
//! ## Dossier
//!
//! - **Endpoints**: authorize
//!   `https://accounts.google.com/o/oauth2/v2/auth`, token
//!   `https://oauth2.googleapis.com/token`, revoke
//!   `https://oauth2.googleapis.com/revoke`. Evidence:
//!   `reference/arctic-oauth-master/src/providers/google.rs`'s endpoint
//!   constants, byte-identical to Suprnova's own well-known table in
//!   `framework/src/torii_integration/oauth.rs`.
//! - **Client authentication**: [`ClientAuthentication::RequestBody`] --
//!   `client_id`/`client_secret` in the token-request body, the RFC 6749
//!   default; no quirk handler needed
//!   ([`AuthorizationRequestShape::default`]/[`TokenRequestShape::default`]
//!   are used unmodified).
//! - **PKCE posture**: [`PkcePosture::Required`] (the 09 engine default).
//! - **Identity source**: a userinfo `GET` the host performs against
//!   `https://www.googleapis.com/oauth2/v3/userinfo`
//!   (`framework/src/torii_integration/oauth.rs`'s well-known table), which
//!   this provider parses for `sub`/`email`/`email_verified`/`name`.
//!   `email_verified` is honored exactly as Google reports it: an
//!   unverified email is *not* filtered out here -- 09's engine centralizes
//!   the "unverified == absent" policy in
//!   [`crate::oauth::identity::IdentityResolver`], not per provider.
//! - **Refresh**: supported; requires `access_type=offline` on the
//!   authorization request for Google to issue a refresh token at all, and
//!   `prompt=consent` to force reissue on a repeat authorization (Google
//!   only issues a refresh token on a user's first consent by default).
//!   [`ClientAuthentication::RequestBody`], no extra required scopes.
//! - **Revocation**: supported; Google's revoke endpoint takes only
//!   `token` (no `token_type_hint`, no client authentication) -- evidence:
//!   `reference/arctic-oauth-master/src/providers/google.rs`'s
//!   `revoke_token`.

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
use crate::oauth::request_shape::{AuthorizationRequestShape, TokenRequestShape};

const AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v3/userinfo";
const REVOCATION_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";

/// Route-level configuration for the Google provider.
#[derive(Clone, Debug)]
pub struct GoogleProviderConfig {
    /// The OAuth client id.
    pub client_id: String,
    /// The OAuth client secret.
    pub client_secret: SecretString,
    /// The registered callback URI, when the client sends one explicitly.
    pub redirect_uri: Option<String>,
    /// The requested scopes (`openid`, `email`, `profile`).
    pub scopes: Vec<String>,
    /// Endpoint URL overrides; defaults to Google's real dossier URLs.
    pub endpoints: EndpointOverrides,
}
#[derive(Deserialize)]
struct GoogleUserInfo {
    sub: Option<String>,
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
    name: Option<String>,
}

/// The Google `OAuthProvider` plugin.
pub struct GoogleOAuthProvider {
    /// `client_id()`/`client_authentication()` expose what a caller needs;
    /// the config itself stays private like every other provider's (I5:
    /// this field was briefly `pub` only to satisfy a `dead_code` lint
    /// before those trait methods existed -- that reason no longer holds).
    config: GoogleProviderConfig,
    transport: Arc<dyn RevocationTransport>,
}

impl GoogleOAuthProvider {
    /// Compose the provider from its configuration and a host-supplied
    /// revocation transport.
    #[must_use]
    pub fn new(config: GoogleProviderConfig, transport: Arc<dyn RevocationTransport>) -> Self {
        Self { config, transport }
    }
}

#[async_trait]
impl OAuthProvider for GoogleOAuthProvider {
    fn name(&self) -> &'static str {
        "google"
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        AuthorizationRequestShape::default()
    }

    fn token_shape(&self) -> TokenRequestShape {
        TokenRequestShape::default()
    }

    async fn resolve_identity(&self, response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        let ProviderResponse::UserInfo { body } = response else {
            return Err(OAuthProtocolError::MalformedProviderResponse {
                provider: "google",
                message: "Google identity resolution requires a UserInfo response".to_owned(),
            });
        };
        let info: GoogleUserInfo = serde_json::from_str(&body).map_err(|error| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "google",
                message: format!("failed to parse userinfo body: {error}"),
            }
        })?;
        let subject = info
            .sub
            .filter(|subject| !subject.is_empty())
            .ok_or_else(|| OAuthProtocolError::MalformedProviderResponse {
                provider: "google",
                message: "userinfo response missing `sub`".to_owned(),
            })?;
        Ok(ProviderIdentity {
            provider: "google".to_owned(),
            subject,
            email: info.email,
            email_verified: info.email_verified,
            display_name: info.name,
        })
    }

    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()> {
        let _ = hint; // Google's revoke endpoint has no `token_type_hint`.
        let endpoint = self
            .config
            .endpoints
            .revocation_endpoint
            .clone()
            .unwrap_or_else(|| REVOCATION_ENDPOINT.to_owned());
        let request = RevocationRequest {
            method: "POST",
            endpoint,
            placement: ParamPlacement::Body,
            params: vec![("token".to_owned(), token.to_owned())],
            headers: Vec::new(),
        };
        self.transport.send(request).await
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: true,
            token_client_authentication: ClientAuthentication::RequestBody,
            extra_authorization_params: vec![("access_type".to_owned(), "offline".to_owned())],
            required_scopes: Vec::new(),
            requires_reconsent_for_reissue: true,
            invalid_grant_meaning: InvalidGrantMeaning::OrdinaryRevocation,
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
