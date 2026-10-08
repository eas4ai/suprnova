//! PAR-046: the asset version resolves from the asset URL setting, else the
//! Vite manifest's hash, else the empty string.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use suprnova::testing::TestContainer;
use suprnova::{
    Inertia, InertiaConfig, InertiaResponse, InertiaVersionMiddleware, MiddlewareRegistry,
};

use super::support::{MockReq, no_manifest, page_of, page_router, spawn_server};
use crate::http_wire::request;

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

#[tokio::test]
async fn inp_inertia_version_sets_what_get_version_reads_and_the_409_compares() {
    // Laravel's `Inertia::version` / `Inertia::getVersion`, and the version
    // middleware compares against what they hold.
    let _guard = TestContainer::fake();
    Inertia::install(&InertiaConfig::new().version("installed").development(true))
        .expect("dev-mode install must not require a manifest");
    assert_eq!(Inertia::get_version(), "installed");

    Inertia::version("v9");
    assert_eq!(Inertia::get_version(), "v9");

    let resp = InertiaResponse::new("Home")
        .resolve(&MockReq::new("/").inertia())
        .await
        .expect("a page");
    assert_eq!(
        page_of(resp).await["version"],
        "v9",
        "the page advertises the version the middleware compares against"
    );

    let registry = MiddlewareRegistry::new().append(InertiaVersionMiddleware::with_resolver(
        Inertia::get_version,
    ));
    let addr = spawn_server(page_router(), registry, 2).await;
    let (stale, _, _) = request(
        addr,
        "GET",
        "/page",
        &[("X-Inertia", "true"), ("X-Inertia-Version", "installed")],
    )
    .await;
    assert_eq!(stale, 409, "the version set at run time is the current one");
    let (current, _, _) = request(
        addr,
        "GET",
        "/page",
        &[("X-Inertia", "true"), ("X-Inertia-Version", "v9")],
    )
    .await;
    assert_eq!(current, 200);
}

#[test]
fn inp_inertia_version_takes_a_function_or_none() {
    let _guard = TestContainer::fake();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    Inertia::version(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        "from-a-function".to_string()
    });
    assert_eq!(Inertia::get_version(), "from-a-function");
    assert_eq!(Inertia::get_version(), "from-a-function");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the function runs on every read"
    );

    // Laravel casts `null` to "".
    Inertia::version(None::<String>);
    assert_eq!(Inertia::get_version(), "");
}
