//! RFC 7523 §2.1 JWT-bearer grant: a self-signed assertion in place of an
//! authorization code
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s "Client credentials
//! and JWT bearer" section -- "JWT-bearer assertions ... as grant (service
//! accounts)"). Distinct from JWT-bearer *client authentication*
//! (RFC 7523 §2.2), which every provider already renders through
//! [`crate::oauth::provider::OAuthProvider::client_authentication`]
//! (Apple's minted ES256 client secret is that path, not this one).

use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};
use crate::oauth::protocol::TokenSuccessResponse;
use crate::oauth::provider::OAuthProvider;
use crate::plugin::HttpTransport;

/// RFC 7523 §3 claims for one minted assertion. `exp`/`iat` are derived by
/// [`execute`] from its `ttl` argument, not supplied here.
#[derive(Clone, Debug)]
pub struct JwtBearerAssertion {
    /// The `iss` claim: the assertion issuer (service account identity).
    pub issuer: String,
    /// The `sub` claim: the principal the assertion is asserted for.
    pub subject: String,
    /// The `aud` claim: ordinarily the token endpoint URL.
    pub audience: String,
    /// An optional `kid` header identifying which key signed this
    /// assertion.
    pub key_id: Option<String>,
}

/// Host-supplied signing material for one assertion. The encoding key
/// itself never implements `Debug`/`Display` (`jsonwebtoken::EncodingKey`),
/// so signing key material is never logged; this type never persists
/// anything -- it is used once, for one [`encode`] call, by [`execute`].
pub struct JwtBearerSigningKey {
    /// The signature algorithm the key material is valid for.
    pub algorithm: Algorithm,
    /// The signing key.
    pub encoding_key: EncodingKey,
}

#[derive(Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    exp: i64,
    iat: i64,
}

/// Mint and exchange one RFC 7523 §2.1 JWT-bearer assertion for tokens.
///
/// # Errors
///
/// Returns [`OAuthProtocolError::ProviderConfiguration`] when the assertion
/// fails to sign (an algorithm/key-material mismatch in the host-supplied
/// `signing_key`) -- caught before any network call. Otherwise propagates
/// provider/network failure classes from [`super::execute_token_request`].
pub async fn execute(
    provider: &dyn OAuthProvider,
    transport: &dyn HttpTransport,
    assertion: &JwtBearerAssertion,
    signing_key: &JwtBearerSigningKey,
    ttl: Duration,
    scopes: &[String],
) -> OAuthResult<TokenSuccessResponse> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    let signed = sign_assertion(
        assertion,
        signing_key,
        issued_at,
        expires_at,
        provider.name(),
    )?;

    let shape = provider.token_shape();
    let mut wire = vec![
        (
            shape.client_id_param.clone(),
            provider.client_id().to_owned(),
        ),
        (
            "grant_type".to_owned(),
            "urn:ietf:params:oauth:grant-type:jwt-bearer".to_owned(),
        ),
        ("assertion".to_owned(), signed.expose_secret().to_owned()),
    ];
    if shape.always_send_scope || !scopes.is_empty() {
        wire.push(("scope".to_owned(), scopes.join(&shape.scope_delimiter)));
    }
    let auth = provider.client_authentication().await?;
    wire.extend(auth.params);

    super::execute_token_request(
        transport,
        provider.name(),
        &provider.token_endpoint(),
        wire,
        auth.headers,
        shape.accept_http_success_error_body,
    )
    .await
}

fn sign_assertion(
    assertion: &JwtBearerAssertion,
    signing_key: &JwtBearerSigningKey,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    provider: &'static str,
) -> OAuthResult<SecretString> {
    let mut header = Header::new(signing_key.algorithm);
    header.kid.clone_from(&assertion.key_id);
    let claims = AssertionClaims {
        iss: &assertion.issuer,
        sub: &assertion.subject,
        aud: &assertion.audience,
        exp: expires_at.timestamp(),
        iat: issued_at.timestamp(),
    };
    let compact = encode(&header, &claims, &signing_key.encoding_key).map_err(|error| {
        OAuthProtocolError::ProviderConfiguration {
            provider,
            message: format!("failed to sign jwt-bearer assertion: {error}"),
        }
    })?;
    Ok(SecretString::from(compact))
}
