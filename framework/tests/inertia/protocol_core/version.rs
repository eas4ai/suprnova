//! PAR-046: the asset version resolves from the asset URL setting, else the
//! Vite manifest's hash, else the empty string.

use sha2::{Digest, Sha256};
use suprnova::{InertiaConfig, InertiaResponse};

use super::support::{MockReq, no_manifest, page_of};

/// The version a source of `bytes` hashes to: the first 16 bytes of its
/// SHA-256, hex-encoded.
fn hashed(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes)[..16])
}

/// Write a throwaway manifest and return its path.
fn write_manifest(body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "inp-version-manifest-{}.json",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, body).expect("write the manifest");
    path
}

async fn version_of(config: InertiaConfig) -> serde_json::Value {
    let resp = InertiaResponse::new("Home")
        .with_config(config)
        .resolve(&MockReq::new("/").inertia())
        .await
        .expect("a page");
    page_of(resp).await["version"].clone()
}

#[tokio::test]
async fn inp_the_version_is_empty_without_an_asset_url_or_a_manifest() {
    // Laravel's `Middleware::version` returns null with nothing to hash,
    // and the page carries "". Suprnova reported "1.0".
    assert_eq!(version_of(no_manifest()).await, "");
}

#[tokio::test]
async fn inp_the_version_prefers_the_asset_url_then_the_manifest_hash() {
    let body = r#"{"src/main.ts":{"file":"main-AAA.js","isEntry":true}}"#;
    let manifest = write_manifest(body);
    let config = InertiaConfig::new().manifest_path(&manifest);

    let from_manifest = version_of(config.clone()).await;
    let from_asset_url =
        version_of(config.clone().asset_url("https://cdn.example.com/build-7")).await;
    // An explicit version is a deliberate statement and wins over both.
    let explicit = version_of(
        config
            .asset_url("https://cdn.example.com/build-7")
            .version("pinned"),
    )
    .await;
    std::fs::remove_file(&manifest).ok();

    assert_eq!(from_manifest, hashed(body.as_bytes()));
    assert_eq!(
        from_asset_url,
        hashed(b"https://cdn.example.com/build-7"),
        "a configured asset URL comes first, as in Laravel's Middleware::version"
    );
    assert_eq!(explicit, "pinned");
}
