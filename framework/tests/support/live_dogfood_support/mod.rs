//! Shared dogfood fixtures: a production-shaped middleware stack, a guarded
//! Live router with one public island, and request helpers.
#![allow(dead_code)]

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use suprnova::auth::Authenticatable;
use suprnova::live::{
    CanonicalValue, LiveBootstrapOptions, LiveComponent, LiveDocument, LiveMount, LiveRegistry,
    LiveRouteGuard, LiveTenantMiddleware, LiveTenantResolver, MountFlags, live,
};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, Auth, AuthMiddleware, Crypt, CsrfMiddleware, EncryptionKey, FrameworkError, HttpResponse,
    Middleware, MiddlewareRegistry, Next, RateLimitMiddleware, Request, Response, Router,
    SessionConfig, SessionData, SessionMiddleware, SessionStore, SlidingWindowConfig, StatusCode,
    async_trait, handle_request,
};

pub mod filters {
    pub use suprnova::view::filters::trusted_html;
}

pub const DOCUMENT_PATH: &str = "/dogfood";
pub const DOCUMENT_KEY: &str = "dogfood-counter";
pub const ACTION_PATH: &str = "/__live/v1/action";
pub const SUBSCRIPTION_PATH: &str = "/__live/v1/async/subscriptions";

#[derive(LiveComponent)]
#[live(
    name = "tests.dogfood-counter",
    view = "live/tests/dogfood-counter.html"
)]
pub struct DogfoodCounter {
    #[public]
    count: u64,
}

#[live]
impl DogfoodCounter {
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}

#[suprnova::view(path = "live/dogfood-document.html")]
pub struct DogfoodDocument<'a> {
    bootstrap: &'a TrustedHtml,
    island: &'a TrustedHtml,
}

pub struct Principal(String);

impl Authenticatable for Principal {
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

/// Stands in for the application's sign-in: a request carrying
/// `x-test-login: <id>` is treated as that authenticated user before the
/// framework's own `AuthMiddleware` (attached through the Live guard) runs.
pub struct LoginHeader;

#[async_trait]
impl Middleware for LoginHeader {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(id) = request.header("x-test-login") {
            Auth::set_user(Arc::new(Principal(id.to_owned())));
        }
        next(request).await
    }
}

pub struct Tenantless;

#[async_trait]
impl LiveTenantResolver for Tenantless {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct MemorySessionStore(Mutex<HashMap<String, SessionData>>);

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

/// Registers the dogfood component into the active container and initializes
/// the key material once per process.
pub fn fixture() {
    static CRYPT: OnceLock<()> = OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<DogfoodCounter>()
            .expect("register dogfood counter")
            .build(),
    );
}

/// The production-shaped global stack: session, then origin-verified CSRF,
/// then the test sign-in shim.
pub fn production_middleware() -> Arc<MiddlewareRegistry> {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    Arc::new(
        MiddlewareRegistry::new()
            .append(SessionMiddleware::with_store(
                config,
                Arc::new(MemorySessionStore::default()),
            ))
            .append(CsrfMiddleware::new())
            .append(LoginHeader),
    )
}

/// A guard that lets anonymous visitors reach the reserved routes; the mount
/// kind then decides whether an anonymous request may proceed.
pub fn public_live_guard(guard: LiveRouteGuard) -> LiveRouteGuard {
    guard
        .middleware(AuthMiddleware::optional())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .middleware(RateLimitMiddleware::new(
            Arc::new(InMemoryRateLimiter::new()),
            SlidingWindowConfig {
                max_requests: 1_000,
                window: Duration::from_secs(60),
            },
            |_request| "live-dogfood".to_owned(),
        ))
}

pub const PRIVATE_DOCUMENT_PATH: &str = "/dogfood/private";
pub const PRIVATE_DOCUMENT_KEY: &str = "dogfood-private";

pub fn live_guard(guard: LiveRouteGuard) -> LiveRouteGuard {
    guard
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .middleware(RateLimitMiddleware::new(
            Arc::new(InMemoryRateLimiter::new()),
            SlidingWindowConfig {
                max_requests: 1_000,
                window: Duration::from_secs(60),
            },
            |_request| "live-dogfood".to_owned(),
        ))
}

pub fn build_router() -> Router {
    build_router_with(live_guard)
}

/// The same document with the permissive guard plus an identity-bound copy of
/// the counter at `PRIVATE_DOCUMENT_PATH`.
pub fn build_public_router() -> Router {
    let private = LiveMount::<DogfoodCounter>::identity_bound(
        PRIVATE_DOCUMENT_PATH,
        "counter",
        PRIVATE_DOCUMENT_KEY,
    )
    .expect("declare private dogfood mount");
    let handler_mount = private.clone();
    let router: Router = build_router_with(public_live_guard)
        .get(PRIVATE_DOCUMENT_PATH, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_document(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .into();
    router
        .try_live_mount(&private)
        .expect("register private dogfood mount")
}

async fn render_document(
    request: Request,
    mount: LiveMount<DogfoodCounter>,
) -> Result<HttpResponse, HttpResponse> {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)
            .map_err(|error| FrameworkError::internal(format!("from_request {error}")))?;
        let island = document
            .mount(
                &mount,
                CanonicalValue::Object(BTreeMap::new()),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
        let bootstrap = document
            .bootstrap(LiveBootstrapOptions::esm())
            .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
        document
            .render(
                ViewName::parse("live/dogfood-document.html")
                    .map_err(|_| FrameworkError::internal("view identity"))?,
                &DogfoodDocument {
                    bootstrap: bootstrap.html(),
                    island: island.html(),
                },
                DocumentResponseIntent::html(StatusCode::OK)
                    .map_err(|_| FrameworkError::internal("response intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|error| HttpResponse::text(format!("Live document failed: {error}")).status(500))
}

fn build_router_with(configure: fn(LiveRouteGuard) -> LiveRouteGuard) -> Router {
    let mount = LiveMount::<DogfoodCounter>::public_seed(DOCUMENT_PATH, "counter", DOCUMENT_KEY)
        .expect("declare dogfood mount");
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get(DOCUMENT_PATH, move |request: Request| {
            let mount = handler_mount.clone();
            async move {
                let result: Result<HttpResponse, FrameworkError> = async {
                    let mut document = LiveDocument::from_request(&request).map_err(|error| {
                        FrameworkError::internal(format!("from_request {error}"))
                    })?;
                    let island = document
                        .mount(
                            &mount,
                            CanonicalValue::Object(BTreeMap::new()),
                            MountFlags::empty(),
                        )
                        .await
                        .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
                    let bootstrap = document
                        .bootstrap(LiveBootstrapOptions::esm())
                        .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
                    document
                        .render(
                            ViewName::parse("live/dogfood-document.html")
                                .map_err(|_| FrameworkError::internal("view identity"))?,
                            &DogfoodDocument {
                                bootstrap: bootstrap.html(),
                                island: island.html(),
                            },
                            DocumentResponseIntent::html(StatusCode::OK)
                                .map_err(|_| FrameworkError::internal("response intent"))?,
                            AssetSet::empty(),
                        )
                        .map_err(FrameworkError::from)
                }
                .await;
                result.map_err(|error| {
                    HttpResponse::text(format!("Live document failed: {error}")).status(500)
                })
            }
        })
        .into();
    router
        .try_live_with(configure)
        .expect("install guarded Live routes")
        .try_live_mount(&mount)
        .expect("register dogfood mount")
}

pub async fn dispatch(
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    request: hyper::Request<Full<Bytes>>,
) -> (StatusCode, hyper::HeaderMap, Bytes) {
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
    send(address, request).await
}

pub async fn send(
    address: std::net::SocketAddr,
    request: hyper::Request<Full<Bytes>>,
) -> (StatusCode, hyper::HeaderMap, Bytes) {
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

pub fn get(path: &str) -> hyper::Request<Full<Bytes>> {
    hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(path)
        .header("host", "127.0.0.1")
        .body(Full::new(Bytes::new()))
        .expect("build request")
}

pub fn html_attribute<'html>(html: &'html str, name: &str) -> &'html str {
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

pub fn decoded_snapshot(document: &[u8]) -> Value {
    let document = std::str::from_utf8(document).expect("document UTF-8");
    let encoded = html_attribute(document, "data-suprnova-live-snapshot");
    let snapshot = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("decode emitted Live snapshot");
    serde_json::from_slice(&snapshot).expect("parse emitted Live snapshot")
}

/// The session cookie pair from a response, skipping the XSRF token cookie
/// the CSRF middleware attaches alongside it.
pub fn session_cookie(headers: &hyper::HeaderMap) -> String {
    headers
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            let pair = value.to_str().ok()?.split(';').next()?.to_owned();
            (!pair.starts_with("XSRF-TOKEN=")).then_some(pair)
        })
        .expect("session response must emit a session cookie")
}

pub struct ActionRequest<'a> {
    pub snapshot: Value,
    pub cookie: &'a str,
    pub fetch_site: Option<&'a str>,
    pub login: Option<&'a str>,
    pub idempotency_key: &'a str,
}

pub fn action_request(spec: ActionRequest<'_>) -> hyper::Request<Full<Bytes>> {
    action_request_for(spec, DOCUMENT_KEY, "seed_promotion", "0")
}

/// An action against the identity-bound copy of the counter.
pub fn private_action_request(spec: ActionRequest<'_>) -> hyper::Request<Full<Bytes>> {
    let revision = match &spec.snapshot["body"]["revision"] {
        Value::String(revision) => revision.clone(),
        Value::Number(revision) => revision.to_string(),
        other => panic!("instance snapshot carries no revision: {other}"),
    };
    action_request_for(spec, PRIVATE_DOCUMENT_KEY, "instance", &revision)
}

fn action_request_for(
    spec: ActionRequest<'_>,
    document_key: &str,
    snapshot_kind: &str,
    base_revision: &str,
) -> hyper::Request<Full<Bytes>> {
    let snapshot = if snapshot_kind == "seed_promotion" {
        json!({
            "browser_nonce": "ICEiIyQlJicoKSorLC0uLw",
            "envelope": spec.snapshot,
            "kind": snapshot_kind,
        })
    } else {
        json!({ "envelope": spec.snapshot, "kind": snapshot_kind })
    };
    let body = serde_json::to_vec(&json!({
        "base_revision": base_revision,
        "child_parameters": null,
        "component": "tests.dogfood-counter",
        "correlation_id": "MDEyMzQ1Njc4OTo7PD0-Pw",
        "extensions": {"x_suprnova_live_document_key_v1": document_key},
        "idempotency_key": spec.idempotency_key,
        "model_proposals": {},
        "operations": [{"arguments": {}, "kind": "invoke_action", "name": "increment"}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": snapshot,
        "snapshot_schema_version": 1,
    }))
    .expect("encode Live action request");
    let mut builder = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(ACTION_PATH)
        .header("host", "127.0.0.1")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=2",
        )
        .header("cookie", spec.cookie);
    if let Some(site) = spec.fetch_site {
        builder = builder.header("sec-fetch-site", site);
    }
    if let Some(id) = spec.login {
        builder = builder.header("x-test-login", id);
    }
    builder
        .body(Full::new(Bytes::from(body)))
        .expect("build Live action request")
}
