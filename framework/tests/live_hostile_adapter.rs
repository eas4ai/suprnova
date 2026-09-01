//! Hostile traffic coverage through the real Suprnova Live HTTP adapter.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use suprnova::container::testing::TestContainer;
use suprnova::live::{
    CanonicalValue, LiveComponent, LiveDocument, LiveMount, LiveRegistry, MountFlags, live,
    testing::{
        LiveChildParameterDeliveryFixture, LiveSecurityCheck,
        prepare_child_parameter_delivery_for_test, prepare_live_router_for_test,
        record_live_security_not_required_for_test, record_live_security_pass_for_test,
    },
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, Crypt, EncryptionKey, FrameworkError, HttpResponse, Middleware, MiddlewareRegistry, Next,
    Request, Response, Router, StatusCode, async_trait, handle_request,
};
use suprnova_live::snapshot::state::encode_u64;

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

static ACTION_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);
static PARAMS_CHANGED_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(LiveComponent)]
#[live(
    name = "tests.hostile-counter",
    view = "live/tests/hostile-counter.html"
)]
pub struct HostileCounter {
    #[public]
    count: u64,
}

#[live]
impl HostileCounter {
    #[action]
    pub fn increment(&mut self) {
        ACTION_INVOCATIONS.fetch_add(1, Ordering::SeqCst);
        self.count += 1;
    }
}

#[derive(LiveComponent)]
#[live(
    name = "tests.hostile-child",
    view = "live/tests/hostile-child.html",
    minimum_protocol_version = 2
)]
pub struct HostileChild {
    #[public]
    count: u64,
    #[locked]
    query: String,
}

#[live]
impl HostileChild {
    #[mount]
    pub fn mount(query: String) -> Self {
        Self { count: 0, query }
    }

    #[params_changed]
    pub fn params_changed(&mut self) {
        PARAMS_CHANGED_INVOCATIONS.fetch_add(1, Ordering::SeqCst);
        self.count += 1;
    }
}

#[suprnova::view(path = "live/public-document.html")]
struct HostileDocumentView<'a> {
    island: &'a TrustedHtml,
}

struct StrictActionFacts;

#[async_trait]
impl Middleware for StrictActionFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        for (check, fact) in [
            (LiveSecurityCheck::Session, Some(b"session-42".as_slice())),
            (LiveSecurityCheck::Origin, None),
            (LiveSecurityCheck::Csrf, None),
            (
                LiveSecurityCheck::Principal,
                Some(b"principal-42".as_slice()),
            ),
            (LiveSecurityCheck::Tenant, Some(b"tenant-42".as_slice())),
            (LiveSecurityCheck::RateLimit, None),
        ] {
            if !record_live_security_pass_for_test(&mut request, check, fact) {
                return Err(HttpResponse::text("hostile test facts rejected").status(500));
            }
        }
        next(request).await
    }
}

struct PublicActionFacts;

#[async_trait]
impl Middleware for PublicActionFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        for check in [
            LiveSecurityCheck::Session,
            LiveSecurityCheck::Origin,
            LiveSecurityCheck::Csrf,
            LiveSecurityCheck::Principal,
            LiveSecurityCheck::Tenant,
            LiveSecurityCheck::RateLimit,
        ] {
            if !record_live_security_not_required_for_test(&mut request, check) {
                return Err(HttpResponse::text("public test facts rejected").status(500));
            }
        }
        next(request).await
    }
}

fn ensure_crypt() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

fn live_router() -> Router {
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<HostileCounter>()
            .expect("register hostile-test component")
            .register::<HostileChild>()
            .expect("register hostile child component")
            .build(),
    );
    let mount = LiveMount::<HostileCounter>::public_seed("/hostile", "counter", "hostile-counter")
        .expect("declare hostile-test mount");
    let child_mount = LiveMount::<HostileChild>::public_seed("/hostile", "child", "hostile-child")
        .expect("declare hostile child mount");
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get("/hostile", move |request: Request| {
            let mount = handler_mount.clone();
            async move {
                let result: Result<HttpResponse, FrameworkError> = async {
                    let mut document = LiveDocument::from_request(&request)?;
                    let island = document
                        .mount(
                            &mount,
                            CanonicalValue::Object(BTreeMap::new()),
                            MountFlags::empty(),
                        )
                        .await?;
                    document
                        .render(
                            ViewName::parse("live/public-document.html")
                                .map_err(|_| FrameworkError::internal("test view identity"))?,
                            &HostileDocumentView {
                                island: island.html(),
                            },
                            DocumentResponseIntent::html(StatusCode::OK)
                                .map_err(|_| FrameworkError::internal("test response intent"))?,
                            AssetSet::empty(),
                        )
                        .map_err(FrameworkError::from)
                }
                .await;
                result.map_err(|_| HttpResponse::text("Live document failed").status(500))
            }
        })
        .into();
    let router = router
        .try_live()
        .expect("install Live endpoint")
        .try_live_mount(&mount)
        .expect("register hostile-test document mount")
        .try_live_mount(&child_mount)
        .expect("register hostile child mount");
    prepare_live_router_for_test(&router).expect("prepare immutable Live runtime");
    router
}

async fn dispatch_request(
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    request: hyper::Request<Full<Bytes>>,
) -> (hyper::StatusCode, hyper::HeaderMap, Bytes) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept test request");
        let service = service_fn(move |request| {
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            async move {
                Ok::<_, std::convert::Infallible>(handle_request(router, middleware, request).await)
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect test request");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender.send_request(request).await.expect("send request");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    (status, headers, body)
}

fn html_attribute<'html>(html: &'html str, name: &str) -> &'html str {
    let prefix = format!("{name}=\"");
    let start = html
        .find(&prefix)
        .map(|index| index + prefix.len())
        .unwrap_or_else(|| panic!("missing HTML attribute {name}"));
    let tail = &html[start..];
    let end = tail
        .find('"')
        .unwrap_or_else(|| panic!("unterminated HTML attribute {name}"));
    &tail[..end]
}

async fn public_seed(router: Arc<Router>) -> Value {
    let request = hyper::Request::builder()
        .uri("/hostile")
        .body(Full::new(Bytes::new()))
        .expect("build document request");
    let (status, _, body) =
        dispatch_request(router, Arc::new(MiddlewareRegistry::new()), request).await;
    assert_eq!(status, hyper::StatusCode::OK);
    let document = std::str::from_utf8(&body).expect("document UTF-8");
    let encoded_seed = html_attribute(document, "data-suprnova-live-snapshot");
    let seed = URL_SAFE_NO_PAD
        .decode(encoded_seed)
        .expect("decode emitted seed snapshot");
    serde_json::from_slice(&seed).expect("parse emitted seed snapshot")
}

fn action_request(envelope: Value, document_key: &str) -> hyper::Request<Full<Bytes>> {
    let body = serde_json::to_vec(&json!({
        "base_revision": "0",
        "component": "tests.hostile-counter",
        "correlation_id": "AAECAwQFBgcICQoLDA0ODw",
        "extensions": {
            "x_suprnova_live_document_key_v1": document_key,
        },
        "idempotency_key": "EBESExQVFhcYGRobHB0eHw",
        "model_proposals": {},
        "operations": [{
            "arguments": {},
            "kind": "invoke_action",
            "name": "increment",
        }],
        "protocol_version": 1,
        "runtime_contract_version": 1,
        "snapshot": {
            "browser_nonce": "ICEiIyQlJicoKSorLC0uLw",
            "envelope": envelope,
            "kind": "seed_promotion",
        },
        "snapshot_schema_version": 1,
    }))
    .expect("encode hostile-test request");
    hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/__live/v1/action")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .body(Full::new(Bytes::from(body)))
        .expect("build hostile-test request")
}

fn assert_closed_response(headers: &hyper::HeaderMap, body: &Bytes) {
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control"),
        "no-store"
    );
    assert_eq!(headers.get("content-length").expect("Content-Length"), "0");
    assert!(body.is_empty());
}

fn params_changed_request(
    child_snapshot: Value,
    child_parameters: Value,
) -> hyper::Request<Full<Bytes>> {
    let revision = child_snapshot["body"]["revision"]
        .as_str()
        .expect("child snapshot revision");
    let body = serde_json::to_vec(&json!({
        "base_revision": revision,
        "child_parameters": child_parameters,
        "component": "tests.hostile-child",
        "correlation_id": "MDEyMzQ1Njc4OTo7PD0-Pw",
        "extensions": {
            "x_suprnova_live_document_key_v1": "hostile-child",
        },
        "idempotency_key": "QEFCQ0RFRkdISUpLTE1OTw",
        "model_proposals": {},
        "operations": [{"kind": "params_changed"}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": {
            "envelope": child_snapshot,
            "kind": "instance",
        },
        "snapshot_schema_version": 1,
    }))
    .expect("encode child-parameter request");
    hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/__live/v1/action")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=2",
        )
        .body(Full::new(Bytes::from(body)))
        .expect("build child-parameter request")
}

async fn child_delivery_fixture(
    session: Option<&[u8]>,
    principal: Option<&[u8]>,
    tenant: Option<&[u8]>,
) -> LiveChildParameterDeliveryFixture {
    let parent_mount =
        LiveMount::<HostileCounter>::public_seed("/hostile", "counter", "hostile-counter")
            .expect("parent mount");
    child_delivery_fixture_with_parent(&parent_mount, None, session, principal, tenant).await
}

async fn child_delivery_fixture_with_parent(
    parent_mount: &LiveMount<HostileCounter>,
    parent_build_override: Option<&str>,
    session: Option<&[u8]>,
    principal: Option<&[u8]>,
    tenant: Option<&[u8]>,
) -> LiveChildParameterDeliveryFixture {
    let child_mount = LiveMount::<HostileChild>::public_seed("/hostile", "child", "hostile-child")
        .expect("child mount");
    prepare_child_parameter_delivery_for_test(
        parent_mount,
        &child_mount,
        parent_build_override,
        CanonicalValue::Object(BTreeMap::from([(
            "query".to_owned(),
            CanonicalValue::String("rust".to_owned()),
        )])),
        CanonicalValue::Object(BTreeMap::from([(
            "query".to_owned(),
            CanonicalValue::String("zig".to_owned()),
        )])),
        CanonicalValue::Object(BTreeMap::from([("count".to_owned(), encode_u64(0))])),
        CanonicalValue::Object(BTreeMap::from([
            ("count".to_owned(), encode_u64(0)),
            (
                "query".to_owned(),
                CanonicalValue::String("rust".to_owned()),
            ),
        ])),
        session,
        principal,
        tenant,
    )
    .await
    .expect("prepare exact child delivery")
}

fn assert_engine_rejection(
    status: hyper::StatusCode,
    headers: &hyper::HeaderMap,
    body: &Bytes,
    expected_status: hyper::StatusCode,
) {
    assert_eq!(status, expected_status, "{}", String::from_utf8_lossy(body));
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control"),
        "no-store"
    );
    assert_eq!(
        headers.get("content-length").expect("Content-Length"),
        body.len().to_string().as_str(),
    );
    assert!(body.is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn valid_v2_child_delivery_runs_once_through_the_real_live_endpoint() {
    ensure_crypt();
    let _container = TestContainer::fake();
    PARAMS_CHANGED_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let parent_mount =
        LiveMount::<HostileCounter>::public_seed("/hostile", "counter", "hostile-counter")
            .expect("parent mount");
    let child_mount = LiveMount::<HostileChild>::public_seed("/hostile", "child", "hostile-child")
        .expect("child mount");
    let fixture = prepare_child_parameter_delivery_for_test(
        &parent_mount,
        &child_mount,
        None,
        CanonicalValue::Object(BTreeMap::from([(
            "query".to_owned(),
            CanonicalValue::String("rust".to_owned()),
        )])),
        CanonicalValue::Object(BTreeMap::from([(
            "query".to_owned(),
            CanonicalValue::String("zig".to_owned()),
        )])),
        CanonicalValue::Object(BTreeMap::from([("count".to_owned(), encode_u64(0))])),
        CanonicalValue::Object(BTreeMap::from([
            ("count".to_owned(), encode_u64(0)),
            (
                "query".to_owned(),
                CanonicalValue::String("rust".to_owned()),
            ),
        ])),
        None,
        None,
        None,
    )
    .await
    .expect("prepare exact child delivery");

    let (status, headers, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(PublicActionFacts)),
        params_changed_request(fixture.child_snapshot(), fixture.admission_carrier()),
    )
    .await;

    assert_eq!(
        status,
        hyper::StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control"),
        "no-store"
    );
    assert_eq!(
        headers.get("content-length").expect("Content-Length"),
        body.len().to_string().as_str(),
    );
    assert_eq!(
        headers.get("content-type").expect("Content-Type"),
        "application/vnd.suprnova.live+json; charset=utf-8; version=2",
    );
    let response: Value = serde_json::from_slice(&body).expect("sealed accepted response");
    assert_eq!(response["outcome"], "accepted");
    assert_eq!(response["accepted_revision"], "1");
    assert_eq!(response["protocol_version"], 2);
    assert_eq!(response["snapshot"]["body"]["revision"], "1");
    assert_eq!(response["snapshot"]["body"]["state"]["query"], "zig");
    assert!(
        response["render"]["html"]
            .as_str()
            .expect("accepted child HTML")
            .contains(">zig<"),
        "{}",
        response["render"]["html"],
    );
    assert_eq!(
        response["snapshot"]["body"]["extensions"]["x_suprnova_live_composition_v1"]["owner"]["parent_revision"],
        "1",
    );
    assert_eq!(PARAMS_CHANGED_INVOCATIONS.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .current_child_revision()
            .await
            .expect("child revision"),
        1
    );
    assert_eq!(
        fixture
            .current_parent_revision()
            .await
            .expect("parent revision"),
        1
    );
}

#[tokio::test]
#[serial_test::serial]
async fn stale_parent_build_and_unregistered_parent_slot_fail_before_component_or_ledger_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    PARAMS_CHANGED_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let registered =
        LiveMount::<HostileCounter>::public_seed("/hostile", "counter", "hostile-counter")
            .expect("registered parent mount");
    let removed =
        LiveMount::<HostileCounter>::public_seed("/hostile", "removed-parent", "removed-parent")
            .expect("removed parent mount");
    let fixtures = [
        child_delivery_fixture_with_parent(
            &registered,
            Some("suprnova-stale-build"),
            None,
            None,
            None,
        )
        .await,
        child_delivery_fixture_with_parent(&removed, None, None, None, None).await,
    ];

    for fixture in fixtures {
        let (status, headers, body) = dispatch_request(
            Arc::clone(&router),
            Arc::new(MiddlewareRegistry::new().append(PublicActionFacts)),
            params_changed_request(fixture.child_snapshot(), fixture.admission_carrier()),
        )
        .await;

        assert_engine_rejection(status, &headers, &body, hyper::StatusCode::CONFLICT);
        assert_eq!(PARAMS_CHANGED_INVOCATIONS.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture
                .current_child_revision()
                .await
                .expect("child revision"),
            0
        );
        assert_eq!(
            fixture
                .current_parent_revision()
                .await
                .expect("parent revision"),
            1
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn malformed_and_raw_envelope_child_carriers_fail_before_component_or_ledger_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    PARAMS_CHANGED_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let fixture = child_delivery_fixture(None, None, None).await;
    let carrier = fixture.admission_carrier();
    let raw_envelope = carrier["envelope"].clone();
    let malformed = json!({
        "envelope": carrier["envelope"].clone(),
        "parent_snapshot": carrier["parent_snapshot"].clone(),
        "unexpected": true,
    });

    for rejected in [raw_envelope, malformed] {
        let (status, headers, body) = dispatch_request(
            Arc::clone(&router),
            Arc::new(MiddlewareRegistry::new().append(PublicActionFacts)),
            params_changed_request(fixture.child_snapshot(), rejected),
        )
        .await;

        assert_engine_rejection(status, &headers, &body, hyper::StatusCode::BAD_REQUEST);
        assert_eq!(PARAMS_CHANGED_INVOCATIONS.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture
                .current_child_revision()
                .await
                .expect("child revision"),
            0
        );
        assert_eq!(
            fixture
                .current_parent_revision()
                .await
                .expect("parent revision"),
            1
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn signed_historical_v1_in_the_exact_carrier_fails_before_component_or_ledger_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    PARAMS_CHANGED_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let fixture = child_delivery_fixture(None, None, None).await;
    let mut carrier = fixture.admission_carrier();
    carrier["envelope"] = fixture.historical_v1_envelope();

    let (status, headers, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(PublicActionFacts)),
        params_changed_request(fixture.child_snapshot(), carrier),
    )
    .await;

    assert_engine_rejection(status, &headers, &body, hyper::StatusCode::CONFLICT);
    assert_eq!(PARAMS_CHANGED_INVOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .current_child_revision()
            .await
            .expect("child revision"),
        0
    );
    assert_eq!(
        fixture
            .current_parent_revision()
            .await
            .expect("parent revision"),
        1
    );
}

#[tokio::test]
#[serial_test::serial]
async fn forged_and_cross_child_carriers_fail_before_component_or_ledger_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    PARAMS_CHANGED_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let fixture = child_delivery_fixture(None, None, None).await;
    let other = child_delivery_fixture(None, None, None).await;
    let mut forged = fixture.admission_carrier();
    forged["envelope"]["signature"] =
        Value::String("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned());

    for rejected in [forged, other.admission_carrier()] {
        let (status, headers, body) = dispatch_request(
            Arc::clone(&router),
            Arc::new(MiddlewareRegistry::new().append(PublicActionFacts)),
            params_changed_request(fixture.child_snapshot(), rejected),
        )
        .await;

        assert_engine_rejection(status, &headers, &body, hyper::StatusCode::CONFLICT);
        assert_eq!(PARAMS_CHANGED_INVOCATIONS.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture
                .current_child_revision()
                .await
                .expect("child revision"),
            0
        );
        assert_eq!(
            fixture
                .current_parent_revision()
                .await
                .expect("parent revision"),
            1
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn cross_session_and_cross_tenant_requests_fail_before_component_or_ledger_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    PARAMS_CHANGED_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let cross_session = child_delivery_fixture(
        Some(b"session-other"),
        Some(b"principal-42"),
        Some(b"tenant-42"),
    )
    .await;
    let cross_tenant = child_delivery_fixture(
        Some(b"session-42"),
        Some(b"principal-42"),
        Some(b"tenant-other"),
    )
    .await;

    for fixture in [&cross_session, &cross_tenant] {
        let (status, headers, body) = dispatch_request(
            Arc::clone(&router),
            Arc::new(MiddlewareRegistry::new().append(StrictActionFacts)),
            params_changed_request(fixture.child_snapshot(), fixture.admission_carrier()),
        )
        .await;

        assert_engine_rejection(status, &headers, &body, hyper::StatusCode::CONFLICT);
        assert_eq!(PARAMS_CHANGED_INVOCATIONS.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture
                .current_child_revision()
                .await
                .expect("child revision"),
            0
        );
        assert_eq!(
            fixture
                .current_parent_revision()
                .await
                .expect("parent revision"),
            1
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn superseded_parent_revision_fails_before_component_or_child_ledger_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    PARAMS_CHANGED_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let fixture = child_delivery_fixture(None, None, None).await;
    assert_eq!(
        fixture
            .advance_parent_revision()
            .await
            .expect("advance parent revision"),
        2,
    );

    let (status, headers, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(PublicActionFacts)),
        params_changed_request(fixture.child_snapshot(), fixture.admission_carrier()),
    )
    .await;

    assert_engine_rejection(status, &headers, &body, hyper::StatusCode::NOT_FOUND);
    assert_eq!(PARAMS_CHANGED_INVOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .current_child_revision()
            .await
            .expect("child revision"),
        0
    );
    assert_eq!(
        fixture
            .current_parent_revision()
            .await
            .expect("parent revision"),
        2
    );
}

#[tokio::test]
#[serial_test::serial]
async fn missing_owner_evidence_is_concealed_before_component_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    ACTION_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let seed = public_seed(Arc::clone(&router)).await;

    let (status, headers, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new()),
        action_request(seed, "hostile-counter"),
    )
    .await;

    assert_eq!(status, hyper::StatusCode::NOT_FOUND);
    assert_closed_response(&headers, &body);
    assert_eq!(ACTION_INVOCATIONS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial_test::serial]
async fn tampered_seed_authority_is_rejected_before_component_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    ACTION_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let mut seed = public_seed(Arc::clone(&router)).await;
    seed["signature"] = Value::String("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned());

    let (status, headers, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(StrictActionFacts)),
        action_request(seed, "hostile-counter"),
    )
    .await;

    assert_eq!(status, hyper::StatusCode::CONFLICT);
    assert_closed_response(&headers, &body);
    assert_eq!(ACTION_INVOCATIONS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial_test::serial]
async fn cross_island_document_key_is_concealed_before_component_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    ACTION_INVOCATIONS.store(0, Ordering::SeqCst);
    let router = Arc::new(live_router());
    let seed = public_seed(Arc::clone(&router)).await;

    let (status, headers, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(StrictActionFacts)),
        action_request(seed, "another-document"),
    )
    .await;

    assert_eq!(status, hyper::StatusCode::NOT_FOUND);
    assert_closed_response(&headers, &body);
    assert_eq!(ACTION_INVOCATIONS.load(Ordering::SeqCst), 0);
}
