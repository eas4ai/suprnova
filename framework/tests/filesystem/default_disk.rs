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
    "S3_ROOT",
    "S3_USE_PATH_STYLE_ENDPOINT",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_BUCKET",
    "AWS_URL",
    "AWS_ENDPOINT",
    "AWS_USE_PATH_STYLE_ENDPOINT",
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
    default
        .write("avatar.txt", b"pixels".to_vec())
        .await
        .unwrap();

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
        .expect_err("with no local disk registered, the implicit default is unavailable");
    assert!(error.to_string().contains("local"), "{error}");
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
        .expect_err("startup must fail when the default disk is not registered");
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

#[tokio::test]
async fn without_settings_the_default_is_the_registered_local_disk() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    Storage::register_memory("local");
    Storage::default_disk()
        .expect("local is the default")
        .write("default.txt", b"local bytes".to_vec())
        .await
        .unwrap();
    assert_eq!(
        Storage::disk("local")
            .unwrap()
            .read("default.txt")
            .await
            .unwrap()
            .to_vec(),
        b"local bytes"
    );
}

#[tokio::test]
async fn laravel_aws_variables_configure_the_registered_s3_disk() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let _guard = Storage::fake();
    clear_variables();
    for (name, value) in [
        ("FILESYSTEM_DISK", "s3"),
        ("AWS_BUCKET", "laravel-bucket"),
        ("AWS_DEFAULT_REGION", "eu-west-3"),
        ("AWS_ACCESS_KEY_ID", "laravel-key"),
        ("AWS_SECRET_ACCESS_KEY", "laravel-secret"),
        ("AWS_ENDPOINT", "https://objects.example.test"),
        ("AWS_URL", "https://cdn.example.test/files"),
        ("AWS_USE_PATH_STYLE_ENDPOINT", "true"),
    ] {
        set_env(name, Some(value));
    }
    let config = suprnova::S3Config::from_env()
        .unwrap()
        .expect("AWS_BUCKET configures s3");
    assert_eq!(config.bucket, "laravel-bucket");
    assert_eq!(config.region.as_deref(), Some("eu-west-3"));
    assert_eq!(config.access_key_id.as_deref(), Some("laravel-key"));
    assert_eq!(config.secret_access_key.as_deref(), Some("laravel-secret"));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://objects.example.test")
    );
    suprnova::filesystem::bootstrap_from_env().expect("AWS disk registers at boot");
    assert_eq!(
        Storage::url("s3", "a.txt").unwrap(),
        "https://cdn.example.test/files/a.txt"
    );
    let request = Storage::default_disk()
        .unwrap()
        .presign_read("a.txt", std::time::Duration::from_secs(60))
        .await
        .unwrap();
    let url = request.uri().to_string();
    assert!(
        url.starts_with("https://objects.example.test/laravel-bucket/a.txt?"),
        "{url}"
    );
    assert!(
        url.contains("laravel-key%2F") && url.contains("eu-west-3%2Fs3"),
        "{url}"
    );
}

#[tokio::test]
async fn s3_names_override_each_aws_alias_and_virtual_host_style_is_respected() {
    let _lock = lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(VARIABLES);
    for name in VARIABLES {
        set_env(name, None);
    }
    let _guard = Storage::fake();
    for (name, value) in [
        ("S3_BUCKET", "chosen-bucket"),
        ("AWS_BUCKET", "ignored-bucket"),
        ("S3_REGION", "us-east-1"),
        ("AWS_DEFAULT_REGION", "eu-west-3"),
        ("S3_ACCESS_KEY", "chosen-key"),
        ("AWS_ACCESS_KEY_ID", "ignored-key"),
        ("S3_SECRET_KEY", "chosen-secret"),
        ("AWS_SECRET_ACCESS_KEY", "ignored-secret"),
        ("S3_ENDPOINT", "https://chosen.example.test"),
        ("AWS_ENDPOINT", "https://ignored.example.test"),
        ("S3_PUBLIC_URL", "https://chosen.example.test/files"),
        ("AWS_URL", "https://ignored.example.test/files"),
        ("S3_USE_PATH_STYLE_ENDPOINT", "false"),
        ("AWS_USE_PATH_STYLE_ENDPOINT", "true"),
    ] {
        set_env(name, Some(value));
    }
    let config = suprnova::S3Config::from_env().unwrap().unwrap();
    assert_eq!(config.bucket, "chosen-bucket");
    assert_eq!(config.region.as_deref(), Some("us-east-1"));
    assert_eq!(config.access_key_id.as_deref(), Some("chosen-key"));
    assert_eq!(config.secret_access_key.as_deref(), Some("chosen-secret"));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://chosen.example.test")
    );
    suprnova::filesystem::bootstrap_from_env().unwrap();
    assert_eq!(
        Storage::url("s3", "a.txt").unwrap(),
        "https://chosen.example.test/files/a.txt"
    );
    let signed = Storage::disk("s3")
        .unwrap()
        .presign_read("a.txt", std::time::Duration::from_secs(60))
        .await
        .unwrap();
    assert!(
        signed
            .uri()
            .to_string()
            .starts_with("https://chosen-bucket.chosen.example.test/a.txt?")
    );
}

#[tokio::test]
async fn an_incomplete_aws_key_pair_and_invalid_path_style_fail_without_repeating_values() {
    let _lock = lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(VARIABLES);
    for name in VARIABLES {
        set_env(name, None);
    }
    set_env("AWS_BUCKET", Some("files"));
    set_env("AWS_DEFAULT_REGION", Some("us-east-1"));
    set_env("AWS_ACCESS_KEY_ID", Some("private-key-value"));
    let error = suprnova::S3Config::from_env().unwrap_err().to_string();
    assert!(error.contains("AWS_SECRET_ACCESS_KEY"));
    assert!(!error.contains("private-key-value"));
    set_env("AWS_SECRET_ACCESS_KEY", Some("private-secret-value"));
    set_env("AWS_USE_PATH_STYLE_ENDPOINT", Some("invalid-private-value"));
    let error = suprnova::filesystem::bootstrap_from_env()
        .unwrap_err()
        .to_string();
    assert!(error.contains("AWS_USE_PATH_STYLE_ENDPOINT"));
    assert!(!error.contains("invalid-private-value"));
    assert!(Storage::disk("s3").is_err());
}
