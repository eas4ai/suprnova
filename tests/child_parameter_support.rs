//! Shared deterministic child-parameter authority fixtures.

#![allow(
    dead_code,
    reason = "shared helpers are used by separate integration-test crates"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use suprnova_live::canonical::CanonicalValue;
use suprnova_live::child::{
    AcceptedParentRevision, ChildParameterLimits, ExpectedChildParametersV1,
    PreparedChildParametersV1,
};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::component::composition::{
    ChildDeclaration, ChildKey, ChildParameterField, ChildParameterSchema, ChildState,
    CompositionAncestry, CompositionLimits, CompositionPlanner, PendingChildParameters,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{
    ComponentName, ContentDigest, IdempotencyKey, InstanceId, KeyId, ModelField, Revision,
    ScopeFingerprint, UnixMillis, ViewName,
};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimOutcome, ClaimRequest, LedgerLimits,
    LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{ComponentMetadata, ContractVersions};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::state::ModelCodec;

pub(crate) const NOW: UnixMillis = UnixMillis::new(1_000);
pub(crate) const EXPIRES: UnixMillis = UnixMillis::new(1_500);

#[derive(Debug)]
struct ManualClock {
    now: AtomicU64,
}

impl ManualClock {
    fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Ok(UnixMillis::new(self.now.load(Ordering::SeqCst)))
    }
}

pub(crate) fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

pub(crate) fn scope(start: u8) -> ScopeFingerprint {
    ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("scope")
}

pub(crate) fn instance(start: u8) -> InstanceId {
    InstanceId::from_bytes(&bytes::<16>(start)).expect("instance")
}

pub(crate) fn digest(start: u8) -> ContentDigest {
    ContentDigest::from_bytes(&bytes::<32>(start)).expect("digest")
}

pub(crate) fn key_ring(key_id: &str, fill: u8) -> SnapshotKeyRing {
    let record = KeyRecord::new(
        KeyId::parse(key_id).expect("key id"),
        RootKey::new(vec![fill; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )
    .expect("key record");
    SnapshotKeyRing::new(record, Vec::new()).expect("key ring")
}

pub(crate) fn parameter_limits() -> ChildParameterLimits {
    ChildParameterLimits::new(
        InputLimits::new(16 * 1024, 8, 128, 512).expect("input limits"),
        50,
        2_000,
    )
    .expect("child parameter limits")
}

pub(crate) fn parameter_schema() -> ChildParameterSchema {
    ChildParameterSchema::new(
        1,
        vec![ChildParameterField::new(
            ModelField::parse("query").expect("parameter name"),
            ModelCodec::String,
            true,
        )],
    )
    .expect("parameter schema")
}

pub(crate) fn parameters(query: &str) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([(
        "query".to_owned(),
        CanonicalValue::String(query.to_owned()),
    )]))
}

fn descriptor(schema: ChildParameterSchema) -> ComponentDescriptor {
    let metadata = ComponentMetadata::new(
        ComponentName::parse("catalog.results").expect("component"),
        ViewName::parse("live/catalog/results.html").expect("view"),
        ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
        vec![],
        vec![],
    )
    .expect("metadata");
    ComponentDescriptor::new(metadata).with_composition(schema, true, false)
}

pub(crate) fn pending_parameters(query: &str) -> (PendingChildParameters, ChildParameterSchema) {
    let schema = parameter_schema();
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor(schema.clone()))
        .expect("component registers")
        .build();
    let planner = CompositionPlanner::new(
        CompositionLimits::new(4, 4, 4_096, 4).expect("composition limits"),
    );
    let ancestry =
        CompositionAncestry::root(ComponentName::parse("catalog.search").expect("parent"));
    let declaration = |value: &str| {
        ChildDeclaration::new(
            ChildKey::parse("results").expect("child key"),
            ComponentName::parse("catalog.results").expect("child component"),
            parameters(value),
        )
    };
    let initial = planner
        .reconcile(&registry, &ancestry, &[], vec![declaration("rust")])
        .expect("initial child");
    let [ChildState::Remount(prepared)] = initial.as_slice() else {
        panic!("initial child remounts");
    };
    let handle = prepared.clone().into_handle(instance(0x90));
    let changed = planner
        .reconcile(
            &registry,
            &ancestry,
            std::slice::from_ref(&handle),
            vec![declaration(query)],
        )
        .expect("changed child");
    let [ChildState::PendingParams(pending)] = changed.as_slice() else {
        panic!("changed parameters become pending");
    };
    (pending.clone(), schema)
}

pub(crate) struct AcceptedParentFixture {
    pub(crate) accepted: AcceptedParentRevision,
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance: InstanceId,
    pub(crate) revision: Revision,
}

pub(crate) async fn accepted_parent() -> AcceptedParentFixture {
    let scope = scope(0x10);
    let instance = instance(0x40);
    let clock = Arc::new(ManualClock::new(NOW.get()));
    let ledger = MemoryInstanceLedger::new(
        clock.clone() as Arc<dyn Clock>,
        LedgerLimits::new(100, 10_000, 8, 64).expect("ledger limits"),
    );
    ledger
        .mount_instance(MountInstanceRecord::new(
            scope.clone(),
            instance.clone(),
            digest(0x20),
            Revision::new(4),
            UnixMillis::new(5_000),
        ))
        .await
        .expect("parent mount authority");

    let request = ClaimRequest::new(
        scope.clone(),
        instance.clone(),
        Revision::new(4),
        IdempotencyKey::from_bytes(&bytes::<16>(0x50)).expect("idempotency"),
        digest(0x60),
    );
    let grant = match ledger.claim(request.clone()).await.expect("parent claim") {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected claim grant, got {other:?}"),
    };
    let revision = grant.successor_revision();
    ledger
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, digest(0x70)),
        )
        .await
        .expect("parent outcome accepted");
    let metadata = match ledger.claim(request).await.expect("accepted duplicate") {
        ClaimOutcome::Accepted(metadata) => metadata,
        other => panic!("expected accepted metadata, got {other:?}"),
    };

    AcceptedParentFixture {
        accepted: AcceptedParentRevision::from_accepted_outcome(&metadata),
        scope,
        instance,
        revision,
    }
}

pub(crate) struct IssuedChildFixture {
    pub(crate) encoded: Vec<u8>,
    pub(crate) expected: ExpectedChildParametersV1,
    pub(crate) keys: SnapshotKeyRing,
    pub(crate) limits: ChildParameterLimits,
    pub(crate) parameters: CanonicalValue,
    pub(crate) parent_scope: ScopeFingerprint,
    pub(crate) parent_instance: InstanceId,
    pub(crate) parent_revision: Revision,
    pub(crate) child_key: ChildKey,
    pub(crate) child_contract: ContentDigest,
}

pub(crate) async fn issued_child(query: &str) -> IssuedChildFixture {
    let parent = accepted_parent().await;
    let (pending, schema) = pending_parameters(query);
    let parameters = pending.parameters().clone();
    let child_key = pending.child().key().clone();
    let child_contract = pending.child().component_contract().clone();
    let keys = key_ring("child-v1", 0x33);
    let limits = parameter_limits();
    let prepared = PreparedChildParametersV1::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        pending,
        NOW,
        EXPIRES,
        keys.active_key_id().clone(),
        &limits,
    )
    .expect("prepared child parameters");
    let encoded = prepared
        .publish(&parent.accepted, &keys, NOW, &limits)
        .expect("published child parameters");
    let expected = ExpectedChildParametersV1::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        child_key.clone(),
        child_contract.clone(),
        schema,
    );
    IssuedChildFixture {
        encoded,
        expected,
        keys,
        limits,
        parameters,
        parent_scope: parent.scope,
        parent_instance: parent.instance,
        parent_revision: parent.revision,
        child_key,
        child_contract,
    }
}
