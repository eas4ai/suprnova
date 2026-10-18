//! The JSON Web Tokens agent auth signs and checks: its own identity
//! assertions (ES256, `typ: oauth-id-jag+jwt`), and the ID-JAGs and
//! Security Event Tokens trusted providers sign.

use std::collections::HashSet;

use base64::Engine;
use chrono::{DateTime, Utc};
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk};
use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::config::{SigningKey, TrustedIssuer};
use super::key_sets::{KeySetError, KeySetFetcher};
use crate::{Error, Result};

/// The `typ` of an ID-JAG and of this server's identity assertions.
pub(crate) const ID_JAG_TYPE: &str = "oauth-id-jag+jwt";
/// The `typ` of a Security Event Token.
pub(crate) const SET_TYPE: &str = "secevent+jwt";
/// The longest token read; anything longer is refused unparsed.
const MAX_TOKEN_LENGTH: usize = 16 * 1024;

/// The claims of an identity assertion this server signs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AssertionClaims {
    pub(crate) iss: String,
    pub(crate) sub: String,
    pub(crate) aud: String,
    pub(crate) iat: i64,
    pub(crate) exp: i64,
    pub(crate) jti: String,
    pub(crate) registration_type: String,
    /// The registration's assertion version when this was signed.
    pub(crate) ver: u32,
    /// The bound user's authentication epoch when this was signed; absent
    /// before a user is bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) auth_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) email_verified: Option<bool>,
}

/// Sign an identity assertion with the current key.
pub(crate) fn sign_assertion(key: &SigningKey, claims: &AssertionClaims) -> Result<String> {
    let mut header = Header::new(Algorithm::ES256);
    header.typ = Some(ID_JAG_TYPE.to_owned());
    header.kid = Some(key.kid().to_owned());
    jsonwebtoken::encode(&header, claims, key.encoding()).map_err(|error| Error::Internal {
        message: format!("sign an identity assertion: {error}"),
    })
}

/// Check an identity assertion this server signed, with any key it signs
/// or used to sign with: the type, the signature, the issuer and audience,
/// and the expiry. `Err` says why, for the `invalid_grant` description.
pub(crate) fn verify_assertion<'a>(
    token: &str,
    keys: impl IntoIterator<Item = &'a SigningKey>,
    issuer: &str,
    now: DateTime<Utc>,
) -> std::result::Result<AssertionClaims, &'static str> {
    if token.len() > MAX_TOKEN_LENGTH {
        return Err("the assertion is too long");
    }
    let header = jsonwebtoken::decode_header(token).map_err(|_| "the assertion is malformed")?;
    if !is_type(header.typ.as_deref(), ID_JAG_TYPE) {
        return Err("the assertion is not an identity assertion");
    }
    if header.alg != Algorithm::ES256 {
        return Err("the assertion's algorithm is not accepted");
    }
    let kid = header.kid.as_deref().ok_or("the assertion names no key")?;
    let key = keys
        .into_iter()
        .find(|key| key.kid() == kid)
        .ok_or("the assertion's key is not one of this server's")?;
    let data = jsonwebtoken::decode::<AssertionClaims>(
        token,
        key.decoding(),
        &bare_validation(header.alg),
    )
    .map_err(|_| "the assertion's signature does not verify")?;
    let claims = data.claims;
    if claims.iss != issuer || claims.aud != issuer {
        return Err("the assertion was issued for another server");
    }
    if claims.exp <= now.timestamp() {
        return Err("the assertion expired");
    }
    Ok(claims)
}

/// A validation that checks the signature and the algorithm only; every
/// claim is checked by the caller, which knows the error each one maps to.
fn bare_validation(algorithm: Algorithm) -> Validation {
    let mut validation = Validation::new(algorithm);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    validation.required_spec_claims = HashSet::new();
    validation
}

/// Whether a `typ` header names `expected`, compared without case and with
/// or without the `application/` prefix (RFC 7515 section 4.1.9).
fn is_type(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| {
        let value = value
            .get(..12)
            .filter(|prefix| prefix.eq_ignore_ascii_case("application/"))
            .map_or(value, |_| &value[12..]);
        value.eq_ignore_ascii_case(expected)
    })
}

/// The asymmetric algorithms a provider may sign with. HMAC is never
/// accepted: its key would be a secret shared with every provider.
fn provider_algorithm(algorithm: Algorithm) -> bool {
    matches!(
        algorithm,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

/// Why a provider's token was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderTokenError {
    /// The token is not a JWT of the expected type, or its payload is not
    /// a JSON object.
    Malformed(&'static str),
    /// The issuer is not on the trust list.
    UntrustedIssuer,
    /// The provider's key set could not be fetched.
    KeysUnavailable(String),
    /// The provider's key set has no key with the token's key identifier.
    UnknownKey,
    /// The signature does not verify against the provider's key.
    BadSignature,
}

/// A provider token whose signature verified, with its claims unchecked.
pub(crate) struct VerifiedProviderToken {
    pub(crate) trusted: TrustedIssuer,
    pub(crate) claims: Map<String, Value>,
}

/// Check a provider token's type, issuer and signature: the issuer must be
/// on the trust list and the signature must verify against the key the
/// token names in the issuer's key set.
pub(crate) async fn verify_provider_token(
    token: &str,
    expected_type: &str,
    trusted_issuers: &[TrustedIssuer],
    key_sets: &KeySetFetcher,
) -> std::result::Result<VerifiedProviderToken, ProviderTokenError> {
    if token.len() > MAX_TOKEN_LENGTH {
        return Err(ProviderTokenError::Malformed("the token is too long"));
    }
    let header = jsonwebtoken::decode_header(token)
        .map_err(|_| ProviderTokenError::Malformed("the token is not a signed JWT"))?;
    if !is_type(header.typ.as_deref(), expected_type) {
        return Err(ProviderTokenError::Malformed("the token has the wrong typ"));
    }
    let unverified = unverified_claims(token)?;
    let issuer = unverified
        .get("iss")
        .and_then(Value::as_str)
        .ok_or(ProviderTokenError::Malformed("the token names no issuer"))?;
    let trusted = trusted_issuers
        .iter()
        .find(|entry| entry.issuer == issuer)
        .ok_or(ProviderTokenError::UntrustedIssuer)?;
    if !provider_algorithm(header.alg) {
        return Err(ProviderTokenError::BadSignature);
    }
    let Some(kid) = header.kid.as_deref() else {
        return Err(ProviderTokenError::UnknownKey);
    };
    let jwk = key_sets
        .key(&trusted.jwks_uri, kid)
        .await
        .map_err(|error| match error {
            KeySetError::UnknownKey => ProviderTokenError::UnknownKey,
            KeySetError::Unavailable(reason) => ProviderTokenError::KeysUnavailable(reason),
        })?;
    let key = decoding_key(&jwk, header.alg).ok_or(ProviderTokenError::BadSignature)?;
    let data =
        jsonwebtoken::decode::<Map<String, Value>>(token, &key, &bare_validation(header.alg))
            .map_err(|_| ProviderTokenError::BadSignature)?;
    Ok(VerifiedProviderToken {
        trusted: trusted.clone(),
        claims: data.claims,
    })
}

/// The verification key a JWK gives for `algorithm`, or `None` for a key of
/// another family, a symmetric key, or one the JWK pins to another
/// algorithm.
fn decoding_key(jwk: &Jwk, algorithm: Algorithm) -> Option<DecodingKey> {
    if matches!(jwk.algorithm, AlgorithmParameters::OctetKey(_)) {
        return None;
    }
    if let Some(pinned) = &jwk.common.key_algorithm
        && format!("{pinned:?}") != format!("{algorithm:?}")
    {
        return None;
    }
    DecodingKey::from_jwk(jwk).ok()
}

/// The payload of a JWT before its signature is checked, read only to find
/// the issuer whose keys check it.
fn unverified_claims(token: &str) -> std::result::Result<Map<String, Value>, ProviderTokenError> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or(ProviderTokenError::Malformed(
            "the token is not a signed JWT",
        ))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| ProviderTokenError::Malformed("the token payload is not base64url"))?;
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(claims)) => Ok(claims),
        _ => Err(ProviderTokenError::Malformed(
            "the token payload is not a JSON object",
        )),
    }
}

/// A numeric date claim (RFC 7519 section 2), whole seconds.
pub(crate) fn numeric_date(claims: &Map<String, Value>, name: &str) -> Option<i64> {
    let value = claims.get(name)?;
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|seconds| seconds.is_finite())
            .map(|seconds| seconds as i64)
    })
}

/// Whether an `aud` claim, a string or an array of strings, names one of
/// `audiences`.
pub(crate) fn audience_matches(claims: &Map<String, Value>, audiences: &[String]) -> bool {
    match claims.get("aud") {
        Some(Value::String(audience)) => audiences.iter().any(|accepted| accepted == audience),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .any(|audience| audiences.iter().any(|accepted| accepted == audience)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_007_typ_compares_without_case_and_media_prefix() {
        assert!(is_type(Some("oauth-id-jag+jwt"), ID_JAG_TYPE));
        assert!(is_type(Some("application/OAuth-ID-JAG+JWT"), ID_JAG_TYPE));
        assert!(!is_type(Some("JWT"), ID_JAG_TYPE));
        assert!(!is_type(None, ID_JAG_TYPE));
        assert!(!is_type(Some("é"), ID_JAG_TYPE));
    }
}
