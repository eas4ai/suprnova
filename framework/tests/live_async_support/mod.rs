//! Shared fixture for the Suprnova Live asynchronous-transport route tests.
#![allow(dead_code)]

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use suprnova::auth::Authenticatable;
use suprnova::live::testing::{
    AdjustableTestClock, LiveSecurityCheck, inspect_request_attestation,
    prepare_live_router_for_test, prepare_live_router_with_clock_for_test,
    record_live_security_pass_for_test,
};
use suprnova::live::{
    EventPayloadMetadata, LiveComponent, LiveMount, LiveRegistry, LiveRuntime, live,
};
use suprnova::{
    App, Auth, Crypt, EncryptionKey, Gate, Middleware, MiddlewareRegistry, Next, Request, Response,
    Router, async_trait, handle_request,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::protocol::Message;

pub const SUBSCRIPTION_PATH: &str = "/__live/v1/async/subscriptions";
pub const MEMBERSHIP_PATH: &str = "/__live/v1/async/memberships";
pub const EVENTS_PATH: &str = "/__live/v1/async/events";
pub const SOCKET_PATH: &str = "/__live/v1/async/socket";
pub const ORDERS_COMPONENT: &str = "tests.async-orders";
pub const INVENTORY_COMPONENT: &str = "tests.async-inventory";

pub struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
}

pub struct StockChanged;

impl EventPayloadMetadata for StockChanged {
    const NAME: &'static str = "stock.changed";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "tests.async-orders",
    view = "live/tests/async-orders.html",
    minimum_protocol_version = 2,
    streams(stream(
        name = "orders",
        topics("orders", "orders/:principal"),
        events(OrdersUpdated),
        targets("self", "document"),
        fanout = 4,
    ))
)]
pub struct OrdersComponent {
    #[model]
    filter: String,
}

#[live]
impl OrdersComponent {
    #[mount]
    pub fn mount() -> Self {
        Self {
            filter: String::new(),
        }
    }
}

#[derive(LiveComponent)]
#[live(
    name = "tests.async-inventory",
    view = "live/tests/async-inventory.html",
    minimum_protocol_version = 2,
    streams(stream(
        name = "inventory",
        topics("inventory"),
        events(StockChanged),
        modes("sse"),
        reconnect = "refresh_on_reconnect",
    ))
)]
pub struct InventoryComponent {
    #[model]
    warehouse: String,
}

#[live]
impl InventoryComponent {
    #[mount]
    pub fn mount() -> Self {
        Self {
            warehouse: String::new(),
        }
    }
}

struct AsyncTestPrincipal(String);

impl Authenticatable for AsyncTestPrincipal {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Counts every request whose security facts the fixture middleware recorded.
pub static FACTS_RECORDED: AtomicU64 = AtomicU64::new(0);

/// Records the ordinary host security facts from test headers.
pub struct StrictAsyncFacts;

#[async_trait]
impl Middleware for StrictAsyncFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        // A chain that records no facts at all models a misconfigured host.
        if request.header("x-test-no-facts") == Some("1") {
            return next(request).await;
        }
        let session = request
            .header("x-test-session")
            .unwrap_or("async-session")
            .to_owned();
        let principal = request
            .header("x-test-principal")
            .unwrap_or("alice")
            .to_owned();
        let tenant = request
            .header("x-test-tenant")
            .unwrap_or("async-tenant")
            .to_owned();
        FACTS_RECORDED.fetch_add(1, Ordering::SeqCst);
        if request.header("x-test-no-auth") != Some("1") {
            Auth::set_user(Arc::new(AsyncTestPrincipal(principal.clone())));
        }
        // A WebSocket upgrade proves Origin and Csrf before this chain runs.
        let missing = inspect_request_attestation(&request)
            .missing_checks()
            .to_vec();
        for (check, fact) in [
            (LiveSecurityCheck::Session, Some(session.as_bytes())),
            (LiveSecurityCheck::Origin, None),
            (LiveSecurityCheck::Csrf, None),
            (LiveSecurityCheck::Principal, Some(principal.as_bytes())),
            (LiveSecurityCheck::Tenant, Some(tenant.as_bytes())),
            (LiveSecurityCheck::RateLimit, None),
        ] {
            if !missing.contains(&check) {
                continue;
            }
            // Routes outside the Live namespace carry no attestation binding.
            let _ = record_live_security_pass_for_test(&mut request, check, fact);
        }
        next(request).await
    }
}

pub fn ensure_crypt() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

fn define_gates() {
    Gate::define::<String, String>("live:tests.async-orders.stream.orders", |_, _| true);
    Gate::define::<String, String>(
        "live:tests.async-inventory.stream.inventory",
        |principal, _| principal != "mallory",
    );
}

fn build_router() -> Router {
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<OrdersComponent>()
            .expect("register orders component")
            .register::<InventoryComponent>()
            .expect("register inventory component")
            .build(),
    );
    define_gates();
    let orders =
        LiveMount::<OrdersComponent>::identity_bound("/orders", "orders-slot", "orders-document")
            .expect("declare orders mount");
    let inventory = LiveMount::<InventoryComponent>::identity_bound(
        "/orders",
        "inventory-slot",
        "inventory-document",
    )
    .expect("declare inventory mount");
    Router::new()
        .try_live()
        .expect("install Live routes")
        .try_live_mount(&orders)
        .expect("register orders mount")
        .try_live_mount(&inventory)
        .expect("register inventory mount")
}

/// Prepares the production runtime for the shared fixture components.
///
/// The runtime is bound in the global container because the test server
/// dispatches requests on other worker threads, exactly like production.
pub fn router_and_runtime() -> (Arc<Router>, LiveRuntime) {
    ensure_crypt();
    let router = build_router();
    let runtime = prepare_live_router_for_test(&router).expect("prepare async runtime");
    App::singleton(runtime.clone());
    (Arc::new(router), runtime)
}

/// Prepares the production runtime with an adjustable clock for expiry tests.
pub fn router_and_runtime_with_clock() -> (Arc<Router>, LiveRuntime, Arc<AdjustableTestClock>) {
    ensure_crypt();
    let router = build_router();
    let clock = Arc::new(AdjustableTestClock::new(1_700_000_000_000));
    let runtime = prepare_live_router_with_clock_for_test(&router, Arc::clone(&clock))
        .expect("prepare async runtime with clock");
    App::singleton(runtime.clone());
    (Arc::new(router), runtime, clock)
}

/// A running test server with the shared fixture middleware.
pub struct TestServer {
    pub port: u16,
    pub requests: Arc<AtomicU64>,
}

impl TestServer {
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

pub async fn spawn_server(router: Arc<Router>) -> TestServer {
    ensure_crypt();
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictAsyncFacts));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind async test listener");
    let port = listener.local_addr().expect("listener address").port();
    let requests = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<Incoming>| {
                    let router = Arc::clone(&router);
                    let middleware = Arc::clone(&middleware);
                    counter.fetch_add(1, Ordering::SeqCst);
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            handle_request(router, middleware, request).await,
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    TestServer { port, requests }
}

/// Test identity facts carried on every request of one simulated browser document.
#[derive(Clone, Debug)]
pub struct Identity {
    pub session: String,
    pub principal: String,
    pub tenant: String,
    pub authenticated: bool,
}

impl Identity {
    pub fn alice() -> Self {
        Self {
            session: "async-session".to_owned(),
            principal: "alice".to_owned(),
            tenant: "async-tenant".to_owned(),
            authenticated: true,
        }
    }

    pub fn with_session(mut self, session: &str) -> Self {
        self.session = session.to_owned();
        self
    }

    pub fn with_principal(mut self, principal: &str) -> Self {
        self.principal = principal.to_owned();
        self
    }

    pub fn with_tenant(mut self, tenant: &str) -> Self {
        self.tenant = tenant.to_owned();
        self
    }

    pub fn anonymous(mut self) -> Self {
        self.authenticated = false;
        self
    }

    fn apply(&self, builder: hyper::http::request::Builder) -> hyper::http::request::Builder {
        let builder = builder
            .header("x-test-session", &self.session)
            .header("x-test-principal", &self.principal)
            .header("x-test-tenant", &self.tenant);
        if self.authenticated {
            builder
        } else {
            builder.header("x-test-no-auth", "1")
        }
    }
}

pub struct HttpReply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl HttpReply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| {
            panic!(
                "response body was not JSON: {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    pub fn error_code(&self) -> String {
        self.json()["error"]
            .as_str()
            .expect("error code")
            .to_owned()
    }
}

async fn connect(port: u16) -> hyper::client::conn::http1::SendRequest<Full<Bytes>> {
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to async test server");
    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender
}

pub async fn send(
    port: u16,
    identity: &Identity,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: Bytes,
) -> HttpReply {
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("127.0.0.1:{port}"));
    builder = identity.apply(builder);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(Full::new(body)).expect("build async request");
    let mut sender = connect(port).await;
    let response = sender
        .send_request(request)
        .await
        .expect("send async request");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect async response")
        .to_bytes();
    HttpReply {
        status,
        headers,
        body,
    }
}

pub fn control_headers(credential: Option<&str>) -> Vec<(&str, String)> {
    let mut headers = vec![
        ("content-type", "application/json".to_owned()),
        ("x-suprnova-live", "async-v1".to_owned()),
    ];
    if let Some(credential) = credential {
        headers.push(("authorization", format!("SuprnovaAsync {credential}")));
    }
    headers
}

pub async fn post_control(
    port: u16,
    identity: &Identity,
    path: &str,
    credential: Option<&str>,
    body: Value,
) -> HttpReply {
    let headers = control_headers(credential);
    let borrowed = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect::<Vec<_>>();
    send(
        port,
        identity,
        Method::POST,
        path,
        &borrowed,
        Bytes::from(serde_json::to_vec(&body).expect("encode control body")),
    )
    .await
}

pub fn issue_body(
    transport: &str,
    component: &str,
    slot: &str,
    document_key: &str,
    stream: &str,
    document_instance: &str,
) -> Value {
    json!({
        "protocol_version": 1,
        "operation": "issue",
        "transport": transport,
        "stream": stream,
        "island": {
            "component": component,
            "slot": slot,
            "document_key": document_key,
        },
        "document_instance": document_instance,
    })
}

pub fn orders_issue_body(transport: &str, document_instance: &str) -> Value {
    issue_body(
        transport,
        ORDERS_COMPONENT,
        "orders-slot",
        "orders-document",
        "orders",
        document_instance,
    )
}

pub fn inventory_issue_body(transport: &str, document_instance: &str) -> Value {
    issue_body(
        transport,
        INVENTORY_COMPONENT,
        "inventory-slot",
        "inventory-document",
        "inventory",
        document_instance,
    )
}

/// One issued logical subscription as the browser adapter sees it.
#[derive(Clone, Debug)]
pub struct Issued {
    pub subscription_id: String,
    pub descriptor_binding: String,
    pub credential: Option<String>,
    pub stream: String,
    pub baseline: (String, String),
    pub value: Value,
}

pub fn parse_issued(value: &Value) -> Issued {
    let subscription = &value["subscription"];
    Issued {
        subscription_id: subscription["subscription_id"]
            .as_str()
            .expect("subscription_id")
            .to_owned(),
        descriptor_binding: subscription["descriptor_binding"]
            .as_str()
            .expect("descriptor_binding")
            .to_owned(),
        credential: subscription["authorization"]["credential"]
            .as_str()
            .map(str::to_owned),
        stream: subscription["stream"].as_str().expect("stream").to_owned(),
        baseline: (
            subscription["baseline"]["epoch"]
                .as_str()
                .expect("baseline epoch")
                .to_owned(),
            subscription["baseline"]["sequence"]
                .as_str()
                .expect("baseline sequence")
                .to_owned(),
        ),
        value: value.clone(),
    }
}

pub async fn issue(port: u16, identity: &Identity, body: Value) -> Issued {
    let reply = post_control(port, identity, SUBSCRIPTION_PATH, None, body).await;
    assert_eq!(
        reply.status,
        StatusCode::CREATED,
        "issue failed: {}",
        String::from_utf8_lossy(&reply.body)
    );
    parse_issued(&reply.json())
}

pub fn membership_body(
    operation: &str,
    issued: &Issued,
    control_nonce: &str,
    transport_generation: u64,
) -> Value {
    json!({
        "protocol_version": 1,
        "operation": operation,
        "subscription_id": issued.subscription_id,
        "descriptor_binding": issued.descriptor_binding,
        "stream": issued.stream,
        "control_nonce": control_nonce,
        "transport_generation": transport_generation,
    })
}

pub async fn subscribe(
    port: u16,
    identity: &Identity,
    credential: &str,
    issued: &Issued,
    control_nonce: &str,
    generation: u64,
) -> HttpReply {
    post_control(
        port,
        identity,
        MEMBERSHIP_PATH,
        Some(credential),
        membership_body("subscribe", issued, control_nonce, generation),
    )
    .await
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
        port: u16,
        identity: &Identity,
        credential: &str,
        generation: u64,
        extra: &[(&str, &str)],
    ) -> Self {
        // Extra headers replace the defaults so a test can send exactly one
        // `Accept` value, as a browser would.
        let mut headers = vec![
            ("host", format!("127.0.0.1:{port}")),
            ("accept", "text/event-stream".to_owned()),
            ("authorization", format!("SuprnovaAsync {credential}")),
            ("suprnova-transport-generation", generation.to_string()),
        ];
        for (name, value) in extra {
            headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
            headers.push((name, (*value).to_owned()));
        }
        let mut builder = hyper::Request::builder()
            .method(Method::GET)
            .uri(EVENTS_PATH);
        for (name, value) in &headers {
            builder = builder.header(*name, value.as_str());
        }
        builder = identity.apply(builder);
        let request = builder
            .body(Full::new(Bytes::new()))
            .expect("build SSE request");
        let mut sender = connect(port).await;
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

    /// Reads the complete response body of a rejected stream request.
    pub async fn rejection(mut self) -> HttpReply {
        let mut bytes = BytesMut::new();
        while let Some(frame) = self.body.frame().await {
            if let Ok(data) = frame.expect("read rejection frame").into_data() {
                bytes.extend_from_slice(&data);
            }
        }
        HttpReply {
            status: self.status,
            headers: self.headers,
            body: bytes.freeze(),
        }
    }

    /// Returns the next complete record, or `None` when the stream ended.
    pub async fn next_record(&mut self) -> Option<SseRecord> {
        loop {
            if let Some(end) = find_record_end(&self.buffer) {
                let record = self.buffer.split_to(end);
                let _ = self.buffer.split_to(2);
                return Some(parse_record(&record));
            }
            let frame = self.body.frame().await?.expect("read SSE frame");
            if let Ok(data) = frame.into_data() {
                self.buffer.extend_from_slice(&data);
            }
        }
    }

    /// Returns the next record carrying `data`, skipping comments.
    pub async fn next_data(&mut self) -> Option<Value> {
        loop {
            let record = self.next_record().await?;
            if let Some(data) = record.data {
                return Some(serde_json::from_str(&data).expect("SSE data is JSON"));
            }
        }
    }
}

fn find_record_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|pair| pair == b"\n\n")
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

pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens the Live WebSocket transport with an explicit browser `Origin`.
pub async fn connect_ws(
    port: u16,
    identity: &Identity,
    origin: Option<&str>,
) -> Result<WsStream, tokio_tungstenite::tungstenite::Error> {
    let url = format!("ws://127.0.0.1:{port}{SOCKET_PATH}");
    let mut request = url.into_client_request()?;
    let headers = request.headers_mut();
    headers.insert("x-test-session", identity.session.parse().expect("header"));
    headers.insert(
        "x-test-principal",
        identity.principal.parse().expect("header"),
    );
    headers.insert("x-test-tenant", identity.tenant.parse().expect("header"));
    if !identity.authenticated {
        headers.insert("x-test-no-auth", "1".parse().expect("header"));
    }
    if let Some(origin) = origin {
        headers.insert("origin", origin.parse().expect("origin header"));
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(stream, _)| stream)
}

pub fn ws_subscribe_frame(issued: &Issued, control_nonce: &str, generation: u64) -> String {
    json!({
        "control_nonce": control_nonce,
        "descriptor_binding": issued.descriptor_binding,
        "kind": "subscribe",
        "stream": issued.stream,
        "subscription": issued.subscription_id,
        "transport_generation": generation,
    })
    .to_string()
}

pub async fn ws_send(ws: &mut WsStream, text: String) {
    ws.send(Message::text(text)).await.expect("send WS frame");
}

pub async fn ws_next_text(ws: &mut WsStream) -> Option<String> {
    loop {
        match ws.next().await? {
            Ok(Message::Text(text)) => return Some(text.to_string()),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

pub async fn ws_next_close(ws: &mut WsStream) -> Option<(u16, String)> {
    loop {
        match ws.next().await? {
            Ok(Message::Close(Some(frame))) => {
                return Some((frame.code.into(), frame.reason.to_string()));
            }
            Ok(Message::Close(None)) => return Some((1005, String::new())),
            Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

pub fn control_nonce(index: u64) -> String {
    format!("{index:016x}")
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .collect()
}
