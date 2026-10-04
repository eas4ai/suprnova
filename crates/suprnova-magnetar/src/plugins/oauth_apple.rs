//! Apple ("Sign in with Apple") `OAuthProvider` plugin: thin glue over
//! `suprnova-apple-rs` behind the `oauth-apple` feature
//! (`docs/specs/suprnova-magnetar/10-providers.md`'s Apple section).
//!
//! ## Dossier
//!
//! - **Endpoints**: authorize `https://appleid.apple.com/auth/authorize`,
//!   token `https://appleid.apple.com/auth/token`, revoke
//!   `https://appleid.apple.com/auth/revoke`. Evidence: arctic
//!   `reference/arctic-oauth-master/src/providers/apple.rs`'s
//!   `AUTHORIZATION_ENDPOINT`/`TOKEN_ENDPOINT` constants (authorize/token,
//!   byte-identical to Suprnova's own well-known table in
//!   `framework/src/torii_integration/oauth.rs`); revoke per Apple's
//!   published "Sign in with Apple REST API -- Token revocation" reference
//!   (`developer.apple.com/documentation/signinwithapplerestapi/revoke-tokens`,
//!   verified live 2026-08-19): `POST` with `client_id`, `client_secret`,
//!   `token`, `token_type_hint`, returning `200` with no body.
//! - **Client authentication**: [`ClientAuthentication::SignedJwt`] -- an
//!   ES256 JWT client secret minted per-request from the host-supplied key
//!   (`iss` = team id, `sub` = client id, `aud` =
//!   `https://appleid.apple.com`), assembled following Apple's own
//!   published token-generation spec (not
//!   `suprnova-apple-rs`'s private `AppleAuthImpl::client_secret`, which
//!   is inaccessible from outside that crate). Signed directly against a
//!   `p256::SecretKey` decoded from the configured PKCS8 PEM via
//!   `p256::pkcs8::DecodePrivateKey`/`EncodePrivateKey` --
//!   `suprnova-apple-rs`'s own `apple::signing::AppleKeyPair` is not used
//!   anywhere in this provider (see [`AppleOAuthProvider::new`]'s doc for
//!   why: its `from_pem_bytes`/`from_file`/`from_base64` cannot load a
//!   real Apple `.p8` key in this crate's v0.3.1). Never persisted; minted
//!   fresh (5-minute TTL) for every call rather than cached, so no
//!   long-lived secret sits in process memory.
//! - **PKCE posture**: [`PkcePosture::Disabled`] -- Apple rejects
//!   `code_challenge` on this flow. Evidence:
//!   `framework/src/torii_integration/oauth.rs`'s `build_authorization_url`
//!   doc ("Sending `code_challenge` ... would make Apple reject the
//!   request"); already the exact shape Task 1's
//!   `tests/oauth_request_shapes.rs` exercises.
//! - **Response mode**: `form_post` (Apple requires it for
//!   authorization-code callbacks).
//! - **Nonce**: `requires_nonce = true`. With PKCE disabled, `state` alone
//!   would be this flow's only replay/injection defense; a nonce minted at
//!   [`crate::oauth::authorization::OAuthAuthorizationService::begin`] and
//!   bound into `resolve_identity`'s [`ApplePublicKeySource::verify`] call
//!   closes that gap the same way Apple's own OIDC guidance recommends for
//!   PKCE-less clients: the nonce is checked against the verified ID
//!   token's `nonce` claim, and a mismatch classifies as
//!   [`crate::oauth::errors::OAuthProtocolError::IdentityVerificationFailed`].
//! - **Identity source**: the signed ID token returned by the token
//!   endpoint, verified via JWKS
//!   (`https://appleid.apple.com/auth/keys`) through
//!   [`apple::jwks::AppleJwksClient`]/[`apple::user::get_user_info_from_id_token`].
//!   Apple has no userinfo endpoint.
//! - **Email/name availability**: both are supplied only on the account's
//!   first authorization and never resent
//!   (`docs/specs/suprnova-magnetar/10-providers.md`: "Apple returns
//!   name/email only on the first authorization"). Email rides the
//!   verified ID token's `email`/`email_verified` claims; the display name
//!   is delivered separately in the form_post callback body's `user` JSON
//!   parameter (never part of the JWT), so [`ProviderResponse::AppleIdToken`]
//!   carries it alongside the token. A private-relay address
//!   (`is_private_email`) is an ordinary verified email, not treated
//!   differently.
//! - **Refresh**: supported (Task 4 wires `apple::auth`'s refresh grant);
//!   [`ClientAuthentication::SignedJwt`], no extra authorization params, no
//!   required extra scopes.
//! - **Revocation**: supported (see endpoint above); [`TokenHint`] maps
//!   directly to the `token_type_hint` parameter.
//!
//! An **unverified** email from Apple is never treated as absent-and-safe
//! nor as an error here: this provider reports exactly what the ID token
//! asserts and lets [`crate::oauth::identity::IdentityResolver`] apply 09's
//! "unverified == absent" policy uniformly across providers (a deliberate
//! divergence from `torii_integration/oauth.rs`'s `apple_verified_email`,
//! which errors 401 on an unverified email -- Magnetar's engine centralizes
//! that decision instead of duplicating it per provider).

use std::sync::Arc;

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use crate::oauth::OAuthProtocolError;
use crate::oauth::errors::OAuthResult;
use crate::oauth::provider::{
    ClientAuthentication, ClientAuthenticationMaterial, EndpointOverrides, InvalidGrantMeaning,
    OAuthProvider, ParamPlacement, ProviderIdentity, ProviderResponse, RefreshPolicy,
    RevocationRequest, RevocationTransport, TokenHint,
};
use crate::oauth::request_shape::{AuthorizationRequestShape, PkcePosture, TokenRequestShape};

const AUTHORIZATION_ENDPOINT: &str = "https://appleid.apple.com/auth/authorize";
const REVOCATION_ENDPOINT: &str = "https://appleid.apple.com/auth/revoke";
const TOKEN_ENDPOINT: &str = "https://appleid.apple.com/auth/token";
const AUDIENCE: &str = "https://appleid.apple.com";
/// Minted fresh per call rather than cached; long enough to cover one
/// request round trip, short enough that a leaked value has little value.
const CLIENT_SECRET_TTL_SECS: i64 = 300;

/// Route-level configuration for the Apple provider.
///
/// `private_key_pem` carries the `.p8` PKCS8 key contents the host loaded
/// from its own secret-management system; Magnetar never persists it (spec
/// 10's "never persisted by Magnetar"). Deriving [`Debug`] is safe: the
/// [`SecretString`] field redacts itself.
#[derive(Clone, Debug)]
pub struct AppleProviderConfig {
    /// Apple's Services ID (the OAuth `client_id`/audience for the ID
    /// token).
    pub client_id: String,
    /// Apple's 10-character Team ID (populates the client-secret JWT's
    /// `iss` claim).
    pub team_id: String,
    /// The `.p8` key's key ID (populates the client-secret JWT header's
    /// `kid`).
    pub key_id: String,
    /// The `.p8` private key's PKCS8 PEM contents.
    pub private_key_pem: SecretString,
    /// The registered callback URI, when the client sends one explicitly.
    pub redirect_uri: Option<String>,
    /// The requested scopes (`name`, `email`).
    pub scopes: Vec<String>,
    /// Endpoint URL overrides; defaults to Apple's real dossier URLs.
    pub endpoints: EndpointOverrides,
}

/// Claims extracted from Apple's verified ID token.
#[derive(Clone, Debug)]
pub struct AppleClaims {
    /// The stable `sub` claim -- Apple's account identifier.
    pub subject: String,
    /// The `email` claim, when present (first authorization only).
    pub email: Option<String>,
    /// Whether Apple asserts `email` is verified.
    pub email_verified: bool,
    /// Whether `email` is a private-relay address.
    pub is_private_email: bool,
}

/// The JWKS ID-token verification seam a host implements
/// (`docs/specs/suprnova-magnetar/10-providers.md`: "any network seam is a
/// trait the host implements"). [`LiveApplePublicKeySource`] wraps
/// `suprnova-apple-rs`'s real JWKS client for production; offline test
/// suites supply a fake returning canned claims.
#[async_trait]
pub trait ApplePublicKeySource: Send + Sync {
    /// Verify `id_token`'s signature, issuer, audience, expiry, and (when
    /// supplied) nonce, returning its claims.
    async fn verify(
        &self,
        id_token: &str,
        audience: &str,
        nonce: Option<&str>,
    ) -> OAuthResult<AppleClaims>;
}

/// Production [`ApplePublicKeySource`]: thin glue over
/// [`apple::jwks::AppleJwksClient`]/[`apple::user::get_user_info_from_id_token`].
/// Performs real network I/O (JWKS fetch) -- never constructed by an
/// offline test suite.
pub struct LiveApplePublicKeySource {
    jwks: apple::jwks::AppleJwksClient,
}

impl LiveApplePublicKeySource {
    /// Construct a JWKS client with `suprnova-apple-rs`'s default 1-hour
    /// cache TTL.
    ///
    /// # Errors
    ///
    /// Returns [`OAuthProtocolError::UpstreamUnavailable`] when the
    /// underlying HTTP client cannot be built.
    pub fn new() -> OAuthResult<Self> {
        let jwks = apple::jwks::AppleJwksClient::new().map_err(map_apple_error)?;
        Ok(Self { jwks })
    }
}

#[async_trait]
impl ApplePublicKeySource for LiveApplePublicKeySource {
    async fn verify(
        &self,
        id_token: &str,
        audience: &str,
        nonce: Option<&str>,
    ) -> OAuthResult<AppleClaims> {
        let user = apple::user::get_user_info_from_id_token(id_token, audience, nonce, &self.jwks)
            .await
            .map_err(map_apple_error)?;
        let subject = user
            .subject
            .filter(|subject| !subject.is_empty())
            .ok_or_else(|| OAuthProtocolError::MalformedProviderResponse {
                provider: "apple",
                message: "ID token missing `sub`".to_owned(),
            })?;
        Ok(AppleClaims {
            subject,
            email: user.email.filter(|email| !email.is_empty()),
            email_verified: user.email_verified,
            is_private_email: user.is_private_email,
        })
    }
}

/// Map `suprnova-apple-rs`'s error into this crate's protocol error space,
/// mirroring `framework/src/torii_integration/oauth.rs`'s `map_apple_error`
/// classification (signature/audience/expiry/nonce -> identity-verification
/// failure; JWKS/HTTP -> upstream; the provider's own OAuth error body ->
/// provider-reported; anything else -> a local configuration/implementation
/// fault).
fn map_apple_error(error: apple::error::AppleError) -> OAuthProtocolError {
    use apple::error::AppleError;
    match error {
        AppleError::TokenValidationError(reason) => {
            OAuthProtocolError::IdentityVerificationFailed {
                provider: "apple",
                reason,
            }
        }
        AppleError::MissingCertificateChain => OAuthProtocolError::IdentityVerificationFailed {
            provider: "apple",
            reason: "JWS missing certificate chain".to_owned(),
        },
        AppleError::JwksError(message) | AppleError::HttpError(message) => {
            OAuthProtocolError::UpstreamUnavailable {
                provider: "apple",
                message,
                retry_after_seconds: None,
            }
        }
        AppleError::ResponseError(response) => OAuthProtocolError::ProviderReportedError {
            provider: "apple",
            code: format!("{:?}", response.error_type),
            message: Some(response.message.to_owned()),
        },
        AppleError::StateMismatchError => OAuthProtocolError::MalformedProviderResponse {
            provider: "apple",
            message: "OAuth state mismatch".to_owned(),
        },
        other => OAuthProtocolError::ProviderConfiguration {
            provider: "apple",
            message: other.to_string(),
        },
    }
}

/// Parse the display name out of Apple's form_post callback `user` JSON
/// parameter (`{"name":{"firstName":"...","lastName":"..."}}`), present
/// only on the account's first authorization.
fn parse_form_post_display_name(raw: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct FormPostUser {
        name: Option<FormPostName>,
    }
    #[derive(serde::Deserialize)]
    struct FormPostName {
        #[serde(rename = "firstName")]
        first_name: Option<String>,
        #[serde(rename = "lastName")]
        last_name: Option<String>,
    }
    let user: FormPostUser = serde_json::from_str(raw).ok()?;
    let name = user.name?;
    let parts: Vec<&str> = [name.first_name.as_deref(), name.last_name.as_deref()]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// The Apple `OAuthProvider` plugin.
pub struct AppleOAuthProvider {
    config: AppleProviderConfig,
    /// The parsed ECDSA P-256 signing key, decoded once at construction.
    /// `p256::SecretKey` zeroizes its scalar on drop; this field is the
    /// *only* copy of the raw key material this provider ever holds --
    /// unlike an earlier version, nothing re-encodes it into an
    /// intermediate `String`/PEM along the way.
    signing_key: p256::SecretKey,
    key_source: Arc<dyn ApplePublicKeySource>,
    transport: Arc<dyn RevocationTransport>,
}

impl AppleOAuthProvider {
    /// Compose the provider, parsing `config.private_key_pem` into a
    /// signing key up front so a malformed key surfaces at construction
    /// rather than on the first `revoke` call.
    ///
    /// `config.private_key_pem` is a standard PKCS8 PEM -- the format
    /// Apple's Developer portal issues `.p8` downloads in. Parsed directly
    /// via `p256::pkcs8::DecodePrivateKey`; this provider never depends on
    /// `suprnova-apple-rs`'s `apple::signing::AppleKeyPair` (whose
    /// `from_pem_bytes`/`from_file`/`from_base64` in v0.3.1 parse only a
    /// raw, unwrapped 32-byte scalar via `SigningKey::from_slice`, not a
    /// PKCS8 DER structure, and so cannot load a real Apple `.p8` file at
    /// all).
    ///
    /// # Errors
    ///
    /// Returns [`OAuthProtocolError::ProviderConfiguration`] when the
    /// configured private key is not a valid PKCS8 PEM ECDSA P-256 key.
    pub fn new(
        config: AppleProviderConfig,
        key_source: Arc<dyn ApplePublicKeySource>,
        transport: Arc<dyn RevocationTransport>,
    ) -> OAuthResult<Self> {
        use p256::pkcs8::DecodePrivateKey as _;

        let signing_key = p256::SecretKey::from_pkcs8_pem(config.private_key_pem.expose_secret())
            .map_err(|error| OAuthProtocolError::ProviderConfiguration {
            provider: "apple",
            message: format!("invalid Apple signing key (expected PKCS8 PEM): {error}"),
        })?;
        Ok(Self {
            config,
            signing_key,
            key_source,
            transport,
        })
    }

    /// Mint Apple's ES256 client-secret JWT per its published token
    /// generation spec (`iss`/`sub`/`aud`/`iat`/`exp`, `ES256`, `kid` in the
    /// header). This is Apple's own documented format, not
    /// `suprnova-apple-rs`'s private `AppleAuthImpl::client_secret` (which
    /// is inaccessible from outside the crate and only reachable through
    /// its network-performing `validate_code` path); `to_pkcs8_der` is
    /// `elliptic_curve::SecretKey`'s own `EncodePrivateKey` impl, not a
    /// `suprnova-apple-rs` accessor.
    ///
    /// # Errors
    ///
    /// Returns [`OAuthProtocolError::ProviderConfiguration`] when the key
    /// cannot be encoded or the JWT cannot be signed.
    fn client_secret(&self) -> OAuthResult<SecretString> {
        use p256::pkcs8::EncodePrivateKey as _;

        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            sub: &'a str,
            aud: &'a str,
            iat: i64,
            exp: i64,
        }

        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            iss: &self.config.team_id,
            sub: &self.config.client_id,
            aud: AUDIENCE,
            iat: now,
            exp: now + CLIENT_SECRET_TTL_SECS,
        };
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());

        let der = self.signing_key.to_pkcs8_der().map_err(|error| {
            OAuthProtocolError::ProviderConfiguration {
                provider: "apple",
                message: format!("failed to encode Apple signing key: {error}"),
            }
        })?;
        let token = encode(&header, &claims, &EncodingKey::from_ec_der(der.as_bytes())).map_err(
            |error| OAuthProtocolError::ProviderConfiguration {
                provider: "apple",
                message: format!("failed to sign Apple client secret: {error}"),
            },
        )?;
        Ok(SecretString::from(token))
    }
}

#[async_trait]
impl OAuthProvider for AppleOAuthProvider {
    fn name(&self) -> &'static str {
        "apple"
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        AuthorizationRequestShape {
            pkce: PkcePosture::Disabled,
            response_mode: Some("form_post".to_owned()),
            requires_nonce: true,
            ..AuthorizationRequestShape::default()
        }
    }

    fn token_shape(&self) -> TokenRequestShape {
        TokenRequestShape::default()
    }

    async fn resolve_identity(&self, response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        let ProviderResponse::AppleIdToken {
            id_token,
            nonce,
            form_post_user,
        } = response
        else {
            return Err(OAuthProtocolError::MalformedProviderResponse {
                provider: "apple",
                message: "Apple identity resolution requires an AppleIdToken response".to_owned(),
            });
        };

        // This provider's dossier requires a nonce (I6: with PKCE
        // disabled, it is Apple's only replay/injection defense).
        // `suprnova-apple-rs`'s JWKS verifier skips the nonce check
        // entirely when `None` is passed -- it does not fail closed on
        // our behalf -- so an absent nonce must be rejected here, before
        // ever calling the key source, rather than silently accepted as
        // "no nonce to check".
        let Some(nonce) = nonce else {
            return Err(OAuthProtocolError::IdentityVerificationFailed {
                provider: "apple",
                reason:
                    "nonce is required by this provider's dossier but was absent from the response"
                        .to_owned(),
            });
        };
        let claims = self
            .key_source
            .verify(
                id_token.expose_secret(),
                &self.config.client_id,
                Some(&nonce),
            )
            .await?;
        let display_name = form_post_user
            .as_deref()
            .and_then(parse_form_post_display_name);

        Ok(ProviderIdentity {
            provider: "apple".to_owned(),
            subject: claims.subject,
            email: claims.email,
            email_verified: claims.email_verified,
            display_name,
        })
    }

    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()> {
        let auth = self.client_authentication().await?;
        let mut params = vec![("client_id".to_owned(), self.config.client_id.clone())];
        params.extend(auth.params);
        params.push(("token".to_owned(), token.to_owned()));
        params.push(("token_type_hint".to_owned(), hint.wire_value().to_owned()));
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
            token_client_authentication: ClientAuthentication::SignedJwt,
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
        // Apple has no userinfo endpoint -- identity comes entirely from
        // the verified ID token, so an override here would have nothing
        // real to redirect.
        None
    }
    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial> {
        let client_secret = self.client_secret()?;
        Ok(ClientAuthenticationMaterial {
            params: vec![(
                "client_secret".to_owned(),
                client_secret.expose_secret().to_owned(),
            )],
            headers: Vec::new(),
        })
    }
}
