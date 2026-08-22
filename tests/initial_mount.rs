//! Identity-bound initial mount success, collision retry, and publication ordering.

mod component_support;

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use component_support::{
    FailurePoint, FixtureControl, ManualClock, SequenceGenerator, install, key_ring, metadata,
    schema_set, snapshot_limits, trusted_context,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::{BuildId, InstanceId, Revision, UnixMillis};
use suprnova_live::ledger::{
    AcceptedOutcome, ClaimOutcome, ClaimRequest, ClaimToken, InstanceAuthority, LedgerError,
    LedgerLimits, LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord, PromotionOutcome,
    PromotionRecord,
};
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, MountLimits, MountProviders,
    PrivateMountRequest, PrivateMountService,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::{ComponentContract, ExpectedInstanceV1, verify_instance};
use suprnova_live::view::{RenderLimits, ViewRenderer};

fn service(
    control: Arc<FixtureControl>,
    clock: Arc<ManualClock>,
    ledger: Arc<dyn LiveInstanceLedger>,
    ids: Arc<SequenceGenerator>,
) -> PrivateMountService {
    let registry = ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::with_hooks(
            metadata().clone(),
            install(control),
        ))
        .expect("component registers")
        .build();
    PrivateMountService::new(
        MountProviders::new(Arc::new(registry), ledger, clock, ids, Arc::new(key_ring())),
        snapshot_limits(),
        ViewRenderer::new(RenderLimits::standard()).expect("render limits"),
        MountLimits::new(1_000, 3, 8_192, 8).expect("mount limits"),
    )
    .expect("mount service configuration")
}

fn request(key: &str) -> PrivateMountRequest {
    PrivateMountRequest::new(
        DocumentMountKey::parse(key).expect("document mount key"),
        CanonicalValue::Object(BTreeMap::new()),
        MountFlags::empty(),
    )
}

fn memory_ledger(clock: Arc<ManualClock>, max_instances: usize) -> Arc<MemoryInstanceLedger> {
    Arc::new(MemoryInstanceLedger::new(
        clock,
        LedgerLimits::new(100, 10_000, 4, max_instances).expect("ledger limits"),
    ))
}

#[tokio::test]
async fn private_mount_signs_complete_state_then_creates_authority_and_publishes_one_wrapper() {
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(clock.clone(), 8);
    let ids = Arc::new(SequenceGenerator::new(0x20));
    let service = service(control.clone(), clock, ledger.clone(), ids);
    let context = trusted_context();
    let mut document = DocumentMountScope::new();

    let output = service
        .mount(&mut document, request("primary-search"), &context)
        .await
        .expect("private mount succeeds");

    let authority = ledger
        .inspect(context.scope(), output.instance_id())
        .expect("ledger inspection")
        .expect("authority exists before output is returned");
    assert_eq!(authority.current_revision(), Revision::new(0));
    assert_eq!(output.revision(), Revision::new(0));
    assert_eq!(output.expires_at(), UnixMillis::new(2_000));

    let html = std::str::from_utf8(output.body()).expect("mount HTML is UTF-8");
    assert_eq!(html.matches("data-suprnova-live-root=").count(), 1);
    assert!(html.contains("data-suprnova-live-component=\"tests.trace\""));
    assert!(html.contains("data-suprnova-live-key=\"primary-search\""));
    assert!(html.contains("<p>1</p>"));

    let contract = ComponentContract::new(
        metadata().identity().clone(),
        metadata().contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let verified = verify_instance(
        output.metadata().signed_snapshot(),
        &ExpectedInstanceV1::new(
            contract,
            BuildId::parse("build-lifecycle-tests").expect("build identity"),
            context.mount().route().clone(),
            context.mount().slot().clone(),
            context.scope().clone(),
            schema_set(),
        ),
        &key_ring(),
        UnixMillis::new(1_000),
        &snapshot_limits(),
    )
    .expect("published instanced snapshot verifies");
    assert_eq!(verified.body().instance_id(), output.instance_id());
    assert_eq!(control.values().last(), Some(&"teardown"));
}

#[tokio::test]
async fn instance_collision_repeats_effect_free_mount_under_a_fresh_identity() {
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(clock.clone(), 8);
    let occupied =
        InstanceId::from_bytes(&component_support::bytes::<16>(0x20)).expect("occupied identity");
    ledger
        .mount_instance(MountInstanceRecord::new(
            trusted_context().scope().clone(),
            occupied,
            metadata().contract_digest().clone(),
            Revision::new(0),
            UnixMillis::new(2_000),
        ))
        .await
        .expect("collision fixture authority");
    let ids = Arc::new(SequenceGenerator::new(0x20));
    let service = service(control.clone(), clock, ledger, ids.clone());
    let context = trusted_context();
    let mut document = DocumentMountScope::new();

    let output = service
        .mount(&mut document, request("retrying-search"), &context)
        .await
        .expect("fresh candidate retries the collision");

    assert_eq!(ids.calls(), 2);
    assert_eq!(
        output.instance_id(),
        &InstanceId::from_bytes(&component_support::bytes::<16>(0x21)).expect("second identity")
    );
    assert_eq!(
        control
            .values()
            .into_iter()
            .filter(|phase| *phase == "mount")
            .count(),
        2
    );
}

struct BlockingLedger {
    inner: Arc<MemoryInstanceLedger>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl LiveInstanceLedger for BlockingLedger {
    async fn mount_instance(
        &self,
        record: MountInstanceRecord,
    ) -> Result<InstanceAuthority, LedgerError> {
        self.entered.notify_one();
        self.release.notified().await;
        self.inner.mount_instance(record).await
    }

    async fn promote(&self, record: PromotionRecord) -> Result<PromotionOutcome, LedgerError> {
        self.inner.promote(record).await
    }

    async fn claim(&self, request: ClaimRequest) -> Result<ClaimOutcome, LedgerError> {
        self.inner.claim(request).await
    }

    async fn commit(&self, claim: ClaimToken, outcome: AcceptedOutcome) -> Result<(), LedgerError> {
        self.inner.commit(claim, outcome).await
    }

    async fn abandon(&self, claim: ClaimToken) -> Result<(), LedgerError> {
        self.inner.abandon(claim).await
    }
}

#[tokio::test]
async fn no_publishable_output_exists_while_ledger_authority_is_blocked() {
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let blocking = Arc::new(BlockingLedger {
        inner: memory_ledger(clock.clone(), 8),
        entered: entered.clone(),
        release: release.clone(),
    });
    let service = service(
        control.clone(),
        clock,
        blocking,
        Arc::new(SequenceGenerator::new(0x20)),
    );
    let context = trusted_context();
    let task = tokio::spawn(async move {
        let mut document = DocumentMountScope::new();
        service
            .mount(&mut document, request("blocked-search"), &context)
            .await
    });

    entered.notified().await;
    assert!(!task.is_finished());
    assert_eq!(
        control.values().last(),
        Some(&"teardown"),
        "complete render and dehydration precede the ledger boundary"
    );
    release.notify_one();

    let output = task
        .await
        .expect("mount task does not panic")
        .expect("mount publishes after release");
    assert!(!output.body().is_empty());
}
