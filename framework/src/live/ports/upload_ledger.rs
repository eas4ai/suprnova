//! Complete single-process upload authority and cleanup ledger.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, MutexGuard};

use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::UploadLimits;
use suprnova_live::upload::{
    CleanupBatchRequest, CleanupClaim, CleanupCompletion, CleanupCompletionKind,
    CleanupLedgerDisposition, ConditionalTransition, ConditionalUploadCreate,
    TransitionDisposition, TransitionOutcome, UploadCleanupLedger, UploadCreateCommand,
    UploadError, UploadErrorKind, UploadFuture, UploadHandle, UploadLedger,
    UploadLedgerCreateOutcome, UploadRecord, UploadReplacementPolicy, UploadState,
    UploadStateMachine, UploadTransition,
};

struct StoredUpload {
    record: UploadRecord,
    create_key: suprnova_live::upload::UploadIdempotencyKey,
    declared_bytes: u64,
    policy: suprnova_live::upload::UploadFieldPolicy,
    creation_sequence: u64,
    machine: UploadStateMachine,
    transition_keys: HashSet<suprnova_live::upload::UploadIdempotencyKey>,
    retained_bytes: u64,
    cleanup_lease: Option<(
        suprnova_live::upload::CleanupLeaseId,
        suprnova_live::upload::UploadRevision,
        UnixMillis,
    )>,
    cleanup_guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    cleanup_retries: u32,
    cleanup_orphaned: bool,
    cleanup_at: Option<UnixMillis>,
}

struct CreationEvent {
    scope: suprnova_live::host::HostScopeFacts,
    admitted_at: UnixMillis,
}

#[derive(Default)]
struct State {
    records: HashMap<UploadHandle, StoredUpload>,
    creations: Vec<CreationEvent>,
    cleanup: BTreeMap<UnixMillis, BTreeSet<UploadHandle>>,
    next_creation_sequence: u64,
}

impl State {
    fn schedule(&mut self, handle: &UploadHandle, at: UnixMillis) {
        self.unschedule(handle);
        if let Some(stored) = self.records.get_mut(handle) {
            stored.cleanup_at = Some(at);
        }
        self.cleanup.entry(at).or_default().insert(handle.clone());
    }

    fn unschedule(&mut self, handle: &UploadHandle) {
        let previous = self
            .records
            .get_mut(handle)
            .and_then(|stored| stored.cleanup_at.take());
        let Some(previous) = previous else { return };
        let empty = self.cleanup.get_mut(&previous).is_some_and(|bucket| {
            bucket.remove(handle);
            bucket.is_empty()
        });
        if empty {
            self.cleanup.remove(&previous);
        }
    }

    fn pop_due(&mut self, now: UnixMillis) -> Option<UploadHandle> {
        let deadline = *self.cleanup.first_key_value()?.0;
        if deadline > now {
            return None;
        }
        let (handle, empty) = {
            let bucket = self.cleanup.get_mut(&deadline)?;
            let handle = bucket.pop_first()?;
            (handle, bucket.is_empty())
        };
        if empty {
            self.cleanup.remove(&deadline);
        }
        if let Some(stored) = self.records.get_mut(&handle) {
            stored.cleanup_at = None;
        }
        Some(handle)
    }
}

pub(crate) struct SuprnovaUploadLedger {
    limits: UploadLimits,
    operation_locks: Arc<super::super::upload::UploadOperationLocks>,
    state: Mutex<State>,
}

impl SuprnovaUploadLedger {
    pub(crate) fn new(
        limits: UploadLimits,
        operation_locks: Arc<super::super::upload::UploadOperationLocks>,
    ) -> Result<Self, UploadError> {
        NonZeroUsize::new(limits.max_idempotency_outcomes())
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            limits,
            operation_locks,
            state: Mutex::new(State::default()),
        })
    }
}

impl UploadLedger for SuprnovaUploadLedger {
    fn create<'a>(
        &'a self,
        request: UploadCreateCommand,
    ) -> UploadFuture<'a, Result<UploadLedgerCreateOutcome, UploadError>> {
        Box::pin(async move {
            if request.limits() != self.limits {
                return Err(UploadError::new(UploadErrorKind::LedgerUnavailable));
            }
            let mut state = lock(&self.state);
            let handle = request.record().authority().handle();
            if let Some(existing) = state.records.get(handle) {
                let exact = existing.create_key == *request.idempotency_key()
                    && existing.record.authority() == request.record().authority()
                    && existing.record.expires_at() == request.record().expires_at()
                    && existing.declared_bytes == request.declared_bytes()
                    && existing.policy == *request.policy();
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

            let cutoff = request
                .admitted_at()
                .get()
                .saturating_sub(request.limits().creation_window_ms());
            state
                .creations
                .retain(|event| event.admitted_at.get() > cutoff);
            let scope = request.record().authority().host_scope();
            if state
                .creations
                .iter()
                .filter(|event| &event.scope == scope)
                .count()
                >= request.limits().max_creations_per_window()
            {
                return Err(UploadError::new(UploadErrorKind::CreationRateExceeded));
            }
            let mut field_active = state
                .records
                .iter()
                .filter(|(_, existing)| {
                    !existing.record.state().is_terminal()
                        && existing.record.authority().host_scope() == scope
                        && existing.record.authority().component()
                            == request.record().authority().component()
                        && existing.record.authority().field()
                            == request.record().authority().field()
                })
                .map(|(handle, existing)| (existing.creation_sequence, handle.clone()))
                .collect::<Vec<_>>();
            field_active.sort_unstable_by_key(|(sequence, _)| *sequence);
            let retire_count = field_active
                .len()
                .saturating_add(1)
                .saturating_sub(request.policy().maximum_files());
            let retiring = if retire_count == 0 {
                HashSet::new()
            } else if request.policy().replacement() == UploadReplacementPolicy::RetirePrevious {
                field_active
                    .into_iter()
                    .take(retire_count)
                    .map(|(_, handle)| handle)
                    .collect::<HashSet<_>>()
            } else {
                return Err(UploadError::new(UploadErrorKind::FileCountExceeded));
            };
            let is_retiring = |handle: &UploadHandle| retiring.contains(handle);
            let pending = state.records.iter().filter(|(handle, existing)| {
                !existing.record.state().is_terminal()
                    && existing.record.authority().host_scope() == scope
                    && !is_retiring(handle)
            });
            if pending.clone().count() >= request.limits().max_pending_per_scope() {
                return Err(UploadError::new(UploadErrorKind::PendingLimitExceeded));
            }
            let _scope_bytes = state
                .records
                .iter()
                .filter(|(handle, existing)| {
                    !existing.record.state().is_terminal()
                        && existing.record.authority().host_scope() == scope
                        && !is_retiring(handle)
                })
                .try_fold(request.declared_bytes(), |total, (_, existing)| {
                    total.checked_add(existing.declared_bytes)
                })
                .filter(|total| *total <= request.limits().max_aggregate_bytes())
                .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            state
                .records
                .iter()
                .filter(|(handle, existing)| {
                    !existing.record.state().is_terminal() && !is_retiring(handle)
                })
                .try_fold(request.declared_bytes(), |total, (_, existing)| {
                    total.checked_add(existing.declared_bytes)
                })
                .filter(|total| *total <= request.limits().max_storage_bytes())
                .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;

            for retiring_handle in &retiring {
                let stored = state
                    .records
                    .get_mut(retiring_handle)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::LedgerUnavailable))?;
                let expired = stored
                    .machine
                    .expire_for_cleanup(stored.record.revision())?;
                stored.record = stored.record.with_outcome(expired)?;
            }
            for retiring_handle in retiring {
                state.schedule(&retiring_handle, request.admitted_at());
            }

            let record = request.record().clone();
            let machine = machine_for(&record, self.limits)?;
            let handle = record.authority().handle().clone();
            let creation_sequence = state.next_creation_sequence;
            state.next_creation_sequence = state.next_creation_sequence.saturating_add(1);
            state.creations.push(CreationEvent {
                scope: scope.clone(),
                admitted_at: request.admitted_at(),
            });
            state.records.insert(
                handle.clone(),
                StoredUpload {
                    record: record.clone(),
                    create_key: request.idempotency_key().clone(),
                    declared_bytes: request.declared_bytes(),
                    policy: request.policy().clone(),
                    creation_sequence,
                    machine,
                    transition_keys: HashSet::new(),
                    retained_bytes: 0,
                    cleanup_lease: None,
                    cleanup_guard: None,
                    cleanup_retries: 0,
                    cleanup_orphaned: false,
                    cleanup_at: None,
                },
            );
            state.schedule(&handle, record.expires_at());
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
            let mut state = lock(&self.state);
            let handle = request.transition().handle().clone();
            let (outcome, cleanup_at) = {
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
                let retained = match request.transition().transition() {
                    UploadTransition::PutChunk(chunk)
                        if !stored
                            .transition_keys
                            .contains(request.transition().idempotency_key()) =>
                    {
                        Some(
                            stored
                                .retained_bytes
                                .checked_add(chunk.size())
                                .filter(|bytes| *bytes <= self.limits.max_file_bytes())
                                .ok_or_else(|| {
                                    UploadError::new(UploadErrorKind::ResourceExhausted)
                                })?,
                        )
                    }
                    _ => None,
                };
                let outcome = stored.machine.apply(request.transition().clone())?;
                if outcome.disposition() == TransitionDisposition::Applied {
                    stored.record = stored.record.with_outcome(outcome)?;
                    if let Some(retained) = retained {
                        stored.retained_bytes = retained;
                    }
                    stored
                        .transition_keys
                        .insert(request.transition().idempotency_key().clone());
                    (
                        outcome,
                        cleanup_deadline(&stored.record, request.admitted_at()),
                    )
                } else {
                    (outcome, stored.cleanup_at)
                }
            };
            match cleanup_at {
                Some(at) => state.schedule(&handle, at),
                None => state.unschedule(&handle),
            }
            Ok(outcome)
        })
    }
}

impl UploadCleanupLedger for SuprnovaUploadLedger {
    fn claim_cleanup<'a>(
        &'a self,
        request: CleanupBatchRequest,
    ) -> UploadFuture<'a, Result<Vec<CleanupClaim>, UploadError>> {
        Box::pin(async move {
            let mut state = lock(&self.state);
            let mut claims = Vec::with_capacity(request.max_items());
            let mut claimed_bytes = 0_u64;
            while claims.len() < request.max_items() {
                let Some(handle) = state.pop_due(request.now()) else {
                    break;
                };
                if let Some(stored) = state.records.get_mut(&handle)
                    && stored
                        .cleanup_lease
                        .as_ref()
                        .is_some_and(|lease| lease.2 <= request.now())
                {
                    stored.cleanup_lease = None;
                    stored.cleanup_guard = None;
                }
                let Some(cleanup_guard) = self.operation_locks.try_acquire(&handle) else {
                    state.schedule(
                        &handle,
                        UnixMillis::new(request.now().get().saturating_add(100)),
                    );
                    continue;
                };
                let (future_lease, eligible, expires_at, retained_bytes) = {
                    let stored = state
                        .records
                        .get(&handle)
                        .ok_or_else(|| UploadError::new(UploadErrorKind::LedgerUnavailable))?;
                    (
                        stored
                            .cleanup_lease
                            .as_ref()
                            .filter(|lease| lease.2 > request.now())
                            .map(|lease| lease.2),
                        cleanup_eligible(&stored.record, request.now()),
                        stored.record.expires_at(),
                        stored.retained_bytes,
                    )
                };
                if let Some(at) = future_lease {
                    state.schedule(&handle, at);
                    continue;
                }
                if !eligible {
                    state.schedule(&handle, expires_at);
                    continue;
                }
                let Some(total) = claimed_bytes.checked_add(retained_bytes) else {
                    state.schedule(&handle, request.now());
                    break;
                };
                if total > request.max_bytes() {
                    state.schedule(&handle, request.now());
                    break;
                }
                let stored = state
                    .records
                    .get_mut(&handle)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::LedgerUnavailable))?;
                if !stored.record.state().is_terminal() {
                    let expired = stored
                        .machine
                        .expire_for_cleanup(stored.record.revision())?;
                    stored.record = stored.record.with_outcome(expired)?;
                }
                let claim = CleanupClaim::from_store(
                    &stored.record,
                    stored.retained_bytes,
                    request.lease_id().clone(),
                    request.lease_expires_at(),
                    stored.cleanup_retries,
                    stored.cleanup_orphaned,
                )?;
                stored.cleanup_lease = Some((
                    request.lease_id().clone(),
                    stored.record.revision(),
                    request.lease_expires_at(),
                ));
                stored.cleanup_guard = Some(cleanup_guard);
                state.schedule(&handle, request.lease_expires_at());
                claimed_bytes = total;
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
            let current = stored.cleanup_lease.as_ref().is_some_and(|lease| {
                lease.0 == *completion.lease_id()
                    && lease.1 == completion.revision()
                    && stored.record.revision() == completion.revision()
                    && completion.completed_at() < lease.2
            });
            if !current {
                return Ok(CleanupLedgerDisposition::Stale);
            }
            match completion.kind() {
                CleanupCompletionKind::Reclaimed => {
                    state.unschedule(completion.handle());
                    state.records.remove(completion.handle());
                }
                CleanupCompletionKind::Retry { retry_at, orphaned } => {
                    let stored = state.records.get_mut(completion.handle()).expect("present");
                    stored.cleanup_lease = None;
                    stored.cleanup_guard = None;
                    stored.cleanup_retries = stored.cleanup_retries.saturating_add(1);
                    stored.cleanup_orphaned |= orphaned;
                    state.schedule(completion.handle(), retry_at);
                }
                CleanupCompletionKind::Deferred { retry_at } => {
                    let stored = state.records.get_mut(completion.handle()).expect("present");
                    stored.cleanup_lease = None;
                    stored.cleanup_guard = None;
                    state.schedule(completion.handle(), retry_at);
                }
            }
            Ok(CleanupLedgerDisposition::Applied)
        })
    }
}

fn machine_for(
    record: &UploadRecord,
    limits: UploadLimits,
) -> Result<UploadStateMachine, UploadError> {
    let outcomes = NonZeroUsize::new(limits.max_idempotency_outcomes())
        .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
    UploadStateMachine::with_outcome_limit(
        record.authority().handle().clone(),
        record.state(),
        record.revision(),
        outcomes,
    )
}

fn cleanup_eligible(record: &UploadRecord, now: UnixMillis) -> bool {
    record.state().is_terminal() || record.expires_at() <= now
}

fn cleanup_deadline(record: &UploadRecord, now: UnixMillis) -> Option<UnixMillis> {
    match record.state() {
        UploadState::Created
        | UploadState::Queued
        | UploadState::Transferring
        | UploadState::Verifying
        | UploadState::Ready => Some(record.expires_at()),
        UploadState::Rejected
        | UploadState::Canceled
        | UploadState::Expired
        | UploadState::Failed => Some(now),
        UploadState::Finalizing | UploadState::Finalized => None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
