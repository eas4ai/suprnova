//! Carrier-neutral authentication sessions and host-bound remember-me state.
//!
//! This module deliberately contains no HTTP, cookie, or framework types. API
//! adapters consume [`SessionGrant`] into a bearer carrier, while web adapters
//! retain only a [`WebSessionBinding`].

use crate::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub mod grant;
pub mod jwt;
pub mod opaque;
pub mod remember;

pub use grant::{BearerSession, SessionGrant, WebSessionBinding};
pub use jwt::{JwtConfig, JwtEpochStore, JwtSessionProvider};
pub use opaque::{OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, StoredSession};
pub use remember::{
    RememberCredential, RememberFacade, RememberRow, RememberService, RememberStore,
};

/// Metadata recorded when a session is established.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionMetadata {
    /// Host-supplied user-agent, when available.
    pub user_agent: Option<String>,
    /// Host-supplied source address, when available.
    pub ip_address: Option<String>,
}

/// A verified authenticated session principal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSession {
    /// Generated session identifier.
    pub session_id: String,
    /// Application user identifier.
    pub user_id: String,
    /// Session expiry.
    pub expires_at: DateTime<Utc>,
    /// Issuance metadata.
    pub metadata: SessionMetadata,
}

/// A compact session listing entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    /// Generated session identifier.
    pub session_id: String,
    /// Application user identifier.
    pub user_id: String,
    /// Session expiry.
    pub expires_at: DateTime<Utc>,
    /// Issuance metadata.
    pub metadata: SessionMetadata,
}

/// Query-only session operations available to providers and plugins.
///
/// Deliberately, this trait has no issue or mint operation. Session issuance is
/// an internal effect performed only after the host's factor gate approves it.
#[async_trait]
pub trait SessionQueries: Send + Sync {
    /// Verify a plaintext API bearer token.
    async fn verify_bearer(&self, token: &str) -> Result<VerifiedSession>;
    /// Resolve a web binding using an authenticated host-context witness.
    async fn resolve_web_binding(
        &self,
        binding: &WebSessionBinding,
        approval: &HostSessionApproval,
    ) -> Result<VerifiedSession>;
    /// Revoke all opaque sessions for a user and return the number changed.
    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64>;
    /// Revoke exactly one presented session without touching the user's
    /// authentication epoch or other sessions (ordinary logout).
    ///
    /// Opaque providers revoke the stored row and return whether a live row
    /// changed. Self-contained JWT sessions have no per-token row: JWT
    /// providers return `Ok(false)` and the host clears its carrier; only
    /// logout-all/reset invalidates outstanding JWTs through the epoch.
    async fn revoke_session(&self, session_id: &str) -> Result<bool>;
    /// List currently active sessions for a user.
    async fn list_for_user(&self, user_id: &str) -> Result<Vec<SessionSummary>>;
}

/// Authenticated host-context witness required to resolve a web binding.
#[derive(Debug)]
pub struct HostSessionApproval(());

impl HostSessionApproval {
    #[allow(dead_code)]
    pub(crate) fn authenticated() -> Self {
        Self(())
    }
}

/// A crate-private issuance witness owned by the factor gate.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct GateApproval(());
/// The only internal session minting boundary.
///
/// No public constructor exists, and the approval witness is crate-private;
/// external providers and plugins can only use [`SessionQueries`].
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct SessionIssuer;
#[allow(dead_code)]
impl SessionIssuer {
    pub(crate) fn approval() -> GateApproval {
        GateApproval(())
    }

    pub(crate) fn approval_from_factor(_approval: crate::auth::FactorGateApproval) -> GateApproval {
        GateApproval(())
    }

    pub(crate) async fn issue_opaque<S: OpaqueSessionStore>(
        &self,
        provider: &OpaqueSessionProvider<S>,
        approval: GateApproval,
        user_id: String,
        metadata: SessionMetadata,
        now: DateTime<Utc>,
    ) -> Result<SessionGrant> {
        provider
            .issue_after_gate(approval, user_id, metadata, now)
            .await
    }

    pub(crate) async fn issue_jwt<E: JwtEpochStore>(
        &self,
        provider: &JwtSessionProvider<E>,
        approval: GateApproval,
        user_id: String,
        metadata: SessionMetadata,
        now: DateTime<Utc>,
    ) -> Result<SessionGrant> {
        provider
            .issue_after_gate(approval, user_id, metadata, now)
            .await
    }
}
#[cfg(test)]
mod tests;

pub(crate) fn invalid(field: &str, message: impl Into<String>) -> crate::Error {
    crate::Error::InvalidInput {
        field: field.to_owned(),
        message: message.into(),
    }
}

pub(crate) fn expired(resource: &str) -> crate::Error {
    crate::Error::NotFound {
        resource: resource.to_owned(),
        identifier: "expired or revoked".to_owned(),
    }
}
