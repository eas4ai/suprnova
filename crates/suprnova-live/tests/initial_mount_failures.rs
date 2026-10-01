//! Private-mount hostile metadata, authority, and publication failures.

mod component_support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use component_support::{
    FailurePoint, FixtureControl, ManualClock, SequenceGenerator, install, key_ring, metadata,
    snapshot_limits, trusted_context,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::{Clock, ClockError, ClockErrorKind};
use suprnova_live::identity::{InstanceId, Revision, ScopeFingerprint, UnixMillis};
use suprnova_live::ledger::{
    AcceptedOutcome, ClaimOutcome, ClaimRequest, ClaimToken, InstanceAuthority, LedgerError,
    LedgerErrorKind, LedgerLimits, LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord,
    PromotionOutcome, PromotionRecord,
};
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountErrorKind, MountFailure, MountFlags, MountLimits,
    MountProviders, PrivateMountRequest, PrivateMountService,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::SnapshotErrorKind;
use suprnova_live::view::{RenderLimits, ViewErrorKind, ViewRenderer};

fn service(
    control: Arc<FixtureControl>,
    clock: Arc<dyn Clock>,
    ledger: Arc<dyn LiveInstanceLedger>,
    ids: Arc<SequenceGenerator>,
    limits: MountLimits,
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
        limits,
    )
    .expect("mount service")
}

#[derive(Debug)]
struct FailingClock;

impl Clock for FailingClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Err(ClockError::timestamp_overflow())
    }
}

#[derive(Debug)]
struct ExpiringClock {
    calls: AtomicUsize,
}

impl Clock for ExpiringClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(UnixMillis::new(if call == 0 { 1_000 } else { 2_000 }))
    }
}

/// Instance ledger that refuses every operation with one closed kind.
struct RefusingLedger {
    kind: LedgerErrorKind,
}

impl RefusingLedger {
    fn refusal(&self) -> LedgerError {
        LedgerError::new(self.kind)
    }
}

#[async_trait::async_trait]
impl LiveInstanceLedger for RefusingLedger {
    async fn mount_instance(
        &self,
        _record: MountInstanceRecord,
    ) -> Result<InstanceAuthority, LedgerError> {
        Err(self.refusal())
    }

    async fn promote(&self, _request: PromotionRecord) -> Result<PromotionOutcome, LedgerError> {
        Err(self.refusal())
    }

    async fn claim(&self, _request: ClaimRequest) -> Result<ClaimOutcome, LedgerError> {
        Err(self.refusal())
    }

    async fn current_accepted_revision(
        &self,
        _scope: &ScopeFingerprint,
        _instance_id: &InstanceId,
    ) -> Result<Option<Revision>, LedgerError> {
        Err(self.refusal())
    }

    async fn commit(
        &self,
        _claim: &ClaimToken,
        _outcome: AcceptedOutcome,
    ) -> Result<(), LedgerError> {
        Err(self.refusal())
    }

    async fn abandon(&self, _claim: &ClaimToken) -> Result<(), LedgerError> {
        Err(self.refusal())
    }

    fn abandon_on_drop(&self, _claim: ClaimToken) {}

    fn fence_on_drop(&self, _claim: ClaimToken) {}
}

fn memory_ledger(clock: Arc<ManualClock>, max_instances: usize) -> Arc<MemoryInstanceLedger> {
    Arc::new(MemoryInstanceLedger::new(
        clock,
        LedgerLimits::new(100, 10_000, 4, max_instances).expect("ledger limits"),
    ))
}

fn request(key: &str, flags: MountFlags) -> PrivateMountRequest {
    PrivateMountRequest::new(
        DocumentMountKey::parse(key).expect("document key"),
        CanonicalValue::Object(BTreeMap::new()),
        flags,
    )
}

fn limits(attempts: usize, metadata_bytes: usize) -> MountLimits {
    MountLimits::new(1_000, attempts, metadata_bytes, 8).expect("mount limits")
}

#[tokio::test]
async fn duplicate_document_identity_is_rejected_before_second_component_construction() {
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let service = service(
        control.clone(),
        clock.clone(),
        memory_ledger(clock, 8),
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 8_192),
    );
    let context = trusted_context();
    let mut document = DocumentMountScope::new();

    service
        .mount(
            &mut document,
            request("duplicate", MountFlags::empty()),
            &context,
        )
        .await
        .expect("first document key succeeds");
    let error = service
        .mount(
            &mut document,
            request("duplicate", MountFlags::empty()),
            &context,
        )
        .await
        .expect_err("duplicate key fails");

    assert_eq!(error.kind(), MountErrorKind::DuplicateDocumentKey);
    assert_eq!(
        control
            .values()
            .into_iter()
            .filter(|phase| *phase == "mount")
            .count(),
        1
    );
}

#[tokio::test]
async fn document_mount_capacity_is_enforced_before_component_construction() {
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let service = service(
        control.clone(),
        clock.clone(),
        memory_ledger(clock, 8),
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 8_192),
    );
    let context = trusted_context();
    let mut document = DocumentMountScope::with_limit(1).expect("bounded document scope");

    service
        .mount(
            &mut document,
            request("first", MountFlags::empty()),
            &context,
        )
        .await
        .expect("first mount fits document capacity");
    let error = service
        .mount(
            &mut document,
            request("second", MountFlags::empty()),
            &context,
        )
        .await
        .expect_err("second distinct mount exceeds document capacity");

    assert_eq!(error.kind(), MountErrorKind::DocumentCapacity);
    assert_eq!(
        control
            .values()
            .into_iter()
            .filter(|phase| *phase == "mount")
            .count(),
        1,
        "capacity is rejected before a second component is constructed"
    );
}

#[tokio::test]
async fn expired_context_and_executable_mount_metadata_fail_before_publication() {
    let expired_control = FixtureControl::new(FailurePoint::None);
    let expired_clock = Arc::new(ManualClock::new(2_000));
    let expired_service = service(
        expired_control.clone(),
        expired_clock.clone(),
        memory_ledger(expired_clock, 8),
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 8_192),
    );
    let mut document = DocumentMountScope::new();
    let error = expired_service
        .mount(
            &mut document,
            request("expired", MountFlags::empty()),
            &trusted_context(),
        )
        .await
        .expect_err("expired host authority fails");
    assert_eq!(error.kind(), MountErrorKind::ContextRejected);
    assert_eq!(error.cause(), None);
    assert!(expired_control.values().is_empty());

    let executable_control = FixtureControl::new(FailurePoint::ExecutableRender);
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(clock.clone(), 8);
    let executable_service = service(
        executable_control,
        clock,
        ledger.clone(),
        Arc::new(SequenceGenerator::new(0x30)),
        limits(3, 8_192),
    );
    let context = trusted_context();
    let mut document = DocumentMountScope::new();
    let error = executable_service
        .mount(
            &mut document,
            request("executable", MountFlags::empty()),
            &context,
        )
        .await
        .expect_err("script-bearing nested mount metadata fails");
    assert_eq!(error.kind(), MountErrorKind::RenderRejected);
    assert_eq!(
        error.cause(),
        Some(MountFailure::View(ViewErrorKind::ExecutableMountMetadata))
    );
    assert!(
        ledger
            .inspect(
                context.scope(),
                &InstanceId::from_bytes(&component_support::bytes::<16>(0x30))
                    .expect("candidate identity")
            )
            .expect("ledger inspection")
            .is_none()
    );
}

#[tokio::test]
async fn oversized_inert_metadata_and_ledger_capacity_fail_without_output() {
    let flags = MountFlags::new([("label", "x".repeat(256))])
        .expect("flag shape is valid before service byte budget");
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let oversized_service = service(
        control.clone(),
        clock.clone(),
        memory_ledger(clock, 8),
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 128),
    );
    let mut document = DocumentMountScope::new();
    let error = oversized_service
        .mount(
            &mut document,
            request("oversized", flags),
            &trusted_context(),
        )
        .await
        .expect_err("metadata budget fails closed");
    assert_eq!(error.kind(), MountErrorKind::MetadataTooLarge);
    assert!(control.values().is_empty());

    let capacity_control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(clock.clone(), 1);
    let context = trusted_context();
    ledger
        .mount_instance(MountInstanceRecord::new(
            context.scope().clone(),
            InstanceId::from_bytes(&component_support::bytes::<16>(0x70))
                .expect("occupied identity"),
            metadata().contract_digest().clone(),
            Revision::new(0),
            UnixMillis::new(2_000),
        ))
        .await
        .expect("fill ledger capacity");
    let service = service(
        capacity_control.clone(),
        clock,
        ledger,
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 8_192),
    );
    let mut document = DocumentMountScope::new();
    let error = service
        .mount(
            &mut document,
            request("capacity", MountFlags::empty()),
            &context,
        )
        .await
        .expect_err("ledger rejection prevents publication");
    assert_eq!(error.kind(), MountErrorKind::LedgerRejected);
    assert_eq!(error.ledger_kind(), Some(LedgerErrorKind::CapacityExceeded));
    assert_eq!(
        capacity_control.values().last(),
        Some(&"teardown"),
        "complete lifecycle precedes the atomic ledger write"
    );
}

#[tokio::test]
async fn ledger_refusal_keeps_the_ledger_kind_behind_the_coarse_mount_kind() {
    let kinds = [
        LedgerErrorKind::ProviderUnavailable,
        LedgerErrorKind::CapacityExceeded,
        LedgerErrorKind::ClockUnavailable,
        LedgerErrorKind::InvalidExpiry,
        LedgerErrorKind::InvalidConfiguration,
    ];

    for kind in kinds {
        let control = FixtureControl::new(FailurePoint::None);
        let service = service(
            control.clone(),
            Arc::new(ManualClock::new(1_000)),
            Arc::new(RefusingLedger { kind }),
            Arc::new(SequenceGenerator::new(0x20)),
            limits(3, 8_192),
        );
        let mut document = DocumentMountScope::new();

        let error = service
            .mount(
                &mut document,
                request("refused", MountFlags::empty()),
                &trusted_context(),
            )
            .await
            .expect_err("a ledger refusal prevents publication");

        assert_eq!(error.kind(), MountErrorKind::LedgerRejected, "{kind:?}");
        assert_eq!(error.cause(), Some(MountFailure::Ledger(kind)), "{kind:?}");
        assert_eq!(error.ledger_kind(), Some(kind), "{kind:?}");
        assert_eq!(error.to_string(), "mount_ledger_rejected", "{kind:?}");
        assert_eq!(
            format!("{error:?}"),
            format!("mount_ledger_rejected:{:?}", MountFailure::Ledger(kind)),
            "{kind:?}"
        );
        assert_eq!(
            control
                .values()
                .into_iter()
                .filter(|phase| *phase == "mount")
                .count(),
            1,
            "{kind:?}: a refusal other than a collision is never retried"
        );
    }
}

#[tokio::test]
async fn collision_retry_is_hard_bounded_and_never_reuses_prepared_output() {
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(clock.clone(), 8);
    let context = trusted_context();
    let occupied =
        InstanceId::from_bytes(&component_support::bytes::<16>(0x20)).expect("occupied identity");
    ledger
        .mount_instance(MountInstanceRecord::new(
            context.scope().clone(),
            occupied,
            metadata().contract_digest().clone(),
            Revision::new(0),
            UnixMillis::new(2_000),
        ))
        .await
        .expect("collision fixture");
    let ids = Arc::new(SequenceGenerator::fixed(0x20));
    let service = service(
        control.clone(),
        clock,
        ledger,
        ids.clone(),
        limits(2, 8_192),
    );
    let mut document = DocumentMountScope::new();

    let error = service
        .mount(
            &mut document,
            request("bounded-collision", MountFlags::empty()),
            &context,
        )
        .await
        .expect_err("collision budget exhausts");

    assert_eq!(error.kind(), MountErrorKind::IdentityCollision);
    assert_eq!(error.cause(), None);
    assert_eq!(ids.calls(), 2);
    assert_eq!(
        control
            .values()
            .into_iter()
            .filter(|phase| *phase == "mount")
            .count(),
        2
    );
}

#[tokio::test]
async fn snapshot_and_clock_failures_never_reach_instance_authority() {
    let snapshot_control = FixtureControl::new(FailurePoint::InvalidSnapshotState);
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(clock.clone(), 8);
    let snapshot_service = service(
        snapshot_control,
        clock,
        ledger.clone(),
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 8_192),
    );
    let context = trusted_context();
    let mut document = DocumentMountScope::new();
    let error = snapshot_service
        .mount(
            &mut document,
            request("bad-state", MountFlags::empty()),
            &context,
        )
        .await
        .expect_err("invalid dehydrated state cannot be signed");
    assert_eq!(error.kind(), MountErrorKind::SnapshotRejected);
    assert_eq!(
        error.cause(),
        Some(MountFailure::Snapshot(SnapshotErrorKind::UnknownStateField))
    );
    assert!(
        ledger
            .inspect(
                context.scope(),
                &InstanceId::from_bytes(&component_support::bytes::<16>(0x20))
                    .expect("candidate identity")
            )
            .expect("ledger inspection")
            .is_none()
    );

    let provider_control = FixtureControl::new(FailurePoint::None);
    let ledger_clock = Arc::new(ManualClock::new(1_000));
    let clock_service = service(
        provider_control.clone(),
        Arc::new(FailingClock),
        memory_ledger(ledger_clock, 8),
        Arc::new(SequenceGenerator::new(0x30)),
        limits(3, 8_192),
    );
    let mut document = DocumentMountScope::new();
    let error = clock_service
        .mount(
            &mut document,
            request("clock-failure", MountFlags::empty()),
            &trusted_context(),
        )
        .await
        .expect_err("clock provider failure is closed");
    assert_eq!(error.kind(), MountErrorKind::ClockUnavailable);
    assert_eq!(
        error.cause(),
        Some(MountFailure::Clock(ClockErrorKind::TimestampOverflow))
    );
    assert!(provider_control.values().is_empty());
}

#[tokio::test]
async fn host_authority_expiring_during_render_is_rechecked_before_ledger_creation() {
    let control = FixtureControl::new(FailurePoint::None);
    let ledger_clock = Arc::new(ManualClock::new(1_000));
    let ledger = memory_ledger(ledger_clock, 8);
    let service = service(
        control.clone(),
        Arc::new(ExpiringClock {
            calls: AtomicUsize::new(0),
        }),
        ledger.clone(),
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 8_192),
    );
    let context = trusted_context();
    let mut document = DocumentMountScope::new();

    let error = service
        .mount(
            &mut document,
            request("expires-during-render", MountFlags::empty()),
            &context,
        )
        .await
        .expect_err("expired authority cannot create ledger state");

    assert_eq!(error.kind(), MountErrorKind::ContextRejected);
    assert_eq!(control.values().last(), Some(&"teardown"));
    assert!(
        ledger
            .inspect(
                context.scope(),
                &InstanceId::from_bytes(&component_support::bytes::<16>(0x20))
                    .expect("candidate identity")
            )
            .expect("ledger inspection")
            .is_none()
    );
}

#[tokio::test]
async fn mount_flag_values_are_escaped_and_never_become_executable_attributes() {
    let flags =
        MountFlags::new([("label", "\"><script>alert(1)</script>")]).expect("bounded inert flag");
    let control = FixtureControl::new(FailurePoint::None);
    let clock = Arc::new(ManualClock::new(1_000));
    let service = service(
        control,
        clock.clone(),
        memory_ledger(clock, 8),
        Arc::new(SequenceGenerator::new(0x20)),
        limits(3, 8_192),
    );
    let mut document = DocumentMountScope::new();
    let output = service
        .mount(
            &mut document,
            request("escaped-flags", flags),
            &trusted_context(),
        )
        .await
        .expect("inert flag mounts");
    let html = std::str::from_utf8(output.body()).expect("mount HTML");
    assert!(html.contains(
        "data-suprnova-live-flag-label=\"&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;\""
    ));
    assert!(!html.contains("<script>"));
}

#[test]
fn inert_flag_collection_is_hard_bounded_before_service_allocation() {
    let error = MountFlags::new((0..65).map(|index| (format!("flag-{index}"), "x")))
        .expect_err("flag count above the hard cap is rejected");

    assert_eq!(error.kind(), MountErrorKind::MetadataTooLarge);
}
