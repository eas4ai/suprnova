//! Facebook Graph API `OAuthProvider` plugin behind the `oauth-facebook`
//! feature (`docs/specs/suprnova-magnetar/10-providers.md`'s Facebook
//! section: "Graph API identity fetch shape, no OIDC id_token path").
//!
//! ## Dossier
//!
//! - **Endpoints**: authorize
//!   `https://www.facebook.com/{version}/dialog/oauth`, token
//!   `https://graph.facebook.com/{version}/oauth/access_token`, identity
//!   `https://graph.facebook.com/{version}/me`, de-authorize
//!   `https://graph.facebook.com/{version}/me/permissions`, where
//!   `{version}` is [`FacebookProviderConfig::graph_api_version`]
//!   (default `v26.0`, the current Graph API release per Meta's own
//!   changelog, `developers.facebook.com/docs/graph-api/changelog/`,
//!   verified live 2026-08-19: "The latest Graph API version is: v26.0").
//!   Configurable, not hardcoded, because Meta retires a version roughly
//!   every two years on a rolling schedule and rotates the "current"
//!   release about twice a year -- an earlier version of this dossier
//!   pinned `v16.0` (retired 2025-05-14), which the same changelog would
//!   have caught.
//! - **Client authentication**: [`ClientAuthentication::RequestBody`] (the
//!   RFC 6749 default; no quirk handler needed for the request shapes).
//! - **PKCE posture**: [`PkcePosture::Required`] -- the 09 engine default
//!   stands. Meta's own manually-built login flow reference does not
//!   *mention* `code_challenge`/`code_verifier`
//!   (`developers.facebook.com/documentation/facebook-login/guides/advanced/manual-flow`),
//!   but that is silence, not evidence of rejection, and 09's rule places
//!   the burden on the latter ("default-on stands unless live evidence
//!   says otherwise"). Independent evidence points the other way: Facebook's
//!   token endpoint is documented to return `oauth_code_verification_failed`
//!   when a `code_verifier` is presented without a matching
//!   `code_challenge` on the original authorization request (observed
//!   integration behavior cited in `better-auth/better-auth#186`), which
//!   can only happen if the endpoint validates PKCE. Sending PKCE
//!   parameters Facebook does not reject removes authorization-code
//!   interception protection for zero benefit if wrong; not sending them
//!   when Facebook silently ignores them costs nothing either way, so the
//!   engine default is kept rather than guessed away.
//! - **Identity source**: a Graph API `GET` the host performs against
//!   `https://graph.facebook.com/{version}/me?fields=id,name,email` (Meta's
//!   Graph API User reference,
//!   `developers.facebook.com/docs/graph-api/reference/user/`, verified
//!   live 2026-08-19). Facebook's Graph API only ever returns a
//!   user-confirmed email address (Meta's documented policy since 2016), so
//!   this provider treats a present `email` as verified and an absent one
//!   as unverified/absent -- there is no separate `email_verified` field to
//!   read.
//! - **Refresh**: **not** RFC 6749 `refresh_token`-grant shaped. Facebook
//!   extends token lifetime via a separate long-lived-token exchange
//!   (`grant_type=fb_exchange_token`), which [`RefreshPolicy`] does not
//!   model; `RefreshPolicy::supported` is `false` here to say so plainly
//!   rather than mis-claim RFC 6749 refresh support Task 4 cannot use.
//! - **Revocation**: Facebook has no RFC 7009 endpoint; de-authorization is
//!   `DELETE https://graph.facebook.com/{version}/me/permissions` with the
//!   access token supplied as a **query** parameter, not a body parameter
//!   (Meta's documented "de-authorize" pattern;
//!   [`crate::oauth::provider::ParamPlacement::Query`]). [`TokenHint`] is
//!   accepted but ignored: Facebook has no refresh token to distinguish.

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

/// The current Graph API version, per Meta's own changelog (see this
/// module's dossier doc). Used as [`FacebookProviderConfig::graph_api_version`]'s
/// default; hosts should override it directly rather than wait for a
/// crate update once Meta rotates the current release.
pub const DEFAULT_GRAPH_API_VERSION: &str = "v26.0";

/// Route-level configuration for the Facebook provider.
#[derive(Clone, Debug)]
pub struct FacebookProviderConfig {
    /// The OAuth client id (Facebook App ID).
    pub client_id: String,
    /// The OAuth client secret (Facebook App Secret).
    pub client_secret: SecretString,
    /// The registered callback URI, when the client sends one explicitly.
    pub redirect_uri: Option<String>,
    /// The requested scopes (`email`, `public_profile`).
    pub scopes: Vec<String>,
    /// The Graph API version segment (e.g. `"v26.0"`), used to build the
    /// identity and de-authorization endpoints. Meta retires a version on
    /// a roughly two-year cycle; override this rather than let it go
    /// stale. Defaults to [`DEFAULT_GRAPH_API_VERSION`] via
    /// [`FacebookProviderConfig::default`].
    pub graph_api_version: String,
    /// Endpoint URL overrides; defaults to Facebook's real, versioned
    /// dossier URLs.
    pub endpoints: EndpointOverrides,
}

impl Default for FacebookProviderConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: SecretString::from(String::new()),
            redirect_uri: None,
            scopes: Vec::new(),
            graph_api_version: DEFAULT_GRAPH_API_VERSION.to_owned(),
            endpoints: EndpointOverrides::default(),
        }
    }
}

/// The raw shape of Facebook's Graph API `/me?fields=id,name,email`
/// response.
#[derive(Deserialize)]
struct FacebookUser {
    id: Option<String>,
    name: Option<String>,
    email: Option<String>,
}

/// The Facebook `OAuthProvider` plugin.
pub struct FacebookOAuthProvider {
    /// `client_id()`/`client_authentication()` expose what a caller needs;
    /// the config itself stays private like every other provider's (I5:
    /// this field was briefly `pub` only to satisfy a `dead_code` lint
    /// before those trait methods existed -- that reason no longer holds).
    config: FacebookProviderConfig,
    transport: Arc<dyn RevocationTransport>,
}

impl FacebookOAuthProvider {
    /// Compose the provider from its configuration and a host-supplied
    /// revocation transport.
    #[must_use]
    pub fn new(config: FacebookProviderConfig, transport: Arc<dyn RevocationTransport>) -> Self {
        Self { config, transport }
    }
}

#[async_trait]
impl OAuthProvider for FacebookOAuthProvider {
    fn name(&self) -> &'static str {
        "facebook"
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
                provider: "facebook",
                message: "Facebook identity resolution requires a UserInfo response".to_owned(),
            });
        };
        let user: FacebookUser = serde_json::from_str(&body).map_err(|error| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "facebook",
                message: format!("failed to parse Graph API user body: {error}"),
            }
        })?;
        let subject = user.id.filter(|id| !id.is_empty()).ok_or_else(|| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "facebook",
                message: "Graph API user response missing `id`".to_owned(),
            }
        })?;
        let email_verified = user.email.is_some();
        Ok(ProviderIdentity {
            provider: "facebook".to_owned(),
            subject,
            email: user.email,
            email_verified,
            display_name: user.name,
        })
    }

    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()> {
        let _ = hint; // Facebook has no refresh token to distinguish.
        let endpoint = self
            .config
            .endpoints
            .revocation_endpoint
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "https://graph.facebook.com/{}/me/permissions",
                    self.config.graph_api_version
                )
            });
        let request = RevocationRequest {
            method: "DELETE",
            endpoint,
            placement: ParamPlacement::Query,
            params: vec![("access_token".to_owned(), token.to_owned())],
            headers: Vec::new(),
        };
        self.transport.send(request).await
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: false,
            token_client_authentication: ClientAuthentication::RequestBody,
            extra_authorization_params: Vec::new(),
            required_scopes: Vec::new(),
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
            .unwrap_or_else(|| {
                format!(
                    "https://graph.facebook.com/{}/oauth/access_token",
                    self.config.graph_api_version
                )
            })
    }
    fn authorization_endpoint(&self) -> String {
        self.config
            .endpoints
            .authorization_endpoint
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "https://www.facebook.com/{}/dialog/oauth",
                    self.config.graph_api_version
                )
            })
    }
    fn userinfo_endpoint(&self) -> Option<String> {
        Some(
            self.config
                .endpoints
                .userinfo_endpoint
                .clone()
                .unwrap_or_else(|| {
                    format!(
                        "https://graph.facebook.com/{}/me",
                        self.config.graph_api_version
                    )
                }),
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
