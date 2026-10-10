//! Shared view data (PAR-165): `View::share` makes a value available to
//! every server-rendered view, read in a template through Askama's runtime
//! values, as Laravel's `View::share` merges shared data under each view's
//! own; `View::share_for_request` does the same for one request, winning
//! over the application's values; `View::shared` answers what a render
//! sees. The application's values belong to the container, so
//! `TestContainer` isolates them, and a request's values never reach
//! another request, through the render cache neither: not through a
//! template's `value` filter, and not through `View::shared`.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::render_cache::collector::{self, Collector};
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::registry::GroupPolicy;
use suprnova::render_cache::{
    CoordinatorConfig, FreshnessPolicy, L1Config, RenderCache, RenderCachePolicy,
    RepresentationClass,
};
use suprnova::testing::{TestClient, TestContainer};
use suprnova::view::{
    AssetSet, DocumentResponseIntent, RenderLimits, ViewName, ViewRenderer, document_response,
};
use suprnova::{
    App, ConnectionTrait, FrameworkError, HttpResponse, Middleware, MiddlewareRegistry, Next,
    Request, Response, Router, View,
};

#[suprnova::view(path = "live/shared-values.html")]
struct SharedPage<'a> {
    title: &'a str,
}

/// Render the page as a complete document, the way a route handler does.
fn render(title: &str) -> Result<HttpResponse, FrameworkError> {
    let view = ViewName::parse("live/shared-values.html").expect("a view name");
    let rendered = ViewRenderer::new(RenderLimits::standard())
        .and_then(|renderer| {
            renderer.render_document(
                view,
                &SharedPage { title },
                DocumentResponseIntent::html(suprnova::StatusCode::OK)?,
                AssetSet::empty(),
                Vec::new(),
            )
        })
        .map_err(|error| FrameworkError::internal(format!("render: {error:?}")))?;
    document_response(rendered)
        .map_err(|error| FrameworkError::internal(format!("respond: {error}")))
}

fn body(response: &HttpResponse) -> String {
    String::from_utf8(response.body().to_vec()).expect("UTF-8")
}

/// The handler every route here runs: it shares the `X-Viewer` header for
/// the request, when there is one, and renders the page.
async fn page(request: Request) -> Response {
    RENDERS.fetch_add(1, Ordering::SeqCst);
    if let Some(viewer) = request.header("X-Viewer") {
        View::share_for_request("viewer", viewer.to_owned())?;
        let seen = View::shared::<String>("viewer").map(|value| value.as_str().to_owned());
        assert_eq!(
            seen.as_deref(),
            Some(viewer),
            "a render sees the request's value"
        );
    }
    Ok(render("Shared")?)
}

static RENDERS: AtomicUsize = AtomicUsize::new(0);

fn page_router() -> Router {
    Router::new().get("/page", page).into()
}

// ---- View::share ------------------------------------------------------------

#[test]
fn a_shared_value_reaches_every_render() {
    let _container = TestContainer::fake();
    let before = body(&render("Before").expect("renders without shared values"));
    assert!(before.contains("no app name"), "{before}");

    View::share("app_name", "Acme");
    assert_eq!(View::shared::<&str>("app_name").as_deref(), Some(&"Acme"));
    for title in ["One", "Two"] {
        let page = body(&render(title).expect("renders"));
        assert!(page.contains("<p id=\"app\">Acme</p>"), "{page}");
        assert!(
            page.contains(title),
            "the view keeps its own fields: {page}"
        );
    }
}

#[test]
fn a_later_share_replaces_the_value_and_values_are_escaped() {
    let _container = TestContainer::fake();
    View::share("app_name", "Acme");
    View::share("app_name", "<b>Acme & Co</b>");
    let page = body(&render("Page").expect("renders"));
    assert!(!page.contains("<b>Acme"), "{page}");
    assert!(page.contains("Acme &#38; Co"), "{page}");
}

#[test]
fn shared_answers_none_for_a_missing_key_or_another_type() {
    let _container = TestContainer::fake();
    View::share("count", 3_u32);
    assert_eq!(View::shared::<u32>("count").as_deref(), Some(&3));
    assert!(View::shared::<String>("count").is_none());
    assert!(View::shared::<u32>("missing").is_none());
}

#[test]
fn a_value_shared_under_one_test_container_never_shows_under_another() {
    {
        let _first = TestContainer::fake();
        View::share("app_name", "First container");
        assert!(body(&render("First").expect("renders")).contains("First container"));
    }
    let _second = TestContainer::fake();
    assert!(View::shared::<&str>("app_name").is_none());
    let page = body(&render("Second").expect("renders"));
    assert!(!page.contains("First container"), "{page}");
    assert!(page.contains("no app name"), "{page}");
}

// ---- View::share_for_request -------------------------------------------------

#[test]
fn share_for_request_outside_a_request_is_an_error() {
    let error =
        View::share_for_request("viewer", "alice".to_owned()).expect_err("no request is running");
    assert!(error.to_string().contains("scope"), "{error}");
}

#[tokio::test]
async fn a_request_value_wins_over_the_application_value() {
    let _container = TestContainer::fake();
    View::share("viewer", "everyone".to_owned());
    let client = TestClient::new(page_router(), MiddlewareRegistry::new());

    let response = client.get("/page").header("X-Viewer", "alice").send().await;
    response.assert_ok();
    assert!(
        response.body_text().contains("<p id=\"viewer\">alice</p>"),
        "{}",
        response.body_text()
    );

    let response = client.get("/page").send().await;
    assert!(
        response
            .body_text()
            .contains("<p id=\"viewer\">everyone</p>"),
        "{}",
        response.body_text()
    );
    assert_eq!(
        View::shared::<String>("viewer")
            .as_deref()
            .map(String::as_str),
        Some("everyone"),
        "outside the request the application's value is the answer"
    );
}

/// What `View::shared` answers for `key` as a `T` inside a render cache
/// collector scope, and whether the render cache could still store the
/// page afterwards.
async fn shared_under_the_render_cache<T: Any + Clone + Send + Sync>(
    key: &str,
) -> (Option<T>, bool) {
    Collector::scope(async {
        let value = View::shared::<T>(key).map(|value| T::clone(&value));
        let report = collector::current_report().expect("a collector scope");
        (value, report.storable().is_some())
    })
    .await
}

#[tokio::test]
async fn shared_stops_the_render_cache_storing_a_page_only_for_a_request_value() {
    let _container = TestContainer::fake();
    View::share("app_name", "Acme".to_owned());
    View::share("viewer", "everyone".to_owned());
    App::run_scoped(async {
        assert_eq!(
            shared_under_the_render_cache::<String>("viewer").await,
            (Some("everyone".to_owned()), true),
            "the application's value is the same for every request: the page can be stored"
        );

        View::share_for_request("viewer", "alice".to_owned()).expect("a container scope");
        assert_eq!(
            shared_under_the_render_cache::<String>("viewer").await,
            (Some("alice".to_owned()), false),
            "the request's value names no cache key: the page must not be stored"
        );
        assert_eq!(
            shared_under_the_render_cache::<String>("app_name").await,
            (Some("Acme".to_owned()), true),
            "a key only the application shared still answers without a mark"
        );

        View::share_for_request("viewer", 7_u32).expect("a container scope");
        assert_eq!(
            shared_under_the_render_cache::<String>("viewer").await,
            (None, false),
            "a request value of another type still decides the answer"
        );
    })
    .await;
}

#[tokio::test]
async fn a_request_value_never_reaches_another_request() {
    let _container = TestContainer::fake();
    let client = TestClient::new(page_router(), MiddlewareRegistry::new());

    let (alice, other) = tokio::join!(
        client.get("/page").header("X-Viewer", "alice").send(),
        client.get("/page").send(),
    );
    assert!(alice.body_text().contains("<p id=\"viewer\">alice</p>"));
    assert!(
        other.body_text().contains("<p id=\"viewer\">nobody</p>"),
        "{}",
        other.body_text()
    );
    let after = client.get("/page").send().await;
    assert!(
        after.body_text().contains("<p id=\"viewer\">nobody</p>"),
        "{}",
        after.body_text()
    );
}

// ---- The render cache in front of the route ----------------------------------

struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// A SQLite database with the RenderCache migration, bound in the test
/// container.
async fn render_cache_database() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("a database directory");
    let config = suprnova::database::DatabaseConfig::builder()
        .url(format!(
            "sqlite://{}",
            tempdir.path().join("render-cache.sqlite3").display()
        ))
        .max_connections(4)
        .min_connections(1)
        .logging(false)
        .build();
    let conn = suprnova::database::DbConnection::connect(&config)
        .await
        .expect("connect SQLite");
    conn.inner()
        .execute_unprepared("PRAGMA journal_mode=WAL")
        .await
        .expect("WAL journaling");
    Migrator::up(conn.inner(), None)
        .await
        .expect("the RenderCache migration");
    TestContainer::singleton(conn);
    tempdir
}

/// A client for `router` with the render cache in front of `path`, under a
/// public shared policy: a stored page is served to every visitor.
async fn render_cached_client(router: Router, path: &str) -> TestClient {
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("a freshness"))
        .build()
        .expect("a policy");
    let router = router
        .try_render_cache(path, GroupPolicy::from(policy))
        .expect("attach the policy");
    let mut config = RenderCacheConfig::from_env().expect("a RenderCache configuration");
    config.enabled = true;
    config.l1 = L1Config::Disabled;
    config.coordinator = CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };
    let router = RenderCache::install(router, config)
        .await
        .expect("install the RenderCache");
    TestClient::new(router, MiddlewareRegistry::from_global())
}

#[tokio::test]
async fn a_request_value_never_reaches_another_request_through_the_render_cache() {
    if crate::own_process_async::delegate(
        module_path!(),
        "a_request_value_never_reaches_another_request_through_the_render_cache",
    )
    .await
    {
        return;
    }
    // The render cache derives its signing keys from the application key.
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    App::init();
    suprnova::middleware::clear_global_middleware_for_test();
    let _container = TestContainer::fake();
    let _database = render_cache_database().await;

    let client = render_cached_client(page_router(), "/page").await;
    RENDERS.store(0, Ordering::SeqCst);

    // The render read alice's request value, so it is not stored.
    let alice = client.get("/page").header("X-Viewer", "alice").send().await;
    assert!(alice.body_text().contains("<p id=\"viewer\">alice</p>"));
    assert_eq!(RENDERS.load(Ordering::SeqCst), 1);

    // The next request renders its own page, without alice's value.
    let other = client.get("/page").send().await;
    assert!(
        other.body_text().contains("<p id=\"viewer\">nobody</p>"),
        "{}",
        other.body_text()
    );
    assert_eq!(RENDERS.load(Ordering::SeqCst), 2);

    // That render read no request value, so it is stored and served: the
    // cache is in front of the route.
    let cached = client.get("/page").send().await;
    assert!(cached.body_text().contains("<p id=\"viewer\">nobody</p>"));
    assert_eq!(
        RENDERS.load(Ordering::SeqCst),
        2,
        "the third request is a hit"
    );
}

/// A middleware that shares the `X-Viewer` header for the request, the way
/// an application shares the signed-in user with its views.
struct SharesViewer;

#[async_trait::async_trait]
impl Middleware for SharesViewer {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(viewer) = request.header("X-Viewer") {
            View::share_for_request("viewer", viewer.to_owned())?;
        }
        next(request).await
    }
}

/// A handler that forms its body from `View::shared` alone. No template
/// reads the shared value, so the getter is the only read the render cache
/// can see.
async fn viewer_page(_request: Request) -> Response {
    VIEWER_RENDERS.fetch_add(1, Ordering::SeqCst);
    let viewer = View::shared::<String>("viewer")
        .map_or_else(|| "nobody".to_owned(), |viewer| viewer.as_str().to_owned());
    Ok(HttpResponse::html(format!("<p id=\"viewer\">{viewer}</p>")))
}

static VIEWER_RENDERS: AtomicUsize = AtomicUsize::new(0);

fn viewer_router() -> Router {
    Router::new()
        .get("/viewer", viewer_page)
        .middleware(SharesViewer)
        .into()
}

#[tokio::test]
async fn a_request_value_read_through_shared_never_reaches_another_request_through_the_render_cache()
 {
    if crate::own_process_async::delegate(
        module_path!(),
        "a_request_value_read_through_shared_never_reaches_another_request_through_the_render_cache",
    )
    .await
    {
        return;
    }
    // The render cache derives its signing keys from the application key.
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    App::init();
    suprnova::middleware::clear_global_middleware_for_test();
    let _container = TestContainer::fake();
    let _database = render_cache_database().await;
    View::share("viewer", "everyone".to_owned());

    let client = render_cached_client(viewer_router(), "/viewer").await;
    VIEWER_RENDERS.store(0, Ordering::SeqCst);

    // The body came from alice's request value, so the page is not stored.
    let alice = client
        .get("/viewer")
        .header("X-Viewer", "alice")
        .send()
        .await;
    assert!(
        alice.body_text().contains("<p id=\"viewer\">alice</p>"),
        "{}",
        alice.body_text()
    );
    assert_eq!(VIEWER_RENDERS.load(Ordering::SeqCst), 1);

    // The next visitor gets a render of their own, never alice's page.
    let bob = client.get("/viewer").header("X-Viewer", "bob").send().await;
    assert!(
        bob.body_text().contains("<p id=\"viewer\">bob</p>"),
        "{}",
        bob.body_text()
    );
    assert_eq!(VIEWER_RENDERS.load(Ordering::SeqCst), 2);

    // A visitor without a value of their own reads the application's, which
    // is the same for everyone: that page is stored and served, so the cache
    // is in front of the route.
    let everyone = client.get("/viewer").send().await;
    assert!(
        everyone
            .body_text()
            .contains("<p id=\"viewer\">everyone</p>"),
        "{}",
        everyone.body_text()
    );
    assert_eq!(VIEWER_RENDERS.load(Ordering::SeqCst), 3);
    let cached = client.get("/viewer").send().await;
    assert!(
        cached.body_text().contains("<p id=\"viewer\">everyone</p>"),
        "{}",
        cached.body_text()
    );
    assert_eq!(
        VIEWER_RENDERS.load(Ordering::SeqCst),
        3,
        "the fourth request is a hit"
    );
}
