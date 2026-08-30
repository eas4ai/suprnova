//! Fenced, bounded cleanup reconciliation for temporary uploads.

use std::fmt;
use std::future::poll_fn;
use std::mem::size_of;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use crate::clock::Clock;
use crate::identity::UnixMillis;
use crate::limits::UploadLimits;
use crate::resource::{CancellationFlag, PermitPool, ResourceBounds, ResourceOwner};

use super::telemetry::{
    CleanupMetricSink, CleanupMetrics, CleanupOutcome, RetryBucket, UploadAgeBucket,
    UploadVolumeBucket, record_metrics,
};
use super::{
    UploadError, UploadErrorKind, UploadFuture, UploadHandle, UploadProvider, UploadRecord,
    UploadRevision, UploadState, UploadValidationStore,
};

const MAX_CLEANUP_LEASE_ID_BYTES: usize = 96;
const MAX_CLEANUP_DURATION_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
const CLAIM_ACCOUNTING_OVERHEAD: usize = size_of::<CleanupClaim>() + MAX_CLEANUP_LEASE_ID_BYTES;

/// Trusted, bounded identity for one scheduler cleanup run.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct CleanupLeaseId(String);

impl CleanupLeaseId {
    /// Parses the bounded ASCII identity supplied by a trusted scheduler.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_CLEANUP_LEASE_ID_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            });
        if !valid {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self(value.to_owned()))
    }

    /// Exposes the bounded value to trusted ledger persistence only.
    ///
    /// Cleanup telemetry and diagnostics must retain the redacted `Debug` and
    /// `Display` representations instead of using this value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CleanupLeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<CleanupLeaseId>")
    }
}

/// Finite exponential retry policy with a capped delay and orphan threshold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundedBackoff {
    initial_ms: u64,
    maximum_ms: u64,
    orphan_after: u32,
}

impl BoundedBackoff {
    /// Validates nonzero durations and a nonzero orphan threshold.
    pub fn new(
        initial: Duration,
        maximum: Duration,
        orphan_after: u32,
    ) -> Result<Self, UploadError> {
        let initial_ms = duration_ms(initial)?;
        let maximum_ms = duration_ms(maximum)?;
        if initial_ms == 0
            || maximum_ms < initial_ms
            || maximum_ms > MAX_CLEANUP_DURATION_MS
            || orphan_after == 0
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            initial_ms,
            maximum_ms,
            orphan_after,
        })
    }

    /// Returns the initial retry delay.
    #[must_use]
    pub const fn initial(self) -> Duration {
        Duration::from_millis(self.initial_ms)
    }

    /// Returns the maximum retry delay.
    #[must_use]
    pub const fn maximum(self) -> Duration {
        Duration::from_millis(self.maximum_ms)
    }

    /// Returns the failed-attempt count that marks an orphan.
    #[must_use]
    pub const fn orphan_after(self) -> u32 {
        self.orphan_after
    }

    fn delay_ms(self, failed_attempt: u32) -> u64 {
        let exponent = failed_attempt.saturating_sub(1).min(63);
        self.initial_ms
            .checked_shl(exponent)
            .unwrap_or(u64::MAX)
            .min(self.maximum_ms)
    }
}

/// Validated item, byte, lease, and retry bounds for one cleanup run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupPolicy {
    batch_items: NonZeroUsize,
    batch_bytes: NonZeroUsize,
    lease_ms: u64,
    retry: BoundedBackoff,
}

impl CleanupPolicy {
    /// Constructs a finite cleanup policy.
    pub fn new(
        batch_items: NonZeroUsize,
        batch_bytes: NonZeroUsize,
        lease: Duration,
        retry: BoundedBackoff,
    ) -> Result<Self, UploadError> {
        let lease_ms = duration_ms(lease)?;
        if lease_ms == 0 || lease_ms > MAX_CLEANUP_DURATION_MS {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            batch_items,
            batch_bytes,
            lease_ms,
            retry,
        })
    }

    /// Returns the maximum number of records claimed per run.
    #[must_use]
    pub const fn batch_items(self) -> NonZeroUsize {
        self.batch_items
    }

    /// Returns the maximum aggregate retained file bytes claimed per run.
    #[must_use]
    pub const fn batch_bytes(self) -> NonZeroUsize {
        self.batch_bytes
    }

    /// Returns the cleanup claim lease duration.
    #[must_use]
    pub const fn lease(self) -> Duration {
        Duration::from_millis(self.lease_ms)
    }

    /// Returns the bounded retry policy.
    #[must_use]
    pub const fn retry(self) -> BoundedBackoff {
        self.retry
    }
}

/// One bounded atomic claim request issued to cleanup authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupBatchRequest {
    lease_id: CleanupLeaseId,
    now: UnixMillis,
    lease_expires_at: UnixMillis,
    max_items: usize,
    max_bytes: u64,
}

impl CleanupBatchRequest {
    fn new(
        lease_id: CleanupLeaseId,
        now: UnixMillis,
        lease_expires_at: UnixMillis,
        policy: CleanupPolicy,
    ) -> Result<Self, UploadError> {
        let max_bytes = u64::try_from(policy.batch_bytes.get())
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            lease_id,
            now,
            lease_expires_at,
            max_items: policy.batch_items.get(),
            max_bytes,
        })
    }

    /// Returns the trusted run identity.
    #[must_use]
    pub const fn lease_id(&self) -> &CleanupLeaseId {
        &self.lease_id
    }

    /// Returns the claim instant.
    #[must_use]
    pub const fn now(&self) -> UnixMillis {
        self.now
    }

    /// Returns the exclusive claim deadline.
    #[must_use]
    pub const fn lease_expires_at(&self) -> UnixMillis {
        self.lease_expires_at
    }

    /// Returns the maximum number of claims.
    #[must_use]
    pub const fn max_items(&self) -> usize {
        self.max_items
    }

    /// Returns the maximum aggregate retained file bytes.
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
}

/// One revision-fenced cleanup lease returned by upload authority.
#[derive(Clone, Eq, PartialEq)]
pub struct CleanupClaim {
    handle: UploadHandle,
    revision: UploadRevision,
    created_at: UnixMillis,
    retained_bytes: u64,
    lease_id: CleanupLeaseId,
    lease_expires_at: UnixMillis,
    failed_attempts: u32,
    orphaned: bool,
}

impl CleanupClaim {
    /// Constructs a claim after the ledger atomically proves terminal eligibility.
    pub fn from_store(
        record: &UploadRecord,
        retained_bytes: u64,
        lease_id: CleanupLeaseId,
        lease_expires_at: UnixMillis,
        failed_attempts: u32,
        orphaned: bool,
    ) -> Result<Self, UploadError> {
        if !matches!(
            record.state(),
            UploadState::Rejected
                | UploadState::Canceled
                | UploadState::Expired
                | UploadState::Failed
        ) {
            return Err(UploadError::new(UploadErrorKind::InvalidTransition));
        }
        Ok(Self {
            handle: record.authority().handle().clone(),
            revision: record.revision(),
            created_at: record.created_at(),
            retained_bytes,
            lease_id,
            lease_expires_at,
            failed_attempts,
            orphaned,
        })
    }

    /// Returns the temporary upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the exact terminal authority revision.
    #[must_use]
    pub const fn revision(&self) -> UploadRevision {
        self.revision
    }

    /// Returns the authority creation instant.
    #[must_use]
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }

    /// Returns the authoritative retained-byte accounting.
    #[must_use]
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    /// Returns the trusted cleanup run identity.
    #[must_use]
    pub const fn lease_id(&self) -> &CleanupLeaseId {
        &self.lease_id
    }

    /// Returns the exclusive lease deadline.
    #[must_use]
    pub const fn lease_expires_at(&self) -> UnixMillis {
        self.lease_expires_at
    }

    /// Returns the failures recorded before this claim.
    #[must_use]
    pub const fn failed_attempts(&self) -> u32 {
        self.failed_attempts
    }

    /// Returns whether earlier failures marked this upload orphaned.
    #[must_use]
    pub const fn orphaned(&self) -> bool {
        self.orphaned
    }

    fn accounted_bytes(&self) -> usize {
        CLAIM_ACCOUNTING_OVERHEAD
    }
}

impl fmt::Debug for CleanupClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CleanupClaim")
            .field("revision", &self.revision)
            .field("retained_bytes", &self.retained_bytes)
            .field("lease_expires_at", &self.lease_expires_at)
            .field("failed_attempts", &self.failed_attempts)
            .field("orphaned", &self.orphaned)
            .finish_non_exhaustive()
    }
}

/// Ledger mutation requested after one claimed cleanup attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupCompletionKind {
    /// Physical bytes and validation evidence were idempotently removed.
    Reclaimed,
    /// Reclamation failed and should retry at the given instant.
    Retry {
        /// Earliest instant at which the next claim may be acquired.
        retry_at: UnixMillis,
        /// Whether this failed attempt crosses the orphan threshold.
        orphaned: bool,
    },
    /// No reclamation was attempted; return the claim without adding a failure.
    Deferred {
        /// Earliest instant at which the claim may be reacquired.
        retry_at: UnixMillis,
    },
}

/// Exact fenced completion request for one cleanup claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupCompletion {
    handle: UploadHandle,
    revision: UploadRevision,
    lease_id: CleanupLeaseId,
    completed_at: UnixMillis,
    kind: CleanupCompletionKind,
}

impl CleanupCompletion {
    fn new(claim: &CleanupClaim, completed_at: UnixMillis, kind: CleanupCompletionKind) -> Self {
        Self {
            handle: claim.handle.clone(),
            revision: claim.revision,
            lease_id: claim.lease_id.clone(),
            completed_at,
            kind,
        }
    }

    /// Returns the target upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the exact claimed authority revision.
    #[must_use]
    pub const fn revision(&self) -> UploadRevision {
        self.revision
    }

    /// Returns the trusted cleanup run identity.
    #[must_use]
    pub const fn lease_id(&self) -> &CleanupLeaseId {
        &self.lease_id
    }

    /// Returns the completion instant used for lease fencing.
    #[must_use]
    pub const fn completed_at(&self) -> UnixMillis {
        self.completed_at
    }

    /// Returns the requested terminal ledger mutation.
    #[must_use]
    pub const fn kind(&self) -> CleanupCompletionKind {
        self.kind
    }
}

/// Whether cleanup authority applied or fenced a completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupLedgerDisposition {
    /// The exact current lease accepted the completion.
    Applied,
    /// The lease expired, was superseded, or no longer matched the revision.
    Stale,
}

/// Host-owned cleanup authority colocated with the upload ledger.
pub trait UploadCleanupLedger: Send + Sync {
    /// Atomically expires eligible active uploads and leases a bounded terminal batch.
    fn claim_cleanup<'a>(
        &'a self,
        request: CleanupBatchRequest,
    ) -> UploadFuture<'a, Result<Vec<CleanupClaim>, UploadError>>;

    /// Atomically applies one exact, unexpired, revision-fenced completion.
    fn complete_cleanup<'a>(
        &'a self,
        completion: CleanupCompletion,
    ) -> UploadFuture<'a, Result<CleanupLedgerDisposition, UploadError>>;
}

/// Aggregate outcome of one bounded cleanup run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupDisposition {
    /// No eligible work was available.
    Idle,
    /// Every claimed upload was reclaimed and terminalized.
    Complete,
    /// Work was canceled, fenced, resource-deferred, or scheduled for retry.
    Deferred,
}

/// Identifier-free aggregate result of one cleanup run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupRunOutcome {
    disposition: CleanupDisposition,
    claimed: usize,
    reclaimed: usize,
    reclaimed_bytes: u64,
    retry_scheduled: usize,
    orphaned: usize,
    deferred: usize,
}

impl CleanupRunOutcome {
    const fn canceled() -> Self {
        Self {
            disposition: CleanupDisposition::Deferred,
            claimed: 0,
            reclaimed: 0,
            reclaimed_bytes: 0,
            retry_scheduled: 0,
            orphaned: 0,
            deferred: 0,
        }
    }

    fn from_claims(claimed: usize) -> Self {
        Self {
            disposition: if claimed == 0 {
                CleanupDisposition::Idle
            } else {
                CleanupDisposition::Complete
            },
            claimed,
            reclaimed: 0,
            reclaimed_bytes: 0,
            retry_scheduled: 0,
            orphaned: 0,
            deferred: 0,
        }
    }

    /// Returns the aggregate run disposition.
    #[must_use]
    pub const fn disposition(self) -> CleanupDisposition {
        self.disposition
    }

    /// Returns the number of records leased by this run.
    #[must_use]
    pub const fn claimed(self) -> usize {
        self.claimed
    }

    /// Returns the number of records successfully reclaimed.
    #[must_use]
    pub const fn reclaimed(self) -> usize {
        self.reclaimed
    }

    /// Returns the authoritative retained bytes successfully reclaimed.
    #[must_use]
    pub const fn reclaimed_bytes(self) -> u64 {
        self.reclaimed_bytes
    }

    /// Returns the number of failed attempts scheduled for retry.
    #[must_use]
    pub const fn retry_scheduled(self) -> usize {
        self.retry_scheduled
    }

    /// Returns the number of attempts marked orphaned during this run.
    #[must_use]
    pub const fn orphaned(self) -> usize {
        self.orphaned
    }

    /// Returns the number of claims returned or fenced without reclamation.
    #[must_use]
    pub const fn deferred(self) -> usize {
        self.deferred
    }
}

/// Bounded cleanup coordinator using shared resource primitives and host authority.
pub struct UploadCleanupService {
    ledger: Arc<dyn UploadCleanupLedger>,
    provider: Arc<dyn UploadProvider>,
    validation_store: Arc<dyn UploadValidationStore>,
    clock: Arc<dyn Clock>,
    permits: PermitPool,
    cancellation: CancellationFlag,
    policy: CleanupPolicy,
    queue_bounds: ResourceBounds,
    metrics: Option<Arc<dyn CleanupMetricSink>>,
}

impl UploadCleanupService {
    /// Constructs a cleanup service from shared authority and resource controls.
    pub fn new(
        ledger: Arc<dyn UploadCleanupLedger>,
        provider: Arc<dyn UploadProvider>,
        validation_store: Arc<dyn UploadValidationStore>,
        clock: Arc<dyn Clock>,
        permits: PermitPool,
        policy: CleanupPolicy,
        limits: UploadLimits,
    ) -> Result<Self, UploadError> {
        let max_file_bytes = usize::try_from(limits.max_file_bytes())
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        if policy.batch_items.get() > limits.max_cleanup_batch()
            || policy.batch_bytes.get() < max_file_bytes
            || policy.retry.orphan_after > limits.max_retries()
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let queue_bytes = CLAIM_ACCOUNTING_OVERHEAD
            .checked_mul(policy.batch_items.get())
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        let queue_bounds = ResourceBounds::new(policy.batch_items.get(), queue_bytes)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            ledger,
            provider,
            validation_store,
            clock,
            permits,
            cancellation: CancellationFlag::new(),
            policy,
            queue_bounds,
            metrics: None,
        })
    }

    /// Installs a non-authoritative identifier-free metric observer.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<dyn CleanupMetricSink>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Requests advisory cancellation, returning `true` only on the first call.
    pub fn cancel(&self) -> bool {
        self.cancellation.cancel()
    }

    /// Claims and reconciles at most one bounded batch without browser cooperation.
    pub async fn run_once(
        &self,
        lease_id: CleanupLeaseId,
    ) -> Result<CleanupRunOutcome, UploadError> {
        if self.cancellation.is_canceled() {
            return Ok(CleanupRunOutcome::canceled());
        }
        let now = self.now()?;
        let lease_expires_at = checked_deadline(now, self.policy.lease_ms)?;
        let request = CleanupBatchRequest::new(lease_id, now, lease_expires_at, self.policy)?;
        let claims = run_upload_future(
            || self.ledger.claim_cleanup(request),
            UploadErrorKind::LedgerUnavailable,
        )
        .await?;
        let mut outcome = CleanupRunOutcome::from_claims(claims.len());
        if claims.is_empty() {
            return Ok(outcome);
        }

        let owner = ResourceOwner::new(self.queue_bounds);
        for claim in claims {
            let accounted_bytes = claim.accounted_bytes();
            if owner.queue().try_push(accounted_bytes, claim).is_err() {
                return Err(UploadError::new(UploadErrorKind::ResourceExhausted));
            }
        }

        while let Some(claim) = owner.queue().pop() {
            if self.cancellation.is_canceled() {
                self.defer_claim(&claim, &mut outcome).await?;
                continue;
            }
            let permit = match self.permits.try_acquire() {
                Ok(permit) => permit,
                Err(_) => {
                    self.defer_claim(&claim, &mut outcome).await?;
                    continue;
                }
            };
            let provider = run_upload_future(
                || self.provider.cleanup(claim.handle()),
                UploadErrorKind::ProviderUnavailable,
            )
            .await;
            let reclaimed = if provider.is_ok() {
                run_upload_future(
                    || self.validation_store.remove(claim.handle()),
                    UploadErrorKind::LedgerUnavailable,
                )
                .await
            } else {
                provider
            };
            drop(permit);
            let completed_at = self.now()?;
            if reclaimed.is_ok() {
                self.complete_reclaimed(&claim, completed_at, &mut outcome)
                    .await?;
            } else {
                self.complete_retry(&claim, completed_at, &mut outcome)
                    .await?;
            }
        }
        if outcome.retry_scheduled > 0 || outcome.deferred > 0 {
            outcome.disposition = CleanupDisposition::Deferred;
        }
        Ok(outcome)
    }

    async fn defer_claim(
        &self,
        claim: &CleanupClaim,
        outcome: &mut CleanupRunOutcome,
    ) -> Result<(), UploadError> {
        let completed_at = self.now()?;
        let completion = CleanupCompletion::new(
            claim,
            completed_at,
            CleanupCompletionKind::Deferred {
                retry_at: completed_at,
            },
        );
        let disposition = self.complete(completion).await?;
        outcome.deferred += 1;
        let metric_outcome = match disposition {
            CleanupLedgerDisposition::Applied => CleanupOutcome::Deferred,
            CleanupLedgerDisposition::Stale => CleanupOutcome::LeaseLost,
        };
        self.record(
            claim,
            completed_at,
            metric_outcome,
            claim.failed_attempts,
            claim.orphaned,
        );
        Ok(())
    }

    async fn complete_reclaimed(
        &self,
        claim: &CleanupClaim,
        completed_at: UnixMillis,
        outcome: &mut CleanupRunOutcome,
    ) -> Result<(), UploadError> {
        let completion =
            CleanupCompletion::new(claim, completed_at, CleanupCompletionKind::Reclaimed);
        match self.complete(completion).await? {
            CleanupLedgerDisposition::Applied => {
                outcome.reclaimed += 1;
                outcome.reclaimed_bytes =
                    outcome.reclaimed_bytes.saturating_add(claim.retained_bytes);
                self.record(
                    claim,
                    completed_at,
                    CleanupOutcome::Reclaimed,
                    claim.failed_attempts,
                    claim.orphaned,
                );
            }
            CleanupLedgerDisposition::Stale => {
                outcome.deferred += 1;
                self.record(
                    claim,
                    completed_at,
                    CleanupOutcome::LeaseLost,
                    claim.failed_attempts,
                    claim.orphaned,
                );
            }
        }
        Ok(())
    }

    async fn complete_retry(
        &self,
        claim: &CleanupClaim,
        completed_at: UnixMillis,
        outcome: &mut CleanupRunOutcome,
    ) -> Result<(), UploadError> {
        let failed_attempt = claim.failed_attempts.saturating_add(1);
        let orphaned = claim.orphaned || failed_attempt >= self.policy.retry.orphan_after;
        let retry_at = checked_deadline(completed_at, self.policy.retry.delay_ms(failed_attempt))?;
        let completion = CleanupCompletion::new(
            claim,
            completed_at,
            CleanupCompletionKind::Retry { retry_at, orphaned },
        );
        match self.complete(completion).await? {
            CleanupLedgerDisposition::Applied => {
                outcome.retry_scheduled += 1;
                if orphaned {
                    outcome.orphaned += 1;
                }
                self.record(
                    claim,
                    completed_at,
                    CleanupOutcome::RetryScheduled,
                    failed_attempt,
                    orphaned,
                );
            }
            CleanupLedgerDisposition::Stale => {
                outcome.deferred += 1;
                self.record(
                    claim,
                    completed_at,
                    CleanupOutcome::LeaseLost,
                    failed_attempt,
                    orphaned,
                );
            }
        }
        Ok(())
    }

    async fn complete(
        &self,
        completion: CleanupCompletion,
    ) -> Result<CleanupLedgerDisposition, UploadError> {
        run_upload_future(
            || self.ledger.complete_cleanup(completion),
            UploadErrorKind::LedgerUnavailable,
        )
        .await
    }

    fn now(&self) -> Result<UnixMillis, UploadError> {
        catch_unwind(AssertUnwindSafe(|| self.clock.now()))
            .map_err(|_| UploadError::new(UploadErrorKind::LedgerUnavailable))?
            .map_err(|_| UploadError::new(UploadErrorKind::LedgerUnavailable))
    }

    fn record(
        &self,
        claim: &CleanupClaim,
        observed_at: UnixMillis,
        cleanup_outcome: CleanupOutcome,
        failed_attempts: u32,
        orphaned: bool,
    ) {
        record_metrics(
            self.metrics.as_deref(),
            CleanupMetrics::new(
                UploadAgeBucket::classify(claim.created_at, observed_at),
                UploadVolumeBucket::classify(claim.retained_bytes),
                cleanup_outcome,
                RetryBucket::classify(failed_attempts),
                orphaned,
            ),
        );
    }
}

impl fmt::Debug for UploadCleanupService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadCleanupService")
            .field("policy", &self.policy)
            .field("permits", &self.permits)
            .field("canceled", &self.cancellation.is_canceled())
            .field("metrics", &self.metrics.is_some())
            .finish_non_exhaustive()
    }
}

fn duration_ms(duration: Duration) -> Result<u64, UploadError> {
    u64::try_from(duration.as_millis()).map_err(|_| UploadError::new(UploadErrorKind::InvalidField))
}

fn checked_deadline(start: UnixMillis, duration_ms: u64) -> Result<UnixMillis, UploadError> {
    start
        .get()
        .checked_add(duration_ms)
        .map(UnixMillis::new)
        .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))
}

async fn run_upload_future<'a, T: Send + 'a>(
    operation: impl FnOnce() -> UploadFuture<'a, Result<T, UploadError>>,
    panic_kind: UploadErrorKind,
) -> Result<T, UploadError> {
    let mut future =
        catch_unwind(AssertUnwindSafe(operation)).map_err(|_| UploadError::new(panic_kind))?;
    let result =
        poll_fn(
            |context| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
                Ok(Poll::Ready(result)) => Poll::Ready(result),
                Ok(Poll::Pending) => Poll::Pending,
                Err(_) => Poll::Ready(Err(UploadError::new(panic_kind))),
            },
        )
        .await;
    if catch_unwind(AssertUnwindSafe(|| drop(future))).is_err() {
        return Err(UploadError::new(panic_kind));
    }
    result
}

impl fmt::Display for CleanupLeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<CleanupLeaseId>")
    }
}
