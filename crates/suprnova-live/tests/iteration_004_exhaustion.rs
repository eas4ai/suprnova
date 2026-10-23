//! Iteration 004 scoped exhaustion and independent-work liveness regressions.

use suprnova_live::host::{
    HostScopeFacts, PrincipalFingerprint, SessionFingerprint, TenantFingerprint,
};
use suprnova_live::identity::{
    ActionName, ComponentName, ModelField, ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::resource::{BoundedQueue, PermitPool, ResourceBounds, ResourceError};
use suprnova_live::upload::{
    ConditionalUploadCreate, TransferGrantScope, UploadCreateCommand, UploadErrorKind,
    UploadFieldPolicy, UploadHandle, UploadIdempotencyKey, UploadLedger, UploadMediaType,
    UploadRecord, UploadReplacementPolicy, UploadRevision, UploadScanPolicy, UploadState,
};
use suprnova_live_test_support::MemoryUploadLedger;

fn fingerprint(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn host_scope(byte: u8) -> HostScopeFacts {
    HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&fingerprint(byte)).expect("scope"),
        Some(SessionFingerprint::from_bytes(&fingerprint(byte)).expect("session")),
        Some(PrincipalFingerprint::from_bytes(&fingerprint(byte)).expect("principal")),
        Some(TenantFingerprint::from_bytes(&fingerprint(byte)).expect("tenant")),
    )
}

fn limits() -> UploadLimits {
    UploadLimits::new(UploadLimitConfig {
        max_files_per_field: 2,
        max_pending_per_scope: 2,
        max_creations_per_window: 2,
        ..UploadLimitConfig::reference()
    })
    .expect("finite exhaustion profile")
}

fn policy() -> UploadFieldPolicy {
    UploadFieldPolicy::new(
        2,
        1_024,
        UploadReplacementPolicy::PreservePrevious,
        vec![UploadMediaType::Png],
        None,
        UploadScanPolicy::Disabled,
        ActionName::parse("finalize_upload").expect("fixture action"),
    )
    .expect("finite exhaustion policy")
}

fn command(handle: &str, scope: HostScopeFacts, key: &str) -> UploadCreateCommand {
    let authority = TransferGrantScope::new(
        UploadHandle::parse(handle).expect("handle"),
        ComponentName::parse("profile.edit").expect("component"),
        ModelField::parse("avatar").expect("field"),
        scope,
        1,
    );
    let record = UploadRecord::new(
        authority,
        UploadState::Created,
        UploadRevision::initial(),
        UnixMillis::new(1_000),
        UnixMillis::new(2_000),
    )
    .expect("record");
    UploadCreateCommand::new(
        record,
        UploadIdempotencyKey::parse(key).expect("idempotency"),
        UnixMillis::new(1_001),
        limits(),
        1,
        policy(),
    )
}

#[tokio::test]
async fn upload_exhaustion_is_exactly_scoped_and_leaves_other_live_work_usable() {
    let ledger = MemoryUploadLedger::new(limits()).expect("ledger");
    let scope_a = host_scope(0x11);
    let scope_b = host_scope(0x22);

    for (handle, key) in [
        ("018f47c1-2af0-7cc4-a001-000000000011", "scope-a-1"),
        ("018f47c1-2af0-7cc4-a001-000000000012", "scope-a-2"),
    ] {
        let outcome = ledger
            .create(command(handle, scope_a.clone(), key))
            .await
            .expect("within exact scope bound");
        assert_eq!(outcome.disposition(), ConditionalUploadCreate::Created);
    }
    assert_eq!(ledger.len(), limits().max_pending_per_scope());

    let error = ledger
        .create(command(
            "018f47c1-2af0-7cc4-a001-000000000013",
            scope_a,
            "scope-a-overflow",
        ))
        .await
        .expect_err("third pending upload in scope A must fail closed");
    assert_eq!(error.kind(), UploadErrorKind::CreationRateExceeded);
    assert_eq!(ledger.len(), limits().max_pending_per_scope());
    assert!(format!("{error:?}:{error}").len() <= 256);

    let independent = ledger
        .create(command(
            "018f47c1-2af0-7cc4-a001-000000000021",
            scope_b,
            "scope-b-1",
        ))
        .await
        .expect("independent scope remains usable");
    assert_eq!(independent.disposition(), ConditionalUploadCreate::Created);
    assert_eq!(ledger.len(), limits().max_pending_per_scope() + 1);
}

#[test]
fn queue_and_permit_exhaustion_preserve_exact_bounds_and_sibling_liveness() {
    let bounds = ResourceBounds::new(2, 8).expect("queue bounds");
    let mut saturated = BoundedQueue::new(bounds);
    saturated.try_push(4, "one").expect("first item");
    saturated.try_push(4, "two").expect("second item");
    assert_eq!(saturated.len(), 2);
    assert_eq!(saturated.retained_bytes(), 8);

    assert_eq!(
        saturated
            .try_push(1, "item-overflow")
            .expect_err("item ceiling"),
        ResourceError::ItemsExceeded
    );
    assert_eq!(saturated.len(), bounds.max_items());
    assert_eq!(saturated.retained_bytes(), bounds.max_bytes());

    let mut byte_limited = BoundedQueue::new(ResourceBounds::new(3, 8).expect("byte bounds"));
    byte_limited.try_push(8, "full").expect("fill byte bound");
    assert_eq!(
        byte_limited
            .try_push(1, "byte-overflow")
            .expect_err("byte ceiling"),
        ResourceError::BytesExceeded
    );
    assert_eq!(byte_limited.len(), 1);
    assert_eq!(byte_limited.retained_bytes(), 8);

    let mut sibling = BoundedQueue::new(bounds);
    sibling
        .try_push(4, "independent-action")
        .expect("unrelated work remains admissible");
    assert_eq!(sibling.pop(), Some("independent-action"));
    assert!(sibling.is_empty());

    let saturated_permits = PermitPool::new(1).expect("permit pool");
    let held = saturated_permits.try_acquire().expect("first permit");
    assert_eq!(
        saturated_permits.try_acquire().expect_err("permit ceiling"),
        ResourceError::PermitsExceeded
    );
    assert_eq!(saturated_permits.active(), saturated_permits.max_active());

    let sibling_permits = PermitPool::new(1).expect("sibling pool");
    let sibling_held = sibling_permits
        .try_acquire()
        .expect("independent island permit remains usable");
    assert_eq!(sibling_permits.active(), 1);
    drop(sibling_held);
    drop(held);
    assert_eq!(saturated_permits.active(), 0);
    assert_eq!(sibling_permits.active(), 0);
}
