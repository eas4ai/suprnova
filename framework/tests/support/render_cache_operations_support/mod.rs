//! Shared boot for the Task 16 operator-control tests in
//! `render_cache_operations.rs`: a minimal, self-contained harness
//! (deliberately not sharing `render_cache_middleware_support`, which is
//! the delicate, extensively fix-rounded harness `render_cache_middleware.rs`'s
//! 43 tests depend on - this file duplicates only the small slice of that
//! setup Task 16's own tests need, so nothing here can regress that suite).
//!
//! Three routes: `/cached/{id}` (`PublicShared`, L0 only), `/private/{id}`
//! (`PrivateCached`, varies on `Principal`, recognizes the same
//! `x-test-login` header shape `render_cache_middleware_support::LoginHeader`
//! uses), and `/stale/{id}` (`PublicShared`, fresh 60s / stale-servable 60s
//! / stale-on-error 120s, `StorageLayers::l0_and_l1()`) for the sweep
//! tests. [`boot_with_render_cache`] configures L1 as disabled;
//! [`boot_with_file_l1`] configures a real file-backed L1 in a fresh temp
//! directory and returns it so a test can inspect the directory directly.
#![allow(dead_code)]

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
    FreshnessPolicy, L1Config, RenderCache, RenderCachePolicy, RepresentationClass, StorageLayers,
    VarianceDimension,
};
use suprnova::testing::TestContainer;
use suprnova::{
    App, Auth, ConnectionTrait, Crypt, EncryptionKey, HttpResponse, MiddlewareRegistry, Next,
    Request, Response, Router, handle_request,
};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::identity::UnixMillis;

struct OperationsMigrator;

#[async_trait::async_trait]
impl MigratorTrait for OperationsMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// A test principal, recognized through the `x-test-login` header (see
/// [`LoginHeader`]).
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
/// reflects it when the middleware builds `Principal` variance.
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

pub mod counting_route {
    use super::*;

    static RENDERS: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn reset() {
        RENDERS.store(0, Ordering::SeqCst);
    }

    /// Total number of times a mock handler in this file has actually run.
    pub fn renders() -> u64 {
        RENDERS.load(Ordering::SeqCst)
    }

    pub(crate) fn record() -> u64 {
        RENDERS.fetch_add(1, Ordering::SeqCst) + 1
    }
}

async fn cached_handler(request: Request) -> Response {
    let n = counting_route::record();
    let id = request.param("id").unwrap_or("0");
    Ok(HttpResponse::html(format!("cached render {n} for {id}")))
}

async fn stale_handler(request: Request) -> Response {
    let n = counting_route::record();
    let id = request.param("id").unwrap_or("0");
    Ok(HttpResponse::html(format!("stale render {n} for {id}")))
}

async fn private_handler(_request: Request) -> Response {
    let n = counting_route::record();
    let _ = Auth::id();
    Ok(HttpResponse::html(format!("private render {n}")))
}

/// Everything one test needs: the router and middleware registry to
/// dispatch through, plus the adjustable clock.
pub struct Harness {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    clock: Arc<AdjustableTestClock>,
    _conn: suprnova::database::DbConnection,
    _guard: suprnova::testing::TestContainerGuard,
    _tempdir: tempfile::TempDir,
    // No L1 tempdir field (fix round 1, R95/F12: an earlier version of
    // this struct carried one that both `boot` match arms always left
    // `None`, with a doc claiming otherwise). L1's directory is owned by
    // the tempdir [`boot_with_file_l1`] returns directly to its caller,
    // which must outlive the harness for the L1 directory to survive - the
    // caller holds both bindings for exactly that reason.
}

/// Boots a fresh SQLite database with RenderCache installed, L1 disabled,
/// and `/cached/{id}` and `/private/{id}` registered.
pub async fn boot_with_render_cache() -> Arc<Harness> {
    boot(None).await
}

/// Boots exactly like [`boot_with_render_cache`], except L1 is a real
/// file-backed store rooted at a fresh temp directory (returned alongside
/// the harness), and `/stale/{id}` additionally uses
/// `StorageLayers::l0_and_l1()` so it is the one route in this harness that
/// actually writes to L1.
pub async fn boot_with_file_l1() -> (Arc<Harness>, tempfile::TempDir) {
    let l1_dir = tempfile::tempdir().expect("l1 tempdir");
    let harness = boot(Some(l1_dir.path().to_path_buf())).await;
    (harness, l1_dir)
}

async fn boot(l1_directory: Option<std::path::PathBuf>) -> Arc<Harness> {
    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    counting_route::reset();
    suprnova::middleware::clear_global_middleware_for_test();

    let guard = TestContainer::fake();
    let tempdir = tempfile::tempdir().expect("tempdir for render cache operations database");
    let db_path = tempdir.path().join("render-cache-operations.sqlite3");
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
    OperationsMigrator::up(conn.inner(), None)
        .await
        .expect("apply render cache migration");
    TestContainer::singleton(conn.clone());

    let clock = Arc::new(AdjustableTestClock::new(1_000_000));

    let cached_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("cached policy");
    let private_policy = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("private policy");
    let stale_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 60_000, 120_000).expect("freshness"))
        .layers(StorageLayers::l0_and_l1())
        .build()
        .expect("stale policy");
    // Fix round 1 (R93/F2, F3): stale_servable_ms (120_000) wider than
    // stale_on_error_ms (0) - the reviewer's exact case, and
    // `FreshnessPolicy::new` accepts it. `fresh_ms + stale_on_error_ms`
    // (the pre-fix-round formula) would give 60_000; the true Dead edge
    // (`dead_after_ms`) is 180_000. Only this route's shape can catch
    // `store_entry` reverting to the wrong formula, to `0`, or to
    // `u64::MAX`, since an ordinary policy (`stale_on_error_ms >=
    // stale_servable_ms`) gives the same answer either way.
    let inverted_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 120_000, 0).expect("freshness"))
        .layers(StorageLayers::l0_and_l1())
        .build()
        .expect("inverted policy");
    // Fix round 2 (R99/N4): same freshness numbers as `stale_policy` above
    // (60_000, 60_000, 120_000), but `PrivateCached` - the class-aware Dead
    // edge (`fresh_ms` alone, 60_000) differs from the `PublicShared`
    // edge with identical numbers (`fresh_ms + max(ss, soe)`, 180_000), so
    // dispatching to both routes and comparing sweep behavior at 60_000 and
    // at 180_000 proves `store_entry` frames a class-aware retention.
    let private_l1_policy = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 60_000, 120_000).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .layers(StorageLayers::l0_and_l1())
        .build()
        .expect("private l1 policy");

    let router: Router = Router::new().get("/cached/{id}", cached_handler).into();
    let router: Router = router.get("/private/{id}", private_handler).into();
    let router: Router = router.get("/stale/{id}", stale_handler).into();
    let router: Router = router.get("/inverted/{id}", stale_handler).into();
    let router: Router = router.get("/private-l1/{id}", private_handler).into();
    let router = router
        .try_render_cache("/cached/{id}", GroupPolicy::from(cached_policy))
        .expect("attach cached policy")
        .try_render_cache("/private/{id}", GroupPolicy::from(private_policy))
        .expect("attach private policy")
        .try_render_cache("/stale/{id}", GroupPolicy::from(stale_policy))
        .expect("attach stale policy")
        .try_render_cache("/inverted/{id}", GroupPolicy::from(inverted_policy))
        .expect("attach inverted policy")
        .try_render_cache("/private-l1/{id}", GroupPolicy::from(private_l1_policy))
        .expect("attach private l1 policy");

    let mut config =
        RenderCacheConfig::from_env().with_clock_for_test(Arc::clone(&clock) as Arc<dyn Clock>);
    config.enabled = true;
    config.l1 = match l1_directory {
        Some(directory) => L1Config::File {
            directory,
            max_bytes: 16 * 1024 * 1024,
        },
        None => L1Config::Disabled,
    };

    // Same ordering requirement `render_cache_middleware_support` documents
    // on its own `LoginHeader` registration: must run before
    // `RenderCache::install` so `Auth::id()` reflects it when the
    // middleware builds `Principal` variance.
    suprnova::middleware::register_global_middleware(LoginHeader);
    let router = RenderCache::install(router, config)
        .await
        .expect("install render cache");
    let middleware = Arc::new(MiddlewareRegistry::from_global());

    Arc::new(Harness {
        router: Arc::new(router),
        middleware,
        clock,
        _conn: conn,
        _guard: guard,
        _tempdir: tempdir,
    })
}

/// The adjustable clock `install` was configured with.
pub fn clock(harness: &Harness) -> &Arc<AdjustableTestClock> {
    &harness.clock
}

/// One dispatched response: status and body bytes.
pub struct TestResponse {
    pub status: hyper::StatusCode,
    pub body: Bytes,
}

/// Dispatches a `GET` request to `path` with `extra_headers`, through a
/// real loopback HTTP connection - the same technique
/// `render_cache_middleware_support::dispatch_get` uses, since the
/// middleware chain is only exercised faithfully behind an actual
/// `hyper` request/response cycle.
pub async fn dispatch_get(
    harness: &Harness,
    path: &str,
    extra_headers: &[(&str, &str)],
) -> TestResponse {
    let mut builder = hyper::Request::builder()
        .method(hyper::Method::GET)
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
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    TestResponse { status, body }
}
