//! X (formerly Twitter) `OAuthProvider` plugin behind the `oauth-x`
//! feature (`docs/specs/suprnova-magnetar/10-providers.md`'s X section: "no
//! reliable email").
//!
//! ## Dossier
//!
//! - **Endpoints**: authorize `https://twitter.com/i/oauth2/authorize`,
//!   token `https://api.twitter.com/2/oauth2/token`, revoke
//!   `https://api.twitter.com/2/oauth2/revoke`. Evidence:
//!   `reference/arctic-oauth-master/src/providers/twitter.rs`'s endpoint
//!   constants.
//! - **Client authentication**: [`ClientAuthentication::HttpBasic`] -- X's
//!   OAuth 2.0 token and revocation endpoints authenticate confidential
//!   clients via `Authorization: Basic base64(client_id:client_secret)`
//!   (X API v2 OAuth 2.0 documentation); this provider builds that header
//!   locally (pure computation, not I/O) and hands it to the injected
//!   [`RevocationTransport`] rather than performing the request itself.
//! - **PKCE posture**: [`PkcePosture::Required`] -- X mandates PKCE on
//!   every client, confidential or not (the 09 engine default already
//!   matches).
//! - **Identity source**: a `GET` the host performs against
//!   `https://api.twitter.com/2/users/me`, whose body wraps the profile in
//!   a `data` object (`{"data":{"id":...,"username":...,"name":...}}`, X
//!   API v2 documentation). **X never supplies a reliable email** without a
//!   separate elevated-access grant this crate does not model
//!   (`docs/specs/suprnova-magnetar/10-providers.md`): this provider always
//!   resolves `email: None`, `email_verified: false`, so every X sign-in
//!   drives [`crate::oauth::identity::IdentityResolver`]'s
//!   `EmailCompletionRequired` outcome by design, not as a fallback for a
//!   parse failure.
//! - **Refresh**: supported; requires the `offline.access` scope to be
//!   requested for X to issue a refresh token at all;
//!   [`ClientAuthentication::HttpBasic`].
//! - **Revocation**: supported; `token` + `token_type_hint` in the body,
//!   HTTP Basic client authentication.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};
use crate::oauth::provider::{
    ClientAuthentication, ClientAuthenticationMaterial, EndpointOverrides, InvalidGrantMeaning,
    OAuthProvider, ParamPlacement, ProviderIdentity, ProviderResponse, RefreshPolicy,
    RevocationRequest, RevocationTransport, TokenHint,
};
use crate::oauth::request_shape::{AuthorizationRequestShape, TokenRequestShape};

const AUTHORIZATION_ENDPOINT: &str = "https://twitter.com/i/oauth2/authorize";
const TOKEN_ENDPOINT: &str = "https://api.twitter.com/2/oauth2/token";
const USERINFO_ENDPOINT: &str = "https://api.twitter.com/2/users/me";
const REVOCATION_ENDPOINT: &str = "https://api.twitter.com/2/oauth2/revoke";

/// Route-level configuration for the X provider.
#[derive(Clone, Debug)]
pub struct XProviderConfig {
    /// The OAuth client id.
    pub client_id: String,
    /// The OAuth client secret.
    pub client_secret: SecretString,
    /// The registered callback URI, when the client sends one explicitly.
    pub redirect_uri: Option<String>,
    /// The requested scopes (`tweet.read`, `users.read`, ...).
    pub scopes: Vec<String>,
    /// Endpoint URL overrides; defaults to X's real dossier URLs.
    pub endpoints: EndpointOverrides,
}

/// The raw shape of X API v2's `/2/users/me` response.
#[derive(Deserialize)]
struct XUsersMeEnvelope {
    data: Option<XUsersMeData>,
}

#[derive(Deserialize)]
struct XUsersMeData {
    id: Option<String>,
    name: Option<String>,
}

/// The X `OAuthProvider` plugin.
pub struct XOAuthProvider {
    config: XProviderConfig,
    transport: Arc<dyn RevocationTransport>,
}

impl XOAuthProvider {
    /// Compose the provider from its configuration and a host-supplied
    /// revocation transport.
    #[must_use]
    pub fn new(config: XProviderConfig, transport: Arc<dyn RevocationTransport>) -> Self {
        Self { config, transport }
    }

    /// The `Authorization: Basic` header value for X's confidential-client
    /// token/revocation endpoints. Pure computation over already-configured
    /// credentials -- not a network call. RFC 6749 §2.3.1: the client
    /// identifier and secret are each `application/x-www-form-urlencoded`
    /// before joining with `:` and base64-encoding -- not merely
    /// interpolated raw, which would corrupt a secret containing `:` or a
    /// reserved character.
    fn basic_authorization(&self) -> String {
        let credentials = format!(
            "{}:{}",
            form_urlencode(&self.config.client_id),
            form_urlencode(self.config.client_secret.expose_secret())
        );
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        )
    }
}

/// `application/x-www-form-urlencoded` encoding of one value (RFC 6749
/// §2.3.1's `client_id`/`client_secret` HTTP Basic encoding step): ASCII
/// alphanumerics and `-_.~` pass through unencoded, space becomes `+`,
/// everything else is percent-encoded as uppercase `%XX`.
fn form_urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[async_trait]
impl OAuthProvider for XOAuthProvider {
    fn name(&self) -> &'static str {
        "x"
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
                provider: "x",
                message: "X identity resolution requires a UserInfo response".to_owned(),
            });
        };
        let envelope: XUsersMeEnvelope = serde_json::from_str(&body).map_err(|error| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "x",
                message: format!("failed to parse /2/users/me body: {error}"),
            }
        })?;
        let data = envelope
            .data
            .ok_or_else(|| OAuthProtocolError::MalformedProviderResponse {
                provider: "x",
                message: "/2/users/me response missing `data`".to_owned(),
            })?;
        let subject = data.id.filter(|id| !id.is_empty()).ok_or_else(|| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "x",
                message: "/2/users/me response missing `data.id`".to_owned(),
            }
        })?;
        // X never supplies a reliable email -- always absent by design, not
        // by omission; see this module's dossier doc.
        Ok(ProviderIdentity {
            provider: "x".to_owned(),
            subject,
            email: None,
            email_verified: false,
            display_name: data.name,
        })
    }

    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()> {
        let auth = self.client_authentication().await?;
        let request = RevocationRequest {
            method: "POST",
            endpoint: self
                .config
                .endpoints
                .revocation_endpoint
                .clone()
                .unwrap_or_else(|| REVOCATION_ENDPOINT.to_owned()),
            placement: ParamPlacement::Body,
            params: vec![
                ("token".to_owned(), token.to_owned()),
                ("token_type_hint".to_owned(), hint.wire_value().to_owned()),
            ],
            headers: auth.headers,
        };
        self.transport.send(request).await
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: true,
            token_client_authentication: ClientAuthentication::HttpBasic,
            extra_authorization_params: Vec::new(),
            required_scopes: vec!["offline.access".to_owned()],
            requires_reconsent_for_reissue: false,
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
            params: Vec::new(),
            headers: vec![("Authorization".to_owned(), self.basic_authorization())],
        })
    }
}
