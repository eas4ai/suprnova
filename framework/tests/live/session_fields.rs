//! `#[session]` component fields through the production stack: they come
//! from the visitor's session on every request that runs the component for
//! that visitor, and an accepted action writes them back.
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
    ACTION_PATH, DogfoodDocument, Tenantless, decoded_snapshot, dispatch, get,
    production_middleware, public_live_guard,
};
use serde_json::{Value, json};
use suprnova::container::testing::TestContainer;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{
    CanonicalValue, LiveBootstrapOptions, LiveComponent, LiveDocument, LiveMount, LiveRegistry,
    LiveTenantMiddleware, MountFlags, live,
};
use suprnova::view::{AssetSet, DocumentResponseIntent, ViewName};
use suprnova::{
    App, AuthMiddleware, Crypt, EncryptionKey, FrameworkError, HttpResponse, MiddlewareRegistry,
    Request, Router, StatusCode,
};

const PUBLIC_PATH: &str = "/session-counter";
const PUBLIC_KEY: &str = "session-counter-public";
const PRIVATE_PATH: &str = "/session-counter/private";
const PRIVATE_KEY: &str = "session-counter-private";

/// Counts a visitor's visits in their session. `count` travels in the
/// snapshot; `visits` never does.
#[derive(LiveComponent)]
#[live(
    name = "tests.session-counter",
    view = "live/tests/session-counter.html"
)]
pub struct SessionCounter {
    #[public]
    count: u64,
    #[session]
    visits: u64,
}

#[live]
impl SessionCounter {
    #[action]
    pub fn visit(&mut self) {
        self.visits += 1;
        self.count += 1;
    }
}

fn fixture() {
    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<SessionCounter>()
            .expect("register session counter")
            .build(),
    );
}

async fn render(
    request: Request,
    mount: LiveMount<SessionCounter>,
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

/// A public seed page every visitor shares, and an identity-bound page.
fn router() -> Router {
    let public = LiveMount::<SessionCounter>::public_seed(PUBLIC_PATH, "counter", PUBLIC_KEY)
        .expect("declare public mount");
    let private = LiveMount::<SessionCounter>::identity_bound(PRIVATE_PATH, "counter", PRIVATE_KEY)
        .expect("declare private mount");
    let public_handler = public.clone();
    let private_handler = private.clone();
    let router: Router = Router::new()
        .get(PUBLIC_PATH, move |request: Request| {
            let mount = public_handler.clone();
            async move { render(request, mount).await }
        })
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

/// The signed-in visitor's request for `path`.
async fn page(
    router: &Arc<Router>,
    middleware: &Arc<MiddlewareRegistry>,
    path: &str,
    cookie: &str,
) -> (String, String, Value) {
    let mut request = get(path);
    request
        .headers_mut()
        .insert("x-test-login", "user-7".parse().expect("header"));
    if !cookie.is_empty() {
        request
            .headers_mut()
            .insert("cookie", cookie.parse().expect("cookie"));
    }
    let (status, headers, body) = dispatch(router.clone(), middleware.clone(), request).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let html = String::from_utf8(body.to_vec()).expect("document UTF-8");
    let snapshot = decoded_snapshot(&body);
    (html, next_cookie(&headers, cookie), snapshot)
}

/// Run `visit` on the identity-bound instance `snapshot` describes, and
/// return the re-rendered island and the session cookie.
async fn visit(
    router: &Arc<Router>,
    middleware: &Arc<MiddlewareRegistry>,
    snapshot: Value,
    cookie: &str,
    idempotency_key: &str,
) -> (String, String) {
    let revision = match &snapshot["body"]["revision"] {
        Value::String(revision) => revision.clone(),
        Value::Number(revision) => revision.to_string(),
        other => panic!("instance snapshot carries no revision: {other}"),
    };
    let body = serde_json::to_vec(&json!({
        "base_revision": revision,
        "child_parameters": null,
        "component": "tests.session-counter",
        "correlation_id": "MDEyMzQ1Njc4OTo7PD0-Pw",
        "extensions": {"x_suprnova_live_document_key_v1": PRIVATE_KEY},
        "idempotency_key": idempotency_key,
        "model_proposals": {},
        "operations": [{"arguments": {}, "kind": "invoke_action", "name": "visit"}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": {"envelope": snapshot, "kind": "instance"},
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

#[tokio::test]
#[serial_test::serial]
async fn session_fields_load_from_and_persist_to_the_visitors_session() {
    let _container = TestContainer::fake();
    fixture();
    let router = Arc::new(router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    // Sign in once, so the pages that follow bind the session that
    // survives the framework's fixation rotation.
    let (_, cookie, _) = page(&router, &middleware, PUBLIC_PATH, "").await;

    let (html, cookie, snapshot) = page(&router, &middleware, PRIVATE_PATH, &cookie).await;
    assert!(html.contains(r#"<span id="visits">0</span>"#), "{html}");

    let (island, cookie) = visit(
        &router,
        &middleware,
        snapshot,
        &cookie,
        "QEFCQ0RFRkdISUpLTE1OTw",
    )
    .await;
    assert!(island.contains(r#"<span id="visits">1</span>"#), "{island}");

    // A new mount reads the session the accepted action wrote.
    let (html, cookie, snapshot) = page(&router, &middleware, PRIVATE_PATH, &cookie).await;
    assert!(
        html.contains(r#"<span id="visits">1</span>"#),
        "a new mount must read the visit count from the session: {html}"
    );

    // So does the next action's reconstruction.
    let (island, cookie) = visit(
        &router,
        &middleware,
        snapshot,
        &cookie,
        "QUFCQ0RFRkdISUpLTE1OTw",
    )
    .await;
    assert!(
        island.contains(r#"<span id="visits">2</span>"#),
        "an action must start from the session's value: {island}"
    );

    // A public seed is shared between visitors, so it never reads one
    // visitor's session.
    let (html, _, _) = page(&router, &middleware, PUBLIC_PATH, &cookie).await;
    assert!(
        html.contains(r#"<span id="visits">0</span>"#),
        "a public seed renders the default: {html}"
    );
}
