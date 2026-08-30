//! Host-neutral conditional authority for temporary uploads.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::identity::UnixMillis;
use crate::limits::UploadLimits;

use super::{
    TransferGrantScope, TransitionOutcome, UploadError, UploadIdempotencyKey, UploadRevision,
    UploadState, UploadTransitionRequest,
};

/// Bounded boxed future used by host upload capabilities.
pub type UploadFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Persisted non-secret authority and lifecycle facts for one temporary upload.
#[derive(Clone, Eq, PartialEq)]
pub struct UploadRecord {
    authority: TransferGrantScope,
    state: UploadState,
    revision: UploadRevision,
    created_at: UnixMillis,
    expires_at: UnixMillis,
}

impl UploadRecord {
    /// Constructs one coherent persisted record.
    pub fn new(
        authority: TransferGrantScope,
        state: UploadState,
        revision: UploadRevision,
        created_at: UnixMillis,
        expires_at: UnixMillis,
    ) -> Result<Self, UploadError> {
        if revision.get() == 0 || expires_at <= created_at {
            return Err(UploadError::new(super::UploadErrorKind::InvalidField));
        }
        Ok(Self {
            authority,
            state,
            revision,
            created_at,
            expires_at,
        })
    }

    /// Returns the complete non-secret authority binding.
    #[must_use]
    pub const fn authority(&self) -> &TransferGrantScope {
        &self.authority
    }

    /// Returns the authoritative lifecycle state.
    #[must_use]
    pub const fn state(&self) -> UploadState {
        self.state
    }

    /// Returns the authoritative monotonic revision.
    #[must_use]
    pub const fn revision(&self) -> UploadRevision {
        self.revision
    }

    /// Returns when the record first consumed creation capacity.
    #[must_use]
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }

    /// Returns the exclusive temporary-authority expiry instant.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Produces the persisted successor after one state-machine outcome.
    pub fn with_outcome(&self, outcome: TransitionOutcome) -> Result<Self, UploadError> {
        if outcome.revision() <= self.revision || outcome.state().rank() < self.state.rank() {
            return Err(UploadError::new(super::UploadErrorKind::UploadConflict));
        }
        Ok(Self {
            authority: self.authority.clone(),
            state: outcome.state(),
            revision: outcome.revision(),
            created_at: self.created_at,
            expires_at: self.expires_at,
        })
    }
}

impl fmt::Debug for UploadRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadRecord")
            .field("authority", &"<redacted>")
            .field("state", &self.state)
            .field("revision", &self.revision)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Whether an atomic creation newly committed or replayed its exact outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConditionalUploadCreate {
    /// The absent upload record was created.
    Created,
    /// The exact creation retry observed its previously committed record.
    ExistingOutcome,
}

/// Atomic ledger creation request carrying the service's validated finite policy.
#[derive(Clone, Eq, PartialEq)]
pub struct UploadCreateCommand {
    record: UploadRecord,
    idempotency_key: UploadIdempotencyKey,
    admitted_at: UnixMillis,
    limits: UploadLimits,
}

impl UploadCreateCommand {
    /// Groups a validated record and retry identity with finite atomic bounds.
    #[must_use]
    pub const fn new(
        record: UploadRecord,
        idempotency_key: UploadIdempotencyKey,
        admitted_at: UnixMillis,
        limits: UploadLimits,
    ) -> Self {
        Self {
            record,
            idempotency_key,
            admitted_at,
            limits,
        }
    }

    /// Returns the proposed initial record.
    #[must_use]
    pub const fn record(&self) -> &UploadRecord {
        &self.record
    }

    /// Returns the retry identity bound to this create operation.
    #[must_use]
    pub const fn idempotency_key(&self) -> &UploadIdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the authoritative admission instant.
    #[must_use]
    pub const fn admitted_at(&self) -> UnixMillis {
        self.admitted_at
    }

    /// Returns the validated policy that must be enforced atomically.
    #[must_use]
    pub const fn limits(&self) -> UploadLimits {
        self.limits
    }
}

impl fmt::Debug for UploadCreateCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadCreateCommand:redacted>")
    }
}

/// Safe result of one atomic creation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadLedgerCreateOutcome {
    disposition: ConditionalUploadCreate,
    record: UploadRecord,
}

impl UploadLedgerCreateOutcome {
    /// Constructs a ledger outcome from the committed or replayed record.
    #[must_use]
    pub const fn new(disposition: ConditionalUploadCreate, record: UploadRecord) -> Self {
        Self {
            disposition,
            record,
        }
    }

    /// Returns whether this call created or replayed the record.
    #[must_use]
    pub const fn disposition(&self) -> ConditionalUploadCreate {
        self.disposition
    }

    /// Returns the authoritative persisted record.
    #[must_use]
    pub const fn record(&self) -> &UploadRecord {
        &self.record
    }
}

/// One scope- and expiry-bound conditional lifecycle mutation.
#[derive(Clone, Eq, PartialEq)]
pub struct ConditionalTransition {
    authority: TransferGrantScope,
    transition: UploadTransitionRequest,
    admitted_at: UnixMillis,
}

impl ConditionalTransition {
    /// Groups the reverified authority with one exact state-machine request.
    #[must_use]
    pub const fn new(
        authority: TransferGrantScope,
        transition: UploadTransitionRequest,
        admitted_at: UnixMillis,
    ) -> Self {
        Self {
            authority,
            transition,
            admitted_at,
        }
    }

    /// Returns the authority that must match the persisted record.
    #[must_use]
    pub const fn authority(&self) -> &TransferGrantScope {
        &self.authority
    }

    /// Returns the conditional state-machine request.
    #[must_use]
    pub const fn transition(&self) -> &UploadTransitionRequest {
        &self.transition
    }

    /// Returns the authoritative admission instant.
    #[must_use]
    pub const fn admitted_at(&self) -> UnixMillis {
        self.admitted_at
    }
}

impl fmt::Debug for ConditionalTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ConditionalTransition:redacted>")
    }
}

/// Host-owned atomic temporary-upload authority.
pub trait UploadLedger: Send + Sync {
    /// Atomically creates an absent record or replays its exact creation outcome.
    fn create<'a>(
        &'a self,
        request: UploadCreateCommand,
    ) -> UploadFuture<'a, Result<UploadLedgerCreateOutcome, UploadError>>;

    /// Loads one current non-secret authority record.
    fn load<'a>(
        &'a self,
        handle: &'a super::UploadHandle,
    ) -> UploadFuture<'a, Result<Option<UploadRecord>, UploadError>>;

    /// Atomically applies one authorized conditional transition.
    fn transition<'a>(
        &'a self,
        request: ConditionalTransition,
    ) -> UploadFuture<'a, Result<TransitionOutcome, UploadError>>;
}
