//! Shared deterministic values for Tier 0 ledger integration tests.

#![allow(
    dead_code,
    reason = "shared helpers are used by separate integration-test crates"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::identity::{
    ContentDigest, IdempotencyKey, InstanceId, Revision, ScopeFingerprint, UnixMillis,
};
use suprnova_live::ledger::{
    LedgerLimits, LiveInstanceLedger, MemoryInstanceLedger, PromotionOutcome, PromotionRecord,
};

pub(crate) fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

pub(crate) fn scope(start: u8) -> ScopeFingerprint {
    ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("scope is valid")
}

pub(crate) fn instance(start: u8) -> InstanceId {
    InstanceId::from_bytes(&bytes::<16>(start)).expect("instance is valid")
}

pub(crate) fn idempotency(start: u8) -> IdempotencyKey {
    IdempotencyKey::from_bytes(&bytes::<16>(start)).expect("idempotency key is valid")
}

pub(crate) fn digest(start: u8) -> ContentDigest {
    ContentDigest::from_bytes(&bytes::<32>(start)).expect("digest is valid")
}

#[derive(Debug)]
pub(crate) struct ManualClock {
    now: AtomicU64,
}

impl ManualClock {
    pub(crate) fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }

    pub(crate) fn set(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Ok(UnixMillis::new(self.now.load(Ordering::SeqCst)))
    }
}

pub(crate) fn limits(max_outcomes: usize) -> LedgerLimits {
    LedgerLimits::new(100, 10_000, max_outcomes, 64).expect("ledger limits are valid")
}

pub(crate) fn ledger(clock: Arc<ManualClock>, max_outcomes: usize) -> MemoryInstanceLedger {
    MemoryInstanceLedger::new(clock, limits(max_outcomes))
}

pub(crate) fn promotion(scope: ScopeFingerprint, instance_id: InstanceId) -> PromotionRecord {
    PromotionRecord::new(
        scope,
        instance_id,
        idempotency(0x40),
        digest(0x50),
        Revision::new(0),
        UnixMillis::new(5_000),
    )
}

pub(crate) async fn promote_default(
    ledger: &MemoryInstanceLedger,
    scope: ScopeFingerprint,
    instance_id: InstanceId,
) {
    let outcome = ledger
        .promote(promotion(scope, instance_id))
        .await
        .expect("promotion succeeds");
    assert!(matches!(outcome, PromotionOutcome::Created(_)));
}
