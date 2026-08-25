//! Deterministic upload authority controls and the complete Tier 0 ledger.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::UploadLimits;
use suprnova_live::upload::{
    ConditionalTransition, ConditionalUploadCreate, TransitionDisposition, TransitionOutcome,
    UploadAuthorizationDecision, UploadAuthorizationPort, UploadAuthorizationRequest,
    UploadControlKind, UploadCreateCommand, UploadError, UploadErrorKind, UploadFuture,
    UploadHandle, UploadLedger, UploadLedgerCreateOutcome, UploadRecord, UploadStateMachine,
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
}

struct CreationEvent {
    scope: suprnova_live::host::HostScopeFacts,
    admitted_at: UnixMillis,
}

#[derive(Default)]
struct MemoryUploadState {
    records: HashMap<UploadHandle, StoredUpload>,
    creations: Vec<CreationEvent>,
}

/// Complete daemon-free reference implementation of conditional upload authority.
pub struct MemoryUploadLedger {
    limits: UploadLimits,
    state: Mutex<MemoryUploadState>,
    fail_transition: AtomicBool,
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
        state.records.insert(
            record.authority().handle().clone(),
            StoredUpload {
                record,
                create_key: suprnova_live::upload::UploadIdempotencyKey::parse("seeded")?,
                machine,
            },
        );
        Ok(())
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
            state.records.insert(
                record.authority().handle().clone(),
                StoredUpload {
                    record: record.clone(),
                    create_key: request.idempotency_key().clone(),
                    machine,
                },
            );
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
            let stored = state
                .records
                .get_mut(request.transition().handle())
                .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
            if stored.record.authority() != request.authority() {
                return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
            }
            if stored.record.expires_at() <= request.admitted_at() {
                return Err(UploadError::new(UploadErrorKind::UploadExpired));
            }
            let outcome = stored.machine.apply(request.transition().clone())?;
            if outcome.disposition() == TransitionDisposition::Applied {
                stored.record = stored.record.with_outcome(outcome)?;
            }
            Ok(outcome)
        })
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
