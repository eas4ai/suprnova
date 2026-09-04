//! File L1 publishes atomically, treats torn files as misses, bounds bytes
//! at the exact edge, and recovers its tally across a reopen.

use bytes::Bytes;
use suprnova::render_cache::file_store::FileRenderStore;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::store::{PublicationFence, PublishOutcome, RenderStore};

fn keys_from(root: u8) -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![root; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn key(path: &str) -> RenderKey {
    RenderKey::for_test(&keys_from(8), path)
}

fn fence(token: u64) -> PublicationFence {
    PublicationFence {
        epoch: 1,
        generation_digest: [0; 32],
        token,
    }
}

fn tmp_files(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().ends_with(".tmp"))
        .collect()
}

fn snrc_files(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().ends_with(".snrc"))
        .collect()
}

#[tokio::test]
async fn publication_is_atomic_and_visible_only_when_complete() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    let key = key("/a");
    assert_eq!(
        store
            .publish(&key, Bytes::from_static(b"entry-bytes"), fence(1), 1_000)
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    let hit = store.get(&key).await.expect("get").expect("hit");
    assert_eq!(hit.bytes.as_ref(), b"entry-bytes");
    assert_eq!(hit.fence.token, 1);
    assert!(
        tmp_files(dir.path()).is_empty(),
        "no temporary file survives a publication"
    );
}

#[tokio::test]
async fn a_torn_or_foreign_file_is_a_miss_and_is_evicted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    let key = key("/a");
    store
        .publish(&key, Bytes::from_static(b"entry-bytes"), fence(1), 1_000)
        .await
        .expect("publish");
    let path = store.path_for_test(&key);
    std::fs::write(&path, b"torn").expect("truncate in place");
    assert!(
        store.get(&key).await.expect("get").is_none(),
        "a file that fails the store frame is a miss"
    );
    assert!(!path.exists(), "the torn file was evicted");
}

#[tokio::test]
async fn fences_and_byte_bounds_hold_across_the_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 64).expect("open");
    let a = key("/a");
    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![1_u8; 40]), fence(2), 1)
            .await
            .expect("p"),
        PublishOutcome::Published
    );
    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![2_u8; 40]), fence(1), 2)
            .await
            .expect("p"),
        PublishOutcome::Fenced
    );
    assert_eq!(
        store
            .publish(&key("/b"), Bytes::from(vec![3_u8; 40]), fence(1), 3)
            .await
            .expect("p"),
        PublishOutcome::Published,
        "the oldest entry made room"
    );
    assert!(store.get(&a).await.expect("get").is_none());
}

#[tokio::test]
async fn an_entry_exactly_at_the_byte_bound_publishes_and_one_byte_more_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 64).expect("open");
    let a = key("/a");
    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![9_u8; 64]), fence(1), 1)
            .await
            .expect("p"),
        PublishOutcome::Published,
        "an entry of exactly the maximum publishes"
    );
    let before = snrc_files(dir.path());
    assert_eq!(before.len(), 1);

    let b = key("/b");
    assert_eq!(
        store
            .publish(&b, Bytes::from(vec![9_u8; 65]), fence(1), 2)
            .await
            .expect("p"),
        PublishOutcome::Rejected,
        "one byte more than the maximum is rejected, not evicted around"
    );
    assert_eq!(
        snrc_files(dir.path()),
        before,
        "a rejected publication leaves the directory unchanged"
    );
    let hit = store.get(&a).await.expect("get").expect("hit");
    assert_eq!(
        hit.bytes.len(),
        64,
        "the existing entry at the bound survives"
    );
}

#[tokio::test]
async fn a_single_flipped_byte_in_the_middle_of_a_valid_frame_is_a_miss_and_is_evicted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    let a = key("/a");
    store
        .publish(&a, Bytes::from(vec![5_u8; 40]), fence(1), 1_000)
        .await
        .expect("publish");
    let path = store.path_for_test(&a);
    let mut bytes = std::fs::read(&path).expect("read the real frame");
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x01;
    std::fs::write(&path, &bytes).expect("write back one flipped byte");

    assert!(
        store.get(&a).await.expect("get").is_none(),
        "a single flipped byte anywhere in the frame fails its digest"
    );
    assert!(!path.exists(), "the corrupted file was evicted");
}

#[tokio::test]
async fn evict_removes_the_file_and_succeeds_when_there_is_nothing_to_remove() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    let a = key("/a");

    store
        .evict(&a)
        .await
        .expect("evicting an absent key succeeds");
    assert!(store.get(&a).await.expect("get").is_none());

    store
        .publish(&a, Bytes::from_static(b"payload"), fence(1), 1)
        .await
        .expect("publish");
    let path = store.path_for_test(&a);
    assert!(path.exists());

    store.evict(&a).await.expect("evict");
    assert!(!path.exists(), "evict removes the file");
    assert!(store.get(&a).await.expect("get").is_none());

    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 0);
    assert_eq!(inspection.bytes, 0, "eviction frees the tracked bytes too");
}

#[tokio::test]
async fn inspect_reports_entries_and_payload_bytes_held() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    let empty = store.inspect().await.expect("inspect");
    assert_eq!(empty.entries, 0);
    assert_eq!(empty.bytes, 0);

    store
        .publish(&key("/a"), Bytes::from(vec![1_u8; 30]), fence(1), 1)
        .await
        .expect("publish a");
    store
        .publish(&key("/b"), Bytes::from(vec![2_u8; 20]), fence(1), 2)
        .await
        .expect("publish b");

    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 2);
    assert_eq!(
        inspection.bytes, 50,
        "the tally counts entry payload bytes, not on-disk frame size"
    );
}

#[tokio::test]
async fn reopening_the_store_recovers_previously_published_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = key("/a");
    {
        let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
        store
            .publish(
                &a,
                Bytes::from_static(b"survives-a-restart"),
                fence(3),
                5_000,
            )
            .await
            .expect("publish");
    }
    // The store above is dropped here, simulating a process restart: the
    // only thing carried forward is what is on disk.
    let reopened = FileRenderStore::open(dir.path(), 1024 * 1024).expect("reopen");
    let hit = reopened.get(&a).await.expect("get").expect("hit");
    assert_eq!(hit.bytes.as_ref(), b"survives-a-restart");
    assert_eq!(hit.published_at_ms, 5_000);
    assert_eq!(hit.fence.token, 3);

    let inspection = reopened.inspect().await.expect("inspect");
    assert_eq!(
        inspection.entries, 1,
        "the byte tally is rebuilt by scanning the directory at open"
    );
    assert_eq!(inspection.bytes, "survives-a-restart".len());

    // The recovered fence still gates future publications correctly.
    assert_eq!(
        reopened
            .publish(&a, Bytes::from_static(b"stale"), fence(2), 6_000)
            .await
            .expect("publish"),
        PublishOutcome::Fenced,
        "a lower token than what was recovered from disk is fenced"
    );
}

#[tokio::test]
async fn a_store_bounded_to_zero_bytes_admits_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 0).expect("open");
    let a = key("/a");
    assert_eq!(
        store
            .publish(&a, Bytes::from_static(b"x"), fence(1), 1)
            .await
            .expect("publish"),
        PublishOutcome::Rejected
    );
    assert!(store.get(&a).await.expect("get").is_none());
    assert!(snrc_files(dir.path()).is_empty());
}
