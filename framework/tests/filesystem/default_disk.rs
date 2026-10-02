#![cfg(all(feature = "filesystem", feature = "testing"))]

//! The default disk (PAR-016) and its startup check (PAR-017).
//!
//! Each test holds the shared env lock, because `FILESYSTEM_DISK` and the
//! S3 variables are process environment, and a `Storage::fake()` guard,
//! which isolates the disk registry and the programmatic default disk.

use suprnova::Storage;

use crate::env_lock::lock_env_async;
use crate::env_snapshot::{EnvSnapshot, set_env};

/// Every variable these tests set, restored when the snapshot drops.
const VARIABLES: &[&str] = &[
    "FILESYSTEM_DISK",
    "S3_ENDPOINT",
    "S3_ACCESS_KEY",
    "S3_SECRET_KEY",
    "S3_BUCKET",
    "S3_REGION",
    "S3_PUBLIC_URL",
];

fn clear_variables() {
    for name in VARIABLES {
        set_env(name, None);
    }
}

#[tokio::test]
async fn bytes_written_through_the_default_disk_are_on_the_disk_filesystem_disk_names() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    set_env("FILESYSTEM_DISK", Some("uploads"));
    Storage::register_memory("uploads");

    let default = Storage::default_disk().expect("FILESYSTEM_DISK names a registered disk");
    default.write("avatar.txt", b"pixels".to_vec()).await.unwrap();

    let named = Storage::disk("uploads").unwrap();
    assert_eq!(
        named.read("avatar.txt").await.unwrap().to_vec(),
        b"pixels",
        "the default disk must be the disk FILESYSTEM_DISK names"
    );
}

#[tokio::test]
async fn a_default_set_in_code_wins_over_filesystem_disk() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    set_env("FILESYSTEM_DISK", Some("uploads"));
    Storage::register_memory("uploads");
    Storage::register_memory("archive");
    Storage::set_default_disk("archive");

    Storage::default_disk()
        .unwrap()
        .write("report.txt", b"q3".to_vec())
        .await
        .unwrap();

    assert!(
        Storage::disk("archive")
            .unwrap()
            .exists("report.txt")
            .await
            .unwrap(),
        "Storage::set_default_disk must win over FILESYSTEM_DISK"
    );
    assert!(
        !Storage::disk("uploads")
            .unwrap()
            .exists("report.txt")
            .await
            .unwrap(),
        "the write went to the disk FILESYSTEM_DISK names, not the one set in code"
    );
}

#[tokio::test]
async fn with_no_default_named_default_disk_is_an_error_naming_filesystem_disk() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    Storage::register_memory("uploads");

    let error = Storage::default_disk()
        .err()
        .expect("with neither a default set in code nor FILESYSTEM_DISK, there is no default disk");
    assert!(
        error.to_string().contains("FILESYSTEM_DISK"),
        "the error must tell the developer which variable names the default disk: {error}"
    );
}

#[tokio::test]
async fn a_named_disk_answers_the_same_with_a_default_set() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    Storage::register_memory("uploads");
    Storage::register_memory("archive");
    Storage::disk("archive")
        .unwrap()
        .write("kept.txt", b"old".to_vec())
        .await
        .unwrap();

    set_env("FILESYSTEM_DISK", Some("uploads"));
    Storage::set_default_disk("uploads");

    let archive = Storage::disk("archive").expect("a named disk is still found by its name");
    assert_eq!(archive.read("kept.txt").await.unwrap().to_vec(), b"old");
    assert!(
        Storage::disk("missing").is_err(),
        "an unregistered name is still an error with a default set"
    );
}

#[tokio::test]
async fn startup_fails_when_filesystem_disk_names_no_registered_disk() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    set_env("FILESYSTEM_DISK", Some("uploads"));

    let error = suprnova::filesystem::bootstrap_from_env()
        .err()
        .expect("startup must fail when the default disk is not registered");
    let text = error.to_string();
    assert!(
        text.contains("uploads"),
        "the error must name the missing disk: {text}"
    );
    assert!(
        text.contains("FILESYSTEM_DISK"),
        "the error must name FILESYSTEM_DISK: {text}"
    );
}

#[tokio::test]
async fn startup_fails_when_the_default_set_in_code_names_no_registered_disk() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    Storage::set_default_disk("archive");

    assert!(
        suprnova::filesystem::bootstrap_from_env().is_err(),
        "a default set in code must be checked at startup too"
    );
}

#[tokio::test]
async fn startup_passes_when_the_default_disk_is_registered() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    set_env("FILESYSTEM_DISK", Some("uploads"));
    Storage::register_memory("uploads");

    suprnova::filesystem::bootstrap_from_env()
        .expect("a registered default disk passes the startup check");
}

#[tokio::test]
async fn startup_passes_when_the_default_is_the_s3_disk_the_environment_registers() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    set_env("FILESYSTEM_DISK", Some("s3"));
    set_env("S3_ENDPOINT", Some("http://localhost:9000"));
    set_env("S3_ACCESS_KEY", Some("minioadmin"));
    set_env("S3_SECRET_KEY", Some("minioadmin"));
    set_env("S3_BUCKET", Some("local"));
    set_env("S3_REGION", Some("us-east-1"));

    suprnova::filesystem::bootstrap_from_env()
        .expect("the s3 disk the environment registers is the default disk FILESYSTEM_DISK names");
    assert!(Storage::disk("s3").is_ok());
}
