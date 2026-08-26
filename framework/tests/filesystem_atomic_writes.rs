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
    let mut partial = Vec::new();
    while !handle.is_finished() && samples < 4096 {
        tokio::time::sleep(Duration::from_micros(250)).await;
        samples += 1;
        if let Ok(metadata) = disk.stat("big.bin").await
            && metadata.content_length() != full_len
        {
            partial.push(metadata.content_length());
        }
    }

    handle.await.expect("the writing task did not panic");
    // How many looks this gets is a property of the host, not of the code under
    // test, so it is reported rather than asserted: a faster disk would turn a
    // correct implementation red.
    eprintln!("sampled the target {samples} times during the write");
    assert!(
        partial.is_empty(),
        "the object was visible at a partial length: {:?}",
        &partial.iter().take(8).collect::<Vec<_>>()
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

/// `append` is the documented exception: opendal writes it in place, because
/// staging an append would mean copying the whole object first. It has to keep
/// working, and it must not stage anything.
#[tokio::test]
async fn an_append_still_extends_the_object_in_place() {
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
    assert_eq!(
        staged_entries(tmp.path()),
        Vec::<String>::new(),
        "an append writes in place, so it stages nothing"
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

/// opendal creates the root and then the staging directory at build time, so
/// neither a root that does not exist yet nor a relative root may break
/// registration - and both must end up with a staging directory of their own.
#[cfg(unix)]
#[tokio::test]
async fn a_relative_root_and_a_missing_root_both_register_with_staging() {
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
