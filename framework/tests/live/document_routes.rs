//! Canonical document mounting through real Suprnova request and route machinery.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::json;
use suprnova::container::testing::TestContainer;
use suprnova::live::action::{
    OutcomeMetadata, action_result, flash_intent, route_intent, url_intent,
};
use suprnova::live::{
    ActionOutcome, ActionResult, CanonicalValue, LiveComponent, LiveDocument, LiveMount,
    LiveRegistry, MountFlags, live,
    testing::{
        LiveSecurityCheck, inspect_request_attestation, prepare_live_router_for_test,
        record_live_security_pass_for_test,
    },
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, Crypt, EncryptionKey, FrameworkError, HttpResponse, Middleware, MiddlewareRegistry, Next,
    Request, Response, Router, SessionConfig, SessionData, SessionMiddleware, SessionStore,
    StatusCode, async_trait, handle_request,
};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[derive(LiveComponent)]
#[live(name = "tests.public-counter", view = "live/tests/public-counter.html")]
pub struct PublicCounter {
    #[public]
    count: u64,
}

#[live]
impl PublicCounter {
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }

    #[action]
    pub fn redirect_to_receipt(&mut self) -> ActionOutcome {
        let parameters = CanonicalValue::Object(BTreeMap::from([(
            "receipt".to_owned(),
            CanonicalValue::String("42".to_owned()),
        )]));
        let intent = route_intent("live-response-intent.receipt", parameters)
            .expect("valid registered-route intent");
        ActionOutcome::Redirect(intent)
    }

    #[action]
    pub fn reflect_query(&mut self) -> ActionResult {
        let query = CanonicalValue::Object(BTreeMap::from([
            ("active".to_owned(), CanonicalValue::Bool(true)),
            (
                "page".to_owned(),
                CanonicalValue::number(2.0).expect("canonical page number"),
            ),
            (
                "q".to_owned(),
                CanonicalValue::String("red shoes".to_owned()),
            ),
        ]));
        let metadata = OutcomeMetadata::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(url_intent(query).expect("valid same-route URL intent")),
        )
        .expect("valid URL metadata");
        action_result::<Self>(ActionOutcome::NoRender, metadata)
            .expect("valid reflected action result")
    }

    #[action]
    pub fn flash_notice(&mut self) -> ActionResult {
        let metadata = OutcomeMetadata::new(
            vec![
                flash_intent("notice", CanonicalValue::String("saved".to_owned()))
                    .expect("valid flash intent"),
            ],
            Vec::new(),
            Vec::new(),
            None,
        )
        .expect("valid flash metadata");
        let parameters = CanonicalValue::Object(BTreeMap::from([(
            "receipt".to_owned(),
            CanonicalValue::String("flash".to_owned()),
        )]));
        let redirect = route_intent("live-response-intent.flash-receipt", parameters)
            .expect("valid flash redirect intent");
        action_result::<Self>(ActionOutcome::Redirect(redirect), metadata)
            .expect("valid flash redirect result")
    }

    #[action]
    pub fn flash_with_invalid_url(&mut self) -> ActionResult {
        let query = CanonicalValue::Object(BTreeMap::from([(
            "nested".to_owned(),
            CanonicalValue::Object(BTreeMap::from([(
                "forbidden".to_owned(),
                CanonicalValue::Bool(true),
            )])),
        )]));
        let metadata = OutcomeMetadata::new(
            vec![
                flash_intent(
                    "notice",
                    CanonicalValue::String("must-not-commit".to_owned()),
                )
                .expect("valid flash intent"),
            ],
            Vec::new(),
            Vec::new(),
            Some(url_intent(query).expect("bounded nested URL intent")),
        )
        .expect("valid bounded metadata");
        action_result::<Self>(ActionOutcome::NoRender, metadata)
            .expect("valid pre-resolution action result")
    }
}

#[derive(Default)]
struct MemorySessionStore(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().expect("session store lock").get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .expect("session store lock")
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().expect("session store lock").remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().expect("session store lock");
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

#[suprnova::view(path = "live/public-document.html")]
struct PublicDocumentView<'a> {
    island: &'a TrustedHtml,
}

struct IdentityFacts;

#[async_trait]
impl Middleware for IdentityFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        if !record_live_security_pass_for_test(
            &mut request,
            LiveSecurityCheck::Session,
            Some(b"session-42"),
        ) {
            return Err(HttpResponse::text("identity facts rejected: session").status(500));
        }
        if !record_live_security_pass_for_test(
            &mut request,
            LiveSecurityCheck::Principal,
            Some(b"principal-42"),
        ) {
            return Err(HttpResponse::text("identity facts rejected: principal").status(500));
        }
        if !record_live_security_pass_for_test(
            &mut request,
            LiveSecurityCheck::Tenant,
            Some(b"tenant-42"),
        ) {
            return Err(HttpResponse::text("identity facts rejected: tenant").status(500));
        }
        next(request).await
    }
}

struct ActionFacts;

#[async_trait]
impl Middleware for ActionFacts {
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
                return Err(HttpResponse::text("action facts rejected").status(500));
            }
        }
        let report = inspect_request_attestation(&request);
        if !report.order_is_valid() {
            return Err(HttpResponse::text("action fact order rejected").status(500));
        }
        next(request).await
    }
}

struct SessionScopedActionFacts;

#[async_trait]
impl Middleware for SessionScopedActionFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        for (check, fact) in [
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
                return Err(HttpResponse::text("session action facts rejected").status(500));
            }
        }
        let report = inspect_request_attestation(&request);
        if !report.order_is_valid() {
            return Err(HttpResponse::text("session action fact order rejected").status(500));
        }
        next(request).await
    }
}

struct FixedSessionActionFacts(Vec<u8>);

#[async_trait]
impl Middleware for FixedSessionActionFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        for (check, fact) in [
            (LiveSecurityCheck::Session, Some(self.0.as_slice())),
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
                return Err(HttpResponse::text("fixed session action facts rejected").status(500));
            }
        }
        next(request).await
    }
}

fn ensure_crypt() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

async fn dispatch_one(router: Router, path: &str) -> (hyper::StatusCode, Bytes) {
    let request = hyper::Request::builder()
        .uri(path)
        .body(Full::new(Bytes::new()))
        .expect("build request");
    let (status, _, body) = dispatch_request(
        Arc::new(router),
        Arc::new(MiddlewareRegistry::new()),
        request,
    )
    .await;
    (status, body)
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

fn decoded_snapshot(document: &[u8]) -> serde_json::Value {
    let document = std::str::from_utf8(document).expect("document UTF-8");
    let encoded = html_attribute(document, "data-suprnova-live-snapshot");
    let snapshot = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("decode emitted Live snapshot");
    serde_json::from_slice(&snapshot).expect("parse emitted Live snapshot")
}

fn live_action_request(
    snapshot: serde_json::Value,
    document_key: &str,
    action: &str,
    base_revision: &str,
    correlation_id: &str,
    idempotency_key: &str,
) -> hyper::Request<Full<Bytes>> {
    let snapshot = if base_revision == "0" {
        json!({
            "browser_nonce": "ICEiIyQlJicoKSorLC0uLw",
            "envelope": snapshot,
            "kind": "seed_promotion",
        })
    } else {
        json!({"envelope": snapshot, "kind": "instance"})
    };
    let body = serde_json::to_vec(&json!({
        "base_revision": base_revision,
        "child_parameters": null,
        "component": "tests.public-counter",
        "correlation_id": correlation_id,
        "extensions": {
            "x_suprnova_framework_document_path_v1": "/browser-forged",
            "x_suprnova_live_document_key_v1": document_key,
        },
        "idempotency_key": idempotency_key,
        "model_proposals": {},
        "operations": [{"arguments": {}, "kind": "invoke_action", "name": action}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": snapshot,
        "snapshot_schema_version": 1,
    }))
    .expect("encode Live action request");
    hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/__live/v1/action")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=2",
        )
        .header("referer", "https://attacker.invalid/referer-forged")
        .body(Full::new(Bytes::from(body)))
        .expect("build Live action request")
}

fn response_intent_router(mount: &LiveMount<PublicCounter>) -> Router {
    let handler_mount = mount.clone();
    let router = Router::new()
        .get("/flash-receipts/{receipt}", |_request: Request| async {
            Ok(HttpResponse::text("receipt"))
        })
        .name("live-response-intent.flash-receipt");
    let router: Router = router
        .get("/catalog/{section}", move |request: Request| {
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
                            &PublicDocumentView {
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
        .get("/session-start", |_request: Request| async {
            let present = suprnova::session::session_mut(|session| {
                session.put("live-test-marker", true);
            })
            .is_some();
            Ok(HttpResponse::text(if present {
                "session-started"
            } else {
                "session-missing"
            }))
        })
        .get("/flash-result", |_request: Request| async {
            let notice =
                suprnova::session::session_mut(|session| session.get_flash::<String>("notice"))
                    .flatten();
            Ok(HttpResponse::text(
                notice.unwrap_or_else(|| "missing".to_owned()),
            ))
        })
        .into();
    let router = router
        .try_live()
        .expect("install Live endpoint")
        .try_live_mount(mount)
        .expect("register response-intent document mount");
    prepare_live_router_for_test(&router).expect("prepare immutable Live runtime");
    router
}

fn session_middleware(store: Arc<MemorySessionStore>) -> Arc<MiddlewareRegistry> {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    Arc::new(MiddlewareRegistry::new().append(SessionMiddleware::with_store(config, store)))
}

fn session_action_middleware(store: Arc<MemorySessionStore>) -> Arc<MiddlewareRegistry> {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    Arc::new(
        MiddlewareRegistry::new()
            .append(SessionMiddleware::with_store(config, store))
            .append(SessionScopedActionFacts),
    )
}

fn session_cookie(headers: &hyper::HeaderMap) -> String {
    headers
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            let value = value.to_str().ok()?;
            value.split(';').next().map(str::to_owned)
        })
        .expect("session response must emit a cookie")
}

#[tokio::test]
#[serial_test::serial]
async fn public_seed_document_is_visible_before_javascript_and_carries_no_instance_authority() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register public component")
            .build(),
    );
    let mount = LiveMount::<PublicCounter>::public_seed("/catalog", "counter", "catalog-counter")
        .expect("declare public mount");
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get("/catalog", move |request: Request| {
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
                            &PublicDocumentView {
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
        .expect("register public document mount");
    prepare_live_router_for_test(&router).expect("prepare immutable Live runtime");

    let (status, body) = dispatch_one(router, "/catalog").await;
    let html = std::str::from_utf8(&body).expect("document is UTF-8");

    assert_eq!(status, hyper::StatusCode::OK, "response body: {html}");
    assert!(html.contains("<button id=\"counter\">0</button>"));
    assert!(html.contains("data-suprnova-live-snapshot-kind=\"seed\""));
    assert!(!html.contains("data-suprnova-live-instance="));
    assert!(!html.contains("<script"));
}

#[tokio::test]
#[serial_test::serial]
async fn identity_bound_document_mints_instance_authority_before_publication() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register identity-bound component")
            .build(),
    );
    let mount =
        LiveMount::<PublicCounter>::identity_bound("/account", "counter", "account-counter")
            .expect("declare identity-bound mount");
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get("/account", move |request: Request| {
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
                            &PublicDocumentView {
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
        .middleware(IdentityFacts)
        .into();
    let router = router
        .try_live()
        .expect("install Live endpoint")
        .try_live_mount(&mount)
        .expect("register identity-bound document mount");
    prepare_live_router_for_test(&router).expect("prepare immutable Live runtime");

    let (status, body) = dispatch_one(router, "/account").await;
    let html = std::str::from_utf8(&body).expect("document is UTF-8");

    assert_eq!(status, hyper::StatusCode::OK, "response body: {html}");
    assert!(html.contains("<button id=\"counter\">0</button>"));
    assert!(html.contains("data-suprnova-live-snapshot-kind=\"instance\""));
    assert!(html.contains("data-suprnova-live-instance=\""));
    assert!(html.contains("data-suprnova-live-document-key=\"account-counter\""));
}

#[tokio::test]
#[serial_test::serial]
async fn duplicate_document_identity_fails_before_any_island_bytes_are_published() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register duplicate-key component")
            .build(),
    );
    let mount = LiveMount::<PublicCounter>::public_seed("/duplicate", "counter", "same-key")
        .expect("declare duplicate-key mount");
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get("/duplicate", move |request: Request| {
            let mount = handler_mount.clone();
            async move {
                let result: Result<HttpResponse, FrameworkError> = async {
                    let mut document = LiveDocument::from_request(&request)?;
                    let _first = document
                        .mount(
                            &mount,
                            CanonicalValue::Object(BTreeMap::new()),
                            MountFlags::empty(),
                        )
                        .await?;
                    let _duplicate = document
                        .mount(
                            &mount,
                            CanonicalValue::Object(BTreeMap::new()),
                            MountFlags::empty(),
                        )
                        .await?;
                    Err(FrameworkError::internal(
                        "duplicate mount unexpectedly succeeded",
                    ))
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
        .expect("register duplicate-key document mount");
    prepare_live_router_for_test(&router).expect("prepare immutable Live runtime");

    let (status, body) = dispatch_one(router, "/duplicate").await;
    let html = std::str::from_utf8(&body).expect("failure body is UTF-8");

    assert_eq!(status, hyper::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(html, "Live document failed");
    assert!(!html.contains("data-suprnova-live-snapshot"));
    assert!(!html.contains("<button"));
}

#[tokio::test]
#[serial_test::serial]
async fn public_seed_get_promotes_and_executes_an_action_through_the_real_http_endpoint() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register interactive component")
            .build(),
    );
    let mount =
        LiveMount::<PublicCounter>::public_seed("/interactive", "counter", "interactive-counter")
            .expect("declare interactive mount");
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get("/interactive", move |request: Request| {
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
                            &PublicDocumentView {
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
        .expect("register interactive document mount");
    prepare_live_router_for_test(&router).expect("prepare immutable Live runtime");
    let router = Arc::new(router);

    let get = hyper::Request::builder()
        .uri("/interactive")
        .body(Full::new(Bytes::new()))
        .expect("build document request");
    let (get_status, _, get_body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new()),
        get,
    )
    .await;
    assert_eq!(get_status, hyper::StatusCode::OK);
    let document = std::str::from_utf8(&get_body).expect("document UTF-8");
    let encoded_seed = html_attribute(document, "data-suprnova-live-snapshot");
    let seed = URL_SAFE_NO_PAD
        .decode(encoded_seed)
        .expect("decode emitted seed snapshot");
    let seed = std::str::from_utf8(&seed).expect("seed snapshot UTF-8");
    let update = format!(
        r#"{{"base_revision":"0","component":"tests.public-counter","correlation_id":"AAECAwQFBgcICQoLDA0ODw","extensions":{{"x_suprnova_live_document_key_v1":"interactive-counter"}},"idempotency_key":"EBESExQVFhcYGRobHB0eHw","model_proposals":{{}},"operations":[{{"arguments":{{}},"kind":"invoke_action","name":"increment"}}],"protocol_version":1,"runtime_contract_version":1,"snapshot":{{"browser_nonce":"ICEiIyQlJicoKSorLC0uLw","envelope":{seed},"kind":"seed_promotion"}},"snapshot_schema_version":1}}"#,
    );
    let post = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/__live/v1/action")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .body(Full::new(Bytes::from(update)))
        .expect("build Live action request");
    let (status, headers, body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new().append(ActionFacts)),
        post,
    )
    .await;
    let response = std::str::from_utf8(&body).expect("Live response UTF-8");

    assert_eq!(status, hyper::StatusCode::OK, "response body: {response}");
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control"),
        "no-store"
    );
    assert!(response.contains("<button id=\\\"counter\\\">1</button>"));
    assert!(response.contains("data-suprnova-live-snapshot-kind=\\\"instance\\\""));
    assert!(response.contains("data-suprnova-live-document-key=\\\"interactive-counter\\\""));

    let accepted: serde_json::Value =
        serde_json::from_slice(&body).expect("parse accepted action response");
    let fresh = serde_json::to_vec(&json!({
        "base_revision": "1",
        "child_parameters": null,
        "component": "tests.public-counter",
        "correlation_id": "MDEyMzQ1Njc4OTo7PD0-Pw",
        "extensions": {
            "x_suprnova_live_document_key_v1": "interactive-counter",
        },
        "idempotency_key": "QEFCQ0RFRkdISUpLTE1OTw",
        "model_proposals": {},
        "operations": [{"kind": "fresh_render"}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": {
            "envelope": accepted["snapshot"].clone(),
            "kind": "instance",
        },
        "snapshot_schema_version": 1,
    }))
    .expect("encode fresh-render request");
    let post = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/__live/v1/action")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=2",
        )
        .body(Full::new(Bytes::from(fresh)))
        .expect("build fresh-render request");
    let (status, _, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(ActionFacts)),
        post,
    )
    .await;
    let response = std::str::from_utf8(&body).expect("fresh-render response UTF-8");

    assert_eq!(status, hyper::StatusCode::OK, "response body: {response}");
    assert!(response.contains("<button id=\\\"counter\\\">1</button>"));
    assert!(response.contains("\"accepted_revision\":\"2\""));
}

#[tokio::test]
#[serial_test::serial]
async fn registered_action_redirect_resolves_through_the_real_http_endpoint() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register redirecting component")
            .build(),
    );
    let mount =
        LiveMount::<PublicCounter>::public_seed("/redirecting", "counter", "redirecting-counter")
            .expect("declare redirecting mount");
    let handler_mount = mount.clone();
    let router = Router::new()
        .get("/receipts/{receipt}", |_request: Request| async {
            Ok(HttpResponse::text("receipt"))
        })
        .name("live-response-intent.receipt");
    let router: Router = router
        .get("/redirecting", move |request: Request| {
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
                            &PublicDocumentView {
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
        .expect("register redirecting document mount");
    prepare_live_router_for_test(&router).expect("prepare immutable Live runtime");
    let router = Arc::new(router);

    let get = hyper::Request::builder()
        .uri("/redirecting")
        .body(Full::new(Bytes::new()))
        .expect("build redirecting document request");
    let (get_status, _, get_body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new()),
        get,
    )
    .await;
    assert_eq!(get_status, hyper::StatusCode::OK);
    let document = std::str::from_utf8(&get_body).expect("document UTF-8");
    let encoded_seed = html_attribute(document, "data-suprnova-live-snapshot");
    let seed = URL_SAFE_NO_PAD
        .decode(encoded_seed)
        .expect("decode emitted seed snapshot");
    let seed = std::str::from_utf8(&seed).expect("seed snapshot UTF-8");
    let update = format!(
        r#"{{"base_revision":"0","component":"tests.public-counter","correlation_id":"UFFSU1RVVldYWVpbXF1eXw","extensions":{{"x_suprnova_live_document_key_v1":"redirecting-counter"}},"idempotency_key":"YGFiY2RlZmdoaWprbG1ubw","model_proposals":{{}},"operations":[{{"arguments":{{}},"kind":"invoke_action","name":"redirect_to_receipt"}}],"protocol_version":1,"runtime_contract_version":1,"snapshot":{{"browser_nonce":"cHFyc3R1dnd4eXp7fH1-fw","envelope":{seed},"kind":"seed_promotion"}},"snapshot_schema_version":1}}"#,
    );
    let post = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/__live/v1/action")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .body(Full::new(Bytes::from(update)))
        .expect("build redirecting Live action request");
    let (status, _, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(ActionFacts)),
        post,
    )
    .await;
    let response = std::str::from_utf8(&body).expect("Live response UTF-8");

    assert_eq!(status, hyper::StatusCode::OK, "response body: {response}");
    let accepted: serde_json::Value =
        serde_json::from_slice(&body).expect("parse accepted redirect response");
    assert_eq!(accepted["outcome"], "accepted");
    assert_eq!(accepted["redirect"], "/receipts/42");
    assert!(accepted.get("snapshot").is_none());
    assert!(accepted.get("render").is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn reflected_url_uses_the_signed_parameterized_document_path_and_survives_successors() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register URL-reflecting component")
            .build(),
    );
    let mount = LiveMount::<PublicCounter>::public_seed(
        "/catalog/{section}",
        "counter",
        "catalog-section-counter",
    )
    .expect("declare parameterized document mount");
    let router = Arc::new(response_intent_router(&mount));

    let get = hyper::Request::builder()
        .uri("/catalog/books?untrusted_original=discarded")
        .body(Full::new(Bytes::new()))
        .expect("build parameterized document request");
    let (status, _, body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new()),
        get,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK);
    let seed = decoded_snapshot(&body);

    let first = live_action_request(
        seed,
        "catalog-section-counter",
        "reflect_query",
        "0",
        "AAECAwQFBgcICQoLDA0ODw",
        "EBESExQVFhcYGRobHB0eHw",
    );
    let (status, _, body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new().append(ActionFacts)),
        first,
    )
    .await;
    assert_eq!(
        status,
        hyper::StatusCode::OK,
        "response body: {}",
        String::from_utf8_lossy(&body)
    );
    let accepted: serde_json::Value =
        serde_json::from_slice(&body).expect("parse first reflected response");
    assert_eq!(
        accepted["url_intent"],
        json!({
            "kind": "reflected",
            "target": "/catalog/books?active=true&page=2&q=red+shoes",
        })
    );

    let second = live_action_request(
        accepted["snapshot"].clone(),
        "catalog-section-counter",
        "reflect_query",
        "1",
        "MDEyMzQ1Njc4OTo7PD0-Pw",
        "QEFCQ0RFRkdISUpLTE1OTw",
    );
    let (status, _, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(ActionFacts)),
        second,
    )
    .await;
    let accepted: serde_json::Value =
        serde_json::from_slice(&body).expect("parse successor reflected response");
    assert_eq!(status, hyper::StatusCode::OK, "response body: {accepted}");
    assert_eq!(
        accepted["url_intent"],
        json!({
            "kind": "reflected",
            "target": "/catalog/books?active=true&page=2&q=red+shoes",
        })
    );
}

#[tokio::test]
#[serial_test::serial]
async fn tampering_with_the_signed_document_path_fails_closed() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register tamper-test component")
            .build(),
    );
    let mount = LiveMount::<PublicCounter>::public_seed(
        "/catalog/{section}",
        "counter",
        "catalog-tamper-counter",
    )
    .expect("declare tamper-test mount");
    let router = Arc::new(response_intent_router(&mount));
    let get = hyper::Request::builder()
        .uri("/catalog/books")
        .body(Full::new(Bytes::new()))
        .expect("build tamper-test document request");
    let (_, _, body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new()),
        get,
    )
    .await;
    let mut seed = decoded_snapshot(&body);
    seed["body"]["extensions"]["x_suprnova_framework_document_path_v1"] =
        serde_json::Value::String("/tampered".to_owned());

    let request = live_action_request(
        seed,
        "catalog-tamper-counter",
        "reflect_query",
        "0",
        "UFFSU1RVVldYWVpbXF1eXw",
        "YGFiY2RlZmdoaWprbG1ubw",
    );
    let (status, _, body) = dispatch_request(
        router,
        Arc::new(MiddlewareRegistry::new().append(ActionFacts)),
        request,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CONFLICT);
    assert!(body.is_empty(), "snapshot failures remain concealed");
    assert!(!String::from_utf8_lossy(&body).contains("/tampered"));
}

#[tokio::test]
#[serial_test::serial]
async fn accepted_flash_commits_once_and_is_consumed_by_the_next_session_backed_get() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register flashing component")
            .build(),
    );
    let mount = LiveMount::<PublicCounter>::public_seed(
        "/catalog/{section}",
        "counter",
        "catalog-flash-counter",
    )
    .expect("declare flashing mount");
    let router = Arc::new(response_intent_router(&mount));
    let store = Arc::new(MemorySessionStore::default());
    let middleware = session_action_middleware(Arc::clone(&store));
    let session_only = session_middleware(Arc::clone(&store));
    let get = hyper::Request::builder()
        .uri("/catalog/books")
        .body(Full::new(Bytes::new()))
        .expect("build flashing document request");
    let (_, _, body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new()),
        get,
    )
    .await;
    let seed = decoded_snapshot(&body);

    let request = live_action_request(
        seed,
        "catalog-flash-counter",
        "flash_notice",
        "0",
        "cHFyc3R1dnd4eXp7fH1-fw",
        "gIGCg4SFhoeIiYqLjI2Ojw",
    );
    let (status, headers, body) =
        dispatch_request(Arc::clone(&router), Arc::clone(&middleware), request).await;
    assert_eq!(
        status,
        hyper::StatusCode::OK,
        "response body: {}",
        String::from_utf8_lossy(&body)
    );
    let accepted: serde_json::Value =
        serde_json::from_slice(&body).expect("parse flashing redirect response");
    assert_eq!(
        accepted["url_intent"],
        json!({"kind": "navigated", "target": "/flash-receipts/flash"})
    );
    let cookie = session_cookie(&headers);

    let get = hyper::Request::builder()
        .uri("/flash-result")
        .header("cookie", &cookie)
        .body(Full::new(Bytes::new()))
        .expect("build flash-consuming request");
    let (status, _, body) =
        dispatch_request(Arc::clone(&router), Arc::clone(&session_only), get).await;
    assert_eq!(status, hyper::StatusCode::OK);
    assert_eq!(&body[..], b"saved");

    let get = hyper::Request::builder()
        .uri("/flash-result")
        .header("cookie", cookie)
        .body(Full::new(Bytes::new()))
        .expect("build second flash-consuming request");
    let (_, _, body) = dispatch_request(router, session_only, get).await;
    assert_eq!(&body[..], b"missing");
}

#[tokio::test]
#[serial_test::serial]
async fn response_intent_failure_does_not_commit_staged_flash() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register failure-test component")
            .build(),
    );
    let mount = LiveMount::<PublicCounter>::public_seed(
        "/catalog/{section}",
        "counter",
        "catalog-failure-counter",
    )
    .expect("declare failure-test mount");
    let router = Arc::new(response_intent_router(&mount));
    let store = Arc::new(MemorySessionStore::default());
    let middleware = session_action_middleware(Arc::clone(&store));
    let session_only = session_middleware(store);
    let start = hyper::Request::builder()
        .uri("/session-start")
        .body(Full::new(Bytes::new()))
        .expect("build session-start request");
    let (_, headers, _) =
        dispatch_request(Arc::clone(&router), Arc::clone(&session_only), start).await;
    let cookie = session_cookie(&headers);
    let get = hyper::Request::builder()
        .uri("/catalog/books")
        .body(Full::new(Bytes::new()))
        .expect("build failure-test document request");
    let (_, _, body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new()),
        get,
    )
    .await;
    let seed = decoded_snapshot(&body);

    let request = live_action_request(
        seed,
        "catalog-failure-counter",
        "increment",
        "0",
        "AAECAwQFBgcICQoLDA0ODw",
        "EBESExQVFhcYGRobHB0eHw",
    );
    let (parts, body) = request.into_parts();
    let mut request = hyper::Request::from_parts(parts, body);
    request
        .headers_mut()
        .insert("cookie", cookie.parse().expect("session cookie header"));
    let (status, _, body) =
        dispatch_request(Arc::clone(&router), Arc::clone(&middleware), request).await;
    assert_eq!(status, hyper::StatusCode::OK);
    let accepted: serde_json::Value =
        serde_json::from_slice(&body).expect("parse promoted instance response");
    let instance = accepted["snapshot"].clone();

    let request = live_action_request(
        instance,
        "catalog-failure-counter",
        "flash_with_invalid_url",
        "1",
        "kJGSk5SVlpeYmZqbnJ2enw",
        "oKGio6SlpqeoqaqrrK2urw",
    );
    let (parts, body) = request.into_parts();
    let mut request = hyper::Request::from_parts(parts, body);
    request
        .headers_mut()
        .insert("cookie", cookie.parse().expect("session cookie header"));
    let (status, _, body) =
        dispatch_request(Arc::clone(&router), Arc::clone(&middleware), request).await;
    assert_eq!(status, hyper::StatusCode::CONFLICT);
    assert!(!String::from_utf8_lossy(&body).contains("accepted_revision"));

    let get = hyper::Request::builder()
        .uri("/flash-result")
        .header("cookie", cookie)
        .body(Full::new(Bytes::new()))
        .expect("build flash absence request");
    let (_, _, body) = dispatch_request(router, session_only, get).await;
    assert_eq!(&body[..], b"missing");
}

#[tokio::test]
#[serial_test::serial]
async fn accepted_flash_without_a_session_scope_fails_closed() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PublicCounter>()
            .expect("register absent-session component")
            .build(),
    );
    let mount = LiveMount::<PublicCounter>::public_seed(
        "/catalog/{section}",
        "counter",
        "catalog-no-session-counter",
    )
    .expect("declare absent-session mount");
    let router = Arc::new(response_intent_router(&mount));
    let store = Arc::new(MemorySessionStore::default());
    let session_only = session_middleware(Arc::clone(&store));
    let start = hyper::Request::builder()
        .uri("/session-start")
        .body(Full::new(Bytes::new()))
        .expect("build absent-session retry scope request");
    let (_, headers, _) =
        dispatch_request(Arc::clone(&router), Arc::clone(&session_only), start).await;
    let cookie = session_cookie(&headers);
    let session_id = store
        .0
        .lock()
        .expect("session store lock")
        .keys()
        .next()
        .expect("started session identity")
        .as_bytes()
        .to_vec();
    let scope_only =
        Arc::new(MiddlewareRegistry::new().append(FixedSessionActionFacts(session_id)));
    let get = hyper::Request::builder()
        .uri("/catalog/books")
        .body(Full::new(Bytes::new()))
        .expect("build absent-session document request");
    let (_, _, body) = dispatch_request(
        Arc::clone(&router),
        Arc::new(MiddlewareRegistry::new()),
        get,
    )
    .await;
    let seed = decoded_snapshot(&body);
    let request = live_action_request(
        seed,
        "catalog-no-session-counter",
        "increment",
        "0",
        "AgMEBQYHCAkKCwwNDg8QEQ",
        "EhMUFRYXGBkaGxwdHh8gIQ",
    );
    let (status, _, body) =
        dispatch_request(Arc::clone(&router), Arc::clone(&scope_only), request).await;
    assert_eq!(status, hyper::StatusCode::OK);
    let accepted: serde_json::Value =
        serde_json::from_slice(&body).expect("parse absent-session fixture instance");
    let instance = accepted["snapshot"].clone();

    let request = live_action_request(
        instance,
        "catalog-no-session-counter",
        "flash_notice",
        "1",
        "sLGys7S1tre4ubq7vL2-vw",
        "wMHCw8TFxsfIycrLzM3Ozw",
    );
    let (status, _, body) = dispatch_request(Arc::clone(&router), scope_only, request).await;

    assert_eq!(status, hyper::StatusCode::CONFLICT);
    assert!(!String::from_utf8_lossy(&body).contains("accepted_revision"));

    let get = hyper::Request::builder()
        .uri("/flash-result")
        .header("cookie", cookie)
        .body(Full::new(Bytes::new()))
        .expect("build absent-session flash inspection request");
    let (_, _, body) = dispatch_request(router, session_only, get).await;
    assert_eq!(&body[..], b"missing");
}
