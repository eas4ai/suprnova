//! Deterministic upload authority controls and the complete Tier 0 ledger.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::UploadLimits;
use suprnova_live::upload::{
    CleanupBatchRequest, CleanupClaim, CleanupCompletion, CleanupCompletionKind, CleanupLeaseId,
    CleanupLedgerDisposition, ConditionalTransition, ConditionalUploadCreate,
    TransitionDisposition, TransitionOutcome, UploadAuthorizationDecision, UploadAuthorizationPort,
    UploadAuthorizationRequest, UploadCleanupLedger, UploadControlKind, UploadCreateCommand,
    UploadError, UploadErrorKind, UploadFuture, UploadHandle, UploadLedger,
    UploadLedgerCreateOutcome, UploadRecord, UploadState, UploadStateMachine, UploadTransition,
};

/// Mutable current upload-authorization control with deterministic observations.
pub struct ControlledUploadAuthorization {
    decision: Mutex<UploadAuthorizationDecision>,
    failing: AtomicBool,
    calls: AtomicUsize,
    last_control: Mutex<Option<UploadControlKind>>,
}

impl ControlledUploadAuthorization {
    /// Creates one healthy allowing authorization control.
    #[must_use]
    pub fn new() -> Self {
        Self {
            decision: Mutex::new(UploadAuthorizationDecision::Allow),
            failing: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            last_control: Mutex::new(None),
        }
    }

    /// Replaces the current authorization decision.
    pub fn set_decision(&self, decision: UploadAuthorizationDecision) {
        *lock(&self.decision) = decision;
    }

    /// Enables or disables a closed provider failure.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// Returns the number of authorization boundaries observed.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Returns the most recently authorized closed control identity.
    #[must_use]
    pub fn last_control(&self) -> Option<UploadControlKind> {
        *lock(&self.last_control)
    }
}

impl Default for ControlledUploadAuthorization {
    fn default() -> Self {
        Self::new()
    }
}

impl UploadAuthorizationPort for ControlledUploadAuthorization {
    fn authorize<'a>(
        &'a self,
        request: UploadAuthorizationRequest<'a>,
    ) -> UploadFuture<'a, Result<UploadAuthorizationDecision, UploadError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *lock(&self.last_control) = Some(request.control());
            if self.failing.load(Ordering::SeqCst) {
                Err(UploadError::new(UploadErrorKind::AuthorizationUnavailable))
            } else {
                Ok(*lock(&self.decision))
            }
        })
    }
}

struct StoredUpload {
    record: UploadRecord,
    create_key: suprnova_live::upload::UploadIdempotencyKey,
    machine: UploadStateMachine,
    transition_keys: HashSet<suprnova_live::upload::UploadIdempotencyKey>,
    cleanup: StoredCleanup,
}

#[derive(Default)]
struct StoredCleanup {
    retained_bytes: u64,
    lease: Option<StoredCleanupLease>,
    retry_at: Option<UnixMillis>,
    retries: u32,
    orphaned: bool,
    scheduled_at: Option<UnixMillis>,
}

struct StoredCleanupLease {
    id: CleanupLeaseId,
    revision: suprnova_live::upload::UploadRevision,
    expires_at: UnixMillis,
}

struct CreationEvent {
    scope: suprnova_live::host::HostScopeFacts,
    admitted_at: UnixMillis,
}

#[derive(Default)]
struct MemoryUploadState {
    records: HashMap<UploadHandle, StoredUpload>,
    creations: Vec<CreationEvent>,
    cleanup_schedule: BTreeMap<UnixMillis, BTreeSet<UploadHandle>>,
    last_cleanup_examined: usize,
}

impl MemoryUploadState {
    fn schedule_cleanup(&mut self, handle: &UploadHandle, at: UnixMillis) {
        self.unschedule_cleanup(handle);
        if let Some(stored) = self.records.get_mut(handle) {
            stored.cleanup.scheduled_at = Some(at);
        }
        self.cleanup_schedule
            .entry(at)
            .or_default()
            .insert(handle.clone());
    }

    fn unschedule_cleanup(&mut self, handle: &UploadHandle) {
        let previous = self
            .records
            .get_mut(handle)
            .and_then(|stored| stored.cleanup.scheduled_at.take());
        let Some(previous) = previous else {
            return;
        };
        let remove_bucket = self
            .cleanup_schedule
            .get_mut(&previous)
            .is_some_and(|bucket| {
                bucket.remove(handle);
                bucket.is_empty()
            });
        if remove_bucket {
            self.cleanup_schedule.remove(&previous);
        }
    }

    fn pop_due_cleanup(&mut self, now: UnixMillis) -> Option<UploadHandle> {
        let deadline = *self.cleanup_schedule.first_key_value()?.0;
        if deadline > now {
            return None;
        }
        let (handle, empty) = {
            let bucket = self.cleanup_schedule.get_mut(&deadline)?;
            let handle = bucket.pop_first()?;
            (handle, bucket.is_empty())
        };
        if empty {
            self.cleanup_schedule.remove(&deadline);
        }
        if let Some(stored) = self.records.get_mut(&handle) {
            stored.cleanup.scheduled_at = None;
        }
        Some(handle)
    }
}

/// Complete daemon-free reference implementation of conditional upload authority.
pub struct MemoryUploadLedger {
    limits: UploadLimits,
    state: Mutex<MemoryUploadState>,
    fail_transition: AtomicBool,
}

/// Identifier-free cleanup state exposed by the Tier 0 test ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryCleanupObservation {
    retained_bytes: u64,
    retries: u32,
    orphaned: bool,
    leased: bool,
}

impl MemoryCleanupObservation {
    /// Returns the authoritative retained byte accounting.
    #[must_use]
    pub const fn retained_bytes(self) -> u64 {
        self.retained_bytes
    }

    /// Returns the recorded failed cleanup attempts.
    #[must_use]
    pub const fn retries(self) -> u32 {
        self.retries
    }

    /// Returns whether bounded retries marked the record orphaned.
    #[must_use]
    pub const fn orphaned(self) -> bool {
        self.orphaned
    }

    /// Returns whether a cleanup lease is currently stored.
    #[must_use]
    pub const fn leased(self) -> bool {
        self.leased
    }
}

impl MemoryUploadLedger {
    /// Creates an empty reference ledger using one validated finite policy.
    pub fn new(limits: UploadLimits) -> Result<Self, UploadError> {
        NonZeroUsize::new(limits.max_idempotency_outcomes())
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            limits,
            state: Mutex::new(MemoryUploadState::default()),
            fail_transition: AtomicBool::new(false),
        })
    }

    /// Seeds one authoritative record for focused state and service tests.
    pub fn seed(&self, record: UploadRecord) -> Result<(), UploadError> {
        let mut state = lock(&self.state);
        if state.records.contains_key(record.authority().handle()) {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        let machine = machine_for(&record, self.limits)?;
        let handle = record.authority().handle().clone();
        let cleanup_at = cleanup_deadline(&record, record.created_at());
        state.records.insert(
            handle.clone(),
            StoredUpload {
                record,
                create_key: suprnova_live::upload::UploadIdempotencyKey::parse("seeded")?,
                machine,
                transition_keys: HashSet::new(),
                cleanup: StoredCleanup::default(),
            },
        );
        if let Some(cleanup_at) = cleanup_at {
            state.schedule_cleanup(&handle, cleanup_at);
        }
        Ok(())
    }

    /// Seeds authoritative retained bytes for focused cleanup tests.
    pub fn seed_cleanup_bytes(
        &self,
        handle: &UploadHandle,
        retained_bytes: u64,
    ) -> Result<(), UploadError> {
        if retained_bytes > self.limits.max_file_bytes() {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let mut state = lock(&self.state);
        let stored = state
            .records
            .get_mut(handle)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        stored.cleanup.retained_bytes = retained_bytes;
        Ok(())
    }

    /// Returns identifier-free cleanup state for deterministic assertions.
    #[must_use]
    pub fn cleanup_observation(&self, handle: &UploadHandle) -> Option<MemoryCleanupObservation> {
        lock(&self.state)
            .records
            .get(handle)
            .map(|stored| MemoryCleanupObservation {
                retained_bytes: stored.cleanup.retained_bytes,
                retries: stored.cleanup.retries,
                orphaned: stored.cleanup.orphaned,
                leased: stored.cleanup.lease.is_some(),
            })
    }

    /// Returns the due records examined by the most recent cleanup claim.
    #[must_use]
    pub fn cleanup_examined_last_run(&self) -> usize {
        lock(&self.state).last_cleanup_examined
    }

    /// Injects one ledger transition failure before any state mutation.
    pub fn fail_next_transition(&self) {
        self.fail_transition.store(true, Ordering::SeqCst);
    }

    /// Returns the number of retained upload records.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.state).records.len()
    }

    /// Returns whether the ledger contains no upload records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl UploadLedger for MemoryUploadLedger {
    fn create<'a>(
        &'a self,
        request: UploadCreateCommand,
    ) -> UploadFuture<'a, Result<UploadLedgerCreateOutcome, UploadError>> {
        Box::pin(async move {
            if request.limits() != self.limits {
                return Err(UploadError::new(UploadErrorKind::LedgerUnavailable));
            }
            let mut state = lock(&self.state);
            if let Some(existing) = state.records.get(request.record().authority().handle()) {
                let exact = existing.create_key == *request.idempotency_key()
                    && existing.record.authority() == request.record().authority()
                    && existing.record.expires_at() == request.record().expires_at();
                return if exact {
                    Ok(UploadLedgerCreateOutcome::new(
                        ConditionalUploadCreate::ExistingOutcome,
                        existing.record.clone(),
                    ))
                } else {
                    Err(UploadError::new(UploadErrorKind::UploadConflict))
                };
            }
            if state.records.values().any(|existing| {
                existing.create_key == *request.idempotency_key()
                    && existing.record.authority().host_scope()
                        == request.record().authority().host_scope()
            }) {
                return Err(UploadError::new(UploadErrorKind::UploadConflict));
            }

            prune_creation_window(
                &mut state.creations,
                request.admitted_at(),
                request.limits().creation_window_ms(),
            );
            let scope = request.record().authority().host_scope();
            let creation_count = state
                .creations
                .iter()
                .filter(|event| &event.scope == scope)
                .count();
            if creation_count >= request.limits().max_creations_per_window() {
                return Err(UploadError::new(UploadErrorKind::CreationRateExceeded));
            }

            let pending_count = state
                .records
                .values()
                .filter(|existing| {
                    !existing.record.state().is_terminal()
                        && existing.record.authority().host_scope() == scope
                })
                .count();
            if pending_count >= request.limits().max_pending_per_scope() {
                return Err(UploadError::new(UploadErrorKind::PendingLimitExceeded));
            }
            let field_count = state
                .records
                .values()
                .filter(|existing| {
                    !existing.record.state().is_terminal()
                        && existing.record.authority().host_scope() == scope
                        && existing.record.authority().component()
                            == request.record().authority().component()
                        && existing.record.authority().field()
                            == request.record().authority().field()
                })
                .count();
            if field_count >= request.limits().max_files_per_field() {
                return Err(UploadError::new(UploadErrorKind::FileCountExceeded));
            }

            let record = request.record().clone();
            let machine = machine_for(&record, request.limits())?;
            state.creations.push(CreationEvent {
                scope: scope.clone(),
                admitted_at: request.admitted_at(),
            });
            let handle = record.authority().handle().clone();
            let cleanup_at = cleanup_deadline(&record, record.created_at());
            state.records.insert(
                handle.clone(),
                StoredUpload {
                    record: record.clone(),
                    create_key: request.idempotency_key().clone(),
                    machine,
                    transition_keys: HashSet::new(),
                    cleanup: StoredCleanup::default(),
                },
            );
            if let Some(cleanup_at) = cleanup_at {
                state.schedule_cleanup(&handle, cleanup_at);
            }
            Ok(UploadLedgerCreateOutcome::new(
                ConditionalUploadCreate::Created,
                record,
            ))
        })
    }

    fn load<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<Option<UploadRecord>, UploadError>> {
        Box::pin(async move {
            Ok(lock(&self.state)
                .records
                .get(handle)
                .map(|stored| stored.record.clone()))
        })
    }

    fn transition<'a>(
        &'a self,
        request: ConditionalTransition,
    ) -> UploadFuture<'a, Result<TransitionOutcome, UploadError>> {
        Box::pin(async move {
            if self.fail_transition.swap(false, Ordering::SeqCst) {
                return Err(UploadError::new(UploadErrorKind::LedgerUnavailable));
            }
            let mut state = lock(&self.state);
            let handle = request.transition().handle().clone();
            let (outcome, schedule_update) = {
                let stored = state
                    .records
                    .get_mut(&handle)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
                if stored.record.authority() != request.authority() {
                    return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
                }
                if stored.record.expires_at() <= request.admitted_at() {
                    return Err(UploadError::new(UploadErrorKind::UploadExpired));
                }
                let retained_bytes = match request.transition().transition() {
                    UploadTransition::PutChunk(chunk) => Some(
                        if stored
                            .transition_keys
                            .contains(request.transition().idempotency_key())
                        {
                            stored.cleanup.retained_bytes
                        } else {
                            stored
                                .cleanup
                                .retained_bytes
                                .checked_add(chunk.size())
                                .filter(|bytes| *bytes <= self.limits.max_file_bytes())
                                .ok_or_else(|| {
                                    UploadError::new(UploadErrorKind::ResourceExhausted)
                                })?
                        },
                    ),
                    _ => None,
                };
                let outcome = stored.machine.apply(request.transition().clone())?;
                if outcome.disposition() == TransitionDisposition::Applied {
                    stored.record = stored.record.with_outcome(outcome)?;
                    if let Some(retained_bytes) = retained_bytes {
                        stored.cleanup.retained_bytes = retained_bytes;
                    }
                    stored
                        .transition_keys
                        .insert(request.transition().idempotency_key().clone());
                    (
                        outcome,
                        Some(cleanup_deadline(&stored.record, request.admitted_at())),
                    )
                } else {
                    (outcome, None)
                }
            };
            if let Some(cleanup_at) = schedule_update {
                match cleanup_at {
                    Some(cleanup_at) => state.schedule_cleanup(&handle, cleanup_at),
                    None => state.unschedule_cleanup(&handle),
                }
            }
            Ok(outcome)
        })
    }
}

impl UploadCleanupLedger for MemoryUploadLedger {
    fn claim_cleanup<'a>(
        &'a self,
        request: CleanupBatchRequest,
    ) -> UploadFuture<'a, Result<Vec<CleanupClaim>, UploadError>> {
        Box::pin(async move {
            let mut state = lock(&self.state);
            state.last_cleanup_examined = 0;
            let mut claims = Vec::with_capacity(request.max_items());
            let mut claimed_bytes = 0_u64;
            while claims.len() < request.max_items() {
                let Some(handle) = state.pop_due_cleanup(request.now()) else {
                    break;
                };
                state.last_cleanup_examined += 1;
                let (retained_bytes, future_lease, active_expiry, eligible) = {
                    let stored = state
                        .records
                        .get(&handle)
                        .ok_or_else(|| UploadError::new(UploadErrorKind::LedgerUnavailable))?;
                    let future_lease = stored
                        .cleanup
                        .lease
                        .as_ref()
                        .filter(|lease| lease.expires_at > request.now())
                        .map(|lease| lease.expires_at);
                    let active_expiry = matches!(
                        stored.record.state(),
                        UploadState::Created
                            | UploadState::Queued
                            | UploadState::Transferring
                            | UploadState::Verifying
                            | UploadState::Ready
                    )
                    .then_some(stored.record.expires_at());
                    let eligible = cleanup_state_eligible(&stored.record, request.now());
                    (
                        stored.cleanup.retained_bytes,
                        future_lease,
                        active_expiry,
                        eligible,
                    )
                };
                if let Some(future_lease) = future_lease {
                    state.schedule_cleanup(&handle, future_lease);
                    continue;
                }
                if !eligible {
                    if let Some(active_expiry) = active_expiry {
                        state.schedule_cleanup(&handle, active_expiry);
                    }
                    continue;
                }
                let Some(next_bytes) = claimed_bytes.checked_add(retained_bytes) else {
                    state.schedule_cleanup(&handle, request.now());
                    break;
                };
                if next_bytes > request.max_bytes() {
                    state.schedule_cleanup(&handle, request.now());
                    break;
                }
                let claim = {
                    let stored = state
                        .records
                        .get_mut(&handle)
                        .ok_or_else(|| UploadError::new(UploadErrorKind::LedgerUnavailable))?;
                    if stored
                        .cleanup
                        .lease
                        .as_ref()
                        .is_some_and(|lease| lease.expires_at <= request.now())
                    {
                        stored.cleanup.lease = None;
                    }
                    if matches!(
                        stored.record.state(),
                        UploadState::Created
                            | UploadState::Queued
                            | UploadState::Transferring
                            | UploadState::Verifying
                            | UploadState::Ready
                    ) {
                        let expired = stored
                            .machine
                            .expire_for_cleanup(stored.record.revision())?;
                        stored.record = stored.record.with_outcome(expired)?;
                    }
                    let claim = CleanupClaim::from_store(
                        &stored.record,
                        stored.cleanup.retained_bytes,
                        request.lease_id().clone(),
                        request.lease_expires_at(),
                        stored.cleanup.retries,
                        stored.cleanup.orphaned,
                    )?;
                    stored.cleanup.retry_at = None;
                    stored.cleanup.lease = Some(StoredCleanupLease {
                        id: request.lease_id().clone(),
                        revision: stored.record.revision(),
                        expires_at: request.lease_expires_at(),
                    });
                    claim
                };
                state.schedule_cleanup(&handle, request.lease_expires_at());
                claimed_bytes = next_bytes;
                claims.push(claim);
            }
            Ok(claims)
        })
    }

    fn complete_cleanup<'a>(
        &'a self,
        completion: CleanupCompletion,
    ) -> UploadFuture<'a, Result<CleanupLedgerDisposition, UploadError>> {
        Box::pin(async move {
            let mut state = lock(&self.state);
            let Some(stored) = state.records.get(completion.handle()) else {
                return Ok(CleanupLedgerDisposition::Stale);
            };
            let Some(lease) = stored.cleanup.lease.as_ref() else {
                return Ok(CleanupLedgerDisposition::Stale);
            };
            let current = lease.id == *completion.lease_id()
                && lease.revision == completion.revision()
                && stored.record.revision() == completion.revision()
                && completion.completed_at() < lease.expires_at;
            if !current {
                return Ok(CleanupLedgerDisposition::Stale);
            }
            match completion.kind() {
                CleanupCompletionKind::Reclaimed => {
                    state.unschedule_cleanup(completion.handle());
                    state.records.remove(completion.handle());
                }
                CleanupCompletionKind::Retry { retry_at, orphaned } => {
                    let stored = state
                        .records
                        .get_mut(completion.handle())
                        .ok_or_else(|| UploadError::new(UploadErrorKind::LedgerUnavailable))?;
                    stored.cleanup.lease = None;
                    stored.cleanup.retries = stored.cleanup.retries.saturating_add(1);
                    stored.cleanup.retry_at = Some(retry_at);
                    stored.cleanup.orphaned |= orphaned;
                    state.schedule_cleanup(completion.handle(), retry_at);
                }
                CleanupCompletionKind::Deferred { retry_at } => {
                    let stored = state
                        .records
                        .get_mut(completion.handle())
                        .ok_or_else(|| UploadError::new(UploadErrorKind::LedgerUnavailable))?;
                    stored.cleanup.lease = None;
                    stored.cleanup.retry_at = Some(retry_at);
                    state.schedule_cleanup(completion.handle(), retry_at);
                }
            }
            Ok(CleanupLedgerDisposition::Applied)
        })
    }
}

fn cleanup_state_eligible(record: &UploadRecord, now: UnixMillis) -> bool {
    match record.state() {
        UploadState::Created
        | UploadState::Queued
        | UploadState::Transferring
        | UploadState::Verifying
        | UploadState::Ready => record.expires_at() <= now,
        UploadState::Rejected
        | UploadState::Canceled
        | UploadState::Expired
        | UploadState::Failed => true,
        UploadState::Finalizing | UploadState::Finalized => false,
    }
}

fn cleanup_deadline(record: &UploadRecord, terminal_at: UnixMillis) -> Option<UnixMillis> {
    match record.state() {
        UploadState::Created
        | UploadState::Queued
        | UploadState::Transferring
        | UploadState::Verifying
        | UploadState::Ready => Some(record.expires_at()),
        UploadState::Rejected
        | UploadState::Canceled
        | UploadState::Expired
        | UploadState::Failed => Some(terminal_at),
        UploadState::Finalizing | UploadState::Finalized => None,
    }
}

fn machine_for(
    record: &UploadRecord,
    limits: UploadLimits,
) -> Result<UploadStateMachine, UploadError> {
    let max_outcomes = NonZeroUsize::new(limits.max_idempotency_outcomes())
        .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
    UploadStateMachine::with_outcome_limit(
        record.authority().handle().clone(),
        record.state(),
        record.revision(),
        max_outcomes,
    )
}

fn prune_creation_window(events: &mut Vec<CreationEvent>, now: UnixMillis, window_ms: u64) {
    let cutoff = now.get().saturating_sub(window_ms);
    events.retain(|event| event.admitted_at.get() > cutoff);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
