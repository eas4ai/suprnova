//! Create-only ledger authority for identity-bound initial mounts.

mod ledger_support;

use std::sync::Arc;

use ledger_support::{ManualClock, digest, instance, ledger, scope};
use suprnova_live::identity::{Revision, UnixMillis};
use suprnova_live::ledger::{LedgerErrorKind, LiveInstanceLedger, MountInstanceRecord};

fn mount_record(instance_start: u8, expires_at: u64) -> MountInstanceRecord {
    MountInstanceRecord::new(
        scope(0x10),
        instance(instance_start),
        digest(0x30),
        Revision::new(0),
        UnixMillis::new(expires_at),
    )
}

#[tokio::test]
async fn private_mount_creates_revision_authority_without_a_promotion_reservation() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let record = mount_record(0x20, 5_000);

    let authority = ledger
        .mount_instance(record.clone())
        .await
        .expect("private mount authority is created");

    assert_eq!(authority.instance_id(), record.instance_id());
    assert_eq!(authority.revision(), Revision::new(0));
    assert_eq!(authority.expires_at(), UnixMillis::new(5_000));
    assert_eq!(
        ledger
            .inspect(record.scope(), record.instance_id())
            .expect("ledger inspection")
            .expect("mounted instance exists")
            .current_revision(),
        Revision::new(0)
    );
}

#[tokio::test]
async fn private_mount_is_create_only_and_never_recovers_an_exact_retry() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let record = mount_record(0x20, 5_000);

    ledger
        .mount_instance(record.clone())
        .await
        .expect("first mount succeeds");
    let error = ledger
        .mount_instance(record)
        .await
        .expect_err("same identity is still a collision");

    assert_eq!(error.kind(), LedgerErrorKind::InstanceConflict);
}

#[tokio::test]
async fn private_mount_rejects_elapsed_expiry_and_capacity_without_partial_authority() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock.clone(), 2);

    let expiry_error = ledger
        .mount_instance(mount_record(0x20, 1_000))
        .await
        .expect_err("exclusive elapsed expiry is rejected");
    assert_eq!(expiry_error.kind(), LedgerErrorKind::InvalidExpiry);

    for start in 0_u8..64 {
        ledger
            .mount_instance(mount_record(start.wrapping_add(0x40), 5_000))
            .await
            .expect("configured capacity remains available");
    }
    let capacity_error = ledger
        .mount_instance(mount_record(0x90, 5_000))
        .await
        .expect_err("one more instance exceeds capacity");
    assert_eq!(capacity_error.kind(), LedgerErrorKind::CapacityExceeded);

    assert!(
        ledger
            .inspect(&scope(0x10), &instance(0x90))
            .expect("ledger inspection")
            .is_none()
    );
}
