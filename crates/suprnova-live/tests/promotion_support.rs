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

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use http::Method;
pub(crate) use ledger_support::{ManualClock, digest, idempotency, instance, scope};
use snapshot_support::{key_ring, seed_fields, snapshot_limits};
use suprnova_live::action::{ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy};
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::endpoint::{
    LIVE_MEDIA_TYPE_V2, LiveEndpointConfig, LiveEndpointRequest, ParsedLiveMediaType,
    RequestCachePolicy,
};
use suprnova_live::host::{
    HostScopeFacts, MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection,
    PrincipalFingerprint, ScopeRequirement, SessionFingerprint, TenantFingerprint,
    TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BrowserNonce, BuildId, ComponentName, CorrelationId, InstanceId, IslandSlot, UnixMillis,
    ViewName,
};
use suprnova_live::ledger::{LedgerLimits, MemoryInstanceLedger};
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::mount::DocumentMountKey;
use suprnova_live::promotion::{
    InstanceIdGenerator, PromotionLimitConfig, PromotionLimits, PromotionService, RandomError,
    TrustedPromotionContext,
};
use suprnova_live::protocol::{BrowserRenderContext, ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedSeedV1, SeedBodyV1, SnapshotLimits, SnapshotSchemaSet,
};
use suprnova_live::state::{BindingTiming, ModelCodec};
use suprnova_live::validation::ValidationSelection;
use suprnova_live_test_support::{
    SyntheticLiveRequestContextBuilder, VerifiedResponseSealing, capture_verified_response_sealer,
};

pub(crate) fn nonce(start: u8) -> BrowserNonce {
    BrowserNonce::from_bytes(&ledger_support::bytes::<16>(start)).expect("nonce is valid")
}

pub(crate) fn browser_context() -> BrowserRenderContext {
    let expected = DocumentMountKey::parse("test-root").expect("document key");
    BrowserRenderContext::checked("test-root", &expected).expect("browser render context")
}

pub(crate) async fn admitted_seed_response_sealing(
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    encoded_seed: &[u8],
    browser_nonce: BrowserNonce,
    correlation_start: u8,
) -> VerifiedResponseSealing {
    let input = suprnova_live::limits::InputLimits::new(64 * 1024, 12, 512, 40 * 1024)
        .expect("protocol input limits");
    let protocol = ProtocolLimits::new(ProtocolLimitConfig {
        input,
        max_snapshot_bytes: 32 * 1024,
        max_html_bytes: 32 * 1024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 16,
        max_validation_entries: 16,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .expect("protocol limits");
    let config = LiveEndpointConfig::new(protocol, snapshot_limits()).expect("endpoint config");
    let media = ParsedLiveMediaType::parse(LIVE_MEDIA_TYPE_V2).expect("live media type");
    let correlation = CorrelationId::from_bytes(&ledger_support::bytes::<16>(correlation_start))
        .expect("correlation identity");
    let seed: serde_json::Value = serde_json::from_slice(encoded_seed).expect("seed JSON");
    let body = serde_json::to_vec(&serde_json::json!({
        "base_revision": "0",
        "child_parameters": null,
        "component": descriptor.metadata().identity().as_str(),
        "correlation_id": correlation.to_base64url(),
        "extensions": {"x_suprnova_live_document_key_v1": "test-root"},
        "idempotency_key": CorrelationId::from_bytes(&ledger_support::bytes::<16>(0x65))
            .expect("idempotency bytes")
            .to_base64url(),
        "model_proposals": {},
        "operations": [{"arguments": {}, "kind": "invoke_action", "name": "search"}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": {
            "browser_nonce": browser_nonce.to_base64url(),
            "envelope": seed,
            "kind": "seed_promotion",
        },
        "snapshot_schema_version": 1,
    }))
    .expect("seed request JSON");
    let request = LiveEndpointRequest::try_new(
        Method::POST,
        media,
        Bytes::from(body),
        Some(context),
        RequestCachePolicy::Bypass,
    )
    .expect("seed endpoint request");
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("seed registry entry")
        .build();
    capture_verified_response_sealer(
        config,
        Arc::new(registry),
        Arc::new(ManualClock::new(1_000)),
        Arc::new(key_ring()),
        request,
    )
    .await
    .expect("verified seed response sealing")
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
    SeedBodyV1::new(fields, &promotion_schema_set(), &snapshot_limits())
        .expect("seed constructs")
        .sign(keys, UnixMillis::new(1_000), &snapshot_limits())
        .expect("seed signs")
}

pub(crate) fn context(scope_start: u8) -> TrustedPromotionContext {
    context_for_route(scope_start, 1)
}

pub(crate) fn context_for_route(scope_start: u8, route_start: u8) -> TrustedPromotionContext {
    trusted_context_for_route(scope_start, route_start).for_promotion()
}

pub(crate) fn trusted_context_for_route(
    scope_start: u8,
    route_start: u8,
) -> TrustedLiveRequestContext {
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
}

pub(crate) fn promotion_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        let mut fields: Vec<_> = ["query", "selected"]
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
        fields.push(FieldMetadata::new(
            suprnova_live::identity::ModelField::parse("server").expect("field identity"),
            FieldCategory::State,
            StateCodec::Json,
            true,
        ));
        fields.push(
            FieldMetadata::new(
                suprnova_live::identity::ModelField::parse("count").expect("field identity"),
                FieldCategory::Model,
                StateCodec::Json,
                true,
            )
            .with_model_binding(ModelCodec::U64, BindingTiming::Submit)
            .expect("model metadata"),
        );
        ComponentMetadata::new(
            ComponentName::parse("catalog.search").expect("component identity"),
            ViewName::parse("live/catalog/search.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 2).expect("versions"),
            fields,
            vec![
                ActionMetadata::new_with_contract(
                    suprnova_live::identity::ActionName::parse("search").expect("action identity"),
                    1,
                    ActionArgumentSchema::empty(),
                    AuthorizationRequirement::Public,
                    ValidationSelection::None,
                    TransactionPolicy::None,
                )
                .expect("action metadata"),
            ],
        )
        .expect("promotion metadata")
    })
}

pub(crate) fn promotion_descriptor() -> ComponentDescriptor {
    ComponentDescriptor::new(promotion_metadata().clone())
}

pub(crate) fn promotion_schema_set() -> SnapshotSchemaSet {
    SnapshotSchemaSet::new(
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("query", StateCodec::Json, FieldCategory::Public, true)
                    .expect("field"),
                FieldSpec::new("selected", StateCodec::Json, FieldCategory::Public, true)
                    .expect("field"),
                FieldSpec::new("server", StateCodec::Json, FieldCategory::State, true)
                    .expect("field"),
                FieldSpec::new("count", StateCodec::Json, FieldCategory::Model, true)
                    .expect("field"),
            ],
        )
        .expect("state schema"),
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("page", StateCodec::Json, FieldCategory::Public, true)
                    .expect("field"),
            ],
        )
        .expect("memo schema"),
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("catalog", StateCodec::Json, FieldCategory::Public, true)
                    .expect("field"),
            ],
        )
        .expect("mount schema"),
    )
    .expect("promotion schema set")
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
        promotion_schema_set(),
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
