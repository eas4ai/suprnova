#![cfg(feature = "filesystem")]

//! Integration tests for atomic ordinary writes on the local-filesystem disk.
//!
//! Every test goes through the real public surface - `Storage::register_fs`,
//! `Storage::disk`, a `tempfile` root - because the staging directory is
//! configured during registration and a disk built any other way takes
//! opendal's non-atomic quick path. Each test takes a `Storage::fake()` guard
//! first, exactly like `framework/tests/filesystem.rs`: the guard serializes
//! against every other fake-using test through a process-wide mutex and wipes
//! the global disk registry on drop.

use std::time::Duration;
use suprnova::opendal::{Error, ErrorKind};
use suprnova::{ATOMIC_STAGING_DIR, DiskExt, Storage};

/// Register an fs disk named `local` over a fresh tempdir and return both the
/// tempdir (which the caller must keep alive) and the disk handle.
fn register_local_disk() -> (tempfile::TempDir, suprnova::opendal::Operator) {
    let tmp = tempfile::tempdir().expect("tempdir");
    Storage::register_fs("local", tmp.path()).expect("fs disk registers");
    let disk = Storage::disk("local").expect("registered fs disk");
    (tmp, disk)
}

/// The names sitting directly inside the staging directory, sorted.
fn staged_entries(root: &std::path::Path) -> Vec<String> {
    let staging = root.join(ATOMIC_STAGING_DIR);
    let mut names: Vec<String> = std::fs::read_dir(&staging)
        .unwrap_or_else(|e| panic!("the staging directory at {staging:?} must exist: {e}"))
        .map(|entry| {
            entry
                .expect("a staging directory entry reads")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// A write must never be observable at a length other than its final one.
///
/// Without a staging directory the fs backend opens the target with
/// `create + truncate` and streams into it in place, so the object is visible
/// at every intermediate length for the whole duration of the write - a
/// concurrent reader gets a short body with no error attached, and a crash
/// leaves that short body behind for good. Sampling the target while a large
/// write runs is the direct observation of that, and it does not depend on
/// winning a race.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ordinary_write_is_never_visible_at_a_partial_length() {
    let _guard = Storage::fake();
    let (_tmp, disk) = register_local_disk();

    // Large enough that the write spans many milliseconds, so the sampling
    // loop below takes a meaningful number of looks at the target.
    let payload = vec![b'x'; 64 * 1024 * 1024];
    let full_len = payload.len() as u64;

    let writer = disk.clone();
    let handle = tokio::spawn(async move {
        writer
            .write("big.bin", payload)
            .await
            .expect("the write resolves")
    });

    let mut samples = 0usize;
    let mut conclusive = 0usize;
    let mut partial = Vec::new();
    while !handle.is_finished() && samples < 4096 {
        tokio::time::sleep(Duration::from_micros(250)).await;
        samples += 1;
        match disk.stat("big.bin").await {
            Ok(metadata) if metadata.content_length() != full_len => {
                partial.push(metadata.content_length());
            }
            // Not published yet, or published whole. Both are states a reader
            // is allowed to see, and counting them is what proves the loop did
            // real work rather than finishing before the write started.
            Ok(_) | Err(_) => conclusive += 1,
        }
    }

    handle.await.expect("the writing task did not panic");
    // How many looks this gets is a property of the host, not of the code under
    // test, so the count is reported rather than pinned: a faster disk would
    // turn a correct implementation red. That it got *any* conclusive look is
    // asserted, because a zero-sample run would otherwise pass meaninglessly.
    eprintln!("sampled the target {samples} times during the write");
    // Order matters: a genuine regression makes every sample partial, and the
    // diagnostic that lists the lengths is the message worth reading. The
    // vacuity checks come after, for the case where nothing was sampled at all.
    assert!(
        partial.is_empty(),
        "the object was visible at a partial length: {:?}",
        &partial.iter().take(8).collect::<Vec<_>>()
    );
    assert!(
        samples > 0,
        "the sampling loop never looked at the target, so this run proves nothing"
    );
    assert!(
        conclusive > 0,
        "no sample resolved to either absent or the full length, so this run \
         proves nothing"
    );
    assert_eq!(
        disk.stat("big.bin")
            .await
            .expect("the finished object stats")
            .content_length(),
        full_len,
        "the rename publishes the whole object"
    );
}

/// The staging directory lives inside the root, so it would otherwise show up
/// as an object. It has to exist (or nothing is being staged) and it has to be
/// invisible to every listing helper, at every depth.
#[tokio::test]
async fn the_staging_directory_exists_and_is_hidden_from_every_listing() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    disk.write("visible.txt", "hi")
        .await
        .expect("write a root-level file");
    disk.write("real-dir/nested.txt", "hi")
        .await
        .expect("write a nested file");

    let staging = tmp.path().join(ATOMIC_STAGING_DIR);
    assert!(
        staging.is_dir(),
        "registering an fs disk must create the staging directory at {staging:?}"
    );

    let files = disk.files("", false).await.expect("files at the root");
    assert!(
        files.iter().all(|name| !name.contains(ATOMIC_STAGING_DIR)),
        "files(\"\") must not surface the staging directory, got: {files:?}"
    );
    assert!(
        files.contains(&"visible.txt".to_string()),
        "files(\"\") must still list real files, got: {files:?}"
    );

    let dirs = disk
        .directories("", false)
        .await
        .expect("directories at the root");
    assert!(
        dirs.iter().all(|name| !name.contains(ATOMIC_STAGING_DIR)),
        "directories(\"\") must not surface the staging directory, got: {dirs:?}"
    );
    assert!(
        dirs.contains(&"real-dir".to_string()),
        "directories(\"\") must still list a real sibling directory, got: {dirs:?}"
    );

    let all_files = disk.all_files("").await.expect("recursive file listing");
    assert!(
        all_files
            .iter()
            .all(|name| !name.contains(ATOMIC_STAGING_DIR)),
        "a recursive listing must not descend into the staging directory, got: {all_files:?}"
    );
    assert!(
        all_files.contains(&"real-dir/nested.txt".to_string()),
        "a recursive listing must still reach nested files, got: {all_files:?}"
    );

    let all_dirs = disk
        .all_directories("")
        .await
        .expect("recursive directory listing");
    assert!(
        all_dirs
            .iter()
            .all(|name| !name.contains(ATOMIC_STAGING_DIR)),
        "a recursive directory listing must not surface the staging directory, got: {all_dirs:?}"
    );
    assert!(
        all_dirs.contains(&"real-dir".to_string()),
        "a recursive directory listing must still list real directories, got: {all_dirs:?}"
    );
}

/// Assert that `err` is the reservation refusal rather than any other failure.
fn assert_reserved(err: &Error, operation: &str) {
    assert_eq!(
        err.kind(),
        ErrorKind::PermissionDenied,
        "{operation} on the reserved staging path must be PermissionDenied, got: {err}"
    );
    let message = err.to_string();
    assert!(
        message.contains(ATOMIC_STAGING_DIR),
        "{operation} must name the reserved directory, got: {message}"
    );
    assert!(
        message.contains("reserved"),
        "{operation} must say the name is reserved, got: {message}"
    );
}

/// A caller must not be able to reach into another writer's staging file, nor
/// collide with the reserved name by writing an object of its own there.
#[tokio::test]
async fn the_reserved_staging_name_is_refused_on_every_operation() {
    let _guard = Storage::fake();
    let (_tmp, disk) = register_local_disk();

    let reserved_file = format!("{ATOMIC_STAGING_DIR}/x");

    assert_reserved(
        &disk
            .write(&reserved_file, "payload")
            .await
            .expect_err("writing into the staging directory must be refused"),
        "write",
    );
    assert_reserved(
        &disk
            .read(&reserved_file)
            .await
            .expect_err("reading out of the staging directory must be refused"),
        "read",
    );
    assert_reserved(
        &disk
            .delete(&reserved_file)
            .await
            .expect_err("deleting inside the staging directory must be refused"),
        "delete",
    );
    assert_reserved(
        &disk
            .exists(&reserved_file)
            .await
            .expect_err("stat-ing inside the staging directory must be refused"),
        "exists",
    );
    // The directory itself, not just a path under it.
    assert_reserved(
        &disk
            .write(ATOMIC_STAGING_DIR, "payload")
            .await
            .expect_err("colliding with the reserved name itself must be refused"),
        "write onto the reserved name",
    );

    disk.write("ordinary-source.txt", "source")
        .await
        .expect("seed an ordinary source object");
    assert_reserved(
        &disk
            .copy("ordinary-source.txt", &reserved_file)
            .await
            .expect_err("copying into the staging directory must be refused"),
        "copy onto a staged path",
    );
    assert_reserved(
        &disk
            .copy(&reserved_file, "stolen.txt")
            .await
            .expect_err("copying out of the staging directory must be refused"),
        "copy out of a staged path",
    );
    assert_reserved(
        &disk
            .rename("ordinary-source.txt", &reserved_file)
            .await
            .expect_err("renaming into the staging directory must be refused"),
        "rename onto a staged path",
    );
    assert_reserved(
        &disk
            .create_dir(&format!("{ATOMIC_STAGING_DIR}/sub/"))
            .await
            .expect_err("creating a directory under staging must be refused"),
        "create_dir under the staging directory",
    );
    assert_reserved(
        &disk
            .presign_read(&reserved_file, Duration::from_secs(60))
            .await
            .expect_err("presigning a staged path must be refused"),
        "presign_read of a staged path",
    );

    let listing_err = disk
        .files(ATOMIC_STAGING_DIR, false)
        .await
        .expect_err("listing the staging directory must be refused");
    let message = listing_err.to_string();
    assert!(
        message.contains(ATOMIC_STAGING_DIR) && message.contains("reserved"),
        "listing the staging directory must be refused by name, got: {message}"
    );

    // A path that merely *contains* the reserved name deeper down is an
    // ordinary object and must be untouched by the reservation.
    disk.write("ordinary.txt", "fine")
        .await
        .expect("an ordinary write is unaffected");
    assert_eq!(
        &disk
            .read("ordinary.txt")
            .await
            .expect("an ordinary read is unaffected")
            .to_vec(),
        b"fine"
    );
    let nested = format!("nested/{ATOMIC_STAGING_DIR}/note.txt");
    disk.write(&nested, "not the reservation")
        .await
        .expect("the reservation only covers the first path component");
    assert!(
        disk.exists(&nested).await.expect("exists answers"),
        "a nested directory of the same name is an ordinary object"
    );
}

/// `abort` on a streaming writer must discard the staged file rather than
/// publish it, and must not leave the temp file behind either.
#[tokio::test]
async fn an_aborted_writer_leaves_no_target_and_an_empty_staging_directory() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    let mut writer = disk.writer("aborted.bin").await.expect("the writer opens");
    writer
        .write(vec![b'a'; 64 * 1024])
        .await
        .expect("a chunk is staged");
    writer
        .abort()
        .await
        .expect("abort discards the staged file");

    assert!(
        !disk
            .exists("aborted.bin")
            .await
            .expect("exists answers for the target"),
        "an aborted write must never publish its target"
    );
    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "abort must remove the temp file it staged"
    );
}

/// A conditional write still refuses to clobber, and the staging path must not
/// turn the refusal into a truncation of the object that was already there.
#[tokio::test]
async fn a_conditional_write_refuses_to_clobber_and_leaves_the_original_bytes() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    disk.write("guarded.txt", "original bytes")
        .await
        .expect("seed the object");

    let err = disk
        .write_with("guarded.txt", "replacement bytes")
        .if_not_exists(true)
        .await
        .expect_err("if_not_exists must refuse an existing object");
    assert_eq!(
        err.kind(),
        ErrorKind::ConditionNotMatch,
        "a refused conditional write reports ConditionNotMatch, got: {err}"
    );
    assert_eq!(
        &disk
            .read("guarded.txt")
            .await
            .expect("the original object is still readable")
            .to_vec(),
        b"original bytes",
        "a refused conditional write must not touch the object it refused to replace"
    );

    disk.write_with("fresh.txt", "brand new bytes")
        .if_not_exists(true)
        .await
        .expect("if_not_exists onto a missing object succeeds");
    assert_eq!(
        &disk
            .read("fresh.txt")
            .await
            .expect("the new object is readable")
            .to_vec(),
        b"brand new bytes"
    );

    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "neither the refused nor the accepted conditional write may leave a temp file"
    );
}

/// `append` is the one in-place operation: staging an append would mean copying
/// the whole object first. It has to keep working, it must not stage anything,
/// and that holds for the append that *creates* the object as much as for every
/// append after it.
#[tokio::test]
async fn an_append_extends_the_object_in_place() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    disk.write("log.txt", "first")
        .await
        .expect("seed the object");
    disk.write_with("log.txt", "-second")
        .append(true)
        .await
        .expect("append onto an existing object");

    assert_eq!(
        &disk
            .read("log.txt")
            .await
            .expect("the appended object is readable")
            .to_vec(),
        b"first-second",
        "append must extend rather than replace"
    );

    // The first append onto a missing object is the case opendal stages by
    // default, and staging it is what loses one of two racing first appends.
    disk.write_with("fresh-log.txt", "opened")
        .append(true)
        .await
        .expect("append onto a missing object creates it");
    disk.write_with("fresh-log.txt", "-extended")
        .append(true)
        .await
        .expect("the second append extends it");
    assert_eq!(
        &disk
            .read("fresh-log.txt")
            .await
            .expect("the created object is readable")
            .to_vec(),
        b"opened-extended",
        "an append that creates the object must still be an append"
    );

    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "an append writes in place, so it stages nothing - not even the first one"
    );
}

/// Build a path that reaches `target` from the process's current directory.
///
/// Used to prove a relative root still registers. `target` must be absolute,
/// which every `tempfile` root is.
#[cfg(unix)]
fn relative_to_current_dir(target: &std::path::Path) -> std::path::PathBuf {
    let current = std::env::current_dir().expect("the current directory resolves");
    let mut relative = std::path::PathBuf::new();
    for component in current.components() {
        if matches!(component, std::path::Component::Normal(_)) {
            relative.push("..");
        }
    }
    relative.join(
        target
            .strip_prefix("/")
            .expect("a tempfile root is absolute"),
    )
}

/// opendal creates the root and then the staging directory at build time, so a
/// root that does not exist yet must not break registration.
#[tokio::test]
async fn a_missing_root_registers_with_staging() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");

    let missing = tmp.path().join("not-created-yet/deeper");
    Storage::register_fs("missing", &missing).expect("a missing root registers");
    assert!(
        missing.join(ATOMIC_STAGING_DIR).is_dir(),
        "registration must create the staging directory under a freshly created root"
    );
    Storage::disk("missing")
        .expect("the missing-root disk")
        .write("x.txt", "created")
        .await
        .expect("the freshly created root is writable");
}

/// A relative root must stage inside itself, not beside the current directory.
#[cfg(unix)]
#[tokio::test]
async fn a_relative_root_registers_with_staging() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");

    let absolute = tmp.path().join("relative-root");
    std::fs::create_dir_all(&absolute).expect("create the relative root");
    let relative = relative_to_current_dir(&absolute);
    Storage::register_fs("relative", &relative).expect("a relative root registers");
    assert!(
        absolute.join(ATOMIC_STAGING_DIR).is_dir(),
        "a relative root must stage inside itself, not beside the current directory"
    );
    let disk = Storage::disk("relative").expect("the relative-root disk");
    disk.write("y.txt", "relative")
        .await
        .expect("the relative root is writable");
    assert_eq!(
        &disk
            .read("y.txt")
            .await
            .expect("the relative root is readable")
            .to_vec(),
        b"relative"
    );
}

/// `if_not_exists` is the primitive callers reach for to claim a key exactly
/// once - idempotency keys, upload dedupe, lock objects - so it has to be a real
/// exclusive create, not a check followed by an unconditional publish. Racing
/// writers are the only way to tell the two apart: a check-then-publish lets
/// every racer past the check, and the last publish silently discards the rest.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_conditional_writes_claim_the_key_exactly_once() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    const RACERS: usize = 16;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(RACERS));

    let handles: Vec<_> = (0..RACERS)
        .map(|racer| {
            let disk = disk.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                let payload = format!("claimed-by-racer-{racer:02}");
                barrier.wait().await;
                let outcome = disk
                    .write_with("claim.txt", payload.clone())
                    .if_not_exists(true)
                    .await;
                (payload, outcome)
            })
        })
        .collect();

    let mut winners = Vec::new();
    for handle in handles {
        let (payload, outcome) = handle.await.expect("a racing task did not panic");
        match outcome {
            Ok(_) => winners.push(payload),
            Err(err) => assert_eq!(
                err.kind(),
                ErrorKind::ConditionNotMatch,
                "a loser must be refused by the condition, not by something else: {err}"
            ),
        }
    }

    assert_eq!(
        winners.len(),
        1,
        "exactly one racer may claim the key, got {} winners: {winners:?}",
        winners.len()
    );
    assert_eq!(
        &disk
            .read("claim.txt")
            .await
            .expect("the claimed object is readable")
            .to_vec(),
        winners[0].as_bytes(),
        "the object must hold the winner's payload, not a later racer's"
    );
    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "neither the winner nor the losers may leave a temp file behind"
    );
}

/// The reservation is defeated by a symlink unless the resolved-path guard
/// enforces it too. This is the module's own threat model - an uploaded or
/// extracted symlink, then a read through it - pointed at the staging
/// directory, where reading an entry discloses another writer's in-flight
/// object and deleting one makes that writer's publish fail.
#[cfg(unix)]
#[tokio::test]
async fn a_symlink_into_the_staging_directory_is_refused() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    let staging = tmp.path().join(ATOMIC_STAGING_DIR);
    std::fs::write(
        staging.join("victim.tmp"),
        b"another writer's in-flight bytes",
    )
    .expect("plant an in-flight staging file");
    std::os::unix::fs::symlink(&staging, tmp.path().join("link"))
        .expect("plant a symlink to the staging directory");

    assert_reserved(
        &disk
            .read("link/victim.tmp")
            .await
            .expect_err("reading a staging file through a symlink must be refused"),
        "read through a symlink into staging",
    );
    assert_reserved(
        &disk
            .delete("link/victim.tmp")
            .await
            .expect_err("deleting a staging file through a symlink must be refused"),
        "delete through a symlink into staging",
    );
    assert_reserved(
        &disk
            .exists("link/victim.tmp")
            .await
            .expect_err("stat-ing a staging file through a symlink must be refused"),
        "stat through a symlink into staging",
    );
    assert_reserved(
        &disk
            .write("link/planted.tmp", "collide with a live publish")
            .await
            .expect_err("writing into staging through a symlink must be refused"),
        "write through a symlink into staging",
    );

    let listing_err = disk
        .files("link", false)
        .await
        .expect_err("listing staging through a symlink must be refused");
    let message = listing_err.to_string();
    assert!(
        message.contains(ATOMIC_STAGING_DIR) && message.contains("reserved"),
        "listing through the symlink must be refused by name, got: {message}"
    );
    // The symlink node itself resolves into staging and is refused too.
    assert_reserved(
        &disk
            .exists("link")
            .await
            .expect_err("the symlink node itself resolves into staging"),
        "stat of the symlink node",
    );

    // The in-flight object is untouched.
    assert_eq!(
        std::fs::read(staging.join("victim.tmp")).expect("the staging file is still there"),
        b"another writer's in-flight bytes",
        "a refused operation must not have reached the staging file"
    );
}

/// `copy` publishes bytes at a path just as `write` does, so it carries the same
/// promise. The fs driver copies straight into the destination, which is
/// observable at every intermediate length and leaves a truncated destination
/// behind a crash - the identical defect this task fixed for `write`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_is_never_visible_at_a_partial_length() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    let payload = vec![b'c'; 64 * 1024 * 1024];
    let full_len = payload.len() as u64;
    disk.write("source.bin", payload)
        .await
        .expect("seed the copy source");

    let copier = disk.clone();
    let handle = tokio::spawn(async move {
        copier
            .copy("source.bin", "destination.bin")
            .await
            .expect("the copy resolves")
    });

    let mut samples = 0usize;
    let mut conclusive = 0usize;
    let mut partial = Vec::new();
    while !handle.is_finished() && samples < 4096 {
        tokio::time::sleep(Duration::from_micros(250)).await;
        samples += 1;
        match disk.stat("destination.bin").await {
            Ok(metadata) if metadata.content_length() != full_len => {
                partial.push(metadata.content_length());
            }
            Ok(_) | Err(_) => conclusive += 1,
        }
    }

    handle.await.expect("the copying task did not panic");
    eprintln!("sampled the destination {samples} times during the copy");
    // Diagnostic first, vacuity checks after; see the write-side test.
    assert!(
        partial.is_empty(),
        "the copy destination was visible at a partial length: {:?}",
        &partial.iter().take(8).collect::<Vec<_>>()
    );
    assert!(
        samples > 0,
        "the sampling loop never looked at the destination, so this run proves nothing"
    );
    assert!(
        conclusive > 0,
        "no sample resolved to either absent or the full length, so this run \
         proves nothing"
    );
    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "a copy must not leave its staging file behind"
    );
}

/// Staging must not turn `copy` into a conditional operation: it overwrites an
/// existing destination, exactly as it did before.
#[tokio::test]
async fn a_copy_still_overwrites_an_existing_destination() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    disk.write("source.txt", "the new bytes")
        .await
        .expect("seed the source");
    disk.write(
        "nested/destination.txt",
        "the bytes that were already there",
    )
    .await
    .expect("seed the destination");

    disk.copy("source.txt", "nested/destination.txt")
        .await
        .expect("a copy onto an existing destination resolves");

    assert_eq!(
        &disk
            .read("nested/destination.txt")
            .await
            .expect("the destination is readable")
            .to_vec(),
        b"the new bytes",
        "copy must still overwrite"
    );
    // A destination whose parent directory does not exist yet is created, the
    // way the driver's own in-place copy created it.
    disk.copy("source.txt", "brand/new/tree.txt")
        .await
        .expect("a copy into a missing directory resolves");
    assert_eq!(
        &disk
            .read("brand/new/tree.txt")
            .await
            .expect("the new destination is readable")
            .to_vec(),
        b"the new bytes"
    );
    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "neither copy may leave a staging file behind"
    );
}

/// Two appenders racing to create the same missing object must both land. If
/// the first append is staged and published by rename, one of the two writes is
/// lost outright - which is the opposite of what an append means.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_first_appends_onto_a_missing_object_both_land() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let handles: Vec<_> = ["AAAAAAAA", "BBBBBBBB"]
        .into_iter()
        .map(|payload| {
            let disk = disk.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                disk.write_with("race-log.txt", payload)
                    .append(true)
                    .await
                    .expect("an append resolves")
            })
        })
        .collect();

    for handle in handles {
        handle.await.expect("an appending task did not panic");
    }

    let bytes = disk
        .read("race-log.txt")
        .await
        .expect("the appended object is readable")
        .to_vec();
    assert!(
        bytes == b"AAAAAAAABBBBBBBB" || bytes == b"BBBBBBBBAAAAAAAA",
        "both first appends must land, in either order; got {:?}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "an append stages nothing, so nothing may be left behind"
    );
}

/// A node sitting where the staging directory belongs would otherwise pass
/// registration - opendal only creates the directory when the path is missing -
/// and fail later, deep in the driver, on the first write. A *symlink* there is
/// worse than a file: opendal canonicalizes `atomic_write_dir`, so every staging
/// file would land at a path that is neither reserved nor filtered from
/// listings, defeating the reservation for that disk entirely. Refuse both where
/// the message can say what to do about it.
#[tokio::test]
async fn a_non_directory_occupying_the_reserved_name_is_refused_at_registration() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(ATOMIC_STAGING_DIR), b"not a directory")
        .expect("plant a file where the staging directory belongs");

    let err = Storage::register_fs("occupied", tmp.path())
        .expect_err("registration must refuse an occupied reserved name");
    let message = err.to_string();
    assert!(
        message.contains(ATOMIC_STAGING_DIR) && message.contains("reserved"),
        "the refusal must name the reservation, got: {message}"
    );

    // A symlink to a real directory satisfies a `metadata` probe, which follows
    // it. Only `symlink_metadata` sees the link for what it is.
    #[cfg(unix)]
    {
        let linked = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(linked.path().join("real-staging"))
            .expect("create the directory the link points at");
        std::os::unix::fs::symlink(
            linked.path().join("real-staging"),
            linked.path().join(ATOMIC_STAGING_DIR),
        )
        .expect("plant a symlink where the staging directory belongs");

        let err = Storage::register_fs("symlinked", linked.path())
            .expect_err("registration must refuse a symlinked reserved name");
        let message = err.to_string();
        assert!(
            message.contains(ATOMIC_STAGING_DIR) && message.contains("reserved"),
            "the refusal must name the reservation, got: {message}"
        );
    }

    // A directory of that name is what registration itself creates, so an
    // existing one must be accepted rather than tripping the same check.
    let reusable = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(reusable.path().join(ATOMIC_STAGING_DIR))
        .expect("pre-create the staging directory");
    Storage::register_fs("reusable", reusable.path())
        .expect("an existing staging directory is reused, not refused");
    Storage::disk("reusable")
        .expect("the reusable disk")
        .write("ok.txt", "written")
        .await
        .expect("the disk works over a pre-existing staging directory");
}

/// Assert `err` is the guard's symlink refusal rather than any other failure.
#[cfg(unix)]
fn assert_symlink_escape(err: &Error, operation: &str) {
    assert_eq!(
        err.kind(),
        ErrorKind::PermissionDenied,
        "{operation} through a dangling symlink must be PermissionDenied, got: {err}"
    );
    let message = err.to_string();
    assert!(
        message.contains("symlink"),
        "{operation} must be refused as a symlink escape, got: {message}"
    );
}

/// Every way of publishing bytes at `link_path` must be refused, and the
/// symlink's absent target must never come into existence.
///
/// `open(.., O_CREAT)` follows a symlink and creates the link's *target*, so a
/// dangling symlink planted in the root is an arbitrary-file-write primitive
/// wherever the layer creates rather than renames. `rename(2)` happens not to
/// follow it, but the guard cannot depend on which publish mechanism a given
/// operation reaches for.
#[cfg(unix)]
async fn assert_dangling_symlink_destination_is_refused(
    disk: &suprnova::opendal::Operator,
    link_path: &str,
    dangling_target: &std::path::Path,
) {
    disk.write("dangle-source.txt", "bytes to plant")
        .await
        .expect("seed an ordinary source object");

    assert_symlink_escape(
        &disk
            .write(link_path, "a plain write")
            .await
            .expect_err("a plain write onto a dangling symlink must be refused"),
        "write",
    );
    assert_symlink_escape(
        &disk
            .write_with(link_path, "an appending write")
            .append(true)
            .await
            .expect_err("an append onto a dangling symlink must be refused"),
        "append onto a missing object",
    );
    assert_symlink_escape(
        &disk
            .copy("dangle-source.txt", link_path)
            .await
            .expect_err("a copy onto a dangling symlink must be refused"),
        "copy destination",
    );
    assert_symlink_escape(
        &disk
            .rename("dangle-source.txt", link_path)
            .await
            .expect_err("a move onto a dangling symlink must be refused"),
        "move destination",
    );

    assert!(
        std::fs::symlink_metadata(dangling_target).is_err(),
        "the symlink's absent target must never come into existence at {dangling_target:?}"
    );
    assert!(
        disk.exists("dangle-source.txt")
            .await
            .expect("exists answers"),
        "a refused move must leave its source where it was"
    );
}

/// A dangling symlink inside the root: the guard's ancestor walk used to read
/// `canonicalize` returning `NotFound` as "nothing here yet, safe to create".
#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_inside_the_root_refuses_every_publish() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    let dangling_target = tmp.path().join("nowhere");
    std::os::unix::fs::symlink(&dangling_target, tmp.path().join("link"))
        .expect("plant a dangling symlink inside the root");

    assert_dangling_symlink_destination_is_refused(&disk, "link", &dangling_target).await;
}

/// The same symlink pointing out of the root, which is the escape itself: an
/// `O_CREAT` through it writes wherever the process can reach.
#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_pointing_outside_the_root_refuses_every_publish() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();
    let outside = tempfile::tempdir().expect("a directory outside the disk root");

    std::fs::create_dir_all(tmp.path().join("sub")).expect("create a real subdirectory");
    let dangling_target = outside.path().join("pwned");
    std::os::unix::fs::symlink(&dangling_target, tmp.path().join("sub/link"))
        .expect("plant a dangling symlink that leaves the root");

    assert_dangling_symlink_destination_is_refused(&disk, "sub/link", &dangling_target).await;
}

/// The positive control for the two above: refusing an unresolvable node must
/// not cost the ordinary case, where a path simply has not been written yet.
#[tokio::test]
async fn a_plain_missing_path_still_writes() {
    let _guard = Storage::fake();
    let (_tmp, disk) = register_local_disk();

    disk.write("existing/seed.txt", "seed")
        .await
        .expect("create the directory");
    disk.write("existing/fresh.txt", "written")
        .await
        .expect("a missing path under an existing directory still writes");
    disk.write_with("existing/appended.txt", "opened")
        .append(true)
        .await
        .expect("a first append under an existing directory still creates the object");
    disk.write("brand/new/deep.txt", "deep")
        .await
        .expect("a path whose whole tree is missing still writes");

    assert_eq!(
        &disk
            .read("existing/fresh.txt")
            .await
            .expect("the fresh object is readable")
            .to_vec(),
        b"written"
    );
    assert_eq!(
        &disk
            .read("existing/appended.txt")
            .await
            .expect("the appended object is readable")
            .to_vec(),
        b"opened"
    );
    assert_eq!(
        &disk
            .read("brand/new/deep.txt")
            .await
            .expect("the deep object is readable")
            .to_vec(),
        b"deep"
    );
}

/// The escape itself, isolated: an append through a dangling symlink must not
/// create the link's target.
///
/// This is the sharp end of the dangling-symlink case and the reason the guard
/// treats an unresolvable node as an escape rather than as free space.
/// `OpenOptions::open` follows the link, so `O_CREAT` lands on whatever it
/// points at - `~/.ssh/authorized_keys`, `/etc/cron.d/*`, a sibling app's
/// config - anywhere the process can reach whose parent directory exists.
#[cfg(unix)]
#[tokio::test]
async fn an_append_through_a_dangling_symlink_creates_nothing_outside_the_root() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();
    let outside = tempfile::tempdir().expect("a directory outside the disk root");

    let victim = outside.path().join("authorized_keys");
    std::os::unix::fs::symlink(&victim, tmp.path().join("innocent.txt"))
        .expect("plant a dangling symlink aimed out of the root");

    let outcome = disk
        .write_with("innocent.txt", "ssh-rsa AAAA... attacker\n")
        .append(true)
        .await;

    assert!(
        std::fs::symlink_metadata(&victim).is_err(),
        "an append must not create a file outside the disk root at {victim:?}; \
         it now holds {:?}",
        std::fs::read_to_string(&victim).unwrap_or_default()
    );
    assert_symlink_escape(
        &outcome.expect_err("the append must be refused, not merely contained"),
        "append",
    );
}

/// The two shapes the guard's ancestor walk has to tell apart once it refuses a
/// node it cannot resolve: a symlinked directory that resolves inside the root
/// is a legitimate layout and must keep working, while a dangling symlink at an
/// *intermediate* component is the same escape as one at the leaf.
#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_directory_still_writes_but_a_dangling_one_never_does() {
    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    std::fs::create_dir_all(tmp.path().join("real")).expect("create the real directory");
    std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("dir_link"))
        .expect("plant a legitimate directory symlink");

    disk.write("dir_link/new.txt", "through the link")
        .await
        .expect("a new leaf under a resolvable symlinked directory still writes");
    assert_eq!(
        std::fs::read(tmp.path().join("real/new.txt"))
            .expect("the object landed in the real directory"),
        b"through the link",
        "a write through a symlinked directory must land inside the root"
    );

    std::fs::create_dir_all(tmp.path().join("a")).expect("create an intermediate directory");
    let dangling_target = tmp.path().join("gone");
    std::os::unix::fs::symlink(&dangling_target, tmp.path().join("a/link"))
        .expect("plant a dangling symlink at an intermediate component");

    let err = disk
        .write("a/link/b.txt", "through a broken link")
        .await
        .expect_err("a write under a dangling intermediate symlink must be refused");
    assert!(
        std::fs::symlink_metadata(&dangling_target).is_err(),
        "nothing may be created at the dangling target {dangling_target:?}"
    );
    assert_symlink_escape(&err, "write under a dangling intermediate symlink");
}

/// The conditional-write race has to survive neighbours, not just itself.
///
/// The guard resolves a path by walking it, and a walk is a sequence of
/// observations of a filesystem other tasks are changing underneath it. If any
/// single component is observed twice and the two observations are combined into
/// one verdict, ordinary concurrent activity - another racer publishing the very
/// key these racers are contending for, an unrelated sibling appearing and
/// vanishing, a staging directory full of other writers' temp files - can be
/// read as an escape and refuse a legitimate path. That failure is invisible in
/// isolation and shows up as a flake under a loaded suite, so the neighbours are
/// part of the test rather than something it hopes for.
///
/// Every loser must be refused by the *condition*. A loser refused by the guard
/// is the bug, and it is asserted separately from the winner count so the two
/// failures never get confused for one another.
///
/// Every assertion runs after the neighbours have been stopped and joined. A
/// panic between spawning them and stopping them would leave two blocking tasks
/// looping forever, and dropping the runtime waits for those - so the test would
/// hang instead of failing.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn conditional_writes_claim_the_key_exactly_once_under_a_hostile_neighbour() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// How long a neighbour keeps going even if nothing ever stops it. Belt and
    /// braces against the hang described above.
    const NEIGHBOUR_BUDGET: Duration = Duration::from_secs(60);

    let _guard = Storage::fake();
    let (tmp, disk) = register_local_disk();

    let stop = Arc::new(AtomicBool::new(false));

    // Neighbour one: an unrelated sibling in the same directory the racers'
    // target lives in, appearing and vanishing throughout.
    let churn_root = tmp.path().to_path_buf();
    let churn_stop = stop.clone();
    let churn = tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + NEIGHBOUR_BUDGET;
        let mut round = 0u64;
        while !churn_stop.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            let sibling = churn_root.join(format!("neighbour-{}.bin", round % 8));
            let _ = std::fs::write(&sibling, b"noise");
            let _ = std::fs::remove_file(&sibling);
            round = round.wrapping_add(1);
            // Throttled on purpose. An unthrottled loop here is a denial of
            // service on the directory rather than a neighbour, and it starves
            // the racers this test is actually about.
            std::thread::sleep(Duration::from_micros(50));
        }
    });

    // Neighbour two: the staging directory under the load a dozen other writers
    // would put it under, since that is where every conditional write stages.
    let staging = tmp.path().join(ATOMIC_STAGING_DIR);
    let hammer_stop = stop.clone();
    let hammer = tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + NEIGHBOUR_BUDGET;
        let mut round = 0u64;
        while !hammer_stop.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            let temp = staging.join(format!("hostile-{}.stage", round % 8));
            let _ = std::fs::write(&temp, b"another writer's in-flight bytes");
            let _ = std::fs::remove_file(&temp);
            round = round.wrapping_add(1);
            std::thread::sleep(Duration::from_micros(50));
        }
    });

    const ROUNDS: usize = 24;
    const RACERS: usize = 16;

    /// What one round produced, collected before the neighbours are stopped so
    /// no assertion can fire while they are still looping.
    struct RoundOutcome {
        round: usize,
        winners: Vec<String>,
        guard_refusals: Vec<String>,
        stored: Vec<u8>,
    }

    let mut results: Vec<RoundOutcome> = Vec::new();

    for round in 0..ROUNDS {
        let key = format!("claim-{round}.txt");
        let barrier = Arc::new(tokio::sync::Barrier::new(RACERS));

        let handles: Vec<_> = (0..RACERS)
            .map(|racer| {
                let disk = disk.clone();
                let barrier = barrier.clone();
                let key = key.clone();
                tokio::spawn(async move {
                    let payload = format!("round-{round}-racer-{racer:02}");
                    barrier.wait().await;
                    // Half go together, half arrive spread out. The simultaneous
                    // half contends for the publish; the staggered half arrives
                    // while an earlier racer is mid-publish, which is when a
                    // walk sees the key appear between two observations.
                    if racer % 2 == 1 {
                        tokio::time::sleep(Duration::from_micros(20 * racer as u64)).await;
                    }
                    let outcome = disk
                        .write_with(key.as_str(), payload.clone())
                        .if_not_exists(true)
                        .await;
                    (payload, outcome)
                })
            })
            .collect();

        let mut winners = Vec::new();
        let mut guard_refusals = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((payload, Ok(_))) => winners.push(payload),
                Ok((_, Err(err))) if err.kind() == ErrorKind::ConditionNotMatch => {}
                Ok((_, Err(err))) => guard_refusals.push(format!("{err}")),
                Err(e) => guard_refusals.push(format!("a racing task panicked: {e}")),
            }
        }
        let stored = disk
            .read(key.as_str())
            .await
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();
        results.push(RoundOutcome {
            round,
            winners,
            guard_refusals,
            stored,
        });
    }

    stop.store(true, Ordering::Relaxed);
    churn.await.expect("the churn neighbour did not panic");
    hammer.await.expect("the staging neighbour did not panic");

    for RoundOutcome {
        round,
        winners,
        guard_refusals,
        stored,
    } in &results
    {
        assert!(
            guard_refusals.is_empty(),
            "round {round}: {} loser(s) were refused by the path guard rather \
             than by the condition; concurrent activity on a path made of \
             ordinary files must never look like an escape: {guard_refusals:#?}",
            guard_refusals.len()
        );
        assert_eq!(
            winners.len(),
            1,
            "round {round}: exactly one racer may claim the key, got: {winners:?}"
        );
        assert_eq!(
            stored,
            winners[0].as_bytes(),
            "round {round}: the object must hold the winner's payload"
        );
    }
}
