#![cfg(all(feature = "filesystem", feature = "testing"))]

//! The Laravel infrastructure gaps of the filesystem: a directory path
//! without its trailing slash, `ensure_directory_exists`, and the
//! temporary upload URL callback (PAR-153).
//!
//! Every test takes a `Storage::fake()` guard first: the registry is
//! process-global, and the guard serializes the tests that use it and
//! wipes it on drop.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::opendal::Operator;
use suprnova::{DiskExt, ReadThroughConfig, Storage, TemporaryUploadUrl};

/// A memory disk named `rt_primary`, one named `rt_fallback`, and the
/// read-through disk `rt` over them, as the falsifier describes it.
fn register_read_through_rt() {
    Storage::register_memory("rt_primary");
    Storage::register_memory("rt_fallback");
    Storage::register_read_through(
        "rt",
        ReadThroughConfig {
            primary: "rt_primary".into(),
            fallback: "rt_fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");
}

/// The upload the falsifier's callback answers: `https://up/<path>`.
fn upload_to_up(path: &str) -> TemporaryUploadUrl {
    TemporaryUploadUrl {
        url: format!("https://up/{path}"),
        method: "PUT".to_owned(),
        headers: BTreeMap::new(),
    }
}

/// An S3 disk that presigns writes by itself. Presigning only signs, so it
/// needs no network and no bucket.
fn offline_s3_disk() -> Operator {
    Operator::new(
        suprnova::opendal::services::S3::default()
            .bucket("uploads")
            .region("us-east-1")
            .endpoint("http://127.0.0.1:9")
            .access_key_id("AKIDEXAMPLE")
            .secret_access_key("example-secret"),
    )
    .expect("build an offline S3 operator")
}

#[tokio::test]
async fn make_directory_creates_a_path_without_its_trailing_slash() {
    let _guard = Storage::fake();
    Storage::register_memory("dirs");
    let disk = Storage::disk("dirs").expect("disk");

    disk.make_directory("a/b")
        .await
        .expect("a directory path without a trailing slash is created");

    assert_eq!(
        disk.directories("a", false).await.expect("list a"),
        vec!["a/b"]
    );
    assert!(disk.directory_exists("a/b/").await.expect("stat a/b/"));
}

#[tokio::test]
async fn make_directory_keeps_a_path_with_the_slash_and_the_empty_path() {
    let _guard = Storage::fake();
    Storage::register_memory("slash");
    let disk = Storage::disk("slash").expect("disk");

    disk.make_directory("with/slash/")
        .await
        .expect("a path that ends in a slash is created as before");
    assert_eq!(
        disk.directories("with", false).await.expect("list with"),
        vec!["with/slash"]
    );

    // The empty path is the root: it reaches `create_dir` unchanged, so it
    // answers what `create_dir("")` answers.
    let ours = disk.make_directory("").await;
    let opendal = disk.create_dir("").await;
    assert_eq!(ours.is_ok(), opendal.is_ok(), "{ours:?} vs {opendal:?}");
}

#[tokio::test]
async fn ensure_directory_exists_creates_once_and_succeeds_again() {
    let _guard = Storage::fake();
    Storage::register_memory("ensure");
    let disk = Storage::disk("ensure").expect("disk");

    disk.ensure_directory_exists("a/b")
        .await
        .expect("the first call creates the directory");
    disk.ensure_directory_exists("a/b")
        .await
        .expect("a second call succeeds on the existing directory");
    disk.ensure_directory_exists("a/b/")
        .await
        .expect("the same directory named with its slash succeeds too");

    assert_eq!(
        disk.directories("a", false).await.expect("list a"),
        vec!["a/b"]
    );
}

#[tokio::test]
async fn make_directory_and_ensure_directory_exists_report_a_refused_path() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    std::fs::create_dir_all(&root).expect("create the disk root");
    Storage::register_fs("guarded", &root).expect("register the local disk");
    let disk = Storage::disk("guarded").expect("disk");

    let made = disk.make_directory("../escaped").await;
    let error = made.expect_err("a path that leaves the root is refused");
    assert!(
        error.to_string().contains("create_dir"),
        "the error names the operation: {error}"
    );
    disk.ensure_directory_exists("../escaped")
        .await
        .expect_err("ensure_directory_exists refuses the same path");
    assert!(!tmp.path().join("escaped").exists());

    disk.ensure_directory_exists("inside/deeper")
        .await
        .expect("a local disk creates a bare directory path");
    assert!(root.join("inside/deeper").is_dir());
}

#[tokio::test]
async fn the_upload_url_callback_answers_a_read_through_disk() {
    let _guard = Storage::fake();
    register_read_through_rt();

    assert!(
        !Storage::provides_temporary_upload_urls("rt").expect("rt is registered"),
        "a read-through disk over memory disks presigns no writes"
    );

    let seen: Arc<Mutex<Vec<(String, Duration)>>> = Arc::default();
    let record = Arc::clone(&seen);
    Storage::build_temporary_upload_urls_using("rt", move |path, expire| {
        record
            .lock()
            .expect("record")
            .push((path.to_owned(), expire));
        Ok(upload_to_up(path))
    })
    .expect("rt is registered");

    assert!(Storage::provides_temporary_upload_urls("rt").expect("rt is registered"));
    let upload = Storage::disk("rt")
        .expect("rt")
        .temporary_upload_url("x", Duration::from_secs(60))
        .await
        .expect("the callback answers");
    assert_eq!(upload.url, "https://up/x");
    assert_eq!(upload.method, "PUT");
    assert!(upload.headers.is_empty());
    assert_eq!(
        *seen.lock().expect("seen"),
        vec![("x".to_owned(), Duration::from_secs(60))],
        "the callback receives the path and the lifetime"
    );
}

#[tokio::test]
async fn a_memory_disk_without_a_callback_provides_no_upload_urls() {
    let _guard = Storage::fake();
    Storage::register_memory("plain");

    assert!(!Storage::provides_temporary_upload_urls("plain").expect("registered"));
    let error = Storage::disk("plain")
        .expect("plain")
        .temporary_upload_url("x", Duration::from_secs(60))
        .await
        .expect_err("a memory disk does not presign");
    assert!(error.to_string().contains("presign_write"), "{error}");
}

#[tokio::test]
async fn a_disk_that_presigns_answers_without_a_callback_and_the_callback_wins() {
    let _guard = Storage::fake();
    Storage::set("s3", offline_s3_disk());

    assert!(
        Storage::provides_temporary_upload_urls("s3").expect("registered"),
        "an S3 disk presigns writes by itself"
    );
    let signed = Storage::disk("s3")
        .expect("s3")
        .temporary_upload_url("docs/a.pdf", Duration::from_secs(60))
        .await
        .expect("S3 presigns");
    assert!(signed.url.contains("X-Amz-Signature"), "{}", signed.url);

    Storage::build_temporary_upload_urls_using("s3", |path, _| Ok(upload_to_up(path)))
        .expect("s3 is registered");
    let upload = Storage::disk("s3")
        .expect("s3")
        .temporary_upload_url("docs/a.pdf", Duration::from_secs(60))
        .await
        .expect("the callback answers");
    assert_eq!(upload.url, "https://up/docs/a.pdf");
}

#[tokio::test]
async fn the_callback_keeps_the_public_url_and_a_later_callback_replaces_it() {
    let _guard = Storage::fake();
    Storage::register_memory("public");
    Storage::set_public_url("public", "https://cdn.example.com/files").expect("public URL");

    Storage::build_temporary_upload_urls_using("public", |path, _| Ok(upload_to_up(path)))
        .expect("registered");
    assert_eq!(
        Storage::url("public", "a.png").expect("the public URL stays"),
        "https://cdn.example.com/files/a.png"
    );

    Storage::build_temporary_upload_urls_using("public", |path, _| {
        Ok(TemporaryUploadUrl {
            url: format!("https://second/{path}"),
            method: "POST".to_owned(),
            headers: BTreeMap::from([("x-upload".to_owned(), "1".to_owned())]),
        })
    })
    .expect("registered");
    let disk = Storage::disk("public").expect("public");
    disk.put("kept.txt", b"kept".to_vec())
        .await
        .expect("the disk still writes");
    assert_eq!(disk.get("kept.txt").await.expect("read"), b"kept");
    let upload = disk
        .temporary_upload_url("a.png", Duration::from_secs(5))
        .await
        .expect("the second callback answers");
    assert_eq!(upload.url, "https://second/a.png");
    assert_eq!(upload.method, "POST");
    assert_eq!(
        upload.headers.get("x-upload").map(String::as_str),
        Some("1")
    );
}

#[tokio::test]
async fn a_callback_error_and_an_unregistered_disk_are_reported() {
    let _guard = Storage::fake();
    Storage::register_memory("failing");

    let missing =
        Storage::build_temporary_upload_urls_using("nope", |path, _| Ok(upload_to_up(path)))
            .expect_err("no disk is registered under the name");
    assert!(missing.to_string().contains("nope"), "{missing}");
    Storage::provides_temporary_upload_urls("nope").expect_err("no disk named nope");

    Storage::build_temporary_upload_urls_using("failing", |_, _| {
        Err(suprnova::FrameworkError::internal("uploads are closed"))
    })
    .expect("registered");
    let error = Storage::disk("failing")
        .expect("failing")
        .temporary_upload_url("x", Duration::from_secs(60))
        .await
        .expect_err("the callback's error reaches the caller");
    assert!(error.to_string().contains("uploads are closed"), "{error}");
}

#[tokio::test]
async fn a_callback_upload_keeps_the_redacting_debug() {
    let _guard = Storage::fake();
    Storage::register_memory("redact");
    Storage::build_temporary_upload_urls_using("redact", |path, _| {
        Ok(TemporaryUploadUrl {
            url: format!("https://up/{path}?signature=secret-signature"),
            method: "PUT".to_owned(),
            headers: BTreeMap::from([("x-token".to_owned(), "secret-header".to_owned())]),
        })
    })
    .expect("registered");

    let upload = Storage::disk("redact")
        .expect("redact")
        .temporary_upload_url("x", Duration::from_secs(60))
        .await
        .expect("the callback answers");
    assert_eq!(upload.url, "https://up/x?signature=secret-signature");
    let debug = format!("{upload:?}");
    assert!(
        !debug.contains("secret-signature") && !debug.contains("secret-header"),
        "Debug hides the query and the header values: {debug}"
    );
}
