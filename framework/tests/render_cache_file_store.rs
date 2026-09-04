//! File L1 publishes atomically, treats torn files as misses, bounds bytes
//! at the exact edge, recovers its tally across a reopen, and keeps the
//! tally consistent with the directory when an evict or a corruption
//! cleanup races a publish for the same key.

use std::sync::Arc;

use bytes::Bytes;
use suprnova::render_cache::file_store::FileRenderStore;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::store::{PublicationFence, PublishOutcome, RenderStore};
use tokio::sync::Barrier;

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
    // Re-read right here, before the eviction below has a chance to remove
    // "/a" for an unrelated reason: a write-through-despite-fencing bug
    // must be caught at the moment of the fenced refusal, not hidden by a
    // later step that happens to make the entry disappear anyway.
    let still_a = store
        .get(&a)
        .await
        .expect("get")
        .expect("a must survive a fenced publish");
    assert_eq!(
        still_a.bytes.as_ref(),
        vec![1_u8; 40].as_slice(),
        "a fenced publish must leave the existing bytes byte-for-byte unchanged"
    );
    assert_eq!(
        still_a.fence.token, 2,
        "the fence itself must be unchanged too"
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
    // The exact edge, not just a non-empty payload above it: `0 > 0` is
    // false, so a size guard that only checks `payload_len > max_payload`
    // would admit this one even though the bound is zero.
    assert_eq!(
        store
            .publish(&a, Bytes::new(), fence(1), 1)
            .await
            .expect("publish"),
        PublishOutcome::Rejected,
        "a zero-byte bound must reject an empty payload too"
    );
    assert!(store.get(&a).await.expect("get").is_none());
    assert!(snrc_files(dir.path()).is_empty());
}

#[tokio::test]
async fn an_equal_fence_is_refused_and_leaves_the_entry_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    let a = key("/a");
    assert_eq!(
        store
            .publish(&a, Bytes::from_static(b"first"), fence(5), 1)
            .await
            .expect("p"),
        PublishOutcome::Published
    );
    assert_eq!(
        store
            .publish(&a, Bytes::from_static(b"second"), fence(5), 2)
            .await
            .expect("p"),
        PublishOutcome::Fenced,
        "an equal fence, same epoch and same token, must not supersede"
    );
    let hit = store.get(&a).await.expect("get").expect("hit");
    assert_eq!(hit.bytes.as_ref(), b"first");
    assert_eq!(hit.fence.token, 5);
}

#[tokio::test]
async fn growing_the_same_key_past_the_bound_evicts_others_but_never_itself() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 100).expect("open");
    let a = key("/a");
    let b = key("/b");
    let c = key("/c");
    store
        .publish(&a, Bytes::from(vec![1_u8; 40]), fence(1), 1)
        .await
        .expect("publish a");
    store
        .publish(&b, Bytes::from(vec![2_u8; 20]), fence(1), 2)
        .await
        .expect("publish b");
    store
        .publish(&c, Bytes::from(vec![3_u8; 20]), fence(1), 3)
        .await
        .expect("publish c");

    // Growing "/a" from 40 to 70 bytes genuinely breaches the bound
    // (80 - 40 + 70 = 110 > 100), so the eviction candidate loop must
    // actually run, not just be entered and immediately satisfied. "/b" is
    // the oldest other entry and must be evicted to make room; "/c" still
    // fits once "/b" is gone (110 - 20 = 90 <= 100) and must be left
    // alone; "/a" itself must never be picked as its own eviction victim,
    // even though its own old published_at_ms is older than both.
    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![4_u8; 70]), fence(2), 4)
            .await
            .expect("publish a again"),
        PublishOutcome::Published
    );

    assert!(
        store.get(&b).await.expect("get b").is_none(),
        "the oldest unrelated entry is evicted to make genuine room"
    );
    let hit_c = store
        .get(&c)
        .await
        .expect("get c")
        .expect("c still fits once b is gone and must be left alone");
    assert_eq!(hit_c.bytes.len(), 20);
    let hit_a = store
        .get(&a)
        .await
        .expect("get a")
        .expect("a must be present with its new, grown bytes");
    assert_eq!(hit_a.bytes.len(), 70);

    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 2, "a and c; b was evicted");
    assert_eq!(inspection.bytes, 90);
}

#[tokio::test]
async fn open_removes_a_pre_existing_torn_entry_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = key("/a");
    let torn_path = dir.path().join(format!("{}.snrc", a.to_base64url()));
    std::fs::write(&torn_path, b"not a valid frame at all").expect("plant a torn entry file");

    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    assert!(
        !torn_path.exists(),
        "open removes a pre-existing torn entry file"
    );
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 0);
    assert!(store.get(&a).await.expect("get").is_none());
}

#[tokio::test]
async fn open_removes_orphaned_temporary_files_left_by_a_crash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stray = dir.path().join("rk1.some-key-text.12345.7.tmp");
    std::fs::write(&stray, b"leftover from a crash between write and rename")
        .expect("plant a stray temporary file");

    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    assert!(
        !stray.exists(),
        "open removes a temporary file left by a crash between write and rename"
    );
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 0);
}

/// Aligns two tasks on a barrier and runs them concurrently, so the race
/// under test starts from the same instant on every trial rather than
/// depending on incidental scheduling.
async fn race<F1, F2>(first: F1, second: F2)
where
    F1: std::future::Future<Output = ()> + Send + 'static,
    F2: std::future::Future<Output = ()> + Send + 'static,
{
    let barrier = Arc::new(Barrier::new(2));
    let first_barrier = barrier.clone();
    let first_task = tokio::spawn(async move {
        first_barrier.wait().await;
        first.await;
    });
    let second_task = tokio::spawn(async move {
        barrier.wait().await;
        second.await;
    });
    first_task.await.expect("first task joins");
    second_task.await.expect("second task joins");
}

/// An evict racing a publish for the same key is a normal event for a
/// web-serving cache (an invalidation racing a slow rebuild). Repeated many
/// times with a real multi-thread runtime so the interleaving that leaves a
/// published file on disk untracked by the tally - reachable only when
/// `evict` removes the file before taking the lock - gets a fair chance to
/// occur on at least one trial if it still can. Reverting the `evict` fix
/// (taking the lock only after `remove_file` again) reliably fails this
/// test within the loop below.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_evict_racing_a_publish_never_leaves_an_untracked_file() {
    for seed in 0_u8..100 {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(FileRenderStore::open(dir.path(), 1024 * 1024).expect("open"));
        let a = key("/a");
        store
            .publish(&a, Bytes::from_static(b"seed-bytes"), fence(1), 1)
            .await
            .expect("seed publish");

        let republished = vec![seed; 24];
        let evict_store = store.clone();
        let evict_key = a.clone();
        let publish_store = store.clone();
        let publish_key = a.clone();
        let publish_bytes = Bytes::from(republished.clone());
        race(
            async move {
                evict_store.evict(&evict_key).await.expect("evict");
            },
            async move {
                let outcome = publish_store
                    .publish(&publish_key, publish_bytes, fence(2), 2)
                    .await
                    .expect("publish");
                assert_eq!(
                    outcome,
                    PublishOutcome::Published,
                    "seed {seed}: a higher fence always publishes regardless of interleaving"
                );
            },
        )
        .await;

        let hit = store.get(&a).await.expect("get");
        let inspection = store.inspect().await.expect("inspect");
        match hit {
            Some(entry) => {
                assert_eq!(
                    entry.bytes.as_ref(),
                    republished.as_slice(),
                    "seed {seed}: a file genuinely on disk must be the republished one"
                );
                assert_eq!(
                    inspection.entries, 1,
                    "seed {seed}: a file genuinely on disk must be tracked"
                );
                assert_eq!(inspection.bytes, republished.len());
            }
            None => {
                assert_eq!(
                    inspection.entries, 0,
                    "seed {seed}: no file on disk means nothing should be tracked either"
                );
                assert_eq!(inspection.bytes, 0);
            }
        }
    }
}

/// The same race, aimed at `get`'s corruption-cleanup path instead of
/// `evict`: a reader observes a corrupted file (as an external actor would
/// leave one) at the same moment a legitimate publish is replacing it.
/// Cleaning up by path alone on the strength of a read taken before any
/// lock was held would delete the republished file; reverting to that
/// shape (drop the second, lock-held re-read and re-decode in `get`, and
/// unconditionally remove the path instead) reliably fails this test
/// within the loop below.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_get_cleanup_racing_a_publish_never_destroys_the_republished_entry() {
    for seed in 0_u8..100 {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(FileRenderStore::open(dir.path(), 1024 * 1024).expect("open"));
        let a = key("/a");
        store
            .publish(&a, Bytes::from_static(b"seed-bytes"), fence(1), 1)
            .await
            .expect("seed publish");
        // Corrupt the file the way an external actor would, so a
        // concurrent `get` takes the corruption-cleanup path.
        let path = store.path_for_test(&a);
        let mut bytes = std::fs::read(&path).expect("read the real frame");
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0x01;
        std::fs::write(&path, &bytes).expect("write back one flipped byte");

        let republished = vec![seed; 24];
        let get_store = store.clone();
        let get_key = a.clone();
        let publish_store = store.clone();
        let publish_key = a.clone();
        let publish_bytes = Bytes::from(republished.clone());
        race(
            async move {
                let _ = get_store.get(&get_key).await;
            },
            async move {
                let outcome = publish_store
                    .publish(&publish_key, publish_bytes, fence(2), 2)
                    .await
                    .expect("publish");
                assert_eq!(
                    outcome,
                    PublishOutcome::Published,
                    "seed {seed}: a higher fence always publishes even racing a corruption cleanup"
                );
            },
        )
        .await;

        let hit = store
            .get(&a)
            .await
            .expect("get")
            .unwrap_or_else(|| panic!("seed {seed}: the republished entry must survive"));
        assert_eq!(
            hit.bytes.as_ref(),
            republished.as_slice(),
            "seed {seed}: a concurrent corruption cleanup must never destroy a fresh publish"
        );
        let inspection = store.inspect().await.expect("inspect");
        assert_eq!(inspection.entries, 1);
        assert_eq!(inspection.bytes, republished.len());
    }
}

/// A directory-sync failure right after a successful rename must not leave
/// the disk and the tally disagreeing: the rename already landed, so the
/// publication succeeded, and the tally must say so.
///
/// Forces the failure with real permissions rather than a fault-injection
/// hook: a directory chmoded to write and execute only (no read) still
/// allows creating a file in it and renaming within it, but not opening the
/// directory itself to `fsync` it - exactly the step this test targets.
#[cfg(unix)]
#[tokio::test]
async fn a_directory_sync_failure_after_a_successful_rename_still_updates_the_tally() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 1024 * 1024).expect("open");
    let a = key("/a");

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o300))
        .expect("restrict the directory to write and execute only");
    if std::fs::OpenOptions::new()
        .read(true)
        .open(dir.path())
        .is_ok()
    {
        // A process that ignores directory modes (running as root, or an
        // exotic filesystem) cannot express this precondition at all.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restore permissions");
        eprintln!(
            "skipping: this process can open a directory it has no read permission on, \
             so a directory-sync failure cannot be simulated"
        );
        return;
    }

    let outcome = store
        .publish(
            &a,
            Bytes::from_static(b"live-despite-a-sync-failure"),
            fence(1),
            1,
        )
        .await
        .expect("a directory sync failure is a durability warning, not a publish failure");
    assert_eq!(
        outcome,
        PublishOutcome::Published,
        "once the rename lands, the publication succeeded"
    );

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions so the assertions below can read the directory");

    let hit = store
        .get(&a)
        .await
        .expect("get")
        .expect("the frame is live on disk regardless of the sync failure");
    assert_eq!(hit.bytes.as_ref(), b"live-despite-a-sync-failure");
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(
        inspection.entries, 1,
        "the tally must agree with the disk even when the directory sync failed"
    );
    assert_eq!(inspection.bytes, "live-despite-a-sync-failure".len());
}

/// Occupies `key`'s on-disk path with an empty directory, so a subsequent
/// `remove_file` against it fails with a real, non-`NotFound` error
/// (`IsADirectory`) instead of silently succeeding. A real filesystem
/// condition rather than a test-only fault-injection hook in the store
/// itself, the same spirit as the directory-sync test above, adapted to a
/// call site where restricting directory permissions would also break the
/// publish under test's own write.
fn make_removal_fail(store: &FileRenderStore, key: &RenderKey) {
    let path = store.path_for_test(key);
    std::fs::remove_file(&path).expect("remove the real file before replacing it");
    std::fs::create_dir(&path).expect("occupy the path with a directory");
}

#[tokio::test]
async fn a_victim_whose_removal_fails_stays_tracked_and_a_different_candidate_is_evicted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 100).expect("open");
    let a = key("/a");
    let b = key("/b");
    let c = key("/c");
    store
        .publish(&a, Bytes::from(vec![1_u8; 40]), fence(1), 1)
        .await
        .expect("publish a");
    store
        .publish(&b, Bytes::from(vec![2_u8; 20]), fence(1), 2)
        .await
        .expect("publish b");
    store
        .publish(&c, Bytes::from(vec![3_u8; 20]), fence(1), 3)
        .await
        .expect("publish c");

    make_removal_fail(&store, &b);

    // Growing "/a" from 40 to 70 genuinely breaches the bound
    // (80 - 40 + 70 = 110 > 100). "/b" is the oldest candidate, but its
    // removal fails, so the loop must leave it tracked and move on to
    // "/c" (110 - 20 = 90 <= 100) instead of silently treating "/b" as
    // freed space it never actually gave back.
    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![4_u8; 70]), fence(2), 4)
            .await
            .expect("publish a again"),
        PublishOutcome::Published
    );

    assert!(
        store.get(&c).await.expect("get c").is_none(),
        "the next candidate is evicted once the first one's removal fails"
    );
    let hit_a = store
        .get(&a)
        .await
        .expect("get a")
        .expect("a must be present with its new, grown bytes");
    assert_eq!(hit_a.bytes.len(), 70);

    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(
        inspection.entries, 2,
        "a and b; b's removal failed, so it stays tracked rather than being written off"
    );
    assert_eq!(
        inspection.bytes, 90,
        "b's bytes are still counted since they were never actually freed"
    );
}

#[tokio::test]
async fn publish_is_rejected_and_changes_nothing_when_eviction_cannot_free_enough_room() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileRenderStore::open(dir.path(), 100).expect("open");
    let a = key("/a");
    let b = key("/b");
    let c = key("/c");
    store
        .publish(&a, Bytes::from(vec![1_u8; 40]), fence(1), 1)
        .await
        .expect("publish a");
    store
        .publish(&b, Bytes::from(vec![2_u8; 20]), fence(1), 2)
        .await
        .expect("publish b");
    store
        .publish(&c, Bytes::from(vec![3_u8; 20]), fence(1), 3)
        .await
        .expect("publish c");

    // Both other entries are made undeletable, so the eviction loop
    // exhausts every candidate without ever freeing enough room.
    make_removal_fail(&store, &b);
    make_removal_fail(&store, &c);

    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![4_u8; 70]), fence(2), 4)
            .await
            .expect("publish a again"),
        PublishOutcome::Rejected,
        "a bound that cannot be honoured is a rejection, not an over-limit acceptance"
    );

    let hit_a = store
        .get(&a)
        .await
        .expect("get a")
        .expect("a's original entry is untouched");
    assert_eq!(hit_a.bytes.len(), 40, "a rejected publish changes nothing");
    assert_eq!(hit_a.fence.token, 1);

    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(
        inspection.entries, 3,
        "nothing was actually evicted, and nothing new was written"
    );
    assert_eq!(inspection.bytes, 80);
}
