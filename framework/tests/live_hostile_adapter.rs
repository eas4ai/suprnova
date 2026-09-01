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
        LiveSecurityCheck, prepare_live_router_for_test, record_live_security_pass_for_test,
    },
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, Crypt, EncryptionKey, FrameworkError, HttpResponse, Middleware, MiddlewareRegistry, Next,
    Request, Response, Router, StatusCode, async_trait, handle_request,
};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

static ACTION_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);

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
            .build(),
    );
    let mount = LiveMount::<HostileCounter>::public_seed("/hostile", "counter", "hostile-counter")
        .expect("declare hostile-test mount");
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
        .expect("register hostile-test document mount");
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
