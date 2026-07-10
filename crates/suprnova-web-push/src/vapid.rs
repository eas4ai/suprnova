//! VAPID — Voluntary Application Server Identification (RFC 8292).
//!
//! ES256 (P-256 ECDSA) signing per spec.

use crate::error::WebPushError;
use base64::Engine;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A P-256 keypair for VAPID.
pub struct VapidKey {
    inner: SigningKey,
}

impl fmt::Debug for VapidKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VapidKey([REDACTED])")
    }
}

impl VapidKey {
    pub fn generate() -> Self {
        Self {
            inner: SigningKey::random(&mut OsRng),
        }
    }

    /// Import a raw 32-byte, big-endian P-256 private scalar.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, WebPushError> {
        if raw.len() != 32 {
            return Err(WebPushError::Vapid(format!(
                "VAPID private key must be exactly 32 bytes (got {})",
                raw.len()
            )));
        }

        let inner = SigningKey::from_slice(raw)
            .map_err(|_| WebPushError::Vapid("invalid P-256 private key scalar".to_string()))?;
        Ok(Self { inner })
    }

    pub fn from_pem(pem: &str) -> Result<Self, WebPushError> {
        let inner = SigningKey::from_pkcs8_pem(pem)
            .map_err(|e| WebPushError::Vapid(format!("invalid PEM: {e}")))?;
        Ok(Self { inner })
    }

    pub fn to_pem(&self) -> Result<String, WebPushError> {
        let secret_key = p256::SecretKey::from_slice(&self.inner.to_bytes())
            .map_err(|e| WebPushError::Vapid(format!("export PEM: {e}")))?;
        secret_key
            .to_pkcs8_pem(LineEnding::LF)
            .map(|pem| pem.to_string())
            .map_err(|e| WebPushError::Vapid(format!("export PEM: {e}")))
    }

    /// Return the uncompressed public key (0x04 || X || Y), base64url-no-pad.
    /// The uncompressed encoding is 65 bytes → 87 base64url chars.
    pub fn public_key_uncompressed_b64url(&self) -> String {
        let public_key = self.inner.verifying_key().to_encoded_point(false);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public_key.as_bytes())
    }
}

/// Custom claims payload. Kept as a named type for callers that need to
/// construct or inspect VAPID claims directly; not used in `VapidSigner::sign`
/// to avoid duplicate standard-claim keys in the JWT payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VapidClaims {
    pub aud: String,
    pub exp: i64,
    pub sub: String,
}

#[derive(Serialize)]
struct JwsHeader {
    typ: &'static str,
    alg: &'static str,
}

#[derive(Serialize)]
struct SignedVapidClaims<'a> {
    iat: i64,
    exp: i64,
    sub: &'a str,
    aud: &'a str,
}

#[derive(Debug)]
pub struct VapidSigner {
    key: VapidKey,
}

impl VapidSigner {
    pub fn new(key: VapidKey) -> Self {
        Self { key }
    }

    /// Sign a VAPID JWT.
    ///
    /// `audience` — push service origin, e.g. `"https://fcm.googleapis.com"`.
    /// `subject` — contact URI, e.g. `"mailto:admin@example.org"`.
    /// `ttl_secs` — token lifetime in seconds. Must be strictly positive and
    /// at most 24 h per RFC 8292; out-of-range values are rejected before
    /// signing.
    pub fn sign(
        &self,
        audience: &str,
        subject: &str,
        ttl_secs: i64,
    ) -> Result<String, WebPushError> {
        // RFC 8292 caps VAPID JWT lifetime at 24 hours. A zero / negative TTL
        // would produce an already-expired token, and the previous `as u64`
        // cast quietly wrapped negatives into multi-century lifetimes. Reject
        // both ends explicitly so the failure mode is a clear `Vapid` error
        // rather than a JWT the push service silently refuses.
        const MAX_TTL_SECS: i64 = 24 * 3600;
        if ttl_secs <= 0 {
            return Err(WebPushError::Vapid(format!(
                "VAPID TTL must be positive (got {ttl_secs} seconds)"
            )));
        }
        if ttl_secs > MAX_TTL_SECS {
            return Err(WebPushError::Vapid(format!(
                "VAPID TTL exceeds RFC 8292 maximum of 24 hours (got {ttl_secs} seconds, max {MAX_TTL_SECS})"
            )));
        }

        let iat = chrono::Utc::now().timestamp();
        let header = JwsHeader {
            typ: "JWT",
            alg: "ES256",
        };
        let claims = SignedVapidClaims {
            iat,
            exp: iat + ttl_secs,
            sub: subject,
            aud: audience,
        };
        let header = serde_json::to_vec(&header)
            .map_err(|e| WebPushError::Vapid(format!("JWT header serialization: {e}")))?;
        let claims = serde_json::to_vec(&claims)
            .map_err(|e| WebPushError::Vapid(format!("JWT claims serialization: {e}")))?;
        let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let signing_input = format!("{}.{}", encoder.encode(header), encoder.encode(claims));
        let signature: Signature = self.key.inner.sign(signing_input.as_bytes());

        Ok(format!(
            "{}.{}",
            signing_input,
            encoder.encode(signature.to_bytes())
        ))
    }

    pub fn public_key_b64url(&self) -> String {
        self.key.public_key_uncompressed_b64url()
    }
}
