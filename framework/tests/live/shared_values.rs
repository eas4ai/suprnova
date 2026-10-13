//! Shared view data in Live component templates (PAR-165): a value the
//! application shares with `View::share` reaches every component render, on
//! a public seed, an identity-bound mount and an action's re-render, through
//! Askama's runtime values. A value shared with `View::share_for_request`
//! never does: a component's view is rendered again on its action requests,
//! and a public seed is shared between visitors.
// Its own test binary: it binds the process-global Live runtime and mount
// catalog, which a second binding in one process rejects.
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/live_dogfood_support/mod.rs"]
mod live_dogfood_support;

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use live_dogfood_support::{
    ACTION_PATH, LoginHeader, MemorySessionStore, Tenantless, decoded_snapshot, dispatch, get,
    public_live_guard,
};
use serde_json::{Value, json};
use suprnova::container::testing::TestContainer;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{
    CanonicalValue, ComponentContract, LiveBootstrapOptions, LiveComponent, LiveDocument,
    LiveMount, LiveRegistry, LiveTenantMiddleware, MountFlags, live,
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, AuthMiddleware, Crypt, CsrfMiddleware, EncryptionKey, FrameworkError, HttpResponse,
    Middleware, MiddlewareRegistry, Next, Request, Response, Router, SessionConfig,
    SessionMiddleware, StatusCode, View, async_trait,
};

pub mod filters {
    pub use suprnova::view::filters::trusted_html;
}

const PUBLIC_PATH: &str = "/shared-badge";
const PUBLIC_KEY: &str = "shared-badge-public";
const PRIVATE_PATH: &str = "/shared-badge/private";
const PRIVATE_KEY: &str = "shared-badge-private";
const STRICT_PATH: &str = "/strict-viewer";
const STRICT_KEY: &str = "strict-viewer-public";

/// Shows the application's `app_name` and the request's `viewer`, each with
/// a fallback when the render cannot read it.
#[derive(LiveComponent)]
#[live(name = "tests.shared-badge", view = "live/tests/shared-badge.html")]
pub struct SharedBadge {
    #[public]
    count: u64,
}

#[live]
impl SharedBadge {
    #[action]
    pub fn bump(&mut self) {
        self.count += 1;
    }
}

/// Requires the `viewer` value: its render fails when it cannot read one.
#[derive(LiveComponent)]
#[live(name = "tests.strict-viewer", view = "live/tests/strict-viewer.html")]
pub struct StrictViewer {
    #[public]
    count: u64,
}

#[live]
impl StrictViewer {}

/// A document that shows the request's `viewer` itself, so a test can see
/// the request shared one while the island beside it renders.
#[suprnova::view(path = "live/shared-values-document.html")]
struct SharedValuesDocument<'a> {
    bootstrap: &'a TrustedHtml,
    island: &'a TrustedHtml,
}

/// Shares the `x-viewer` header for the request, on every route the
/// registry runs, the Live action route among them.
struct ShareViewer;

#[async_trait]
impl Middleware for ShareViewer {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(viewer) = request.header("x-viewer") {
            View::share_for_request("viewer", viewer.to_owned())
                .expect("a request runs in a container scope");
        }
        next(request).await
    }
}

fn fixture() {
    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<SharedBadge>()
            .expect("register shared badge")
            .register::<StrictViewer>()
            .expect("register strict viewer")
            .build(),
    );
}

/// The production-shaped global stack with the viewer share at its end.
fn middleware() -> Arc<MiddlewareRegistry> {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    Arc::new(
        MiddlewareRegistry::new()
            .append(SessionMiddleware::with_store(
                config,
                Arc::new(MemorySessionStore::default()),
            ))
            .append(CsrfMiddleware::new())
            .append(LoginHeader)
            .append(ShareViewer),
    )
}

async fn render<C: ComponentContract>(
    request: Request,
    mount: LiveMount<C>,
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
                ViewName::parse("live/shared-values-document.html")
                    .map_err(|_| FrameworkError::internal("view identity"))?,
                &SharedValuesDocument {
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

/// A public seed and an identity-bound copy of the badge, and a public seed
/// of the strict viewer.
fn router() -> Router {
    let public = LiveMount::<SharedBadge>::public_seed(PUBLIC_PATH, "badge", PUBLIC_KEY)
        .expect("declare public mount");
    let private = LiveMount::<SharedBadge>::identity_bound(PRIVATE_PATH, "badge", PRIVATE_KEY)
        .expect("declare private mount");
    let strict = LiveMount::<StrictViewer>::public_seed(STRICT_PATH, "viewer", STRICT_KEY)
        .expect("declare strict mount");
    let public_handler = public.clone();
    let private_handler = private.clone();
    let strict_handler = strict.clone();
    let router: Router = Router::new()
        .get(PUBLIC_PATH, move |request: Request| {
            let mount = public_handler.clone();
            async move { render(request, mount).await }
        })
        .get(STRICT_PATH, move |request: Request| {
            let mount = strict_handler.clone();
            async move { render(request, mount).await }
        })
        // The identity-bound route comes last: the route middleware below
        // applies to it.
        .get(PRIVATE_PATH, move |request: Request| {
            let mount = private_handler.clone();
            async move { render(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .into();
    router
        .try_live_with(public_live_guard)
        .expect("install Live routes")
        .try_live_mount(&public)
        .expect("register public mount")
        .try_live_mount(&private)
        .expect("register private mount")
        .try_live_mount(&strict)
        .expect("register strict mount")
}

/// The session cookie a response sets, or `current` when it sets none.
fn next_cookie(headers: &hyper::HeaderMap, current: &str) -> String {
    headers
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            let pair = value.to_str().ok()?.split(';').next()?.to_owned();
            (!pair.starts_with("XSRF-TOKEN=")).then_some(pair)
        })
        .unwrap_or_else(|| current.to_owned())
}

/// The signed-in visitor's request for `path`, sharing `viewer` for it.
fn page_request(path: &str, cookie: &str, viewer: &str) -> hyper::Request<Full<Bytes>> {
    let mut request = get(path);
    let headers = request.headers_mut();
    headers.insert("x-test-login", "user-7".parse().expect("header"));
    if !viewer.is_empty() {
        headers.insert("x-viewer", viewer.parse().expect("header"));
    }
    if !cookie.is_empty() {
        headers.insert("cookie", cookie.parse().expect("cookie"));
    }
    request
}

/// The page at `path`: its HTML, the next session cookie and its island's
/// snapshot.
async fn page(
    router: &Arc<Router>,
    middleware: &Arc<MiddlewareRegistry>,
    path: &str,
    cookie: &str,
    viewer: &str,
) -> (String, String, Value) {
    let (status, headers, body) = dispatch(
        router.clone(),
        middleware.clone(),
        page_request(path, cookie, viewer),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{path}: {}",
        String::from_utf8_lossy(&body)
    );
    let html = String::from_utf8(body.to_vec()).expect("document UTF-8");
    let snapshot = decoded_snapshot(&body);
    (html, next_cookie(&headers, cookie), snapshot)
}

/// Run `bump` on the island `snapshot` describes, sharing `viewer` for the
/// action request, and return the re-rendered island and the session cookie.
async fn bump(
    router: &Arc<Router>,
    middleware: &Arc<MiddlewareRegistry>,
    snapshot: Value,
    seed: bool,
    cookie: &str,
    viewer: &str,
    idempotency_key: &str,
) -> (String, String) {
    let (document_key, base_revision, snapshot) = if seed {
        (
            PUBLIC_KEY,
            "0".to_owned(),
            json!({
                "browser_nonce": "ICEiIyQlJicoKSorLC0uLw",
                "envelope": snapshot,
                "kind": "seed_promotion",
            }),
        )
    } else {
        let revision = match &snapshot["body"]["revision"] {
            Value::String(revision) => revision.clone(),
            Value::Number(revision) => revision.to_string(),
            other => panic!("instance snapshot carries no revision: {other}"),
        };
        (
            PRIVATE_KEY,
            revision,
            json!({"envelope": snapshot, "kind": "instance"}),
        )
    };
    let body = serde_json::to_vec(&json!({
        "base_revision": base_revision,
        "child_parameters": null,
        "component": "tests.shared-badge",
        "correlation_id": "MDEyMzQ1Njc4OTo7PD0-Pw",
        "extensions": {"x_suprnova_live_document_key_v1": document_key},
        "idempotency_key": idempotency_key,
        "model_proposals": {},
        "operations": [{"arguments": {}, "kind": "invoke_action", "name": "bump"}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": snapshot,
        "snapshot_schema_version": 1,
    }))
    .expect("encode Live action request");
    let request = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(ACTION_PATH)
        .header("host", "127.0.0.1")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=2",
        )
        .header("cookie", cookie)
        .header("sec-fetch-site", "same-origin")
        .header("x-test-login", "user-7")
        .header("x-viewer", viewer)
        .body(Full::new(Bytes::from(body)))
        .expect("build Live action request");
    let (status, headers, body) = dispatch(router.clone(), middleware.clone(), request).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let accepted: Value = serde_json::from_slice(&body).expect("accepted action JSON");
    assert_eq!(accepted["outcome"], "accepted", "{accepted}");
    let html = accepted["render"]["html"]
        .as_str()
        .expect("the island re-renders")
        .to_owned();
    (html, next_cookie(&headers, cookie))
}

const APP: &str = r#"<span id="app">Acme</span>"#;
const NO_VIEWER: &str = r#"<span id="viewer">nobody</span>"#;

#[tokio::test]
#[serial_test::serial]
async fn application_shared_values_reach_every_component_render_and_request_values_none() {
    let _container = TestContainer::fake();
    fixture();
    View::share("app_name", String::from("Acme"));
    let router = Arc::new(router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = middleware();

    // The public seed every visitor shares. The document beside it reads
    // the request's viewer; the island reads the application's name only.
    let (html, cookie, seed) = page(&router, &middleware, PUBLIC_PATH, "", "alice").await;
    assert!(
        html.contains(r#"<p id="document-viewer">alice</p>"#),
        "the document reads the request's value: {html}"
    );
    assert!(
        html.contains(APP),
        "a public seed reads the application's value: {html}"
    );
    assert!(
        html.contains(NO_VIEWER),
        "a public seed never reads a request's value: {html}"
    );

    // The identity-bound mount.
    let (html, cookie, instance) = page(&router, &middleware, PRIVATE_PATH, &cookie, "").await;
    assert!(
        html.contains(APP),
        "an identity-bound mount reads the application's value: {html}"
    );
    assert!(
        html.contains(NO_VIEWER),
        "an identity-bound mount never reads a request's value: {html}"
    );

    // An action's re-render, on the instance and on a promoted seed.
    let (island, cookie) = bump(
        &router,
        &middleware,
        instance,
        false,
        &cookie,
        "bob",
        "QEFCQ0RFRkdISUpLTE1OTw",
    )
    .await;
    assert!(island.contains(">1</button>"), "{island}");
    assert!(
        island.contains(APP),
        "an action's re-render reads the application's value: {island}"
    );
    assert!(
        island.contains(NO_VIEWER),
        "an action's re-render never reads a request's value: {island}"
    );

    let (island, _) = bump(
        &router,
        &middleware,
        seed,
        true,
        &cookie,
        "bob",
        "QUFCQ0RFRkdISUpLTE1OTw",
    )
    .await;
    assert!(island.contains(">1</button>"), "{island}");
    assert!(
        island.contains(APP),
        "a promoted seed's re-render reads the application's value: {island}"
    );
    assert!(
        island.contains(NO_VIEWER),
        "a promoted seed's re-render never reads a request's value: {island}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_component_that_requires_a_request_value_fails_rather_than_reading_it() {
    let _container = TestContainer::fake();
    fixture();
    let router = Arc::new(router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = middleware();

    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        page_request(STRICT_PATH, "", "alice"),
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        !body.contains("alice"),
        "the request's value leaked: {body}"
    );

    // The same key shared by the application renders.
    View::share("viewer", String::from("everyone"));
    let (html, _, _) = page(&router, &middleware, STRICT_PATH, "", "alice").await;
    assert!(
        html.contains(r#"<span id="viewer">everyone</span>"#),
        "the application's value renders, never the request's: {html}"
    );
}
