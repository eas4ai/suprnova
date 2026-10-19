//! The session records a GET as the previous URL only when a route answered
//! it, as Laravel's `StartSession::storeCurrentUrl` asks for
//! `$request->route()`. A page's own script and style requests, served by
//! `StaticFiles`, never reach Laravel at all, and an AJAX request is skipped
//! there too, so neither may become the URL `Redirect::back()` returns to.
//!
//! Each test drives the real dispatch (`handle_request`) with the session
//! middleware registered globally, as an application registers it.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    Crypt, EncryptionKey, FrameworkError, HttpResponse, Middleware, MiddlewareRegistry, Next,
    Request, Response, Router, StaticFiles, async_trait, fallback,
};

/// The id of the one session every request in a test carries.
fn session_id() -> String {
    "p".repeat(40)
}

/// An in-memory store holding the rows the middleware wrote.
#[derive(Default)]
struct Store {
    rows: Mutex<HashMap<String, SessionData>>,
}

impl Store {
    /// A store holding one stored, empty session.
    fn seeded() -> Arc<Self> {
        let mut session = SessionData::new(session_id(), "t".repeat(40));
        session.mark_clean();
        Arc::new(Self {
            rows: Mutex::new(HashMap::from([(session_id(), session)])),
        })
    }

    /// The previous URL the session holds now.
    fn previous_url(&self) -> Option<String> {
        self.rows
            .lock()
            .expect("store lock")
            .get(&session_id())
            .and_then(SessionData::previous_url)
    }
}

#[async_trait]
impl SessionStore for Store {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.rows.lock().expect("store lock").get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        let mut session = session.clone();
        session.mark_clean();
        self.rows
            .lock()
            .expect("store lock")
            .insert(session.id.clone(), session);
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.rows.lock().expect("store lock").remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// A global middleware that answers `/robots.txt` itself, before any route
/// is consulted, as a middleware serving a fixed file does.
struct RobotsTxt;

#[async_trait]
impl Middleware for RobotsTxt {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.path() == "/robots.txt" {
            return Ok(HttpResponse::text("User-agent: *"));
        }
        next(request).await
    }
}

/// The application's own pages.
fn pages() -> Router {
    Router::new()
        .get("/page", |_request: Request| async {
            suprnova::text("page")
        })
        .get("/second", |_request: Request| async {
            suprnova::text("second page")
        })
        .get("/fragment", |_request: Request| async {
            suprnova::text("a fragment of a page")
        })
        .into()
}

/// The pages, with `public` served by `StaticFiles` as the fallback route,
/// as a new project registers it.
fn pages_with_static_files(public: &Path) -> Router {
    fallback!(StaticFiles::from_dir(public).handler()).register(pages())
}

/// The pages, with an app shell as the fallback route.
fn pages_with_app_shell() -> Router {
    fallback!(|_request: Request| async { suprnova::text("app shell") }).register(pages())
}

/// The session middleware over `store`, registered globally, and the
/// middleware that answers `/robots.txt` inside it.
fn registry(store: Arc<Store>) -> Arc<MiddlewareRegistry> {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    Arc::new(
        MiddlewareRegistry::new()
            .append(SessionMiddleware::with_store(config, store))
            .append(RobotsTxt),
    )
}

/// The `Cookie` header that names the seeded session.
fn session_cookie() -> String {
    let config = SessionConfig::default();
    suprnova::http::cookie::Cookie::encrypted(&config.cookie_name, session_id())
        .expect("encrypted cookie")
        .to_header_value()
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned()
}

/// Send `GET path` with `headers` and the session cookie through the real
/// dispatch, and answer its status.
async fn get(
    router: &Arc<Router>,
    registry: &Arc<MiddlewareRegistry>,
    path: &str,
    headers: &[(&str, &str)],
) -> u16 {
    let router = router.clone();
    let registry = registry.clone();
    let (client, server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let service = service_fn(move |request| {
            let router = router.clone();
            let registry = registry.clone();
            async move { Ok::<_, Infallible>(suprnova::handle_request(router, registry, request).await) }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server), service)
            .await;
    });
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client))
            .await
            .expect("client handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("host", "localhost")
        .header("cookie", session_cookie());
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        sender.send_request(request.body(Full::new(Bytes::new())).expect("request")),
    )
    .await
    .expect("response timeout")
    .expect("response");
    let status = response.status().as_u16();
    response.into_body().collect().await.expect("response body");
    status
}

/// A page's script, served by `StaticFiles` after the page loaded, leaves
/// the previous URL at the page.
#[tokio::test]
async fn a_static_asset_after_a_page_leaves_the_previous_url_at_the_page() {
    let public = tempfile::tempdir().expect("public directory");
    std::fs::create_dir(public.path().join("assets")).expect("assets directory");
    std::fs::write(public.path().join("assets/app.js"), "console.log('app')").expect("script");
    let store = Store::seeded();
    let registry = registry(store.clone());
    let router = Arc::new(pages_with_static_files(public.path()));

    assert_eq!(get(&router, &registry, "/page", &[]).await, 200);
    assert_eq!(store.previous_url().as_deref(), Some("/page"));

    assert_eq!(
        get(&router, &registry, "/assets/app.js", &[]).await,
        200,
        "the script is served"
    );
    assert_eq!(
        store.previous_url().as_deref(),
        Some("/page"),
        "a static file is not a page to come back to"
    );
}

/// An AJAX GET to a route leaves the previous URL unchanged, as Laravel's
/// `$request->ajax()` skips it.
#[tokio::test]
async fn an_ajax_get_leaves_the_previous_url_unchanged() {
    let store = Store::seeded();
    let registry = registry(store.clone());
    let router = Arc::new(pages());

    assert_eq!(get(&router, &registry, "/page", &[]).await, 200);
    assert_eq!(
        get(
            &router,
            &registry,
            "/fragment",
            &[("x-requested-with", "XMLHttpRequest")]
        )
        .await,
        200
    );
    assert_eq!(store.previous_url().as_deref(), Some("/page"));
}

/// A GET to a path no route matches, with no fallback route, is a 404 and
/// leaves the previous URL unchanged.
#[tokio::test]
async fn a_get_no_route_matches_leaves_the_previous_url_unchanged() {
    let store = Store::seeded();
    let registry = registry(store.clone());
    let router = Arc::new(pages());

    assert_eq!(get(&router, &registry, "/page", &[]).await, 200);
    assert_eq!(get(&router, &registry, "/missing", &[]).await, 404);
    assert_eq!(store.previous_url().as_deref(), Some("/page"));
}

/// A GET that a global middleware answers before any route matched leaves
/// the previous URL unchanged, whatever the status: no route answered it.
#[tokio::test]
async fn a_get_answered_without_a_route_leaves_the_previous_url_unchanged() {
    let store = Store::seeded();
    let registry = registry(store.clone());
    let router = Arc::new(pages());

    assert_eq!(get(&router, &registry, "/page", &[]).await, 200);
    assert_eq!(
        get(&router, &registry, "/robots.txt", &[]).await,
        200,
        "the middleware answers the path"
    );
    assert_eq!(store.previous_url().as_deref(), Some("/page"));
}

/// A GET a route answered still becomes the previous URL.
#[tokio::test]
async fn a_routed_page_get_still_records_the_previous_url() {
    let store = Store::seeded();
    let registry = registry(store.clone());
    let router = Arc::new(pages());

    assert_eq!(get(&router, &registry, "/page", &[]).await, 200);
    assert_eq!(get(&router, &registry, "/second", &[]).await, 200);
    assert_eq!(store.previous_url().as_deref(), Some("/second"));
}

/// A page the fallback route answers still becomes the previous URL:
/// Laravel's fallback is a route, so `$request->route()` holds it.
#[tokio::test]
async fn a_page_the_fallback_route_answers_still_records_the_previous_url() {
    let store = Store::seeded();
    let registry = registry(store.clone());
    let router = Arc::new(pages_with_app_shell());

    assert_eq!(get(&router, &registry, "/page", &[]).await, 200);
    assert_eq!(
        get(&router, &registry, "/dashboard/reports", &[]).await,
        200
    );
    assert_eq!(store.previous_url().as_deref(), Some("/dashboard/reports"));
}
