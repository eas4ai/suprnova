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
pub mod route_and_failure;
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
    with_app_env(Some(env)).await
}

/// Run with `APP_ENV` unset until the returned guard drops.
pub async fn no_app_env() -> AppEnv {
    with_app_env(None).await
}

/// Run with `APP_ENV` set to `env`, or unset, and
/// `INERTIA_DEVTOOLS_ENABLED` unset, until the returned guard drops.
async fn with_app_env(env: Option<&str>) -> AppEnv {
    let lock = crate::env_lock::lock_env_async().await;
    let snapshot = EnvSnapshot::capture(&["APP_ENV", "APP_URL", "INERTIA_DEVTOOLS_ENABLED"]);
    set_env("APP_ENV", env);
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

/// Send `head`, a request's line and headers ending in a blank line, on a
/// fresh connection to `addr`, then `body`, then close the write side when
/// `close_write` says so: for what no client sends, a body that ends
/// before its declared length or chunks that are not chunks. The request
/// should say `Connection: close`; the reply is read until the server
/// closes the connection.
pub async fn raw_request(
    addr: std::net::SocketAddr,
    head: &str,
    body: &[u8],
    close_write: bool,
) -> RawReply {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    if close_write {
        stream.shutdown().await.unwrap();
    }
    let mut received = Vec::new();
    stream.read_to_end(&mut received).await.unwrap();
    final_reply(&received).0
}

/// Send `head`, which must carry `Expect: 100-continue`, and send `body`
/// only once the server asks for it with `100 Continue`. Answers the final
/// reply and whether the server asked: a server that never polls the body
/// never asks, so a `false` proves no byte of it was read.
pub async fn raw_request_on_continue(
    addr: std::net::SocketAddr,
    head: &str,
    body: &[u8],
) -> (RawReply, bool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut received = Vec::new();
    let mut chunk = [0u8; 4096];
    // Read until the first response head is complete.
    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "the connection closed before any response");
        received.extend_from_slice(&chunk[..read]);
    }
    let continued = received.starts_with(b"HTTP/1.1 100");
    if continued {
        stream.write_all(body).await.unwrap();
    }
    stream.read_to_end(&mut received).await.unwrap();
    let (reply, interim) = final_reply(&received);
    assert_eq!(interim > 0, continued);
    (reply, continued)
}

/// The final response in `received`, after any `1xx` ones, and how many
/// `1xx` responses came before it.
fn final_reply(received: &[u8]) -> (RawReply, usize) {
    let mut rest = received;
    let mut interim = 0;
    loop {
        let end = rest
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap_or_else(|| {
                panic!(
                    "no complete response in {:?}",
                    String::from_utf8_lossy(received)
                )
            });
        let head = String::from_utf8_lossy(&rest[..end]).into_owned();
        let body = &rest[end + 4..];
        let mut lines = head.split("\r\n");
        let status: u16 = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("no status line in {head:?}"));
        if (100..200).contains(&status) {
            interim += 1;
            rest = body;
            continue;
        }
        let mut headers = std::collections::HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers
                    .entry(name.trim().to_ascii_lowercase())
                    .or_insert_with(|| value.trim().to_string());
            }
        }
        let body = if headers
            .get("transfer-encoding")
            .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
        {
            dechunk(body)
        } else {
            body.to_vec()
        };
        let reply = RawReply {
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        };
        return (reply, interim);
    }
}

/// The data of a chunked body.
fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .expect("a chunk size line");
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&body[..line_end])
                .split(';')
                .next()
                .unwrap_or_default()
                .trim(),
            16,
        )
        .expect("a hexadecimal chunk size");
        body = &body[line_end + 2..];
        if size == 0 {
            return data;
        }
        data.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}
