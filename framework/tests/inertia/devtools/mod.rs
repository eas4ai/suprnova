//! Inertia DevTools server support (PAR-071 to PAR-075): recording each
//! request for the browser extension, the entry it stores, the
//! classification of the props of a rendered page, the endpoints the
//! extension reads entries from, and the store behind them.
//!
//! Every test records into its own temporary directory, through a
//! `TestClient` over a router with `Inertia::middleware` and, where the
//! session matters, a real `SessionMiddleware`. Recording is switched on
//! with `DevToolsConfig::enabled(true)`; the tests of the `local` default
//! and of the endpoints' authorization set `APP_ENV` under the shared
//! environment lock.

pub mod endpoints;
pub mod entry;
pub mod gate;
pub mod props;
pub mod storage;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use suprnova::testing::TestClient;
use suprnova::{
    DevToolsConfig, Inertia, InertiaConfig, MiddlewareRegistry, Router, SessionMiddleware,
};

use crate::env_snapshot::{EnvSnapshot, set_env};
use crate::protocol_harness::MemoryStore;

/// DevTools on, storing into `dir`.
pub fn devtools(dir: &Path) -> DevToolsConfig {
    DevToolsConfig::new().enabled(true).storage_path(dir)
}

/// The Inertia configuration of the tests, with `devtools`.
pub fn inertia(devtools: DevToolsConfig) -> InertiaConfig {
    // The empty version, the one `TestRequest::inertia` sends when no
    // configuration is installed, so no visit is bounced as stale.
    InertiaConfig::new()
        .development(true)
        .version("")
        .devtools(devtools)
}

/// A client for `router` behind the Inertia stack built for `devtools`.
pub fn client(router: impl Into<Router>, devtools: DevToolsConfig) -> TestClient {
    TestClient::new(
        router,
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools))),
    )
}

/// A client for `router` behind a real session over a memory store, then
/// the Inertia stack built for `devtools`.
pub fn session_client(
    router: impl Into<Router>,
    devtools: DevToolsConfig,
) -> (TestClient, Arc<MemoryStore>) {
    suprnova::testing::install_test_encryption_key();
    let store = Arc::new(MemoryStore::default());
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(
            crate::protocol_harness::session_config(),
            store.clone(),
        ))
        .append(Inertia::middleware(&inertia(devtools)));
    let client =
        TestClient::new(router, registry).with_session_store(store.clone(), "suprnova_session");
    (client, store)
}

/// The entry files stored in `dir`, by id.
pub fn entry_ids(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = read
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json") && name != "_meta.json")
        .map(|name| name.trim_end_matches(".json").to_string())
        .collect();
    ids.sort();
    ids
}

/// The stored entry `id` in `dir`, parsed.
pub fn read_entry(dir: &Path, id: &str) -> Value {
    let path: PathBuf = dir.join(format!("{id}.json"));
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("no entry file {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("entry {id} is not JSON: {e}"))
}

/// The stored entry named by a response's `X-Inertia-Devtools-Id`.
pub fn entry_of(dir: &Path, response: &suprnova::testing::TestResponse) -> Value {
    let id = response.header("x-inertia-devtools-id").unwrap_or_else(|| {
        panic!(
            "no X-Inertia-Devtools-Id on a {} response",
            response.status()
        )
    });
    read_entry(dir, id)
}

/// The index `_meta.json` in `dir`, parsed.
pub fn read_index(dir: &Path) -> Value {
    let bytes = std::fs::read(dir.join("_meta.json")).expect("an index file");
    serde_json::from_slice(&bytes).expect("an index that is JSON")
}

/// `APP_ENV` set to `env` while the guard lives, under the shared
/// environment lock.
pub struct AppEnv {
    _env: EnvSnapshot,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

/// Run as the `env` environment until the returned guard drops.
pub async fn app_env(env: &str) -> AppEnv {
    let lock = crate::env_lock::lock_env_async().await;
    let snapshot = EnvSnapshot::capture(&["APP_ENV", "APP_URL", "INERTIA_DEVTOOLS_ENABLED"]);
    set_env("APP_ENV", Some(env));
    set_env("INERTIA_DEVTOOLS_ENABLED", None);
    AppEnv {
        _env: snapshot,
        _lock: lock,
    }
}

/// What a raw request answered: status, the first value of each
/// lower-cased header, and the body.
pub struct RawReply {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
}

/// Send one request with header values as raw bytes and a raw body to a
/// router served on `addr`: for what `TestClient` cannot send, a header
/// value that is not text and a multipart body.
pub async fn raw_send(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &[u8])],
    body: Vec<u8>,
) -> RawReply {
    use http_body_util::{BodyExt, Full};
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<bytes::Bytes>>(
        hyper_util::rt::TokioIo::new(stream),
    )
    .await
    .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", body.len().to_string());
    for (name, value) in headers {
        builder = builder.header(
            *name,
            hyper::header::HeaderValue::from_bytes(value).unwrap(),
        );
    }
    let response = sender
        .send_request(builder.body(Full::new(bytes::Bytes::from(body))).unwrap())
        .await
        .expect("send_request");
    let (parts, body) = response.into_parts();
    let headers = parts
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let bytes = body.collect().await.unwrap().to_bytes();
    RawReply {
        status: parts.status.as_u16(),
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}
