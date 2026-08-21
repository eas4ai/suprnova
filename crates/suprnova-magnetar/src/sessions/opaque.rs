//! Database-backed opaque sessions with hashed-at-rest credentials.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use secrecy::SecretString;
use std::sync::Arc;
use subtle::ConstantTimeEq;

use super::grant::{SessionGrant, WebSessionBinding, digest_token};
use super::{
    GateApproval, SessionMetadata, SessionQueries, SessionSummary, VerifiedSession, expired,
    invalid,
};
use crate::Result;

/// A persisted opaque session record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSession {
    /// Generated session identifier.
    pub session_id: String,
    /// Owning application user identifier.
    pub user_id: String,
    /// Hash of the bearer token; plaintext is never persisted.
    pub token_hash: [u8; 32],
    /// Digest copied into the host web binding.
    pub token_digest: [u8; 32],
    /// Session expiry.
    pub expires_at: DateTime<Utc>,
    /// Revocation timestamp, if revoked.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Host-supplied issuance metadata.
    pub metadata: SessionMetadata,
}

/// Persistence operations required by the opaque provider.
#[async_trait]
pub trait OpaqueSessionStore: Send + Sync {
    /// Persist a newly issued session record.
    async fn insert_session(&self, session: StoredSession) -> Result<()>;
    /// Find an active session by its persisted bearer digest.
    async fn find_by_token_hash(&self, token_hash: [u8; 32]) -> Result<Option<StoredSession>>;
    /// Find an active session by its web binding fields.
    async fn find_by_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> Result<Option<StoredSession>>;
    /// Revoke every active session for a user and return affected rows.
    async fn revoke_all_sessions(&self, user_id: &str, at: DateTime<Utc>) -> Result<u64>;
    /// Revoke exactly one active session by identifier. Returns whether a
    /// live row was revoked by this call.
    async fn revoke_session(&self, session_id: &str, at: DateTime<Utc>) -> Result<bool>;
    /// List non-expired, non-revoked sessions for a user.
    async fn list_active_sessions(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<StoredSession>>;
}

/// Configuration for opaque session issuance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpaqueConfig {
    /// Lifetime of newly issued sessions.
    pub lifetime: Duration,
}

impl Default for OpaqueConfig {
    fn default() -> Self {
        Self {
            lifetime: Duration::days(30),
        }
    }
}

/// Query-only opaque session provider.
pub struct OpaqueSessionProvider<S> {
    store: Arc<S>,
    config: OpaqueConfig,
}

impl<S> Clone for OpaqueSessionProvider<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            config: self.config,
        }
    }
}

impl<S: OpaqueSessionStore> OpaqueSessionProvider<S> {
    /// Bind a provider to application-owned opaque-session storage.
    pub fn new(store: Arc<S>, config: OpaqueConfig) -> Self {
        Self { store, config }
    }

    /// Internal issuance effect. The gate supplies the only approval witness.
    #[allow(dead_code)]
    pub(crate) async fn issue_after_gate(
        &self,
        _approval: GateApproval,
        user_id: String,
        metadata: SessionMetadata,
        now: DateTime<Utc>,
    ) -> Result<SessionGrant> {
        if user_id.is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        if self.config.lifetime <= Duration::zero() {
            return Err(invalid("lifetime", "must be positive"));
        }
        let session_id = new_id();
        let token = new_token();
        let expiry = now + self.config.lifetime;
        let grant = SessionGrant::new_at(
            session_id.clone(),
            user_id.clone(),
            token,
            expiry,
            metadata,
            now,
        )?;
        let digest = grant.token_digest();
        self.store
            .insert_session(StoredSession {
                session_id,
                user_id,
                token_hash: digest,
                token_digest: digest,
                expires_at: expiry,
                revoked_at: None,
                metadata: grant.metadata().clone(),
            })
            .await?;
        Ok(grant)
    }
}

#[async_trait]
impl<S: OpaqueSessionStore> SessionQueries for OpaqueSessionProvider<S> {
    async fn verify_bearer(&self, token: &str) -> Result<VerifiedSession> {
        if token.is_empty() {
            return Err(invalid("token", "must not be empty"));
        }
        let digest = digest_token(token);
        let Some(session) = self.store.find_by_token_hash(digest).await? else {
            return Err(expired("session"));
        };
        if session.token_hash.ct_eq(&digest).unwrap_u8() != 1 {
            return Err(expired("session"));
        }
        active(session, Utc::now())
    }
    async fn resolve_web_binding(
        &self,
        binding: &WebSessionBinding,
        _approval: &super::HostSessionApproval,
    ) -> Result<VerifiedSession> {
        let Some(session) = self.store.find_by_web_binding(binding).await? else {
            return Err(expired("session"));
        };
        if session.session_id != binding.session_id
            || session
                .token_digest
                .ct_eq(&binding.token_digest)
                .unwrap_u8()
                != 1
        {
            return Err(expired("session"));
        }
        active(session, Utc::now())
    }

    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        self.store.revoke_all_sessions(user_id, Utc::now()).await
    }

    async fn revoke_session(&self, session_id: &str) -> Result<bool> {
        if session_id.is_empty() {
            return Err(invalid("session_id", "must not be empty"));
        }
        self.store.revoke_session(session_id, Utc::now()).await
    }

    async fn list_for_user(&self, user_id: &str) -> Result<Vec<SessionSummary>> {
        Ok(self
            .store
            .list_active_sessions(user_id, Utc::now())
            .await?
            .into_iter()
            .filter_map(|session| active(session, Utc::now()).ok())
            .map(|session| SessionSummary {
                session_id: session.session_id,
                user_id: session.user_id,
                expires_at: session.expires_at,
                metadata: session.metadata,
            })
            .collect())
    }
}

fn active(session: StoredSession, now: DateTime<Utc>) -> Result<VerifiedSession> {
    if session.revoked_at.is_some() || session.expires_at <= now {
        return Err(expired("session"));
    }
    Ok(VerifiedSession {
        session_id: session.session_id,
        user_id: session.user_id,
        expires_at: session.expires_at,
        metadata: session.metadata,
    })
}

#[allow(dead_code)]
fn new_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(dead_code)]
fn new_token() -> SecretString {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    SecretString::from(
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    )
}
