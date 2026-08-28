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
use suprnova_live::limits::InputLimits;
use suprnova_live::protocol::{
    ProtocolLimitConfig, ProtocolLimits, VersionedUpdateResponse, parse_versioned_update_response,
};
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
const UPLOAD_CURSOR_PORT: u16 = 4_187;
const UPLOAD_FAULT_PORT: u16 = 4_188;
const FORGED_MANIFEST_PORT: u16 = 4_189;
const POLL_PORT: u16 = 4_191;
const UPLOAD_AUTHORITY_PORT: u16 = 4_192;
const UPLOAD_ABORT_PORT: u16 = 4_193;
const TRANSPORT_BOUND_PORT: u16 = 4_194;
const WEBSOCKET_STREAM_PORT: u16 = 4_195;
const FRESH_CANCEL_PORT: u16 = 4_196;
const WRONG_ISLAND_FRESH_RENDER_REQUEST: &str = r#"{"base_revision":"7","child_parameters":null,"component":"catalog.search","correlation_id":"EBESExQVFhcYGRobHB0eHw","extensions":{"x_suprnova_live_document_key_v1":"primary"},"idempotency_key":"MDEyMzQ1Njc4OTo7PD0-Pw","model_proposals":{},"operations":[{"kind":"fresh_render"}],"protocol_version":2,"runtime_contract_version":2,"snapshot":{"envelope":{"body":{},"signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"},"kind":"instance"},"snapshot_schema_version":1}"#;

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

async fn upload_request(
    host: &ReferenceHost,
    method: Method,
    path: &str,
    grant: Option<&str>,
    body: impl Into<Bytes>,
) -> (StatusCode, hyper::HeaderMap, Bytes) {
    let mut headers = vec![(AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION)];
    if let Some(grant) = grant {
        headers.push(("x-live-upload-grant", grant));
    }
    request(host, method, path, &headers, body).await
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
async fn forged_manifest_relationships_fail_before_the_port_is_bound() {
    for (label, mutation) in [
        ("role-filename-swap", 0_u8),
        ("capability-forgery", 1),
        ("format-forgery", 2),
        ("protocol-forgery", 3),
    ] {
        let artifact_root = TestRoot::new(label);
        let quarantine_root = TestRoot::new(&format!("{label}-quarantine"));
        let source = browser_dist();
        let manifest_bytes =
            std::fs::read(source.join("suprnova-live.assets.json")).expect("read source manifest");
        let mut manifest: Value = serde_json::from_slice(&manifest_bytes).expect("source manifest");
        for asset in manifest["assets"].as_array().expect("manifest assets") {
            let file = asset["file"].as_str().expect("asset file");
            std::fs::copy(source.join(file), artifact_root.path().join(file)).expect("copy asset");
        }
        match mutation {
            0 => {
                let assets = manifest["assets"].as_array_mut().expect("manifest assets");
                let first = assets[0]["role"].clone();
                assets[0]["role"] = assets[2]["role"].clone();
                assets[2]["role"] = first;
            }
            1 => manifest["assets"][0]["capability"] = json!("forged@99"),
            2 => {
                manifest["assets"][0]["script_kind"] = json!("module");
                manifest["assets"][0]["preload_rel"] = json!("modulepreload");
            }
            3 => manifest["protocol_versions"] = json!([99]),
            _ => unreachable!("closed manifest mutation"),
        }
        std::fs::write(
            artifact_root.path().join("suprnova-live.assets.json"),
            serde_json::to_vec_pretty(&manifest).expect("forged manifest bytes"),
        )
        .expect("write forged manifest");

        let address = SocketAddr::from(([127, 0, 0, 1], FORGED_MANIFEST_PORT));
        let result = ReferenceHost::start(ReferenceHostConfig::new(
            address,
            artifact_root.path().to_path_buf(),
            quarantine_root.path().to_path_buf(),
        ))
        .await;
        match result {
            Ok(host) => {
                host.shutdown().await.expect("unexpected host shutdown");
                panic!("{label} unexpectedly started")
            }
            Err(error) => assert!(error.contains("manifest"), "{label}: {error}"),
        }
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .expect("forged manifest never bound the configured port");
        drop(listener);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_reacquire_returns_the_authoritative_partial_multipart_cursor() {
    let root = TestRoot::new("upload-cursor");
    let host = start_host(UPLOAD_CURSOR_PORT, &root, ReferenceFaultSchedule::None).await;
    let (status, _, created) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "avatar",
            "filename": "multipart.txt",
            "content_type": "text/plain",
            "expected_bytes": 10,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("upload handle");
    let grant = created["grant"].as_str().expect("upload grant");
    let response = chunked_upload(&host, handle, grant, 0, &[b"abc", b"de"]).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    let (status, _, reacquired) = upload_request(
        &host,
        Method::POST,
        &format!("/example/uploads/{handle}/reacquire"),
        None,
        "",
    )
    .await;
    let reacquired: Value = serde_json::from_slice(&reacquired).expect("reacquire JSON");
    assert_eq!(status, StatusCode::OK, "{reacquired}");
    assert_eq!(reacquired["received_bytes"], 5);
    assert_eq!(reacquired["next_part"], 1);
    assert_eq!(reacquired["revision"], 4);
    assert_ne!(reacquired["grant"], grant);

    host.shutdown().await.expect("clean upload cursor shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_handles_are_not_authority_and_reacquire_uses_application_session_ownership() {
    let root = TestRoot::new("upload-authority");
    let host = start_host(UPLOAD_AUTHORITY_PORT, &root, ReferenceFaultSchedule::None).await;
    let create = |filename: &'static str| {
        json_request(
            &host,
            Method::POST,
            "/__live/uploads",
            json!({
                "field": "avatar",
                "filename": filename,
                "content_type": "application/octet-stream",
                "expected_bytes": 1,
                "mode": "file"
            }),
        )
    };
    let (first_status, _, first) = create("first.bin").await;
    let (second_status, _, second) = create("second.bin").await;
    assert_eq!(first_status, StatusCode::CREATED, "{first}");
    assert_eq!(second_status, StatusCode::CREATED, "{second}");
    let first_handle = first["handle"].as_str().expect("first handle");
    let first_grant = first["grant"].as_str().expect("first grant");
    let second_grant = second["grant"].as_str().expect("second grant");
    assert_ne!(
        first_handle, "018f47c1-2af0-7cc4-a001-000000000001",
        "the closed host must not expose an enumerable handle sequence"
    );

    let status_path = format!("/__live/uploads/{first_handle}");
    for grant in [None, Some(second_grant)] {
        let (status, _, _) = upload_request(&host, Method::GET, &status_path, grant, "").await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "status accepted {grant:?}"
        );
    }
    let (status, _, _) =
        upload_request(&host, Method::GET, &status_path, Some(first_grant), "").await;
    assert_eq!(status, StatusCode::OK);

    let reacquire_path = format!("/example/uploads/{first_handle}/reacquire");
    let (status, _, reacquired) =
        upload_request(&host, Method::POST, &reacquire_path, None, "").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reacquired)
    );
    let reacquired: Value = serde_json::from_slice(&reacquired).expect("reacquire JSON");
    let successor_grant = reacquired["grant"].as_str().expect("successor grant");
    assert_ne!(successor_grant, first_grant);
    let (status, _, reacquired_again) =
        upload_request(&host, Method::POST, &reacquire_path, Some(first_grant), "").await;
    assert_eq!(status, StatusCode::OK);
    let reacquired_again: Value =
        serde_json::from_slice(&reacquired_again).expect("second reacquire JSON");
    let latest_grant = reacquired_again["grant"].as_str().expect("latest grant");
    assert_ne!(
        latest_grant, successor_grant,
        "reacquire must rotate bearers"
    );
    let (status, _, _) =
        upload_request(&host, Method::GET, &status_path, Some(first_grant), "").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "stale grant remained authority"
    );
    let (status, _, _) =
        upload_request(&host, Method::GET, &status_path, Some(latest_grant), "").await;
    assert_eq!(status, StatusCode::OK);

    for authorization in ["Bearer wrong-principal", "Bearer wrong-session"] {
        let (status, _, _) = request(
            &host,
            Method::POST,
            &reacquire_path,
            &[
                (AUTHORIZATION.as_str(), authorization),
                ("x-live-upload-grant", first_grant),
            ],
            Bytes::new(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let unknown = "018f8f3a-7b2c-4d5e-8f90-abcdef012345";
    let (status, _, _) = upload_request(
        &host,
        Method::POST,
        &format!("/example/uploads/{unknown}/reacquire"),
        None,
        "",
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "unknown handle was treated as owned"
    );

    let cancel_path = format!("/__live/uploads/{first_handle}/cancel");
    let (status, _, _) = upload_request(&host, Method::POST, &cancel_path, None, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) =
        upload_request(&host, Method::POST, &cancel_path, Some(second_grant), "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) =
        upload_request(&host, Method::POST, &cancel_path, Some(latest_grant), "").await;
    assert_eq!(status, StatusCode::OK);

    host.shutdown()
        .await
        .expect("clean upload authority shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborted_chunk_restores_coherent_upload_state_and_shutdown_removes_quarantine_bytes() {
    let root = TestRoot::new("upload-abort");
    let host = start_host(UPLOAD_ABORT_PORT, &root, ReferenceFaultSchedule::None).await;
    let inspection = host.inspection_handle();
    let (status, _, created) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "avatar",
            "filename": "aborted.bin",
            "content_type": "application/octet-stream",
            "expected_bytes": 8,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("handle");
    let grant = created["grant"].as_str().expect("grant");
    let checksum = Sha256::digest(b"abcdefgh")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut stream = TcpStream::connect(host.address())
        .await
        .expect("connect upload");
    stream
        .write_all(
            format!(
                "POST /__live/uploads/{handle}/chunks/0 HTTP/1.1\r\nHost: {}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nX-Live-Upload-Grant: {grant}\r\nX-Live-Chunk-SHA256: {checksum}\r\nX-Live-Chunk-Bytes: 8\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n",
                host.address()
            )
            .as_bytes(),
        )
        .await
        .expect("start partial body");
    timeout(Duration::from_secs(1), async {
        while inspection.snapshot().open_files == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("chunk entered provider I/O");
    drop(stream);
    tokio::time::sleep(Duration::from_millis(25)).await;

    let (status, _, body) = upload_request(
        &host,
        Method::GET,
        &format!("/__live/uploads/{handle}"),
        Some(grant),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let status_body: Value = serde_json::from_slice(&body).expect("status JSON");
    assert_eq!(status_body["received_bytes"], 0);
    assert_eq!(status_body["next_part"], 0);
    assert_eq!(inspection.snapshot().active_uploads, 1);

    host.shutdown().await.expect("bounded abort cleanup");
    assert_eq!(inspection.snapshot().active_uploads, 0);
    let mut entries = std::fs::read_dir(root.path()).expect("read quarantine root");
    assert!(entries.next().is_none(), "shutdown left quarantine files");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multipart_completion_verifies_the_truthful_whole_quarantine_digest() {
    let root = TestRoot::new("upload-whole-digest");
    let host = start_host(4_190, &root, ReferenceFaultSchedule::None).await;
    let (status, _, created) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "avatar",
            "filename": "different-parts.txt",
            "content_type": "text/plain",
            "expected_bytes": 10,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("upload handle");
    let grant = created["grant"].as_str().expect("upload grant");
    let first = chunked_upload(&host, handle, grant, 0, &[b"al", b"pha"]).await;
    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    let second = chunked_upload(&host, handle, grant, 1, &[b"om", b"ega"]).await;
    assert!(second.starts_with("HTTP/1.1 200"), "{second}");

    let (status, _, completed) = json_request(
        &host,
        Method::POST,
        &format!("/__live/uploads/{handle}/complete"),
        json!({"grant": grant}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "ready");
    host.shutdown()
        .await
        .expect("clean multipart digest shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_compiled_upload_interrupt_is_consumed_once_by_the_physical_body() {
    let root = TestRoot::new("upload-interrupt");
    let host = start_host(
        UPLOAD_FAULT_PORT,
        &root,
        ReferenceFaultSchedule::UploadBodyInterruptedOnce,
    )
    .await;
    let (status, _, created) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "avatar",
            "filename": "interrupted.txt",
            "content_type": "text/plain",
            "expected_bytes": 6,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("upload handle");
    let grant = created["grant"].as_str().expect("upload grant");
    let interrupted = chunked_upload(&host, handle, grant, 0, &[b"abc", b"def"]).await;
    assert!(interrupted.starts_with("HTTP/1.1 408"), "{interrupted}");
    assert!(
        interrupted.contains("upload_body_interrupted"),
        "{interrupted}"
    );
    let retried = chunked_upload(&host, handle, grant, 0, &[b"abc", b"def"]).await;
    assert!(retried.starts_with("HTTP/1.1 200"), "{retried}");
    assert_eq!(host.inspection().compiled_faults_applied, 1);
    host.shutdown()
        .await
        .expect("clean upload interrupt shutdown");
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

    let response = chunked_upload(&host, handle, grant, 0, &[b"hello ", b"world"]).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    let (status, _, upload) = upload_request(
        &host,
        Method::GET,
        &format!("/__live/uploads/{handle}"),
        Some(grant),
        Bytes::new(),
    )
    .await;
    let upload: Value = serde_json::from_slice(&upload).expect("upload status");
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

    let (status, _, reacquired) = upload_request(
        &host,
        Method::POST,
        &format!("/example/uploads/{handle}/reacquire"),
        Some(grant),
        Bytes::new(),
    )
    .await;
    let reacquired: Value = serde_json::from_slice(&reacquired).expect("reacquired upload");
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
    let memberships = transport["memberships"].as_array().expect("memberships");
    assert_eq!(memberships.len(), 2);
    for (index, membership) in memberships.iter().enumerate() {
        let subscription = membership["subscription"].as_str().expect("subscription");
        let membership_path =
            format!("/__live/async/transports/{transport_id}/subscriptions/{subscription}");
        let (status, _, rejected) = json_request(
            &host,
            Method::POST,
            &membership_path,
            json!({
                "authority": membership["authority"],
                "control_nonce": "",
                "operation": "subscribe"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{rejected}");
        let (status, _, acknowledgment) = json_request(
            &host,
            Method::POST,
            &membership_path,
            json!({
                "authority": membership["authority"],
                "control_nonce": format!("sse-subscribe-{index}"),
                "operation": "subscribe"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{acknowledgment}");
        assert_eq!(acknowledgment["operation"], "subscribe");
    }
    let subscription = memberships[0]["subscription"]
        .as_str()
        .expect("subscription");
    let authority = memberships[0]["authority"].as_str().expect("authority");

    let (status, _, rejected_poll) = request(
        &host,
        Method::POST,
        "/__live/async/poll",
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION),
            ("x-live-subscription", subscription),
            ("x-live-subscription-authority", authority),
        ],
        WRONG_ISLAND_FRESH_RENDER_REQUEST,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "wrong island reached fresh-render execution: {}",
        String::from_utf8_lossy(&rejected_poll)
    );

    let fresh_render_request = host
        .fresh_render_request("EBESExQVFhcYGRobHB0eHw", 0x51)
        .await
        .expect("current engine-owned fresh-render request");
    let (status, headers, poll_bytes) = request(
        &host,
        Method::POST,
        "/__live/async/poll",
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION),
            ("x-live-subscription", subscription),
            ("x-live-subscription-authority", authority),
        ],
        fresh_render_request.clone().into_bytes(),
    )
    .await;
    let poll: Value = serde_json::from_slice(&poll_bytes).expect("poll JSON");
    assert_eq!(status, StatusCode::OK, "{poll}");
    assert_eq!(
        headers
            .get("x-live-operation")
            .and_then(|value| value.to_str().ok()),
        Some("fresh-render")
    );
    assert_eq!(poll["outcome"], "accepted");
    assert_eq!(poll["render"]["kind"], "html");
    assert_eq!(
        headers
            .get("x-live-action-executed")
            .and_then(|value| value.to_str().ok()),
        Some("false")
    );
    assert!(
        poll["render"]["html"]
            .as_str()
            .unwrap()
            .contains("data-live-render-source=\"component-harness\"")
    );
    let parsed = parse_versioned_update_response(&poll_bytes, &protocol_limits())
        .expect("poll is one accepted Live response");
    assert!(matches!(parsed, VersionedUpdateResponse::V2(_)));
    let (status, _, _) = request(
        &host,
        Method::POST,
        "/__live/async/poll",
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION),
            ("x-live-subscription", subscription),
            ("x-live-subscription-authority", authority),
        ],
        fresh_render_request.clone().into_bytes(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "stale island was re-executed"
    );
    let forged_action = fresh_render_request.replace(
        "{\"kind\":\"fresh_render\"}",
        "{\"arguments\":{},\"kind\":\"invoke_action\",\"name\":\"forbidden\"}",
    );
    let (status, _, _) = request(
        &host,
        Method::POST,
        "/__live/async/poll",
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION),
            ("x-live-subscription", subscription),
            ("x-live-subscription-authority", authority),
        ],
        forged_action,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (mut sse_stream, sse) = read_sse_event(&host, transport_id).await;
    assert!(sse.starts_with("HTTP/1.1 200"), "{sse}");
    assert!(sse.contains("content-type: text/event-stream"), "{sse}");
    assert!(sse.contains("event:suprnova-live-async"), "{sse}");
    for membership in memberships {
        assert!(
            sse.contains(membership["subscription"].as_str().expect("subscription")),
            "{sse}"
        );
    }
    let removed = &memberships[1];
    let removed_subscription = removed["subscription"].as_str().expect("subscription");
    let membership_path =
        format!("/__live/async/transports/{transport_id}/subscriptions/{removed_subscription}");
    let (status, _, rejected) = json_request(
        &host,
        Method::POST,
        &membership_path,
        json!({
            "authority": removed["authority"],
            "control_nonce": "",
            "operation": "unsubscribe"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{rejected}");
    let (status, _, acknowledgment) = json_request(
        &host,
        Method::POST,
        &membership_path,
        json!({
            "authority": removed["authority"],
            "control_nonce": "sse-unsubscribe-1",
            "operation": "unsubscribe"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{acknowledgment}");
    assert_eq!(acknowledgment["operation"], "unsubscribe");
    let next_sse = read_stream_text(&mut sse_stream).await;
    assert!(next_sse.contains(subscription), "{next_sse}");
    assert!(!next_sse.contains(removed_subscription), "{next_sse}");

    let rejected = websocket_upgrade(&host, "https://cross-site.example", transport_id).await;
    assert!(rejected.starts_with("HTTP/1.1 403"), "{rejected}");
    let (status, _, websocket_transport) = json_request(
        &host,
        Method::POST,
        "/__live/async/transports",
        json!({"kind": "websocket", "subscription": "orders"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{websocket_transport}");
    let websocket_transport_id = websocket_transport["transport"]
        .as_str()
        .expect("transport");
    let websocket_membership = &websocket_transport["memberships"][0];
    let websocket = websocket_subscribe(&host, websocket_transport_id, websocket_membership).await;
    assert!(websocket.contains("101 Switching Protocols"), "{websocket}");
    assert!(
        websocket.contains("\"kind\":\"membership_authenticated\""),
        "{websocket}"
    );
    assert!(websocket.contains("\"protocol_version\":1"), "{websocket}");
    assert!(
        websocket.contains("\"kind\":\"unsubscribed\""),
        "{websocket}"
    );

    let inspection = host.inspection();
    assert_eq!(inspection.physical_sse_connections, 1);
    assert_eq!(inspection.physical_websocket_connections, 1);
    assert_eq!(inspection.maximum_logical_memberships, 2);
    assert_eq!(inspection.compiled_faults_applied, 1);
    host.shutdown().await.expect("clean async-host shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn document_transports_are_deduplicated_bounded_and_allow_one_physical_reader() {
    let root = TestRoot::new("transport-bounds");
    let host = start_host(TRANSPORT_BOUND_PORT, &root, ReferenceFaultSchedule::None).await;
    let mut issued = Vec::new();
    for kind in ["sse", "sse", "sse", "websocket", "websocket"] {
        let (status, _, transport) = json_request(
            &host,
            Method::POST,
            "/__live/async/transports",
            json!({"kind": kind, "subscription": "orders"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{transport}");
        issued.push(transport);
    }
    assert_eq!(issued[0]["transport"], issued[1]["transport"]);
    assert_eq!(issued[1]["transport"], issued[2]["transport"]);
    assert_eq!(issued[3]["transport"], issued[4]["transport"]);
    assert_ne!(issued[0]["transport"], issued[3]["transport"]);

    let sse = &issued[0];
    let transport_id = sse["transport"].as_str().expect("transport");
    for (index, membership) in sse["memberships"]
        .as_array()
        .expect("memberships")
        .iter()
        .enumerate()
    {
        let subscription = membership["subscription"].as_str().expect("subscription");
        let (status, _, body) = json_request(
            &host,
            Method::POST,
            &format!("/__live/async/transports/{transport_id}/subscriptions/{subscription}"),
            json!({
                "authority": membership["authority"],
                "control_nonce": format!("reader-subscribe-{index}"),
                "operation": "subscribe"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (first_reader, first) = read_sse_event(&host, transport_id).await;
    assert!(first.contains("HTTP/1.1 200"), "{first}");
    let (second_reader, second) = read_sse_event(&host, transport_id).await;
    assert!(
        second.contains("HTTP/1.1 409") || second.contains("HTTP/1.1 401"),
        "a second physical reader advanced the same document stream: {second}"
    );
    drop(second_reader);
    drop(first_reader);
    host.shutdown()
        .await
        .expect("clean bounded-transport shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_multiplexes_two_memberships_streams_and_shuts_down_an_open_socket() {
    let root = TestRoot::new("websocket-stream");
    let host = start_host(WEBSOCKET_STREAM_PORT, &root, ReferenceFaultSchedule::None).await;
    let inspection = host.inspection_handle();
    let (status, _, transport) = json_request(
        &host,
        Method::POST,
        "/__live/async/transports",
        json!({"kind": "websocket", "subscription": "orders"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{transport}");
    let transport_id = transport["transport"].as_str().expect("transport");
    let memberships = transport["memberships"].as_array().expect("memberships");
    let mut socket = websocket_connect(&host, transport_id).await;
    let mut buffered = Vec::new();
    let mut initial_sequences = std::collections::BTreeMap::new();

    for (index, membership) in memberships.iter().enumerate() {
        let subscription = membership["subscription"].as_str().expect("subscription");
        let binding = membership["descriptor_binding"].as_str().expect("binding");
        let nonce = format!("000000000000000{}", index + 1);
        write_websocket_frame(
            &mut socket,
            &format!(
                r#"{{"control_nonce":"{nonce}","descriptor_binding":"{binding}","kind":"subscribe","stream":"orders","subscription":"{subscription}","transport_generation":1}}"#
            ),
        )
        .await;
        let ack = loop {
            let frame = read_websocket_frame(&mut socket, &mut buffered).await;
            let value: Value = serde_json::from_str(&frame).expect("canonical acknowledgment");
            if value["control_nonce"] == nonce {
                break value;
            }
        };
        assert_eq!(ack["control_nonce"], nonce);
        assert_eq!(ack["subscription"], subscription);
        assert_eq!(ack["stream"], "orders");
        assert_eq!(ack["descriptor_binding"], binding);
        assert_eq!(ack["transport_generation"], 1);
        let envelope = loop {
            let frame = read_websocket_frame(&mut socket, &mut buffered).await;
            if frame.contains(subscription) {
                break frame;
            }
        };
        assert!(envelope.contains(subscription), "{envelope}");
        let envelope: Value = serde_json::from_str(&envelope).expect("initial engine envelope");
        let sequence = envelope["position"]["sequence"]
            .as_str()
            .expect("initial authoritative sequence")
            .parse::<u64>()
            .expect("initial decimal sequence");
        initial_sequences.insert(subscription.to_owned(), sequence);
    }

    let first_subscription = memberships[0]["subscription"]
        .as_str()
        .expect("first subscription");
    write_websocket_frame(
        &mut socket,
        &format!(r#"{{"kind":"unsubscribe","subscription":"{first_subscription}"}}"#),
    )
    .await;
    let unsubscribed = loop {
        let frame = read_websocket_frame(&mut socket, &mut buffered).await;
        let value: Value = serde_json::from_str(&frame).expect("unsubscribe ack");
        if value["kind"] == "unsubscribed" {
            break value;
        }
    };
    assert_eq!(unsubscribed["kind"], "unsubscribed");
    assert_eq!(unsubscribed["subscription"], first_subscription);

    let second_subscription = memberships[1]["subscription"]
        .as_str()
        .expect("second subscription");
    let ongoing = loop {
        let frame = read_websocket_frame(&mut socket, &mut buffered).await;
        if frame.contains(second_subscription) {
            break frame;
        }
    };
    let ongoing: Value = serde_json::from_str(&ongoing).expect("ongoing engine envelope");
    let ongoing_sequence = ongoing["position"]["sequence"]
        .as_str()
        .expect("ongoing authoritative sequence")
        .parse::<u64>()
        .expect("ongoing decimal sequence");
    assert!(
        ongoing_sequence
            > *initial_sequences
                .get(second_subscription)
                .expect("initial sequence for surviving membership"),
        "initial and ongoing frames must share one monotonic authority"
    );

    timeout(Duration::from_secs(2), host.shutdown())
        .await
        .expect("open WebSocket shutdown deadline")
        .expect("open WebSocket shutdown");
    let mut trailing = Vec::new();
    timeout(Duration::from_secs(1), socket.read_to_end(&mut trailing))
        .await
        .expect("WebSocket reached EOF after shutdown")
        .expect("WebSocket shutdown read");
    let final_state = inspection.snapshot();
    assert_eq!(final_state.open_sockets, 0);
    assert_eq!(final_state.logical_memberships, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canceled_fresh_render_does_not_wedge_the_instance_claim() {
    let root = TestRoot::new("fresh-render-cancel");
    let host = start_host(FRESH_CANCEL_PORT, &root, ReferenceFaultSchedule::None).await;
    let request = host
        .fresh_render_request("ZnJlc2gtY2FuY2VsLTAwMQ", 0xd1)
        .await
        .expect("fresh-render request");
    host.pause_fresh_render();
    {
        let execution = host.execute_fresh_render_direct(axum::body::Bytes::from(request.clone()));
        tokio::pin!(execution);
        tokio::select! {
            () = host.wait_until_fresh_render_paused() => {}
            result = &mut execution => panic!("paused render unexpectedly completed: {result:?}"),
        }
    }
    host.resume_fresh_render();

    let response = timeout(
        Duration::from_secs(1),
        host.execute_fresh_render_direct(axum::body::Bytes::from(request)),
    )
    .await
    .expect("immediate retry deadline")
    .expect("immediate retry response");
    assert_eq!(response.status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&response.body).expect("accepted retry body");
    assert_eq!(body["outcome"], "accepted");
    assert_eq!(body["accepted_revision"], "1");
    host.shutdown().await.expect("clean cancellation shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn poll_rejects_a_fresh_render_for_the_wrong_island() {
    let root = TestRoot::new("poll-identity");
    let host = start_host(POLL_PORT, &root, ReferenceFaultSchedule::None).await;
    let (status, _, transport) = json_request(
        &host,
        Method::POST,
        "/__live/async/transports",
        json!({"kind": "sse", "subscription": "orders"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{transport}");
    let membership = &transport["memberships"][0];
    let (status, _, body) = request(
        &host,
        Method::POST,
        "/__live/async/poll",
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION),
            (
                "x-live-subscription",
                membership["subscription"].as_str().expect("subscription"),
            ),
            (
                "x-live-subscription-authority",
                membership["authority"].as_str().expect("authority"),
            ),
        ],
        WRONG_ISLAND_FRESH_RENDER_REQUEST,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "wrong island reached fresh-render execution: {}",
        String::from_utf8_lossy(&body)
    );
    host.shutdown().await.expect("clean poll-host shutdown");
}

fn protocol_limits() -> ProtocolLimits {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024).expect("input limits"),
        max_snapshot_bytes: 32 * 1024,
        max_html_bytes: 32 * 1024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 16,
        max_validation_entries: 16,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .expect("protocol limits")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_closes_owned_sockets_files_and_timers() {
    let root = TestRoot::new("shutdown");
    let host = start_host(SHUTDOWN_PORT, &root, ReferenceFaultSchedule::None).await;
    let inspection = host.inspection_handle();
    let (status, _, created) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "avatar",
            "filename": "pending.bin",
            "content_type": "application/octet-stream",
            "expected_bytes": 8,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let upload_handle = created["handle"].as_str().expect("upload handle");
    let upload_grant = created["grant"].as_str().expect("upload grant");
    let (status, _, independent) = json_request(
        &host,
        Method::POST,
        "/__live/uploads",
        json!({
            "field": "avatar",
            "filename": "independent.bin",
            "content_type": "application/octet-stream",
            "expected_bytes": 1,
            "mode": "file"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{independent}");
    let independent_handle = independent["handle"].as_str().expect("independent handle");
    let independent_grant = independent["grant"].as_str().expect("independent grant");
    let (status, _, transport) = json_request(
        &host,
        Method::POST,
        "/__live/async/transports",
        json!({"kind": "sse", "subscription": "orders"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{transport}");
    let transport_id = transport["transport"].as_str().expect("transport");
    let membership = &transport["memberships"][0];
    let subscription = membership["subscription"].as_str().expect("subscription");
    let path = format!("/__live/async/transports/{transport_id}/subscriptions/{subscription}");
    let (status, _, _) = json_request(
        &host,
        Method::POST,
        &path,
        json!({
            "authority": membership["authority"],
            "control_nonce": "shutdown-subscribe",
            "operation": "subscribe"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_sse_stream, _) = read_sse_event(&host, transport_id).await;
    let upload_bytes = b"abcdefgh";
    let upload_checksum = Sha256::digest(upload_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut upload_stream = TcpStream::connect(host.address())
        .await
        .expect("connect mid-flight upload");
    upload_stream
        .write_all(
            format!(
                "POST /__live/uploads/{upload_handle}/chunks/0 HTTP/1.1\r\nHost: {}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nX-Live-Upload-Grant: {upload_grant}\r\nX-Live-Chunk-Sha256: {upload_checksum}\r\nX-Live-Chunk-Bytes: 8\r\nTransfer-Encoding: chunked\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n4\r\nabcd\r\n",
                host.address()
            )
            .as_bytes(),
        )
        .await
        .expect("start mid-flight upload body");
    timeout(Duration::from_secs(1), async {
        loop {
            if inspection.snapshot().open_files > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("file lease became observable");
    let midflight = inspection.snapshot();
    assert!(midflight.open_sockets > 0, "{midflight:?}");
    assert!(midflight.open_files > 0, "{midflight:?}");
    assert!(midflight.open_timers > 0, "{midflight:?}");
    assert!(midflight.active_uploads > 0, "{midflight:?}");
    assert!(midflight.logical_memberships > 0, "{midflight:?}");
    let status_path = format!("/__live/uploads/{independent_handle}");
    let (status, _, body) = timeout(
        Duration::from_millis(250),
        request(
            &host,
            Method::GET,
            &status_path,
            &[
                (AUTHORIZATION.as_str(), REFERENCE_AUTHORIZATION),
                ("x-live-upload-grant", independent_grant),
            ],
            "",
        ),
    )
    .await
    .expect("an incomplete body must not hold the upload ledger lock");
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    timeout(Duration::from_secs(2), host.shutdown())
        .await
        .expect("shutdown deadline")
        .expect("shutdown result");
    let mut upload_response = Vec::new();
    timeout(
        Duration::from_secs(1),
        upload_stream.read_to_end(&mut upload_response),
    )
    .await
    .expect("aborted upload body reached EOF")
    .expect("read aborted upload response");
    assert!(
        !String::from_utf8_lossy(&upload_response).starts_with("HTTP/1.1 200"),
        "incomplete request unexpectedly committed: {}",
        String::from_utf8_lossy(&upload_response)
    );
    let final_state = inspection.snapshot();
    assert_eq!(final_state.open_sockets, 0);
    assert_eq!(final_state.open_files, 0);
    assert_eq!(final_state.open_timers, 0);
    assert_eq!(final_state.active_uploads, 0);
    assert_eq!(final_state.logical_memberships, 0);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(inspection.snapshot(), final_state, "late resource revival");
}

async fn chunked_upload(
    host: &ReferenceHost,
    handle: &str,
    grant: &str,
    part: u32,
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
        "POST /__live/uploads/{handle}/chunks/{part} HTTP/1.1\r\nHost: {}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nX-Live-Upload-Grant: {grant}\r\nX-Live-Chunk-Sha256: {checksum}\r\nX-Live-Chunk-Bytes: {}\r\nTransfer-Encoding: chunked\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        host.address(),
        bytes.len()
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

async fn read_sse_event(host: &ReferenceHost, transport: &str) -> (TcpStream, String) {
    let mut stream = TcpStream::connect(host.address())
        .await
        .expect("connect SSE");
    let request = format!(
        "GET /__live/async/sse/{transport} HTTP/1.1\r\nHost: {}\r\nAuthorization: {REFERENCE_AUTHORIZATION}\r\nAccept: text/event-stream\r\n\r\n",
        host.address()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = vec![0_u8; 16 * 1024];
    let size = timeout(Duration::from_secs(1), stream.read(&mut response))
        .await
        .expect("SSE response deadline")
        .expect("SSE response");
    (
        stream,
        String::from_utf8(response[..size].to_vec()).expect("SSE UTF-8"),
    )
}

async fn read_stream_text(stream: &mut TcpStream) -> String {
    let mut response = vec![0_u8; 16 * 1024];
    let size = timeout(Duration::from_secs(1), stream.read(&mut response))
        .await
        .expect("stream response deadline")
        .expect("stream response");
    String::from_utf8_lossy(&response[..size]).into_owned()
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

async fn websocket_connect(host: &ReferenceHost, transport: &str) -> TcpStream {
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
    let response = String::from_utf8_lossy(&headers[..size]);
    assert!(response.starts_with("HTTP/1.1 101"), "{response}");
    stream
}

async fn websocket_subscribe(host: &ReferenceHost, transport: &str, membership: &Value) -> String {
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

    let subscription = membership["subscription"].as_str().expect("subscription");
    let binding = membership["descriptor_binding"].as_str().expect("binding");
    let payload = format!(
        r#"{{"control_nonce":"0000000000000001","descriptor_binding":"{binding}","kind":"subscribe","stream":"orders","subscription":"{subscription}","transport_generation":1}}"#
    );
    write_websocket_frame(&mut stream, &payload).await;
    let mut buffered = Vec::new();
    let authenticated = read_websocket_frame(&mut stream, &mut buffered).await;
    let envelope = read_websocket_frame(&mut stream, &mut buffered).await;
    let unsubscribe = format!(r#"{{"kind":"unsubscribe","subscription":"{subscription}"}}"#);
    write_websocket_frame(&mut stream, &unsubscribe).await;
    let unsubscribed = loop {
        let frame = read_websocket_frame(&mut stream, &mut buffered).await;
        if frame.contains("\"kind\":\"unsubscribed\"") {
            break frame;
        }
    };
    format!("{headers}{authenticated}{envelope}{unsubscribed}")
}

async fn write_websocket_frame(stream: &mut TcpStream, payload: &str) {
    let mask = [0x11, 0x22, 0x33, 0x44];
    let mut frame = vec![0x81];
    if payload.len() <= 125 {
        frame.push(0x80 | u8::try_from(payload.len()).unwrap());
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .bytes()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()]),
    );
    stream.write_all(&frame).await.unwrap();
}

async fn read_websocket_frame(stream: &mut TcpStream, buffered: &mut Vec<u8>) -> String {
    loop {
        if buffered.len() >= 2 {
            let marker = buffered[1] & 0x7f;
            let (payload_offset, payload_len) = match marker {
                0..=125 => (2, usize::from(marker)),
                126 if buffered.len() >= 4 => (
                    4,
                    usize::from(u16::from_be_bytes([buffered[2], buffered[3]])),
                ),
                _ => (usize::MAX, usize::MAX),
            };
            if payload_offset != usize::MAX && buffered.len() >= payload_offset + payload_len {
                let payload = buffered[payload_offset..payload_offset + payload_len].to_vec();
                buffered.drain(..payload_offset + payload_len);
                return String::from_utf8_lossy(&payload).into_owned();
            }
        }
        let mut response = vec![0_u8; 4_096];
        let size = timeout(Duration::from_secs(1), stream.read(&mut response))
            .await
            .expect("WebSocket response deadline")
            .expect("WebSocket response");
        assert!(size > 0, "WebSocket closed before a complete frame");
        buffered.extend_from_slice(&response[..size]);
    }
}
