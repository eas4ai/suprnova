//! Shared boot for the Task 14 middleware tests in
//! `render_cache_middleware.rs`: a multi-connection in-memory SQLite
//! database (see [`boot_with_render_cache`] for why one connection is not
//! enough here), a real `RenderCache::install`, an adjustable clock, and
//! the `counting_route` handler that lets the tests observe render counts,
//! block a render, and inject a write mid-render without a timing-based
//! wait anywhere.
#![allow(dead_code)]

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::auth::Authenticatable;
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::registry::GroupPolicy;
use suprnova::render_cache::{
    FreshnessPolicy, QueryPolicy, RenderCache, RenderCachePolicy, RepresentationClass,
    VarianceDimension,
};
use suprnova::testing::TestContainer;
use suprnova::{
    App, Auth, ConnectionTrait, Crypt, EncryptionKey, FrameworkError, HttpResponse,
    MiddlewareRegistry, Model, Next, Request, Response, Router, attrs, handle_request,
};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::identity::UnixMillis;
use suprnova_live::render_cache::RenderCacheError;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, LocalRebuildCoordinator, RebuildAdmission, RebuildCoordinator,
    RebuildLease,
};
use suprnova_live::render_cache::store::PublicationFence;

#[suprnova::model(
    table = "posts",
    timestamps = false,
    fillable = ["title", "views"]
)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub views: i64,
}

struct MiddlewareMigrator;

#[async_trait::async_trait]
impl MigratorTrait for MiddlewareMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// A test principal, recognized through the `x-test-login` header (see
/// [`LoginHeader`]) - the same shape `live_dogfood_support::Principal`
/// uses.
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
/// `x-test-login: <id>` is treated as that authenticated user for the rest
/// of the request. Must run before `RenderCacheMiddleware` so `Auth::id()`
/// reflects it when the middleware builds `Principal` variance and reads
/// what the render observed.
pub struct LoginHeader;

#[async_trait]
impl suprnova::Middleware for LoginHeader {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(id) = request.header("x-test-login") {
            Auth::set_user(Arc::new(Principal(id.to_owned())));
        }
        next(request).await
    }
}

/// A clock the tests can move forward on demand, in whole milliseconds.
pub struct AdjustableTestClock {
    millis: AtomicU64,
}

impl AdjustableTestClock {
    fn new(start_ms: u64) -> Self {
        Self {
            millis: AtomicU64::new(start_ms),
        }
    }

    /// Advances the clock by `delta_ms`. Never goes backwards.
    pub fn advance_ms(&self, delta_ms: u64) {
        self.millis.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for AdjustableTestClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Ok(UnixMillis::new(self.millis.load(Ordering::SeqCst)))
    }
}

/// Wraps [`LocalRebuildCoordinator`] to make singleflight admission
/// observable from a test: [`Harness::wait_until_waiting`] blocks on a
/// state barrier (a counter plus a `Notify`, following the tokio
/// "enable-then-check" pattern so a notification firing between the check
/// and the wait is never lost) rather than a timing-based wait, which this
/// project's own conventions forbid.
struct WaiterTrackingCoordinator {
    inner: LocalRebuildCoordinator,
    waiting: AtomicU64,
    waiting_notify: tokio::sync::Notify,
}

impl WaiterTrackingCoordinator {
    fn new(limits: LocalCoordinatorLimits) -> Self {
        Self {
            inner: LocalRebuildCoordinator::new(limits),
            waiting: AtomicU64::new(0),
            waiting_notify: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait]
impl RebuildCoordinator for WaiterTrackingCoordinator {
    async fn admit(
        &self,
        key: &RenderKey,
        epoch: u64,
        now_ms: u64,
    ) -> Result<RebuildAdmission, RenderCacheError> {
        let admission = self.inner.admit(key, epoch, now_ms).await?;
        if matches!(admission, RebuildAdmission::Wait(_)) {
            self.waiting.fetch_add(1, Ordering::SeqCst);
            self.waiting_notify.notify_waiters();
        }
        Ok(admission)
    }

    async fn publish_token(
        &self,
        lease: &RebuildLease,
        now_ms: u64,
    ) -> Result<PublicationFence, RenderCacheError> {
        self.inner.publish_token(lease, now_ms).await
    }

    async fn release(&self, lease: RebuildLease) -> Result<(), RenderCacheError> {
        self.inner.release(lease).await
    }
}

/// Everything one test needs: the router and middleware registry to
/// dispatch through, the adjustable clock, and the singleflight waiter
/// counter. Held behind an `Arc` so a test can `.clone()` it into a
/// `tokio::spawn`ed task (the singleflight test dispatches two concurrent
/// requests).
pub struct Harness {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    clock: Arc<AdjustableTestClock>,
    waiting: Arc<WaiterTrackingCoordinator>,
    _conn: suprnova::database::DbConnection,
    _guard: suprnova::testing::TestContainerGuard,
    _tempdir: tempfile::TempDir,
}

/// Boots a fresh SQLite database with WAL journaling, installs RenderCache,
/// and registers the routes and policies every test in this file needs.
///
/// # Why a WAL-mode file database, not `TestDatabase`'s in-memory pool
///
/// This project's own `TestDatabase::fresh` opens `sqlite::memory:` with
/// exactly one connection - correct for the write-path tests in
/// `render_cache_orm.rs`, which never hold two transactions open at once.
/// This suite's "a write during the render discards the candidate" test
/// needs the opposite: the render's own read view (`DB::transaction`,
/// opened by the middleware around the handler) must still be open when a
/// *second*, independent write commits on another connection, so that the
/// render's snapshot genuinely predates it and the post-render reread can
/// observe the difference.
///
/// Two SQLite configurations were tried and rejected before this one, each
/// empirically (a hung `Post::create` on the independent connection,
/// confirmed with `eprintln!` checkpoints, not assumed):
/// - A single-connection pool contends the render's own transaction against
///   the injected write for the pool's one connection - the same shape
///   ruling R76 fixed elsewhere, proven to hang the same way.
/// - A multi-connection `sqlite::memory:?cache=shared` pool avoids the pool
///   contention but not a second one: SQLite's shared-cache mode uses
///   table-level locking, so the render's read-only transaction (which has
///   read the `posts` table) still blocks a second connection's write to
///   that same table until the reader's transaction ends - and it can't
///   end until the handler, which is awaiting that write, returns.
///
/// A real file with `journal_mode=WAL` gives genuine reader/writer
/// concurrency instead: a WAL reader sees a fixed snapshot as of when its
/// transaction began and never blocks a writer, and a writer never blocks
/// a reader. That is exactly the isolation the render's read view needs.
pub async fn boot_with_render_cache() -> Arc<Harness> {
    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    counting_route::reset();
    suprnova::middleware::clear_global_middleware_for_test();

    let guard = TestContainer::fake();
    let tempdir = tempfile::tempdir().expect("tempdir for render cache middleware test database");
    let db_path = tempdir.path().join("render-cache-middleware.sqlite3");
    let config = suprnova::database::DatabaseConfig::builder()
        .url(format!("sqlite://{}", db_path.display()))
        .max_connections(4)
        .min_connections(1)
        .logging(false)
        .build();
    let conn = suprnova::database::DbConnection::connect(&config)
        .await
        .expect("connect sqlite");
    conn.inner()
        .execute_unprepared("PRAGMA journal_mode=WAL")
        .await
        .expect("enable WAL journaling");
    conn.inner()
        .execute_unprepared("PRAGMA busy_timeout=5000")
        .await
        .expect("set busy timeout");
    MiddlewareMigrator::up(conn.inner(), None)
        .await
        .expect("apply render cache migration");
    conn.inner()
        .execute_unprepared(
            "CREATE TABLE posts (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                title TEXT NOT NULL, \
                views INTEGER NOT NULL DEFAULT 0\
            )",
        )
        .await
        .expect("create posts table");
    TestContainer::singleton(conn.clone());

    let waiting = Arc::new(WaiterTrackingCoordinator::new(LocalCoordinatorLimits {
        lease_ms: 30_000,
        max_waiters: 128,
    }));
    let clock = Arc::new(AdjustableTestClock::new(1_000_000));

    let cached_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .query(QueryPolicy::declared(["page"]))
        .build()
        .expect("cached policy");
    let stale_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 60_000, 120_000).expect("freshness"))
        .build()
        .expect("stale policy");
    let private_policy = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("private policy");
    let sets_cookie_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("sets-cookie policy");
    let overflow_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("overflow policy");

    let router: Router = Router::new().get("/cached/{id}", cached_handler).into();
    let router: Router = router.get("/stale/{id}", stale_handler).into();
    let router: Router = router.get("/private/{id}", private_handler).into();
    let router: Router = router.get("/sets-cookie", sets_cookie_handler).into();
    let router: Router = router.get("/overflow", overflow_handler).into();
    let router = router
        .try_render_cache("/cached/{id}", GroupPolicy::from(cached_policy))
        .expect("attach cached policy")
        .try_render_cache("/stale/{id}", GroupPolicy::from(stale_policy))
        .expect("attach stale policy")
        .try_render_cache("/private/{id}", GroupPolicy::from(private_policy))
        .expect("attach private policy")
        .try_render_cache("/sets-cookie", GroupPolicy::from(sets_cookie_policy))
        .expect("attach sets-cookie policy")
        .try_render_cache("/overflow", GroupPolicy::from(overflow_policy))
        .expect("attach overflow policy");

    let config = RenderCacheConfig::from_env()
        .with_clock_for_test(Arc::clone(&clock) as Arc<dyn Clock>)
        .with_coordinator_for_test(Arc::clone(&waiting) as Arc<dyn RebuildCoordinator>);
    let mut config = config;
    config.enabled = true;
    config.l1 = suprnova::render_cache::L1Config::Disabled;
    let router = RenderCache::install(router, config)
        .await
        .expect("install render cache");

    let middleware = Arc::new(MiddlewareRegistry::from_global().prepend(LoginHeader));

    Arc::new(Harness {
        router: Arc::new(router),
        middleware,
        clock,
        waiting,
        _conn: conn,
        _guard: guard,
        _tempdir: tempdir,
    })
}

/// The adjustable clock `install` was configured with.
pub fn clock(harness: &Harness) -> &Arc<AdjustableTestClock> {
    &harness.clock
}

/// The ledger reading the same database this harness installed.
pub fn ledger() -> suprnova::render_cache::ledger::SqlGenerationLedger {
    suprnova::render_cache::ledger::SqlGenerationLedger::new()
}

/// Advances the `posts` table's generation directly, through the ORM path,
/// independent of any render.
pub async fn advance_posts(_harness: &Harness) {
    suprnova::DB::transaction(|_tx| {
        Box::pin(async move {
            Post::create(attrs! { title: "advanced" }).await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("advance posts");
}

async fn cached_handler(request: Request) -> Response {
    counting_route::on_render_start().await;
    let id: i64 = request
        .param("id")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let _ = Post::find(id).await;
    counting_route::maybe_write_during_render().await;
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!("cached render {n}")))
}

async fn stale_handler(request: Request) -> Response {
    counting_route::on_render_start().await;
    let id: i64 = request
        .param("id")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let _ = Post::find(id).await;
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!("stale render {n}")))
}

async fn private_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let _ = Auth::id();
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!("private render {n}")))
}

async fn sets_cookie_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    Ok(HttpResponse::html("has a cookie").cookie(suprnova::Cookie::new("session", "abc")))
}

/// Ruling R55: observes more distinct table identities than the collector
/// can hold, so its report overflows. `4_200` clears
/// `suprnova_live::render_cache::generation::MAX_OBSERVATIONS` (4_096) with
/// room to spare without importing the constant just for this bound.
async fn overflow_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    for i in 0..4_200_u32 {
        suprnova::render_cache::collector::observe_table_read(&format!("overflow_table_{i}"));
    }
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!("overflow render {n}")))
}

/// Render-counting and coordination hooks the tests use to observe and
/// steer the mock handlers above, all built on atomics and
/// `tokio::sync::Notify` - never a timing-based wait.
pub mod counting_route {
    use super::*;

    static RENDERS: AtomicU64 = AtomicU64::new(0);
    static RENDER_STARTED: AtomicU64 = AtomicU64::new(0);
    static RENDERING_NOTIFY: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();
    static HOLD_NEXT: AtomicBool = AtomicBool::new(false);
    static RELEASE_NOTIFY: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();
    static WRITE_DURING_NEXT: AtomicBool = AtomicBool::new(false);

    fn rendering_notify() -> &'static tokio::sync::Notify {
        RENDERING_NOTIFY.get_or_init(tokio::sync::Notify::new)
    }

    fn release_notify() -> &'static tokio::sync::Notify {
        RELEASE_NOTIFY.get_or_init(tokio::sync::Notify::new)
    }

    pub(crate) fn reset() {
        RENDERS.store(0, Ordering::SeqCst);
        RENDER_STARTED.store(0, Ordering::SeqCst);
        HOLD_NEXT.store(false, Ordering::SeqCst);
        WRITE_DURING_NEXT.store(false, Ordering::SeqCst);
    }

    /// Total number of times a mock handler in this file has actually run.
    pub fn renders() -> u64 {
        RENDERS.load(Ordering::SeqCst)
    }

    /// Arms the next render to block, once started, until
    /// [`release_render`] is called.
    pub fn hold_next_render(_harness: &super::Harness) {
        HOLD_NEXT.store(true, Ordering::SeqCst);
    }

    /// Releases a render blocked by [`hold_next_render`]. The blocked
    /// render is guaranteed to already be waiting by the time a caller
    /// reaches this after `wait_until_rendering` and `wait_until_waiting`,
    /// so a plain `notify_waiters` cannot race it.
    pub fn release_render(_harness: &super::Harness) {
        release_notify().notify_waiters();
    }

    /// Arms the next render to perform a write on a genuinely independent
    /// connection - spawned so it does not inherit the ambient
    /// `CURRENT_TX` the render's own read-view transaction installed -
    /// after its own read, and to wait for that write to commit before the
    /// render returns.
    pub fn write_during_next_render(_harness: &super::Harness) {
        WRITE_DURING_NEXT.store(true, Ordering::SeqCst);
    }

    /// Waits until a render has started (has called [`on_render_start`]).
    /// Race-free: the notify handle is captured before the condition is
    /// checked, so a notification that fires in between is never missed.
    pub async fn wait_until_rendering(_harness: &super::Harness) {
        loop {
            let notified = rendering_notify().notified();
            if RENDER_STARTED.load(Ordering::SeqCst) > 0 {
                return;
            }
            notified.await;
        }
    }

    /// Waits until at least `n` requests have been admitted as singleflight
    /// waiters for this harness's coordinator.
    pub async fn wait_until_waiting(harness: &super::Harness, n: u64) {
        loop {
            let notified = harness.waiting.waiting_notify.notified();
            if harness.waiting.waiting.load(Ordering::SeqCst) >= n {
                return;
            }
            notified.await;
        }
    }

    pub(crate) async fn on_render_start() {
        RENDERS.fetch_add(1, Ordering::SeqCst);
        RENDER_STARTED.fetch_add(1, Ordering::SeqCst);
        rendering_notify().notify_waiters();
        if HOLD_NEXT.swap(false, Ordering::SeqCst) {
            release_notify().notified().await;
        }
    }

    pub(crate) async fn maybe_write_during_render() {
        if WRITE_DURING_NEXT.swap(false, Ordering::SeqCst) {
            let handle = tokio::spawn(async {
                let _ = super::Post::create(attrs! { title: "raced-write" }).await;
            });
            let _ = handle.await;
        }
    }
}

/// One dispatched response: status, lower-cased header map (first value per
/// name), and body bytes.
pub struct TestResponse {
    pub status: hyper::StatusCode,
    headers: std::collections::HashMap<String, String>,
    pub body: Bytes,
}

impl TestResponse {
    /// The first value of a response header, case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

async fn dispatch(
    harness: &Harness,
    method: hyper::Method,
    path: &str,
    extra_headers: &[(&str, &str)],
) -> TestResponse {
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1");
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Full::new(Bytes::new()))
        .expect("build request");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let router = Arc::clone(&harness.router);
    let middleware = Arc::clone(&harness.middleware);
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
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    TestResponse {
        status,
        headers,
        body,
    }
}

/// Dispatches a `GET` request to `path` with `extra_headers`.
pub async fn dispatch_get(
    harness: &Harness,
    path: &str,
    extra_headers: &[(&str, &str)],
) -> TestResponse {
    dispatch(harness, hyper::Method::GET, path, extra_headers).await
}

/// Dispatches a `HEAD` request to `path`.
pub async fn dispatch_head(harness: &Harness, path: &str) -> TestResponse {
    dispatch(harness, hyper::Method::HEAD, path, &[]).await
}
