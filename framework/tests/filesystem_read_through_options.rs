#![cfg(feature = "filesystem")]

//! Read-through disk options: `copy: false` (no promotion) and `copy` /
//! `rename` of a source that lives only on the fallback.
//!
//! The primary is a local-filesystem disk in the tests that exercise the
//! primary-source branch, because the in-memory driver implements neither
//! `copy` nor `rename`.
//!
//! Every operation used here (`read`, `write`, `exists`, `copy`, `rename`) is
//! native to `Operator`, so `DiskExt` is deliberately not imported.

use suprnova::opendal::ErrorKind;
use suprnova::{ReadThroughConfig, Storage};

#[tokio::test]
async fn copy_false_serves_the_fallback_without_promoting() {
    let _guard = Storage::fake();
    Storage::register_memory("primary");
    Storage::register_memory("fallback");
    Storage::register_read_through(
        "overlay",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            copy: false,
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let overlay = Storage::disk("overlay").expect("read-through disk");
    assert_eq!(
        &overlay
            .read("cold.txt")
            .await
            .expect("read resolves from the fallback")
            .to_vec(),
        b"cold bytes"
    );

    assert!(
        !Storage::disk("primary")
            .expect("primary disk")
            .exists("cold.txt")
            .await
            .expect("primary exists answers"),
        "copy: false must serve the fallback without writing through"
    );
}

#[tokio::test]
async fn copy_false_honours_a_ranged_read() {
    let _guard = Storage::fake();
    Storage::register_memory("primary");
    Storage::register_memory("fallback");
    Storage::register_read_through(
        "overlay",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            copy: false,
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let overlay = Storage::disk("overlay").expect("read-through disk");
    assert_eq!(
        &overlay
            .read_with("cold.txt")
            .range(5..10)
            .await
            .expect("a ranged read resolves from the fallback")
            .to_vec(),
        b"bytes",
        "a non-promoting read still honours the caller's range"
    );
}

#[tokio::test]
async fn copy_defaults_to_true() {
    let _guard = Storage::fake();
    Storage::register_memory("primary");
    Storage::register_memory("fallback");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");
    Storage::disk("assets")
        .expect("read-through disk")
        .read("cold.txt")
        .await
        .expect("read resolves");

    assert!(
        Storage::disk("primary")
            .expect("primary disk")
            .exists("cold.txt")
            .await
            .expect("primary exists answers"),
        "the default configuration still promotes, matching Laravel's copy = true"
    );
}

/// Register an fs `primary` at `root`, a memory `fallback`, and a read-through
/// disk named `disk` over them with the given `copy` flag.
fn register_fs_primary(root: &std::path::Path, disk: &str, copy: bool) {
    Storage::register_fs("primary", root).expect("fs primary");
    Storage::register_memory("fallback");
    Storage::register_read_through(
        disk,
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            copy,
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");
}

#[tokio::test]
async fn copy_streams_a_fallback_only_source_and_leaves_it_in_place() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    let fallback = Storage::disk("fallback").expect("fallback disk");
    fallback
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    Storage::disk("assets")
        .expect("read-through disk")
        .copy("cold.txt", "warm.txt")
        .await
        .expect("copy spans the fallback");

    let primary = Storage::disk("primary").expect("primary disk");
    assert_eq!(
        &primary
            .read("warm.txt")
            .await
            .expect("destination lands on the primary")
            .to_vec(),
        b"cold bytes"
    );
    assert!(
        !primary.exists("cold.txt").await.expect("primary exists"),
        "copy must not promote the source, only write the destination"
    );
    assert!(
        fallback.exists("cold.txt").await.expect("fallback exists"),
        "copy leaves the fallback source in place"
    );
}

#[tokio::test]
async fn copy_of_a_primary_source_stays_on_the_primary() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    let primary = Storage::disk("primary").expect("primary disk");
    let fallback = Storage::disk("fallback").expect("fallback disk");
    primary
        .write("both.txt", "primary copy")
        .await
        .expect("seed the primary");
    fallback
        .write("both.txt", "stale fallback copy")
        .await
        .expect("seed the fallback");

    Storage::disk("assets")
        .expect("read-through disk")
        .copy("both.txt", "warm.txt")
        .await
        .expect("copy uses the primary");

    assert_eq!(
        &primary
            .read("warm.txt")
            .await
            .expect("destination lands on the primary")
            .to_vec(),
        b"primary copy",
        "the primary's copy is the one that is copied"
    );
    assert!(
        fallback.exists("both.txt").await.expect("fallback exists"),
        "a copy never touches the fallback source"
    );
}

#[tokio::test]
async fn rename_streams_a_fallback_only_source_and_deletes_it() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    let fallback = Storage::disk("fallback").expect("fallback disk");
    fallback
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let assets = Storage::disk("assets").expect("read-through disk");
    assets
        .rename("cold.txt", "moved.txt")
        .await
        .expect("rename spans the fallback");

    assert_eq!(
        &Storage::disk("primary")
            .expect("primary disk")
            .read("moved.txt")
            .await
            .expect("destination lands on the primary")
            .to_vec(),
        b"cold bytes"
    );
    assert!(
        !fallback.exists("cold.txt").await.expect("fallback exists"),
        "a move must clear the fallback source"
    );
    assert!(
        !assets
            .exists("cold.txt")
            .await
            .expect("read-through exists answers"),
        "a later read must not resurrect the moved object from the fallback"
    );
}

#[tokio::test]
async fn rename_of_a_primary_source_still_clears_the_fallback_copy() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    let primary = Storage::disk("primary").expect("primary disk");
    let fallback = Storage::disk("fallback").expect("fallback disk");
    primary
        .write("both.txt", "primary copy")
        .await
        .expect("seed the primary");
    fallback
        .write("both.txt", "stale fallback copy")
        .await
        .expect("seed the fallback");

    Storage::disk("assets")
        .expect("read-through disk")
        .rename("both.txt", "moved.txt")
        .await
        .expect("rename uses the primary");

    assert_eq!(
        &primary
            .read("moved.txt")
            .await
            .expect("destination lands on the primary")
            .to_vec(),
        b"primary copy"
    );
    assert!(
        !fallback.exists("both.txt").await.expect("fallback exists"),
        "the fallback source must go too, or the next read promotes it back"
    );
}

#[tokio::test]
async fn copy_false_still_lands_copy_and_rename_on_the_primary() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "overlay", false);

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let overlay = Storage::disk("overlay").expect("read-through disk");
    overlay
        .copy("cold.txt", "warm.txt")
        .await
        .expect("copy spans the fallback even with copy: false");

    let primary = Storage::disk("primary").expect("primary disk");
    assert_eq!(
        &primary
            .read("warm.txt")
            .await
            .expect("destination lands on the primary")
            .to_vec(),
        b"cold bytes",
        "the copy flag governs read-time promotion only"
    );

    overlay
        .rename("cold.txt", "moved.txt")
        .await
        .expect("rename spans the fallback even with copy: false");
    assert_eq!(
        &primary
            .read("moved.txt")
            .await
            .expect("the moved object lands on the primary")
            .to_vec(),
        b"cold bytes",
        "the copy flag governs read-time promotion only"
    );
    assert!(
        !Storage::disk("fallback")
            .expect("fallback disk")
            .exists("cold.txt")
            .await
            .expect("fallback exists"),
        "a move clears the fallback source whatever the copy flag says"
    );
}

#[tokio::test]
async fn copy_and_rename_of_a_source_on_neither_disk_fail() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    let assets = Storage::disk("assets").expect("read-through disk");

    let copy_err = assets
        .copy("nowhere.txt", "warm.txt")
        .await
        .expect_err("a source on neither disk cannot be copied");
    let copy_msg = copy_err.to_string();
    assert!(
        copy_msg.contains("copy") && copy_msg.contains("nowhere.txt"),
        "the copy error must name the operation and the source, got: {copy_msg}"
    );

    let rename_err = assets
        .rename("nowhere.txt", "moved.txt")
        .await
        .expect_err("a source on neither disk cannot be moved");
    let rename_msg = rename_err.to_string();
    assert!(
        rename_msg.contains("move") && rename_msg.contains("nowhere.txt"),
        "the move error must name the operation and the source, got: {rename_msg}"
    );

    assert!(
        !Storage::disk("primary")
            .expect("primary disk")
            .exists("warm.txt")
            .await
            .expect("primary exists answers"),
        "a failed copy must leave no partial destination"
    );
}

#[tokio::test]
async fn copy_streams_a_multi_chunk_object() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    // Two 64 KiB transfer chunks and a few bytes over, so the streaming loop
    // runs more than once - the only shape that walks it.
    let body: Vec<u8> = (0..(64 * 1024 * 2 + 7)).map(|i| (i % 251) as u8).collect();
    Storage::disk("fallback")
        .expect("fallback disk")
        .write("big.bin", body.clone())
        .await
        .expect("seed the fallback");

    Storage::disk("assets")
        .expect("read-through disk")
        .copy("big.bin", "warm.bin")
        .await
        .expect("copy spans the fallback");

    assert_eq!(
        Storage::disk("primary")
            .expect("primary disk")
            .read("warm.bin")
            .await
            .expect("destination lands on the primary")
            .to_vec(),
        body,
        "every chunk has to arrive, in order and unaltered"
    );
}

#[tokio::test]
async fn a_conditional_copy_of_a_fallback_only_source_refuses_to_clobber() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");
    let primary = Storage::disk("primary").expect("primary disk");
    primary
        .write("warm.txt", "the destination that was already there")
        .await
        .expect("seed the destination");

    let err = Storage::disk("assets")
        .expect("read-through disk")
        .copy_with("cold.txt", "warm.txt")
        .if_not_exists(true)
        .await
        .expect_err("if_not_exists must refuse an existing destination");
    assert_eq!(
        err.kind(),
        ErrorKind::ConditionNotMatch,
        "a refused condition must not reach the caller as a transport failure, got: {err}"
    );

    assert_eq!(
        &primary
            .read("warm.txt")
            .await
            .expect("the destination is still there")
            .to_vec(),
        b"the destination that was already there",
        "a refused conditional copy must leave the destination untouched"
    );
}

#[tokio::test]
async fn a_conditional_rename_of_a_fallback_only_source_refuses_to_clobber() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    let fallback = Storage::disk("fallback").expect("fallback disk");
    fallback
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");
    let primary = Storage::disk("primary").expect("primary disk");
    primary
        .write("warm.txt", "the destination that was already there")
        .await
        .expect("seed the destination");

    let err = Storage::disk("assets")
        .expect("read-through disk")
        .rename_with("cold.txt", "warm.txt")
        .if_not_exists(true)
        .await
        .expect_err("if_not_exists must refuse an existing destination");
    assert_eq!(
        err.kind(),
        ErrorKind::ConditionNotMatch,
        "a refused condition must not reach the caller as a transport failure, got: {err}"
    );

    assert_eq!(
        &primary
            .read("warm.txt")
            .await
            .expect("the destination is still there")
            .to_vec(),
        b"the destination that was already there",
        "a refused conditional move must leave the destination untouched"
    );
    assert!(
        fallback.exists("cold.txt").await.expect("fallback exists"),
        "a move that never landed must not have removed its source"
    );
}

#[tokio::test]
async fn a_copy_of_a_fallback_only_source_refuses_an_if_match_condition() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    register_fs_primary(tmp.path(), "assets", true);

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let err = Storage::disk("assets")
        .expect("read-through disk")
        .copy_with("cold.txt", "warm.txt")
        .if_match("\"etag-live\"")
        .await
        .expect_err("if_match is a backend copy condition a streaming transfer cannot apply");
    assert_eq!(
        err.kind(),
        ErrorKind::Unsupported,
        "refusing beats silently dropping the condition, got: {err}"
    );
    let message = err.to_string();
    assert!(
        message.contains("if_match"),
        "the error must name the condition it cannot honour, got: {message}"
    );

    assert!(
        !Storage::disk("primary")
            .expect("primary disk")
            .exists("warm.txt")
            .await
            .expect("primary exists answers"),
        "a refused copy must write nothing"
    );
}
