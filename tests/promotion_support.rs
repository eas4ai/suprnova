//! Shared deterministic values for seed-promotion integration tests.

#![allow(
    dead_code,
    reason = "shared helpers are used by separate integration-test crates"
)]
#![allow(
    unused_imports,
    reason = "shared re-exports vary across separate integration-test crates"
)]

#[path = "ledger_support.rs"]
mod ledger_support;
#[path = "snapshot_support.rs"]
pub(crate) mod snapshot_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

pub(crate) use ledger_support::{ManualClock, digest, idempotency, instance, scope};
use snapshot_support::{expected_seed, key_ring, schema_set, seed_fields, snapshot_limits};
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::identity::{BrowserNonce, InstanceId, UnixMillis};
use suprnova_live::ledger::{LedgerLimits, MemoryInstanceLedger};
use suprnova_live::promotion::{
    InstanceIdGenerator, PromotionAttestations, PromotionLimitConfig, PromotionLimits,
    PromotionService, RandomError, TrustedPromotionContext,
};
use suprnova_live::snapshot::{SeedBodyV1, SnapshotLimits};

pub(crate) fn nonce(start: u8) -> BrowserNonce {
    BrowserNonce::from_bytes(&ledger_support::bytes::<16>(start)).expect("nonce is valid")
}

#[derive(Debug)]
pub(crate) struct SequenceGenerator {
    next: AtomicU8,
    calls: AtomicUsize,
}

impl SequenceGenerator {
    pub(crate) fn new(next: u8) -> Self {
        Self {
            next: AtomicU8::new(next),
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl InstanceIdGenerator for SequenceGenerator {
    fn generate(&self) -> Result<InstanceId, RandomError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let start = self.next.fetch_add(1, Ordering::SeqCst);
        InstanceId::from_bytes(&ledger_support::bytes::<16>(start))
            .map_err(|_| RandomError::generation_failed())
    }
}

pub(crate) fn promotion_limits() -> PromotionLimits {
    PromotionLimits::new(PromotionLimitConfig {
        max_seed_bytes: 4_096,
        window_ms: 1_000,
        max_promotions_per_window: 8,
        max_outstanding_per_scope: 8,
        max_outstanding_per_route_component: 4,
        promotion_lease_ms: 100,
        abandoned_retention_ms: 200,
        instance_lifetime_ms: 1_000,
        max_reservations: 64,
        max_rate_buckets: 32,
    })
    .expect("promotion limits are valid")
}

pub(crate) fn memory_ledger(
    clock: Arc<ManualClock>,
    max_instances: usize,
) -> Arc<MemoryInstanceLedger> {
    Arc::new(MemoryInstanceLedger::new(
        clock,
        LedgerLimits::new(100, 10_000, 4, max_instances).expect("ledger limits are valid"),
    ))
}

pub(crate) fn signed_seed(keys: &SnapshotKeyRing, query: &str) -> Vec<u8> {
    signed_seed_with_refresh(keys, query, true)
}

pub(crate) fn signed_seed_with_refresh(
    keys: &SnapshotKeyRing,
    query: &str,
    refresh_on_promote: bool,
) -> Vec<u8> {
    let mut fields = seed_fields(keys);
    fields.state =
        snapshot_support::public_value(&format!(r#"{{"query":"{query}","selected":"1"}}"#));
    fields.refresh_on_promote = refresh_on_promote;
    SeedBodyV1::new(fields, &schema_set(), &snapshot_limits())
        .expect("seed constructs")
        .sign(keys, UnixMillis::new(1_000), &snapshot_limits())
        .expect("seed signs")
}

pub(crate) fn context(scope_start: u8) -> TrustedPromotionContext {
    TrustedPromotionContext::new(
        expected_seed(schema_set()),
        scope(scope_start),
        PromotionAttestations::verified(),
    )
}

pub(crate) struct Harness {
    pub(crate) clock: Arc<ManualClock>,
    pub(crate) ledger: Arc<MemoryInstanceLedger>,
    pub(crate) generator: Arc<SequenceGenerator>,
    pub(crate) keys: Arc<SnapshotKeyRing>,
    pub(crate) snapshot_limits: SnapshotLimits,
    pub(crate) service: PromotionService,
}

pub(crate) fn harness(limits: PromotionLimits, max_instances: usize) -> Harness {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(clock.clone(), max_instances);
    let generator = Arc::new(SequenceGenerator::new(0xd0));
    let keys = Arc::new(key_ring());
    let snapshot_limits = snapshot_limits();
    let service = PromotionService::new(
        ledger.clone(),
        clock.clone(),
        generator.clone(),
        keys.clone(),
        snapshot_limits.clone(),
        limits,
    )
    .expect("promotion service config is valid");
    Harness {
        clock,
        ledger,
        generator,
        keys,
        snapshot_limits,
        service,
    }
}
