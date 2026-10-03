//! Self-contained signed JWT sessions with immediate epoch revocation.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use subtle::ConstantTimeEq;

use super::grant::{SessionGrant, WebSessionBinding};
use super::{
    GateApproval, SessionMetadata, SessionQueries, SessionSummary, VerifiedSession, expired,
    invalid,
};
use crate::{Error, Result};

type HmacSha256 = Hmac<sha2::Sha256>;

/// JWT issuer and validation policy.
#[derive(Clone, Debug)]
pub struct JwtConfig {
    /// Expected issuer claim.
    pub issuer: String,
    /// HMAC signing key. Keep this in application secret storage.
    pub signing_key: SecretString,
    /// Lifetime of newly issued JWT sessions.
    pub lifetime: Duration,
}

impl JwtConfig {
    /// Construct the default immediate-revocation policy.
    pub fn new(issuer: impl Into<String>, signing_key: SecretString, lifetime: Duration) -> Self {
        Self {
            issuer: issuer.into(),
            signing_key,
            lifetime,
        }
    }
}

/// User-epoch lookup used by JWT issuance and verification.
#[async_trait]
pub trait JwtEpochStore: Send + Sync {
    /// Read the current persisted authentication epoch for a user.
    async fn current_auth_epoch(&self, user_id: &str) -> Result<u64>;
    /// Atomically increment the user's epoch for logout-all/reset.
    async fn bump_auth_epoch(&self, user_id: &str) -> Result<u64>;
}

/// Query-only JWT session provider.
pub struct JwtSessionProvider<E> {
    config: JwtConfig,
    epochs: Arc<E>,
}

impl<E> Clone for JwtSessionProvider<E> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            epochs: Arc::clone(&self.epochs),
        }
    }
}

impl<E: JwtEpochStore> JwtSessionProvider<E> {
    /// Bind a provider to the application's user epoch capability.
    pub fn new(config: JwtConfig, epochs: Arc<E>) -> Result<Self> {
        if config.issuer.is_empty() {
            return Err(invalid("issuer", "must not be empty"));
        }
        if config.lifetime <= Duration::zero() {
            return Err(invalid("lifetime", "must be positive"));
        }
        if config.signing_key.expose_secret().is_empty() {
            return Err(invalid("signing_key", "must not be empty"));
        }
        Ok(Self { config, epochs })
    }

    /// Internal issuance effect. JWTs intentionally create no per-token row.
    #[allow(dead_code)]
    pub(crate) async fn issue_after_gate(
        &self,
        approval: GateApproval,
        user_id: String,
        metadata: SessionMetadata,
        now: DateTime<Utc>,
    ) -> Result<SessionGrant> {
        if user_id.is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        let approval_epoch = approval.auth_epoch();
        let current_epoch = self.epochs.current_auth_epoch(&user_id).await?;
        if approval_epoch != current_epoch {
            return Err(Error::InvalidInput {
                field: "auth_epoch".to_owned(),
                message: "stale gate approval".to_owned(),
            });
        }
        let session_id = random_id();
        let expiry = now + self.config.lifetime;
        let claims = Claims {
            sub: user_id.clone(),
            iss: self.config.issuer.clone(),
            exp: expiry.timestamp(),
            sid: session_id.clone(),
            auth_epoch: approval_epoch,
            metadata: metadata.clone(),
        };
        let token = sign(&claims, self.config.signing_key.expose_secret())?;
        SessionGrant::new_at(
            session_id,
            user_id,
            SecretString::from(token),
            expiry,
            metadata,
            now,
        )
    }

    fn verify_claims(&self, token: &str) -> Result<Claims> {
        let mut parts = token.split('.');
        let header = parts
            .next()
            .ok_or_else(|| invalid("token", "malformed JWT"))?;
        let payload = parts
            .next()
            .ok_or_else(|| invalid("token", "malformed JWT"))?;
        let signature = parts
            .next()
            .ok_or_else(|| invalid("token", "malformed JWT"))?;
        if parts.next().is_some() || header != HEADER {
            return Err(invalid("token", "malformed JWT"));
        }
        let message = format!("{header}.{payload}");
        let expected = mac(message.as_bytes(), self.config.signing_key.expose_secret())?;
        let presented = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| invalid("token", "malformed signature"))?;
        if expected.as_slice().ct_eq(&presented).unwrap_u8() != 1 {
            return Err(invalid("token", "invalid signature"));
        }
        serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(payload)
                .map_err(|_| invalid("token", "malformed claims"))?,
        )
        .map_err(|_| invalid("token", "malformed claims"))
    }
}

impl<E: JwtEpochStore> super::sealed::Sealed for JwtSessionProvider<E> {}

#[async_trait]
impl<E: JwtEpochStore> SessionQueries for JwtSessionProvider<E> {
    async fn verify_bearer(&self, token: &str) -> Result<VerifiedSession> {
        if token.is_empty() {
            return Err(invalid("token", "must not be empty"));
        }
        let claims = self.verify_claims(token)?;
        if claims.iss != self.config.issuer {
            return Err(invalid("issuer", "unexpected issuer"));
        }
        let now = Utc::now().timestamp();
        if claims.exp <= now {
            return Err(expired("jwt session"));
        }
        if claims.sub.is_empty() || claims.sid.is_empty() {
            return Err(invalid("token", "missing subject or session id"));
        }
        let current = self.epochs.current_auth_epoch(&claims.sub).await?;
        if claims.auth_epoch != current {
            return Err(Error::InvalidInput {
                field: "auth_epoch".to_owned(),
                message: "stale session".to_owned(),
            });
        }
        Ok(VerifiedSession::new(
            super::SessionCarrier::Jwt,
            claims.sid,
            claims.sub,
            claims.auth_epoch,
            DateTime::from_timestamp(claims.exp, 0)
                .ok_or_else(|| invalid("exp", "invalid timestamp"))?,
            claims.metadata,
        ))
    }

    async fn resolve_web_binding(
        &self,
        _binding: &WebSessionBinding,
        _approval: &super::HostSessionApproval,
    ) -> Result<VerifiedSession> {
        Err(invalid(
            "binding",
            "JWT sessions do not expose database web bindings",
        ))
    }

    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        self.epochs.bump_auth_epoch(user_id).await.map(|_| 1)
    }

    async fn revoke_session(&self, _session_id: &str) -> Result<bool> {
        // A self-contained JWT has no per-token row to revoke. The host
        // clears its carrier; global invalidation goes through the epoch.
        Ok(false)
    }

    async fn list_for_user(&self, _user_id: &str) -> Result<Vec<SessionSummary>> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    iss: String,
    exp: i64,
    sid: String,
    auth_epoch: u64,
    metadata: SessionMetadata,
}

const HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";

#[allow(dead_code)]
fn sign(claims: &Claims, key: &str) -> Result<String> {
    let payload = serde_json::to_vec(claims).map_err(|error| Error::Internal {
        message: error.to_string(),
    })?;
    let encoded = URL_SAFE_NO_PAD.encode(payload);
    let message = format!("{HEADER}.{encoded}");
    let signature = mac(message.as_bytes(), key)?;
    Ok(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

#[allow(dead_code)]
fn mac(message: &[u8], key: &str) -> Result<[u8; 32]> {
    let mut signer = HmacSha256::new_from_slice(key.as_bytes())
        .map_err(|_| invalid("signing_key", "invalid key"))?;
    signer.update(message);
    Ok(signer.finalize().into_bytes().into())
}

#[allow(dead_code)]
fn random_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_require_auth_epoch() {
        let claims = serde_json::json!({
            "sub": "u1",
            "iss": "issuer-a",
            "exp": 4_102_444_800_i64,
            "sid": "session-1",
            "metadata": {
                "user_agent": null,
                "ip_address": null
            }
        });

        assert!(serde_json::from_value::<Claims>(claims).is_err());
    }
}
