//! The instance record store port from outside the engine: opaque bytes, a
//! version that only the state it was read at can replace, store-time expiry,
//! and a promotion key space of its own.

mod ledger_support;

use std::sync::Arc;

use ledger_support::{ManualClock, idempotency, instance, scope};
use suprnova_live::identity::UnixMillis;
use suprnova_live::ledger::{
    CasOutcome, InstanceRecordKey, InstanceRecordStore, MAX_RECORD_BYTES, MemoryRecordStore,
    PromotionRecordKey, RECORD_VERSION,
};

fn instance_key(start: u8) -> InstanceRecordKey {
    InstanceRecordKey {
        scope: scope(0x10),
        instance_id: instance(start),
    }
}

fn promotion_key(start: u8) -> PromotionRecordKey {
    PromotionRecordKey {
        scope: scope(0x10),
        idempotency_key: idempotency(start),
    }
}

fn store(clock: Arc<ManualClock>) -> MemoryRecordStore {
    MemoryRecordStore::new(clock)
}

fn version(outcome: CasOutcome) -> u64 {
    match outcome {
        CasOutcome::Stored { version } => version,
        other => panic!("the record was expected to be replaced, not {other:?}"),
    }
}

#[tokio::test]
async fn a_record_is_created_once_and_returned_byte_exactly() {
    let store = store(Arc::new(ManualClock::new(0)));
    let key = instance_key(0x20);
    let record = vec![RECORD_VERSION, b'{', b'}'];

    assert!(
        store
            .insert_if_absent(&key, &record, UnixMillis::new(1_000))
            .await
            .expect("the store answers"),
        "the first writer creates the record"
    );
    assert!(
        !store
            .insert_if_absent(&key, b"other", UnixMillis::new(1_000))
            .await
            .expect("the store answers"),
        "a second writer finds the key held"
    );

    let stored = store
        .load(&key)
        .await
        .expect("the store answers")
        .expect("the record is there");
    assert_eq!(
        stored.bytes, record,
        "a store keeps the kernel's bytes unchanged"
    );
    assert_eq!(stored.expires_at, UnixMillis::new(1_000));
}

#[tokio::test]
async fn only_the_version_a_reader_holds_may_replace_the_record() {
    let store = store(Arc::new(ManualClock::new(0)));
    let key = instance_key(0x20);
    store
        .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
        .await
        .expect("the store answers");
    let first = store
        .load(&key)
        .await
        .expect("the store answers")
        .expect("the record is there")
        .version;

    let second = version(
        store
            .compare_and_store(&key, first, b"second", UnixMillis::new(1_000))
            .await
            .expect("the store answers"),
    );
    assert!(second > first, "a replacement advances the version");
    assert_eq!(
        store
            .compare_and_store(&key, first, b"third", UnixMillis::new(1_000))
            .await
            .expect("the store answers"),
        CasOutcome::Conflict,
        "the version that was already replaced is a stale read"
    );
    assert_eq!(
        store
            .load(&key)
            .await
            .expect("the store answers")
            .expect("the record is there")
            .bytes,
        b"second".to_vec(),
        "a conflicting write leaves the record alone"
    );
}

#[tokio::test]
async fn a_key_no_record_holds_is_missing_rather_than_conflicting() {
    let store = store(Arc::new(ManualClock::new(0)));

    assert_eq!(
        store
            .compare_and_store(&instance_key(0x20), 1, b"any", UnixMillis::new(1_000))
            .await
            .expect("the store answers"),
        CasOutcome::Missing
    );
    assert!(
        store
            .load(&instance_key(0x20))
            .await
            .expect("the store answers")
            .is_none()
    );
}

#[tokio::test]
async fn store_time_alone_decides_when_a_record_is_gone() {
    let clock = Arc::new(ManualClock::new(0));
    let store = store(clock.clone());
    let key = instance_key(0x20);
    store
        .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
        .await
        .expect("the store answers");
    let held = store
        .load(&key)
        .await
        .expect("the store answers")
        .expect("the record is there")
        .version;

    clock.set(1_000);
    assert!(
        store.load(&key).await.expect("the store answers").is_none(),
        "the record elapsed by store time"
    );
    assert_eq!(
        store
            .compare_and_store(&key, held, b"second", UnixMillis::new(2_000))
            .await
            .expect("the store answers"),
        CasOutcome::Missing,
        "an elapsed record cannot be replaced"
    );
    assert!(
        store
            .insert_if_absent(&key, b"second", UnixMillis::new(2_000))
            .await
            .expect("the store answers"),
        "an elapsed record does not hold its key"
    );
}

#[tokio::test]
async fn removing_a_record_frees_its_key_and_absent_records_remove_cleanly() {
    let store = store(Arc::new(ManualClock::new(0)));
    let key = instance_key(0x20);
    store
        .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
        .await
        .expect("the store answers");

    store.remove(&key).await.expect("the store answers");
    assert!(store.load(&key).await.expect("the store answers").is_none());
    store
        .remove(&key)
        .await
        .expect("removing what is not there is not a failure");
}

#[tokio::test]
async fn promotion_reservations_live_in_their_own_key_space() {
    let clock = Arc::new(ManualClock::new(0));
    let store = store(clock.clone());
    let key = promotion_key(0x40);

    assert!(
        store
            .insert_promotion_if_absent(&key, b"reservation", UnixMillis::new(1_000))
            .await
            .expect("the store answers")
    );
    assert!(
        !store
            .insert_promotion_if_absent(&key, b"other", UnixMillis::new(1_000))
            .await
            .expect("the store answers")
    );
    assert_eq!(
        store
            .load_promotion(&key)
            .await
            .expect("the store answers")
            .expect("the reservation is there")
            .bytes,
        b"reservation".to_vec()
    );
    assert_eq!(
        store.count_instances().await.expect("the store answers"),
        0,
        "a reservation is not an instance"
    );

    clock.set(1_000);
    assert!(
        store
            .load_promotion(&key)
            .await
            .expect("the store answers")
            .is_none(),
        "reservations elapse by store time too"
    );
}

#[tokio::test]
async fn the_instance_count_is_what_capacity_is_measured_against() {
    let clock = Arc::new(ManualClock::new(0));
    let store = store(clock.clone());
    store
        .insert_if_absent(&instance_key(0x20), b"short", UnixMillis::new(1_000))
        .await
        .expect("the store answers");
    store
        .insert_if_absent(&instance_key(0x30), b"long", UnixMillis::new(3_000))
        .await
        .expect("the store answers");

    assert_eq!(store.count_instances().await.expect("the store answers"), 2);
    clock.set(1_000);
    assert_eq!(
        store.count_instances().await.expect("the store answers"),
        1,
        "an elapsed record is not held against the instance capacity"
    );
}

#[test]
fn the_record_frame_is_versioned_and_bounded() {
    assert_eq!(
        RECORD_VERSION, 1,
        "the record version is part of the stored format and changes deliberately"
    );
    assert_eq!(
        MAX_RECORD_BYTES, 32_768,
        "the record bound is part of the stored format and changes deliberately"
    );
}
