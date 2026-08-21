//! Private in-memory representation for the Tier 0 ledger.

use std::collections::{HashMap, VecDeque};

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
    pub(crate) current_revision: Revision,
    pub(crate) expires_at: UnixMillis,
    pub(crate) phase: InstancePhase,
    pub(crate) accepted: VecDeque<AcceptedOutcomeMetadata>,
}

pub(crate) struct MemoryState {
    pub(crate) instances: HashMap<InstanceKey, InstanceRecord>,
    pub(crate) promotions: HashMap<PromotionKey, PromotionReservation>,
    pub(crate) next_claim_id: u64,
}

impl MemoryState {
    pub(crate) fn new() -> Self {
        Self {
            instances: HashMap::new(),
            promotions: HashMap::new(),
            next_claim_id: 1,
        }
    }

    pub(crate) fn prune_expired(&mut self, now: UnixMillis) {
        self.instances.retain(|_, record| record.expires_at > now);
        self.promotions
            .retain(|_, reservation| reservation.expires_at > now);
    }
}
