//! Shared harness for the Live dogfood tests: the real application router and
//! global middleware stack, an in-memory database with migrations, seeded
//! sessions, and Live request helpers.
#![allow(dead_code)]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, StatusCode};
use hyper_util::rt::TokioIo;
use sea_orm_migration::MigratorTrait;
use serde_json::{Value, json};
use suprnova::http::cookie::Cookie;
use suprnova::live::LiveRuntime;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::session::driver::database::DatabaseSessionDriver;
use suprnova::session::{SessionData, SessionStore, generate_csrf_token, generate_session_id};
use suprnova::{
    App, EncryptionKey, MiddlewareRegistry, Model, SessionConfig, UserProvider, attrs, bind,
    handle_request,
};
use tokio::sync::{Mutex, MutexGuard};

use app::live::providers::upload_finalizer::AppUploadFinalizer;
use app::migrations::Migrator;
use app::models::users::User;
use app::providers::DatabaseUserProvider;

pub const ACTION_PATH: &str = "/__live/v1/action";
pub const UPLOAD_PATH: &str = "/__live/v1/upload";
pub const SUBSCRIPTION_PATH: &str = "/__live/v1/async/subscriptions";
pub const MEMBERSHIP_PATH: &str = "/__live/v1/async/memberships";
pub const EVENTS_PATH: &str = "/__live/v1/async/events";
pub const SOCKET_PATH: &str = "/__live/v1/async/socket";
pub const LIVE_MEDIA: &str = "application/vnd.suprnova.live+json; charset=utf-8; version=2";
pub const CORRELATION_ID: &str = "MDEyMzQ1Njc4OTo7PD0-Pw";
pub const BROWSER_NONCE: &str = "ICEiIyQlJicoKSorLC0uLw";

static TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// One booted application: the accept loop serves `accepts` connections.
pub struct TestApp {
    pub addr: SocketAddr,
    pub port: u16,
    pub session_store: Arc<DatabaseSessionDriver>,
    pub finalizer: Arc<AppUploadFinalizer>,
    pub runtime: LiveRuntime,
    _lock: MutexGuard<'static, ()>,
}

impl TestApp {
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// The global middleware stack is process-wide; register it once so the
/// session and CSRF middleware run exactly once per request.
fn http_stack_once() {
    static STACK: OnceLock<()> = OnceLock::new();
    STACK.get_or_init(app::bootstrap::register_http_stack);
}

pub async fn setup_app(accepts: usize) -> TestApp {
    let lock = TEST_LOCK.lock().await;
    suprnova::Crypt::init(EncryptionKey::generate());

    let conn = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite::memory:");
    Migrator::up(&conn, None)
        .await
        .expect("run migrations against sqlite::memory:");
    App::singleton(suprnova::DbConnection::from_raw(conn));
    bind!(dyn UserProvider, DatabaseUserProvider);

    // The same Live bindings `bootstrap::register` installs.
    App::singleton(app::live::registry().expect("Live component registry"));
    let finalizer = Arc::new(AppUploadFinalizer::default());
    App::singleton(suprnova::live::LiveUploadHost::new().with_finalizer(finalizer.clone()));
    app::live::providers::authorize_live();

    let session_config = SessionConfig::default().secure(false);
    let session_store: Arc<DatabaseSessionDriver> =
        Arc::new(DatabaseSessionDriver::new(session_config.lifetime));

    let router = app::live::routes(app::routes::register()).expect("install Live routes");
    let runtime = prepare_live_router_for_test(&router).expect("prepare Live runtime");
    App::singleton(runtime.clone());
    let router = Arc::new(router);
    http_stack_once();
    let middleware = Arc::new(MiddlewareRegistry::from_global());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .with_upgrades()
                    .await;
            });
        }
    });

    TestApp {
        addr,
        port: addr.port(),
        session_store,
        finalizer,
        runtime,
        _lock: lock,
    }
}

/// The encrypted session cookie value of a signed-in user.
pub struct SeededSession {
    pub user_id: String,
    pub cookie: String,
    pub csrf: String,
}

pub async fn seed_session(app: &TestApp) -> SeededSession {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let user = User::create(attrs! {
        name: "Live Dogfood User",
        email: format!("live-{seq}@example.suprnova.app"),
        password: "hashed-by-test",
    })
    .await
    .expect("insert seed user");
    let session_id = generate_session_id();
    let mut session = SessionData::new(session_id.clone(), generate_csrf_token());
    session.user_id = Some(user.id.to_string());
    session.dirty = true;
    app.session_store
        .write(&session)
        .await
        .expect("write seed session");
    let encrypted = Cookie::encrypted("suprnova_session", &session_id)
        .expect("Crypt installed at setup_app")
        .value()
        .to_string();
    SeededSession {
        user_id: user.id.to_string(),
        cookie: encrypted,
        csrf: session.csrf_token.clone(),
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Reply {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("response body was not JSON: {}", self.text()))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

pub async fn send(addr: SocketAddr, request: hyper::Request<Full<Bytes>>) -> Reply {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = tokio::time::timeout(Duration::from_secs(20), sender.send_request(request))
        .await
        .expect("request completes within the timeout")
        .expect("send request");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    Reply {
        status,
        headers,
        body,
    }
}

/// A request builder carrying the host, an optional session cookie, and the
/// browser's same-origin proof.
pub fn request(
    app: &TestApp,
    method: Method,
    path: &str,
    session: Option<&SeededSession>,
    same_origin: bool,
) -> hyper::http::request::Builder {
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("127.0.0.1:{}", app.port));
    if let Some(session) = session {
        builder = builder.header("cookie", format!("suprnova_session={}", session.cookie));
    }
    if same_origin {
        builder = builder.header("sec-fetch-site", "same-origin");
    }
    builder
}

pub fn empty() -> Full<Bytes> {
    Full::new(Bytes::new())
}

pub async fn get(app: &TestApp, path: &str, session: Option<&SeededSession>) -> Reply {
    let request = request(app, Method::GET, path, session, false)
        .body(empty())
        .expect("build GET");
    send(app.addr, request).await
}

/// The dashboard HTML for a signed-in session, asserting the document rendered.
pub async fn dashboard_html(app: &TestApp, session: &SeededSession) -> String {
    let reply = get(app, "/live", Some(session)).await;
    assert_eq!(reply.status, StatusCode::OK, "dashboard: {}", reply.text());
    reply.text()
}

/// The opening tag of the island whose document key is `key`.
pub fn island_tag<'h>(html: &'h str, key: &str) -> &'h str {
    let needle = format!("data-suprnova-live-document-key=\"{key}\"");
    let position = html
        .find(&needle)
        .unwrap_or_else(|| panic!("no island with document key {key}"));
    let start = html[..position].rfind('<').expect("island tag start");
    let end = html[position..].find('>').expect("island tag end") + position + 1;
    &html[start..end]
}

pub fn attribute<'h>(tag: &'h str, name: &str) -> &'h str {
    let prefix = format!("{name}=\"");
    let start = tag
        .find(&prefix)
        .map(|index| index + prefix.len())
        .unwrap_or_else(|| panic!("missing attribute {name} in {tag}"));
    let tail = &tag[start..];
    let end = tail.find('"').expect("unterminated attribute");
    &tail[..end]
}

pub fn decoded_snapshot(tag: &str) -> Value {
    let encoded = attribute(tag, "data-suprnova-live-snapshot");
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("decode emitted Live snapshot");
    serde_json::from_slice(&bytes).expect("parse emitted Live snapshot")
}

/// The base revision an instance snapshot expects on its next request.
pub fn snapshot_revision(snapshot: &Value) -> String {
    match &snapshot["body"]["revision"] {
        Value::String(revision) => revision.clone(),
        Value::Number(revision) => revision.to_string(),
        other => panic!("snapshot carries no revision: {other}"),
    }
}

pub fn config_json(html: &str) -> Value {
    let start = html
        .find("<script id=\"suprnova-live-config\"")
        .expect("configuration element");
    let open = html[start..].find('>').expect("config open") + start + 1;
    let close = html[open..].find("</script>").expect("config close") + open;
    serde_json::from_str(&html[open..close]).expect("configuration JSON")
}

pub fn idempotency(sequence: u64) -> String {
    URL_SAFE_NO_PAD.encode(format!("{sequence:016}"))
}

/// The snapshot half of an action body: a seed promotion for public seeds,
/// an instance envelope otherwise.
pub fn snapshot_body(snapshot: &Value, seed: bool) -> Value {
    if seed {
        json!({"browser_nonce": BROWSER_NONCE, "envelope": snapshot, "kind": "seed_promotion"})
    } else {
        json!({"envelope": snapshot, "kind": "instance"})
    }
}

pub struct ActionSpec<'a> {
    pub component: &'a str,
    pub document_key: &'a str,
    pub snapshot: Value,
    pub seed: bool,
    pub base_revision: &'a str,
    pub operations: Value,
    pub model_proposals: Value,
    pub idempotency_key: &'a str,
}

pub fn action_request(
    app: &TestApp,
    spec: ActionSpec<'_>,
    session: Option<&SeededSession>,
    same_origin: bool,
) -> hyper::Request<Full<Bytes>> {
    let body = serde_json::to_vec(&json!({
        "base_revision": spec.base_revision,
        "child_parameters": null,
        "component": spec.component,
        "correlation_id": CORRELATION_ID,
        "extensions": {"x_suprnova_live_document_key_v1": spec.document_key},
        "idempotency_key": spec.idempotency_key,
        "model_proposals": spec.model_proposals,
        "operations": spec.operations,
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": snapshot_body(&spec.snapshot, spec.seed),
        "snapshot_schema_version": 1,
    }))
    .expect("encode action body");
    request(app, Method::POST, ACTION_PATH, session, same_origin)
        .header("content-type", LIVE_MEDIA)
        .body(Full::new(Bytes::from(body)))
        .expect("build action request")
}

pub fn invoke(action: &str) -> Value {
    json!([{"arguments": {}, "kind": "invoke_action", "name": action}])
}

pub fn fresh_render() -> Value {
    json!([{"kind": "fresh_render"}])
}

/// Minimal PNG: the signature and a 1 by 1 IHDR header, which the engine's
/// PNG validation accepts.
pub fn tiny_png() -> Vec<u8> {
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01".to_vec()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// One SSE record as the browser fetch adapter would decode it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseRecord {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: Option<String>,
    pub comment: Option<String>,
}

/// Incremental reader over one open Live SSE response.
pub struct SseClient {
    body: Incoming,
    buffer: BytesMut,
    pub status: StatusCode,
    pub headers: HeaderMap,
    _sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
}

impl SseClient {
    pub async fn open(
        app: &TestApp,
        session: &SeededSession,
        credential: &str,
        generation: u64,
    ) -> Self {
        let request = request(app, Method::GET, EVENTS_PATH, Some(session), true)
            .header("accept", "text/event-stream")
            .header("sec-fetch-site", "same-origin")
            .header("authorization", format!("SuprnovaAsync {credential}"))
            .header("suprnova-transport-generation", generation.to_string())
            .body(empty())
            .expect("build SSE request");
        let stream = tokio::net::TcpStream::connect(app.addr)
            .await
            .expect("connect");
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .expect("HTTP handshake");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let response = sender
            .send_request(request)
            .await
            .expect("send SSE request");
        let status = response.status();
        let headers = response.headers().clone();
        Self {
            body: response.into_body(),
            buffer: BytesMut::new(),
            status,
            headers,
            _sender: sender,
        }
    }

    pub async fn next_record(&mut self) -> Option<SseRecord> {
        loop {
            if let Some(end) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
                let record = self.buffer.split_to(end);
                let _ = self.buffer.split_to(2);
                return Some(parse_record(&record));
            }
            let frame = tokio::time::timeout(Duration::from_secs(20), self.body.frame())
                .await
                .expect("SSE frame arrives within the timeout")?
                .expect("read SSE frame");
            if let Ok(data) = frame.into_data() {
                self.buffer.extend_from_slice(&data);
            }
        }
    }

    pub async fn next_data(&mut self) -> Option<Value> {
        loop {
            let record = self.next_record().await?;
            if let Some(data) = record.data {
                return Some(serde_json::from_str(&data).expect("SSE data is JSON"));
            }
        }
    }
}

fn parse_record(record: &[u8]) -> SseRecord {
    let mut parsed = SseRecord {
        id: None,
        event: None,
        data: None,
        comment: None,
    };
    for line in record.split(|byte| *byte == b'\n') {
        let text = std::str::from_utf8(line).expect("SSE line is UTF-8");
        if let Some(comment) = text.strip_prefix(':') {
            parsed.comment = Some(comment.trim_start().to_owned());
        } else if let Some(value) = text.strip_prefix("id:") {
            parsed.id = Some(value.to_owned());
        } else if let Some(value) = text.strip_prefix("event:") {
            parsed.event = Some(value.to_owned());
        } else if let Some(value) = text.strip_prefix("data:") {
            parsed.data = Some(value.to_owned());
        } else if !text.is_empty() {
            panic!("unexpected SSE line {text:?}");
        }
    }
    parsed
}

pub fn control_nonce(index: u64) -> String {
    format!("{index:016x}")
}
