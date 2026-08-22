//! Private in-memory representation for the Tier 0 ledger.

use std::collections::{BTreeMap, HashMap, VecDeque};

use super::{AcceptedOutcomeMetadata, LedgerPhase, RefreshReason};
use crate::identity::{
    ContentDigest, IdempotencyKey, InstanceId, Revision, ScopeFingerprint, UnixMillis,
};

#[derive(Clone, Eq, Hash, PartialEq)]
pub(crate) struct InstanceKey {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance_id: InstanceId,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub(crate) struct PromotionKey {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) idempotency_key: IdempotencyKey,
}

pub(crate) struct PromotionReservation {
    pub(crate) request_digest: ContentDigest,
    pub(crate) instance_id: InstanceId,
    pub(crate) initial_revision: Revision,
    pub(crate) expires_at: UnixMillis,
}

pub(crate) struct PendingClaim {
    pub(crate) claim_id: u64,
    pub(crate) base_revision: Revision,
    pub(crate) successor_revision: Revision,
    pub(crate) idempotency_key: IdempotencyKey,
    pub(crate) request_digest: ContentDigest,
    pub(crate) lease_expires_at: UnixMillis,
}

pub(crate) enum InstancePhase {
    Ready,
    Pending(PendingClaim),
    Consumed {
        reason: RefreshReason,
        claim_id: Option<u64>,
    },
}

impl InstancePhase {
    pub(crate) const fn public_phase(&self) -> LedgerPhase {
        match self {
            Self::Ready => LedgerPhase::Ready,
            Self::Pending(_) => LedgerPhase::Pending,
            Self::Consumed { .. } => LedgerPhase::Consumed,
        }
    }
}

pub(crate) struct InstanceRecord {
    #[allow(
        dead_code,
        reason = "retained authority metadata is consumed by later provider adapters"
    )]
    pub(crate) component_contract: Option<ContentDigest>,
    pub(crate) current_revision: Revision,
    pub(crate) expires_at: UnixMillis,
    pub(crate) phase: InstancePhase,
    pub(crate) accepted: VecDeque<AcceptedOutcomeMetadata>,
}

const MAX_EXPIRED_EVENTS_PER_PRUNE: usize = 64;

struct LedgerExpiry {
    instance_key: InstanceKey,
    promotion_key: Option<PromotionKey>,
}

pub(crate) struct MemoryState {
    pub(crate) instances: HashMap<InstanceKey, InstanceRecord>,
    pub(crate) promotions: HashMap<PromotionKey, PromotionReservation>,
    pub(crate) next_claim_id: u64,
    expiry_deadlines: BTreeMap<UnixMillis, Vec<LedgerExpiry>>,
}

impl MemoryState {
    pub(crate) fn new() -> Self {
        Self {
            instances: HashMap::new(),
            promotions: HashMap::new(),
            next_claim_id: 1,
            expiry_deadlines: BTreeMap::new(),
        }
    }

    pub(crate) fn prune_expired(&mut self, now: UnixMillis) {
        for _ in 0..MAX_EXPIRED_EVENTS_PER_PRUNE {
            let Some((deadline, expiry)) = self.pop_due_expiry(now) else {
                break;
            };
            if self
                .instances
                .get(&expiry.instance_key)
                .is_some_and(|record| record.expires_at == deadline)
            {
                self.instances.remove(&expiry.instance_key);
            }
            if let Some(promotion_key) = expiry.promotion_key
                && self
                    .promotions
                    .get(&promotion_key)
                    .is_some_and(|reservation| reservation.expires_at == deadline)
            {
                self.promotions.remove(&promotion_key);
            }
        }
    }

    pub(crate) fn prune_instance(&mut self, key: &InstanceKey, now: UnixMillis) -> bool {
        if self
            .instances
            .get(key)
            .is_some_and(|record| record.expires_at <= now)
        {
            self.instances.remove(key);
            return true;
        }
        false
    }

    pub(crate) fn prune_promotion(&mut self, key: &PromotionKey, now: UnixMillis) {
        if self
            .promotions
            .get(key)
            .is_some_and(|reservation| reservation.expires_at <= now)
        {
            self.promotions.remove(key);
        }
    }

    pub(crate) fn schedule_expiry(
        &mut self,
        deadline: UnixMillis,
        instance_key: InstanceKey,
        promotion_key: PromotionKey,
    ) {
        self.expiry_deadlines
            .entry(deadline)
            .or_default()
            .push(LedgerExpiry {
                instance_key,
                promotion_key: Some(promotion_key),
            });
    }

    pub(crate) fn schedule_instance_expiry(
        &mut self,
        deadline: UnixMillis,
        instance_key: InstanceKey,
    ) {
        self.expiry_deadlines
            .entry(deadline)
            .or_default()
            .push(LedgerExpiry {
                instance_key,
                promotion_key: None,
            });
    }

    fn pop_due_expiry(&mut self, now: UnixMillis) -> Option<(UnixMillis, LedgerExpiry)> {
        loop {
            let mut entry = self.expiry_deadlines.first_entry()?;
            let deadline = *entry.key();
            if deadline > now {
                return None;
            }
            let expiry = entry.get_mut().pop();
            if entry.get().is_empty() {
                entry.remove();
            }
            if let Some(expiry) = expiry {
                return Some((deadline, expiry));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes<const N: usize>(start: u8) -> [u8; N] {
        std::array::from_fn(|offset| start.wrapping_add(offset as u8))
    }

    #[test]
    fn expiry_cleanup_is_bounded_per_state_operation() {
        let mut state = MemoryState::new();
        for start in 0..66_u8 {
            let scope =
                ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("test scope is valid");
            let instance_key = InstanceKey {
                scope: scope.clone(),
                instance_id: InstanceId::from_bytes(&bytes::<16>(start))
                    .expect("test instance is valid"),
            };
            let promotion_key = PromotionKey {
                scope,
                idempotency_key: IdempotencyKey::from_bytes(&bytes::<16>(start))
                    .expect("test idempotency key is valid"),
            };
            state.instances.insert(
                instance_key.clone(),
                InstanceRecord {
                    component_contract: None,
                    current_revision: Revision::new(0),
                    expires_at: UnixMillis::new(200),
                    phase: InstancePhase::Ready,
                    accepted: VecDeque::new(),
                },
            );
            state.promotions.insert(
                promotion_key.clone(),
                PromotionReservation {
                    request_digest: ContentDigest::from_bytes(&bytes::<32>(start.wrapping_add(1)))
                        .expect("test digest is valid"),
                    instance_id: instance_key.instance_id.clone(),
                    initial_revision: Revision::new(0),
                    expires_at: UnixMillis::new(200),
                },
            );
            state.schedule_expiry(UnixMillis::new(200), instance_key, promotion_key);
        }

        state.prune_expired(UnixMillis::new(201));

        assert_eq!(state.instances.len(), 2);
        assert_eq!(state.promotions.len(), 2);
    }
}
