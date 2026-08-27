//! Browser-facing reference-host transport and production-artifact contracts.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use suprnova_live_test_support::{
    REFERENCE_AUTHORIZATION, ReferenceFaultSchedule, ReferenceHost, ReferenceHostConfig,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

const ASSET_PORT: u16 = 4_181;
const UPLOAD_PORT: u16 = 4_182;
const ASYNC_PORT: u16 = 4_174;
const SHUTDOWN_PORT: u16 = 4_184;
const INVALID_MANIFEST_PORT: u16 = 4_186;

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "suprnova-live-reference-host-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test quarantine root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn browser_dist() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../browser/dist")
}

async fn start_host(port: u16, root: &TestRoot, fault: ReferenceFaultSchedule) -> ReferenceHost {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    ReferenceHost::start(
        ReferenceHostConfig::new(address, browser_dist(), root.path().to_path_buf())
            .with_fault_schedule(fault),
    )
    .await
    .expect("start reference host")
}

type TestClient = Client<HttpConnector, Full<Bytes>>;

fn client() -> TestClient {
    Client::builder(TokioExecutor::new()).build(HttpConnector::new())
}

async fn request(
    host: &ReferenceHost,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: impl Into<Bytes>,
) -> (StatusCode, hyper::HeaderMap, Bytes) {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("{}{}", host.origin(), path));
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = client()
        .request(builder.body(Full::new(body.into())).expect("request"))
        .await
        .expect("reference-host response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    (status, headers, bytes)
}

async fn json_request(
    host: &ReferenceHost,
    method: Method,
    path: &str,
    body: Value,
) -> (StatusCode, hyper::HeaderMap, Value) {
    let bytes = serde_json::to_vec(&body).expect("JSON request");
    let (status, headers, body) = request(
        host,
        method,
        path,
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION),
        ],
        bytes,
    )
    .await;
    let value = serde_json::from_slice(&body).expect("JSON response");
    (status, headers, value)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serves_only_validated_manifest_owned_production_assets() {
    let root = TestRoot::new("assets");
    let host = start_host(ASSET_PORT, &root, ReferenceFaultSchedule::None).await;
    let (status, headers, bytes) = request(
        &host,
        Method::GET,
        "/suprnova-live.assets.json",
        &[],
        Bytes::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        headers
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );

    let manifest: Value = serde_json::from_slice(&bytes).expect("asset manifest");
    let assets = manifest["assets"].as_array().expect("asset list");
    let roles = assets
        .iter()
        .map(|asset| asset["role"].as_str().expect("asset role"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        roles,
        BTreeSet::from([
            "async-classic",
            "async-esm",
            "core-classic",
            "core-esm",
            "stimulus-classic",
            "stimulus-esm",
            "uploads-classic",
            "uploads-esm",
        ])
    );

    for asset in assets {
        let file = asset["file"].as_str().expect("asset file");
        let (status, headers, bytes) =
            request(&host, Method::GET, &format!("/{file}"), &[], Bytes::new()).await;
        assert_eq!(status, StatusCode::OK, "{file}");
        assert_eq!(
            bytes.len() as u64,
            asset["bytes"].as_u64().unwrap(),
            "{file}"
        );
        let digest = Sha256::digest(&bytes);
        let sha256 = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(sha256, asset["sha256"].as_str().unwrap());
        assert_eq!(
            format!("sha256-{}", BASE64.encode(digest)),
            asset["sri"].as_str().unwrap()
        );
        assert_eq!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            asset["content_type"].as_str(),
            "{file}"
        );
        assert_eq!(
            headers
                .get(CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            asset["cache_control"].as_str(),
            "{file}"
        );
    }

    for source_path in [
        "/src/runtime/runtime.ts",
        "/src/uploads/feature.ts",
        "/package.json",
        "/../Cargo.toml",
    ] {
        let (status, _, _) = request(&host, Method::GET, source_path, &[], Bytes::new()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{source_path}");
    }

    host.shutdown().await.expect("clean asset-host shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_production_manifest_fails_before_the_port_is_bound() {
    let artifact_root = TestRoot::new("invalid-artifacts");
    let quarantine_root = TestRoot::new("invalid-quarantine");
    let source = browser_dist();
    let manifest_bytes =
        std::fs::read(source.join("suprnova-live.assets.json")).expect("read source manifest");
    let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("source manifest");
    std::fs::write(
        artifact_root.path().join("suprnova-live.assets.json"),
        manifest_bytes,
    )
    .expect("copy manifest");
    for asset in manifest["assets"].as_array().expect("manifest assets") {
        let file = asset["file"].as_str().expect("asset file");
        std::fs::copy(source.join(file), artifact_root.path().join(file)).expect("copy asset");
    }
    let first = manifest["assets"][0]["file"].as_str().expect("first asset");
    std::fs::write(
        artifact_root.path().join(first),
        b"tampered production artifact",
    )
    .expect("tamper isolated asset");

    let address = SocketAddr::from(([127, 0, 0, 1], INVALID_MANIFEST_PORT));
    let result = ReferenceHost::start(ReferenceHostConfig::new(
        address,
        artifact_root.path().to_path_buf(),
        quarantine_root.path().to_path_buf(),
    ))
    .await;
    let error = match result {
        Ok(host) => {
            host.shutdown().await.expect("unexpected host shutdown");
            panic!("tampered manifest unexpectedly started")
        }
        Err(error) => error,
    };
    assert!(error.contains("integrity mismatch"), "{error}");
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("invalid manifest never bound the configured port");
    drop(listener);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_routes_use_real_chunked_bodies_and_constrained_direct_instructions() {
    let root = TestRoot::new("uploads");
    let host = start_host(UPLOAD_PORT, &root, ReferenceFaultSchedule::None).await;

    let (status, _, created) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "avatar",
            "filename": "avatar.txt",
            "content_type": "text/plain",
            "expected_bytes": 11,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("upload handle");
    let grant = created["grant"].as_str().expect("upload grant");

    let response = chunked_upload(&host, handle, grant, &[b"hello ", b"world"]).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    let (status, _, upload) = json_request(
        &host,
        Method::GET,
        &format!("/__live/uploads/{handle}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(upload["received_bytes"], 11);
    assert_eq!(upload["state"], "transferring");

    let (status, _, completed) = json_request(
        &host,
        Method::POST,
        &format!("/__live/uploads/{handle}/complete"),
        json!({"grant": grant}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "ready");

    let (status, _, reacquired) = json_request(
        &host,
        Method::POST,
        &format!("/example/uploads/{handle}/reacquire"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reacquired}");
    assert_ne!(reacquired["grant"], grant);

    let (status, _, direct) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "evidence",
            "filename": "evidence.bin",
            "content_type": "application/octet-stream",
            "expected_bytes": 8,
            "mode": "direct"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{direct}");
    assert_eq!(direct["instruction"]["method"], "PUT");
    assert_eq!(direct["instruction"]["maximum_bytes"], 8);
    assert!(
        direct["instruction"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://uploads.example.test/temporary/")
    );
    assert!(direct["instruction"].get("credentials").is_none());

    let (status, _, rejected) = json_request(
        &host,
        Method::POST,
        "/__live/uploads?fault=../../arbitrary",
        json!({
            "field": "forbidden",
            "filename": "forbidden.bin",
            "content_type": "application/octet-stream",
            "expected_bytes": 1,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "query_selector_rejected");

    let inspection = host.inspection();
    assert!(inspection.upload_service_calls >= 5);
    assert_eq!(inspection.rejected_arbitrary_fault_selectors, 1);
    host.shutdown().await.expect("clean upload-host shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_routes_authorize_poll_sse_and_one_bounded_websocket() {
    let root = TestRoot::new("async");
    let host = start_host(ASYNC_PORT, &root, ReferenceFaultSchedule::SequenceGapOnce).await;

    let (status, _, transport) = json_request(
        &host,
        Method::POST,
        "/__live/async/transports",
        json!({"kind": "sse", "subscription": "orders"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{transport}");
    let transport_id = transport["transport"].as_str().expect("transport");
    let subscription = transport["subscription"].as_str().expect("subscription");
    let authority = transport["authority"].as_str().expect("authority");

    let membership_path =
        format!("/__live/async/transports/{transport_id}/subscriptions/{subscription}");
    let (status, _, membership) = json_request(
        &host,
        Method::POST,
        &membership_path,
        json!({"authority": authority}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{membership}");

    let (status, headers, poll) = json_request(
        &host,
        Method::POST,
        "/__live/async/poll",
        json!({"subscription": subscription, "authority": authority}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{poll}");
    assert_eq!(
        headers
            .get("x-live-operation")
            .and_then(|value| value.to_str().ok()),
        Some("fresh-render")
    );
    assert!(
        poll["render"]
            .as_str()
            .unwrap()
            .contains("data-live-poll-generation")
    );
    let (status, _, _) = json_request(
        &host,
        Method::POST,
        "/__live/async/poll",
        json!({
            "subscription": subscription,
            "authority": authority,
            "action": "forbidden"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let sse = read_sse_event(&host, transport_id, authority).await;
    assert!(sse.starts_with("HTTP/1.1 200"), "{sse}");
    assert!(sse.contains("content-type: text/event-stream"), "{sse}");
    assert!(sse.contains("event:suprnova-live-async"), "{sse}");
    assert!(sse.contains("\"sequence\":\"3\""), "{sse}");

    let rejected = websocket_upgrade(&host, "https://cross-site.example", transport_id).await;
    assert!(rejected.starts_with("HTTP/1.1 403"), "{rejected}");
    let websocket = websocket_subscribe(&host, transport_id, subscription).await;
    assert!(websocket.contains("101 Switching Protocols"), "{websocket}");
    assert!(websocket.contains("\"kind\":\"subscribed\""), "{websocket}");

    let inspection = host.inspection();
    assert_eq!(inspection.physical_sse_connections, 1);
    assert_eq!(inspection.physical_websocket_connections, 1);
    assert_eq!(inspection.maximum_logical_memberships, 1);
    assert_eq!(inspection.compiled_faults_applied, 1);
    host.shutdown().await.expect("clean async-host shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_closes_owned_sockets_files_and_timers() {
    let root = TestRoot::new("shutdown");
    let host = start_host(SHUTDOWN_PORT, &root, ReferenceFaultSchedule::None).await;
    let inspection = host.inspection_handle();
    timeout(Duration::from_secs(2), host.shutdown())
        .await
        .expect("shutdown deadline")
        .expect("shutdown result");
    let final_state = inspection.snapshot();
    assert_eq!(final_state.open_sockets, 0);
    assert_eq!(final_state.open_files, 0);
    assert_eq!(final_state.open_timers, 0);
    assert_eq!(final_state.active_uploads, 0);
    assert_eq!(final_state.logical_memberships, 0);
}

async fn chunked_upload(
    host: &ReferenceHost,
    handle: &str,
    grant: &str,
    chunks: &[&[u8]],
) -> String {
    let mut stream = TcpStream::connect(host.address())
        .await
        .expect("connect upload");
    let bytes = chunks.concat();
    let checksum = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let request = format!(
        "POST /__live/uploads/{handle}/chunks/0 HTTP/1.1\r\nHost: {}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nX-Live-Upload-Grant: {grant}\r\nX-Live-Chunk-Sha256: {checksum}\r\nTransfer-Encoding: chunked\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        host.address()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    for chunk in chunks {
        stream
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .await
            .unwrap();
        stream.write_all(chunk).await.unwrap();
        stream.write_all(b"\r\n").await.unwrap();
    }
    stream.write_all(b"0\r\n\r\n").await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8(response).expect("HTTP response")
}

async fn read_sse_event(host: &ReferenceHost, transport: &str, authority: &str) -> String {
    let mut stream = TcpStream::connect(host.address())
        .await
        .expect("connect SSE");
    let request = format!(
        "GET /__live/async/sse/{transport} HTTP/1.1\r\nHost: {}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nX-Live-Subscription-Authority: {authority}\r\nAccept: text/event-stream\r\n\r\n",
        host.address()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = vec![0_u8; 16 * 1024];
    let size = timeout(Duration::from_secs(1), stream.read(&mut response))
        .await
        .expect("SSE response deadline")
        .expect("SSE response");
    String::from_utf8(response[..size].to_vec()).expect("SSE UTF-8")
}

async fn websocket_upgrade(host: &ReferenceHost, origin: &str, transport: &str) -> String {
    let mut stream = TcpStream::connect(host.address())
        .await
        .expect("connect WebSocket");
    let request = format!(
        "GET /__live/async/ws HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nOrigin: {origin}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nX-Live-Transport: {transport}\r\n\r\n",
        host.address()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = vec![0_u8; 4_096];
    let size = timeout(Duration::from_secs(1), stream.read(&mut response))
        .await
        .expect("upgrade deadline")
        .expect("upgrade response");
    String::from_utf8_lossy(&response[..size]).into_owned()
}

async fn websocket_subscribe(host: &ReferenceHost, transport: &str, subscription: &str) -> String {
    let mut stream = TcpStream::connect(host.address())
        .await
        .expect("connect WebSocket");
    let origin = host.origin();
    let request = format!(
        "GET /__live/async/ws HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nOrigin: {origin}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nX-Live-Transport: {transport}\r\n\r\n",
        host.address()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut headers = vec![0_u8; 4_096];
    let size = timeout(Duration::from_secs(1), stream.read(&mut headers))
        .await
        .expect("upgrade deadline")
        .expect("upgrade response");
    let headers = String::from_utf8_lossy(&headers[..size]).into_owned();
    assert!(headers.contains("101 Switching Protocols"), "{headers}");

    let payload = format!(r#"{{"kind":"subscribe","subscription":"{subscription}"}}"#);
    let mask = [0x11, 0x22, 0x33, 0x44];
    let mut frame = vec![0x81, 0x80 | u8::try_from(payload.len()).unwrap()];
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .bytes()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()]),
    );
    stream.write_all(&frame).await.unwrap();
    let mut response = vec![0_u8; 4_096];
    let size = timeout(Duration::from_secs(1), stream.read(&mut response))
        .await
        .expect("WebSocket response deadline")
        .expect("WebSocket response");
    let frame = &response[..size];
    let payload_offset = 2;
    let payload_len = usize::from(frame[1] & 0x7f);
    format!(
        "{headers}{}",
        String::from_utf8_lossy(&frame[payload_offset..payload_offset + payload_len])
    )
}
