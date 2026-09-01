//! Complete single-process Tier 0 instance-ledger provider.

use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use super::state::{
    InstanceKey, InstancePhase, InstanceRecord, MemoryState, PendingClaim, PromotionKey,
    PromotionReservation,
};
use super::{
    AcceptedOutcome, AcceptedOutcomeMetadata, ClaimGrant, ClaimOutcome, ClaimRequest, ClaimToken,
    InstanceAuthority, LedgerError, LedgerErrorKind, LedgerInspection, LedgerLimits,
    LiveInstanceLedger, MountInstanceRecord, PromotionOutcome, PromotionRecord, RefreshReason,
};
use crate::clock::Clock;
use crate::identity::{InstanceId, Revision, ScopeFingerprint, UnixMillis};

/// Complete zero-daemon instance revision authority for one application process.
#[derive(Clone)]
pub struct MemoryInstanceLedger {
    clock: Arc<dyn Clock>,
    limits: LedgerLimits,
    provider_identity: Arc<()>,
    state: Arc<Mutex<MemoryState>>,
}

impl MemoryInstanceLedger {
    /// Creates an empty bounded provider using the injected wall clock.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, limits: LedgerLimits) -> Self {
        Self {
            clock,
            limits,
            provider_identity: Arc::new(()),
            state: Arc::new(Mutex::new(MemoryState::new())),
        }
    }

    /// Returns metadata-only provider inspection for tests and trusted diagnostics.
    pub fn inspect(
        &self,
        scope: &crate::identity::ScopeFingerprint,
        instance_id: &crate::identity::InstanceId,
    ) -> Result<Option<LedgerInspection>, LedgerError> {
        let now = self.now()?;
        let key = InstanceKey {
            scope: scope.clone(),
            instance_id: instance_id.clone(),
        };
        let mut state = self.lock()?;
        if state.prune_instance(&key, now) {
            return Ok(None);
        }
        state.prune_expired(now);
        let record = match state.instances.get_mut(&key) {
            Some(record) => record,
            None => return Ok(None),
        };
        expire_pending(record, now);
        Ok(Some(LedgerInspection {
            current_revision: record.current_revision,
            accepted_outcome_count: record.accepted.len(),
            phase: record.phase.public_phase(),
        }))
    }

    fn now(&self) -> Result<UnixMillis, LedgerError> {
        self.clock
            .now()
            .map_err(|_| LedgerError::new(LedgerErrorKind::ClockUnavailable))
    }

    fn lock(&self) -> Result<MutexGuard<'_, MemoryState>, LedgerError> {
        self.state
            .lock()
            .map_err(|_| LedgerError::new(LedgerErrorKind::ProviderUnavailable))
    }
}

#[async_trait]
impl LiveInstanceLedger for MemoryInstanceLedger {
    async fn mount_instance(
        &self,
        record: MountInstanceRecord,
    ) -> Result<InstanceAuthority, LedgerError> {
        let now = self.now()?;
        validate_expiry(now, record.expires_at, self.limits)?;
        let key = InstanceKey {
            scope: record.scope.clone(),
            instance_id: record.instance_id.clone(),
        };
        let mut state = self.lock()?;
        state.prune_expired(now);
        state.prune_instance(&key, now);
        if state.instances.contains_key(&key) {
            return Err(LedgerError::new(LedgerErrorKind::InstanceConflict));
        }
        if state.instances.len() >= self.limits.max_instances() {
            return Err(LedgerError::new(LedgerErrorKind::CapacityExceeded));
        }

        let authority = InstanceAuthority::new(
            record.instance_id,
            record.initial_revision,
            record.expires_at,
        );
        state.instances.insert(
            key.clone(),
            InstanceRecord {
                component_contract: Some(record.component_contract),
                current_revision: record.initial_revision,
                expires_at: record.expires_at,
                phase: InstancePhase::Ready,
                accepted: std::collections::VecDeque::new(),
            },
        );
        state.schedule_instance_expiry(record.expires_at, key);
        Ok(authority)
    }

    async fn promote(&self, request: PromotionRecord) -> Result<PromotionOutcome, LedgerError> {
        let now = self.now()?;
        validate_expiry(now, request.expires_at, self.limits)?;
        let mut state = self.lock()?;
        state.prune_expired(now);

        let promotion_key = PromotionKey {
            scope: request.scope.clone(),
            idempotency_key: request.idempotency_key.clone(),
        };
        state.prune_promotion(&promotion_key, now);
        if let Some(existing) = state.promotions.get(&promotion_key) {
            if existing.request_digest == request.request_digest {
                return Ok(PromotionOutcome::Existing(InstanceAuthority::new(
                    existing.instance_id.clone(),
                    existing.initial_revision,
                    existing.expires_at,
                )));
            }
            return Ok(PromotionOutcome::IdempotencyConflict);
        }

        let instance_key = InstanceKey {
            scope: request.scope.clone(),
            instance_id: request.instance_id.clone(),
        };
        state.prune_instance(&instance_key, now);
        if state.instances.contains_key(&instance_key) {
            return Err(LedgerError::new(LedgerErrorKind::InstanceConflict));
        }
        if state.instances.len() >= self.limits.max_instances() {
            return Err(LedgerError::new(LedgerErrorKind::CapacityExceeded));
        }

        let authority = InstanceAuthority::new(
            request.instance_id.clone(),
            request.initial_revision,
            request.expires_at,
        );
        state.instances.insert(
            instance_key.clone(),
            InstanceRecord {
                component_contract: None,
                current_revision: request.initial_revision,
                expires_at: request.expires_at,
                phase: InstancePhase::Ready,
                accepted: std::collections::VecDeque::new(),
            },
        );
        state.promotions.insert(
            promotion_key.clone(),
            PromotionReservation {
                request_digest: request.request_digest,
                instance_id: request.instance_id,
                initial_revision: request.initial_revision,
                expires_at: request.expires_at,
            },
        );
        state.schedule_expiry(request.expires_at, instance_key, promotion_key);
        Ok(PromotionOutcome::Created(authority))
    }

    async fn claim(&self, request: ClaimRequest) -> Result<ClaimOutcome, LedgerError> {
        let now = self.now()?;
        let key = InstanceKey {
            scope: request.scope.clone(),
            instance_id: request.instance_id.clone(),
        };
        let mut state = self.lock()?;
        if state.prune_instance(&key, now) {
            return Ok(ClaimOutcome::RefreshRequired(
                RefreshReason::InstanceExpired,
            ));
        }
        state.prune_expired(now);

        let record = match state.instances.get_mut(&key) {
            Some(record) => record,
            None => return Ok(ClaimOutcome::RefreshRequired(RefreshReason::Missing)),
        };
        if expire_pending(record, now) {
            return Ok(ClaimOutcome::RefreshRequired(RefreshReason::ClaimExpired));
        }
        if let InstancePhase::Consumed { reason, .. } = record.phase {
            return Ok(ClaimOutcome::RefreshRequired(reason));
        }

        if let Some(accepted) = record
            .accepted
            .iter()
            .find(|accepted| accepted.base_revision == request.base_revision)
            && accepted.idempotency_key == request.idempotency_key
        {
            return if accepted.request_digest == request.request_digest {
                Ok(ClaimOutcome::Accepted(accepted.clone()))
            } else {
                Ok(ClaimOutcome::IdempotencyConflict)
            };
        }

        if let InstancePhase::Pending(pending) = &record.phase {
            if pending.base_revision == request.base_revision {
                return if pending.idempotency_key == request.idempotency_key
                    && pending.request_digest == request.request_digest
                {
                    Ok(ClaimOutcome::InProgress {
                        successor_revision: pending.successor_revision,
                    })
                } else {
                    Ok(ClaimOutcome::IdempotencyConflict)
                };
            }
            return Ok(ClaimOutcome::Stale {
                current_revision: record.current_revision,
            });
        }

        if record.current_revision != request.base_revision {
            return Ok(ClaimOutcome::Stale {
                current_revision: record.current_revision,
            });
        }

        let successor_revision = match request.base_revision.checked_next() {
            Ok(revision) => revision,
            Err(_) => {
                record.phase = InstancePhase::Consumed {
                    reason: RefreshReason::RevisionExhausted,
                    claim_id: None,
                };
                return Ok(ClaimOutcome::RefreshRequired(
                    RefreshReason::RevisionExhausted,
                ));
            }
        };
        let lease_expires_at = now
            .get()
            .checked_add(self.limits.claim_lease_ms())
            .map(UnixMillis::new)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::CounterExhausted))?;
        let claim_id = state.next_claim_id;
        state.next_claim_id = state
            .next_claim_id
            .checked_add(1)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::CounterExhausted))?;

        let record = state
            .instances
            .get_mut(&key)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::ProviderUnavailable))?;
        record.current_revision = successor_revision;
        record.phase = InstancePhase::Pending(PendingClaim {
            claim_id,
            base_revision: request.base_revision,
            successor_revision,
            idempotency_key: request.idempotency_key,
            request_digest: request.request_digest,
            lease_expires_at,
        });
        Ok(ClaimOutcome::Granted(ClaimGrant::new(
            ClaimToken {
                provider_identity: self.provider_identity.clone(),
                scope: request.scope,
                instance_id: request.instance_id,
                claim_id,
            },
            successor_revision,
        )))
    }

    async fn current_accepted_revision(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
    ) -> Result<Option<Revision>, LedgerError> {
        let now = self.now()?;
        let key = InstanceKey {
            scope: scope.clone(),
            instance_id: instance_id.clone(),
        };
        let mut state = self.lock()?;
        if state.prune_instance(&key, now) {
            return Ok(None);
        }
        state.prune_expired(now);
        let Some(record) = state.instances.get_mut(&key) else {
            return Ok(None);
        };
        expire_pending(record, now);
        match &record.phase {
            InstancePhase::Ready => Ok(Some(record.current_revision)),
            InstancePhase::Pending(pending) => Ok(Some(pending.base_revision)),
            InstancePhase::Consumed { .. } => Ok(None),
        }
    }

    async fn commit(
        &self,
        claim: &ClaimToken,
        outcome: AcceptedOutcome,
    ) -> Result<(), LedgerError> {
        if !Arc::ptr_eq(&self.provider_identity, &claim.provider_identity) {
            return Err(LedgerError::new(LedgerErrorKind::ClaimMismatch));
        }
        let now = self.now()?;
        let key = InstanceKey {
            scope: claim.scope.clone(),
            instance_id: claim.instance_id.clone(),
        };
        let mut state = self.lock()?;
        if state.prune_instance(&key, now) {
            return Err(LedgerError::new(LedgerErrorKind::InstanceExpired));
        }
        let record = state
            .instances
            .get_mut(&key)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::ClaimMismatch))?;

        let metadata = match &record.phase {
            InstancePhase::Pending(pending) if pending.claim_id == claim.claim_id => {
                if pending.lease_expires_at <= now {
                    record.phase = InstancePhase::Consumed {
                        reason: RefreshReason::ClaimExpired,
                        claim_id: Some(claim.claim_id),
                    };
                    return Err(LedgerError::new(LedgerErrorKind::ClaimExpired));
                }
                AcceptedOutcomeMetadata {
                    scope: key.scope.clone(),
                    instance_id: key.instance_id.clone(),
                    base_revision: pending.base_revision,
                    successor_revision: pending.successor_revision,
                    idempotency_key: pending.idempotency_key.clone(),
                    request_digest: pending.request_digest.clone(),
                    outcome,
                }
            }
            InstancePhase::Consumed {
                reason: RefreshReason::ClaimExpired,
                claim_id: Some(expired_claim_id),
            } if *expired_claim_id == claim.claim_id => {
                return Err(LedgerError::new(LedgerErrorKind::ClaimExpired));
            }
            _ => return Err(LedgerError::new(LedgerErrorKind::ClaimMismatch)),
        };
        record.phase = InstancePhase::Ready;
        record.accepted.push_back(metadata);
        while record.accepted.len() > self.limits.max_accepted_outcomes() {
            record.accepted.pop_front();
        }
        Ok(())
    }

    async fn abandon(&self, claim: &ClaimToken) -> Result<(), LedgerError> {
        self.abandon_claim(claim)
    }

    fn abandon_on_drop(&self, claim: ClaimToken) {
        let _ = self.release_claim(&claim);
    }

    fn fence_on_drop(&self, claim: ClaimToken) {
        let _ = self.fence_claim(&claim);
    }
}

impl MemoryInstanceLedger {
    fn release_claim(&self, claim: &ClaimToken) -> Result<(), LedgerError> {
        if !Arc::ptr_eq(&self.provider_identity, &claim.provider_identity) {
            return Err(LedgerError::new(LedgerErrorKind::ClaimMismatch));
        }
        let now = self.now()?;
        let key = InstanceKey {
            scope: claim.scope.clone(),
            instance_id: claim.instance_id.clone(),
        };
        let mut state = self.lock()?;
        if state.prune_instance(&key, now) {
            return Err(LedgerError::new(LedgerErrorKind::InstanceExpired));
        }
        let record = state
            .instances
            .get_mut(&key)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::ClaimMismatch))?;
        match &record.phase {
            InstancePhase::Pending(pending) if pending.claim_id == claim.claim_id => {
                if pending.lease_expires_at <= now {
                    record.phase = InstancePhase::Consumed {
                        reason: RefreshReason::ClaimExpired,
                        claim_id: Some(claim.claim_id),
                    };
                    return Err(LedgerError::new(LedgerErrorKind::ClaimExpired));
                }
                record.current_revision = pending.base_revision;
                record.phase = InstancePhase::Ready;
                Ok(())
            }
            InstancePhase::Consumed {
                reason: RefreshReason::ClaimExpired,
                claim_id: Some(expired_claim_id),
            } if *expired_claim_id == claim.claim_id => {
                Err(LedgerError::new(LedgerErrorKind::ClaimExpired))
            }
            _ => Err(LedgerError::new(LedgerErrorKind::ClaimMismatch)),
        }
    }

    fn abandon_claim(&self, claim: &ClaimToken) -> Result<(), LedgerError> {
        if !Arc::ptr_eq(&self.provider_identity, &claim.provider_identity) {
            return Err(LedgerError::new(LedgerErrorKind::ClaimMismatch));
        }
        let now = self.now()?;
        let key = InstanceKey {
            scope: claim.scope.clone(),
            instance_id: claim.instance_id.clone(),
        };
        let mut state = self.lock()?;
        if state.prune_instance(&key, now) {
            return Err(LedgerError::new(LedgerErrorKind::InstanceExpired));
        }
        let record = state
            .instances
            .get_mut(&key)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::ClaimMismatch))?;
        match &record.phase {
            InstancePhase::Pending(pending) if pending.claim_id == claim.claim_id => {
                let expired = pending.lease_expires_at <= now;
                record.phase = InstancePhase::Consumed {
                    reason: if expired {
                        RefreshReason::ClaimExpired
                    } else {
                        RefreshReason::Consumed
                    },
                    claim_id: Some(claim.claim_id),
                };
                if expired {
                    return Err(LedgerError::new(LedgerErrorKind::ClaimExpired));
                }
                Ok(())
            }
            InstancePhase::Consumed {
                reason: RefreshReason::ClaimExpired,
                claim_id: Some(expired_claim_id),
            } if *expired_claim_id == claim.claim_id => {
                Err(LedgerError::new(LedgerErrorKind::ClaimExpired))
            }
            _ => Err(LedgerError::new(LedgerErrorKind::ClaimMismatch)),
        }
    }

    fn fence_claim(&self, claim: &ClaimToken) -> Result<(), LedgerError> {
        if !Arc::ptr_eq(&self.provider_identity, &claim.provider_identity) {
            return Err(LedgerError::new(LedgerErrorKind::ClaimMismatch));
        }
        let key = InstanceKey {
            scope: claim.scope.clone(),
            instance_id: claim.instance_id.clone(),
        };
        let mut state = self.lock()?;
        let record = state
            .instances
            .get_mut(&key)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::ClaimMismatch))?;
        match &record.phase {
            InstancePhase::Pending(pending) if pending.claim_id == claim.claim_id => {
                record.phase = InstancePhase::Consumed {
                    reason: RefreshReason::Consumed,
                    claim_id: Some(claim.claim_id),
                };
                Ok(())
            }
            InstancePhase::Ready
                if record.accepted.back().is_some_and(|accepted| {
                    accepted.successor_revision == record.current_revision
                }) =>
            {
                Ok(())
            }
            InstancePhase::Consumed { claim_id, .. }
                if claim_id.is_none_or(|claim_id| claim_id == claim.claim_id) =>
            {
                Ok(())
            }
            _ => Err(LedgerError::new(LedgerErrorKind::ClaimMismatch)),
        }
    }
}

fn validate_expiry(
    now: UnixMillis,
    expires_at: UnixMillis,
    limits: LedgerLimits,
) -> Result<(), LedgerError> {
    let lifetime = expires_at
        .get()
        .checked_sub(now.get())
        .filter(|lifetime| *lifetime > 0)
        .ok_or_else(|| LedgerError::new(LedgerErrorKind::InvalidExpiry))?;
    if lifetime > limits.max_instance_lifetime_ms() {
        return Err(LedgerError::new(LedgerErrorKind::InvalidExpiry));
    }
    Ok(())
}

fn expire_pending(record: &mut InstanceRecord, now: UnixMillis) -> bool {
    let expired = matches!(
        &record.phase,
        InstancePhase::Pending(pending) if pending.lease_expires_at <= now
    );
    if expired {
        let claim_id = match &record.phase {
            InstancePhase::Pending(pending) => Some(pending.claim_id),
            _ => None,
        };
        record.phase = InstancePhase::Consumed {
            reason: RefreshReason::ClaimExpired,
            claim_id,
        };
    }
    expired
}
