//! Provider-neutral instance-ledger inputs, outcomes, errors, and trait.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

use crate::identity::{
    ContentDigest, IdempotencyKey, InstanceId, Revision, ScopeFingerprint, UnixMillis,
};

/// Hard maximum for a claim lease: five minutes.
const MAX_CLAIM_LEASE_MS: u64 = 300_000;
/// Hard maximum for an instance lifetime: seven days.
const MAX_INSTANCE_LIFETIME_MS: u64 = 604_800_000;
/// Hard maximum retained accepted outcomes per instance.
const MAX_ACCEPTED_OUTCOMES: usize = 64;
/// Hard maximum instances held by one embedded provider.
const MAX_INSTANCES: usize = 1_000_000;

/// Closed reason a ledger operation or configuration failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerErrorKind {
    /// A configured duration or count was zero or above its hard ceiling.
    InvalidConfiguration,
    /// Promotion expiry was elapsed, overflowed, or exceeded configured lifetime.
    InvalidExpiry,
    /// The proposed server instance identity already belongs to another promotion.
    InstanceConflict,
    /// The embedded provider reached its configured live-instance capacity.
    CapacityExceeded,
    /// The clock provider could not supply a usable timestamp.
    ClockUnavailable,
    /// Revision or internal claim identity could not advance monotonically.
    CounterExhausted,
    /// A commit or abandon token did not identify the current pending claim.
    ClaimMismatch,
    /// The claim lease elapsed and authority was terminally consumed.
    ClaimExpired,
    /// The instance expired before its pending operation completed.
    InstanceExpired,
    /// Provider synchronization state became unavailable.
    ProviderUnavailable,
}

impl LedgerErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_ledger_configuration",
            Self::InvalidExpiry => "invalid_instance_expiry",
            Self::InstanceConflict => "instance_conflict",
            Self::CapacityExceeded => "ledger_capacity_exceeded",
            Self::ClockUnavailable => "clock_unavailable",
            Self::CounterExhausted => "counter_exhausted",
            Self::ClaimMismatch => "claim_mismatch",
            Self::ClaimExpired => "claim_expired",
            Self::InstanceExpired => "instance_expired",
            Self::ProviderUnavailable => "provider_unavailable",
        }
    }
}

/// Redacted instance-ledger error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LedgerError {
    kind: LedgerErrorKind,
}

impl LedgerError {
    pub(crate) const fn new(kind: LedgerErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure reason.
    #[must_use]
    pub const fn kind(self) -> LedgerErrorKind {
        self.kind
    }
}

impl fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for LedgerError {}

/// Validated memory and validity bounds for one ledger provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedgerLimits {
    claim_lease_ms: u64,
    max_instance_lifetime_ms: u64,
    max_accepted_outcomes: usize,
    max_instances: usize,
}

impl LedgerLimits {
    /// Creates bounded non-zero provider limits.
    pub fn new(
        claim_lease_ms: u64,
        max_instance_lifetime_ms: u64,
        max_accepted_outcomes: usize,
        max_instances: usize,
    ) -> Result<Self, LedgerError> {
        let valid = claim_lease_ms > 0
            && claim_lease_ms <= MAX_CLAIM_LEASE_MS
            && max_instance_lifetime_ms > 0
            && max_instance_lifetime_ms <= MAX_INSTANCE_LIFETIME_MS
            && max_accepted_outcomes > 0
            && max_accepted_outcomes <= MAX_ACCEPTED_OUTCOMES
            && max_instances > 0
            && max_instances <= MAX_INSTANCES;
        if !valid {
            return Err(LedgerError::new(LedgerErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            claim_lease_ms,
            max_instance_lifetime_ms,
            max_accepted_outcomes,
            max_instances,
        })
    }

    pub(crate) const fn claim_lease_ms(self) -> u64 {
        self.claim_lease_ms
    }

    pub(crate) const fn max_instance_lifetime_ms(self) -> u64 {
        self.max_instance_lifetime_ms
    }

    pub(crate) const fn max_accepted_outcomes(self) -> usize {
        self.max_accepted_outcomes
    }

    pub(crate) const fn max_instances(self) -> usize {
        self.max_instances
    }
}

/// Metadata-only request to create one scoped instance authority record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromotionRecord {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance_id: InstanceId,
    pub(crate) idempotency_key: IdempotencyKey,
    pub(crate) request_digest: ContentDigest,
    pub(crate) initial_revision: Revision,
    pub(crate) expires_at: UnixMillis,
}

/// Create-only metadata for one identity-bound initial mount.
///
/// Unlike [`PromotionRecord`], this request has no browser nonce,
/// idempotency key, or retry-recovery semantics. A repeated instance identity
/// is always an [`LedgerErrorKind::InstanceConflict`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountInstanceRecord {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance_id: InstanceId,
    pub(crate) component_contract: ContentDigest,
    pub(crate) initial_revision: Revision,
    pub(crate) expires_at: UnixMillis,
}

impl MountInstanceRecord {
    /// Creates a private-mount authority request from trusted fixed-size metadata.
    #[must_use]
    pub const fn new(
        scope: ScopeFingerprint,
        instance_id: InstanceId,
        component_contract: ContentDigest,
        initial_revision: Revision,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            scope,
            instance_id,
            component_contract,
            initial_revision,
            expires_at,
        }
    }

    /// Returns the trusted scope that owns the instance.
    #[must_use]
    pub const fn scope(&self) -> &ScopeFingerprint {
        &self.scope
    }

    /// Returns the proposed server-generated identity.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the generated component contract bound to the mount.
    #[must_use]
    pub const fn component_contract(&self) -> &ContentDigest {
        &self.component_contract
    }
}

impl PromotionRecord {
    /// Creates a proposed instance record from trusted fixed-size metadata.
    #[must_use]
    pub const fn new(
        scope: ScopeFingerprint,
        instance_id: InstanceId,
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
        initial_revision: Revision,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            scope,
            instance_id,
            idempotency_key,
            request_digest,
            initial_revision,
            expires_at,
        }
    }

    /// Replaces the proposed server identity while preserving retry identity.
    #[must_use]
    pub fn with_instance_id(mut self, instance_id: InstanceId) -> Self {
        self.instance_id = instance_id;
        self
    }

    /// Replaces the exact-request digest for conflict testing or construction.
    #[must_use]
    pub fn with_request_digest(mut self, request_digest: ContentDigest) -> Self {
        self.request_digest = request_digest;
        self
    }
}

/// Scoped authority returned after new or recovered promotion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceAuthority {
    instance_id: InstanceId,
    revision: Revision,
    expires_at: UnixMillis,
}

impl InstanceAuthority {
    pub(crate) const fn new(
        instance_id: InstanceId,
        revision: Revision,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            instance_id,
            revision,
            expires_at,
        }
    }

    /// Returns the server-assigned instance identity.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the current ledger revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the instance expiry deadline.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }
}

/// Result of atomically creating or recovering promotion authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromotionOutcome {
    /// A new independent scoped instance was created.
    Created(InstanceAuthority),
    /// An exact idempotent retry recovered the previously created instance.
    Existing(InstanceAuthority),
    /// The promotion identity was reused with a different request digest.
    IdempotencyConflict,
}

/// Fixed metadata required to claim one base revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRequest {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance_id: InstanceId,
    pub(crate) base_revision: Revision,
    pub(crate) idempotency_key: IdempotencyKey,
    pub(crate) request_digest: ContentDigest,
}

impl ClaimRequest {
    /// Creates an expected-revision claim request.
    #[must_use]
    pub const fn new(
        scope: ScopeFingerprint,
        instance_id: InstanceId,
        base_revision: Revision,
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
    ) -> Self {
        Self {
            scope,
            instance_id,
            base_revision,
            idempotency_key,
            request_digest,
        }
    }
}

/// Opaque single-use proof that one successor revision was claimed.
pub struct ClaimToken {
    pub(crate) provider_identity: Arc<()>,
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance_id: InstanceId,
    pub(crate) claim_id: u64,
}

impl fmt::Debug for ClaimToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ClaimToken:redacted>")
    }
}

/// Granted claim plus its already-consumed successor revision.
pub struct ClaimGrant {
    token: ClaimToken,
    successor_revision: Revision,
}

impl ClaimGrant {
    pub(crate) const fn new(token: ClaimToken, successor_revision: Revision) -> Self {
        Self {
            token,
            successor_revision,
        }
    }

    /// Returns the successor revision claimed before action execution.
    #[must_use]
    pub const fn successor_revision(&self) -> Revision {
        self.successor_revision
    }

    /// Consumes the grant and returns its single-use commit/abandon token.
    #[must_use]
    pub fn into_token(self) -> ClaimToken {
        self.token
    }
}

impl fmt::Debug for ClaimGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimGrant")
            .field("token", &"<redacted>")
            .field("successor_revision", &self.successor_revision)
            .finish()
    }
}

/// Bounded category retained for an accepted Live result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcceptedOutcomeKind {
    /// Server render and snapshot production succeeded.
    Rendered,
    /// Validation state was accepted without domain success.
    Validation,
    /// The action explicitly accepted a no-render outcome.
    NoRender,
    /// A redirect outcome superseded rendering.
    Redirect,
    /// A classified accepted recovery outcome was produced.
    Recovery,
}

/// Fixed-size digest and category of one committed accepted outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedOutcome {
    kind: AcceptedOutcomeKind,
    digest: ContentDigest,
}

impl AcceptedOutcome {
    /// Creates bounded outcome metadata without storing response or component bytes.
    #[must_use]
    pub const fn new(kind: AcceptedOutcomeKind, digest: ContentDigest) -> Self {
        Self { kind, digest }
    }

    /// Returns the accepted outcome category.
    #[must_use]
    pub const fn kind(&self) -> AcceptedOutcomeKind {
        self.kind
    }
}

/// Retained metadata that lets an exact duplicate observe its committed outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedOutcomeMetadata {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance_id: InstanceId,
    pub(crate) base_revision: Revision,
    pub(crate) successor_revision: Revision,
    pub(crate) idempotency_key: IdempotencyKey,
    pub(crate) request_digest: ContentDigest,
    pub(crate) outcome: AcceptedOutcome,
}

impl AcceptedOutcomeMetadata {
    /// Returns the revision on which the accepted request was based.
    #[must_use]
    pub const fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// Returns the committed successor revision.
    #[must_use]
    pub const fn successor_revision(&self) -> Revision {
        self.successor_revision
    }

    /// Returns the fixed-size accepted outcome metadata.
    #[must_use]
    pub const fn outcome(&self) -> &AcceptedOutcome {
        &self.outcome
    }
}

/// Fresh-render reason returned instead of reconstructing authority from a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshReason {
    /// No authoritative record exists for the supplied scoped instance.
    Missing,
    /// The instance lifetime elapsed and its record was removed.
    InstanceExpired,
    /// A claimed operation failed or was explicitly abandoned.
    Consumed,
    /// A pending claim lease elapsed and was terminally consumed.
    ClaimExpired,
    /// The monotonic revision space was exhausted.
    RevisionExhausted,
}

/// Result of atomically evaluating one expected-revision request.
#[derive(Debug)]
pub enum ClaimOutcome {
    /// This request exclusively claimed the successor revision.
    Granted(ClaimGrant),
    /// The exact request is already executing under the stated successor.
    InProgress {
        /// Revision already claimed by the matching request.
        successor_revision: Revision,
    },
    /// The exact request already committed and retained bounded metadata.
    Accepted(AcceptedOutcomeMetadata),
    /// The request's base is no longer the current authority.
    Stale {
        /// Current accepted or consumed revision.
        current_revision: Revision,
    },
    /// A retry identity was reused for different request metadata.
    IdempotencyConflict,
    /// Browser-carried state cannot recover ledger authority.
    RefreshRequired(RefreshReason),
}

/// Current coarse lifecycle phase exposed by metadata-only inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerPhase {
    /// The current revision may be claimed.
    Ready,
    /// One successor is pending commit or abandonment.
    Pending,
    /// Authority was advanced but can no longer accept outcomes.
    Consumed,
}

/// Bounded provider inspection with no component state or response bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedgerInspection {
    pub(crate) current_revision: Revision,
    pub(crate) accepted_outcome_count: usize,
    pub(crate) phase: LedgerPhase,
}

impl LedgerInspection {
    /// Returns the current accepted or consumed revision.
    #[must_use]
    pub const fn current_revision(self) -> Revision {
        self.current_revision
    }

    /// Returns the bounded number of retained outcome metadata entries.
    #[must_use]
    pub const fn accepted_outcome_count(self) -> usize {
        self.accepted_outcome_count
    }

    /// Returns the current coarse phase.
    #[must_use]
    pub const fn phase(self) -> LedgerPhase {
        self.phase
    }
}

/// Atomic provider contract for scoped instance revision authority.
///
/// Implementations store only fixed concurrency metadata. Component state,
/// rendered HTML, action arguments, and response bodies are deliberately absent
/// from every input and output type.
#[async_trait]
pub trait LiveInstanceLedger: Send + Sync {
    /// Atomically creates one private mount without idempotent recovery.
    async fn mount_instance(
        &self,
        record: MountInstanceRecord,
    ) -> Result<InstanceAuthority, LedgerError>;

    /// Atomically creates a scoped instance or recovers an exact promotion retry.
    async fn promote(&self, request: PromotionRecord) -> Result<PromotionOutcome, LedgerError>;

    /// Atomically claims a monotonic successor or returns a classified rejection.
    async fn claim(&self, request: ClaimRequest) -> Result<ClaimOutcome, LedgerError>;

    /// Commits fixed outcome metadata for exactly the matching pending token.
    ///
    /// A provider accepts at most one committed outcome for an instance base
    /// revision. It does not promise exactly-once Rust method invocation or
    /// exactly-once effects outside the coordinated host transaction.
    async fn commit(&self, claim: &ClaimToken, outcome: AcceptedOutcome)
    -> Result<(), LedgerError>;

    /// Terminally consumes authority for exactly the matching pending token.
    async fn abandon(&self, claim: &ClaimToken) -> Result<(), LedgerError>;

    /// Synchronously releases an uncommitted claim when its owner is canceled or dropped.
    ///
    /// Implementations must not block on remote I/O. Distributed providers use their owned
    /// coordinator to enqueue the cleanup; in-process providers may restore the base revision
    /// immediately so an exact request can retry. This is distinct from explicit terminal
    /// [`LiveInstanceLedger::abandon`].
    fn abandon_on_drop(&self, claim: ClaimToken);

    /// Synchronously fences a claim whose coordinated host effects may have committed.
    ///
    /// Implementations must never restore the base revision to retryable authority. If the
    /// matching outcome already committed, this operation is an idempotent no-op. Otherwise it
    /// terminally consumes the claim, or hands it to provider-owned finalization that cannot
    /// recreate base-revision authority after its lease expires.
    fn fence_on_drop(&self, claim: ClaimToken);
}
