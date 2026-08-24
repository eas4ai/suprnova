//! Carrier-neutral authentication sessions and host-bound remember-me state.
//!
//! This module deliberately contains no HTTP, cookie, or framework types. API
//! adapters consume [`SessionGrant`](crate::sessions::SessionGrant) into a bearer carrier, while web adapters
//! retain only a [`WebSessionBinding`](crate::sessions::WebSessionBinding).

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
    RememberAnomaly, RememberAnomalyHook, RememberAnomalyKind, RememberCredential, RememberFacade,
    RememberRow, RememberService, RememberSignInOutcome, RememberSignInService, RememberStore,
    RememberTokenService,
};

/// Metadata recorded when a session is established.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionMetadata {
    /// Host-supplied user-agent, when available.
    pub user_agent: Option<String>,
    /// Host-supplied source address, when available.
    pub ip_address: Option<String>,
}

/// Carrier whose successful verification produced a session principal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionCarrier {
    /// Database-backed opaque bearer or web session.
    Opaque,
    /// Self-contained signed bearer session.
    Jwt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VerifiedSessionWitness;

/// A verified authenticated session principal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSession {
    /// Verified session carrier.
    carrier: SessionCarrier,
    /// Generated session identifier.
    session_id: String,
    /// Application user identifier.
    user_id: String,
    /// Authentication epoch observed when the session was issued.
    auth_epoch: u64,
    /// Session expiry.
    expires_at: DateTime<Utc>,
    /// Issuance metadata.
    metadata: SessionMetadata,
    _verified: VerifiedSessionWitness,
}
impl VerifiedSession {
    pub(crate) fn new(
        carrier: SessionCarrier,
        session_id: String,
        user_id: String,
        auth_epoch: u64,
        expires_at: DateTime<Utc>,
        metadata: SessionMetadata,
    ) -> Self {
        Self {
            carrier,
            session_id,
            user_id,
            auth_epoch,
            expires_at,
            metadata,
            _verified: VerifiedSessionWitness,
        }
    }

    /// Return the verified session carrier.
    #[must_use]
    pub fn carrier(&self) -> SessionCarrier {
        self.carrier
    }

    /// Return the verified session identifier.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Return the verified application user identifier.
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Return the authentication epoch bound to this session.
    #[must_use]
    pub fn auth_epoch(&self) -> u64 {
        self.auth_epoch
    }

    /// Return the verified session expiry.
    #[must_use]
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Return the verified issuance metadata.
    #[must_use]
    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }
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

mod sealed {
    pub trait Sealed {}
}

/// Query-only session operations available to providers and plugins.
///
/// Deliberately, this trait has no issue or mint operation. Session issuance is
/// an internal effect performed only after the host's factor gate approves it.
#[async_trait]
pub trait SessionQueries: sealed::Sealed + Send + Sync {
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
    /// Approve resolving a digest-only binding held in trusted host session state.
    pub fn authenticated() -> Self {
        Self(())
    }
}

/// A crate-private issuance witness owned by the factor gate.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct GateApproval {
    auth_epoch: u64,
}

impl GateApproval {
    const fn auth_epoch(&self) -> u64 {
        self.auth_epoch
    }
}
/// The only internal session minting boundary.
///
/// No public constructor exists, and the approval witness is crate-private;
/// external providers and plugins can only use [`SessionQueries`].
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct SessionIssuer;
#[allow(dead_code)]
impl SessionIssuer {
    pub(crate) fn approval(auth_epoch: u64) -> GateApproval {
        GateApproval { auth_epoch }
    }

    pub(crate) fn approval_from_factor(approval: crate::auth::FactorGateApproval) -> GateApproval {
        GateApproval {
            auth_epoch: approval.context.auth_epoch,
        }
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
