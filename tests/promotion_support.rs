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
use snapshot_support::{key_ring, schema_set, seed_fields, snapshot_limits};
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::host::{
    HostScopeFacts, MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection,
    PrincipalFingerprint, ScopeRequirement, SessionFingerprint, TenantFingerprint,
};
use suprnova_live::identity::{
    BrowserNonce, BuildId, ComponentName, InstanceId, IslandSlot, UnixMillis, ViewName,
};
use suprnova_live::ledger::{LedgerLimits, MemoryInstanceLedger};
use suprnova_live::metadata::{ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::promotion::{
    InstanceIdGenerator, PromotionLimitConfig, PromotionLimits, PromotionService, RandomError,
    TrustedPromotionContext,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1, SeedBodyV1, SnapshotLimits};
use suprnova_live_test_support::SyntheticLiveRequestContextBuilder;

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
    fields.component = promotion_component_contract();
    fields.state =
        snapshot_support::public_value(&format!(r#"{{"query":"{query}","selected":"1"}}"#));
    fields.refresh_on_promote = refresh_on_promote;
    SeedBodyV1::new(fields, &schema_set(), &snapshot_limits())
        .expect("seed constructs")
        .sign(keys, UnixMillis::new(1_000), &snapshot_limits())
        .expect("seed signs")
}

pub(crate) fn context(scope_start: u8) -> TrustedPromotionContext {
    context_for_route(scope_start, 1)
}

pub(crate) fn context_for_route(scope_start: u8, route_start: u8) -> TrustedPromotionContext {
    let descriptor = promotion_descriptor();
    let component = descriptor.metadata().identity().clone();
    let contract_digest = descriptor.contract_digest().clone();
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("promotion component registers")
        .build();
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                promotion_expected_seed(route_start),
                MountScopeRequirements::new(
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                ),
            ),
        )
        .expect("promotion mount registers")
        .build();
    let selection = MountSelection::new(
        snapshot_support::route(route_start),
        IslandSlot::parse("search-results").expect("slot is valid"),
        component,
        contract_digest,
        2,
    );
    let scope_facts = HostScopeFacts::new(
        scope(scope_start),
        Some(
            SessionFingerprint::from_bytes(&snapshot_support::bytes::<32>(
                scope_start.wrapping_add(1),
            ))
            .expect("session fingerprint"),
        ),
        Some(
            PrincipalFingerprint::from_bytes(&snapshot_support::bytes::<32>(
                scope_start.wrapping_add(2),
            ))
            .expect("principal fingerprint"),
        ),
        Some(
            TenantFingerprint::from_bytes(&snapshot_support::bytes::<32>(
                scope_start.wrapping_add(3),
            ))
            .expect("tenant fingerprint"),
        ),
    );
    SyntheticLiveRequestContextBuilder::new(
        catalog,
        selection,
        scope_facts,
        UnixMillis::new(1_000),
        UnixMillis::new(2_000),
    )
    .build()
    .expect("synthetic context passes production validation")
    .for_promotion()
}

fn promotion_descriptor() -> ComponentDescriptor {
    let fields = ["query", "selected"]
        .into_iter()
        .map(|name| {
            FieldMetadata::new(
                suprnova_live::identity::ModelField::parse(name).expect("field identity"),
                FieldCategory::Public,
                StateCodec::Json,
                true,
            )
        })
        .collect();
    ComponentDescriptor::new(
        ComponentMetadata::new(
            ComponentName::parse("catalog.search").expect("component identity"),
            ViewName::parse("live/catalog/search.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 2).expect("versions"),
            fields,
            vec![],
        )
        .expect("promotion metadata"),
    )
}

pub(crate) fn promotion_component_contract() -> ComponentContract {
    let descriptor = promotion_descriptor();
    ComponentContract::new(
        descriptor.metadata().identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("promotion component contract")
}

fn promotion_expected_seed(route_start: u8) -> ExpectedSeedV1 {
    ExpectedSeedV1::new(
        promotion_component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        snapshot_support::route(route_start),
        IslandSlot::parse("search-results").expect("slot is valid"),
        schema_set(),
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
