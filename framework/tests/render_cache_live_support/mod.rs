//! Shared boot for `render_cache_live.rs`: the same production-shaped Live
//! router `live_dogfood_support` builds (a public-seed document and an
//! identity-bound one), with RenderCache's middleware installed on top and
//! a single [`AdjustableTestClock`] shared between the Live runtime and the
//! RenderCache runtime, so a seed's promotion deadline and the cache
//! entry's publication time agree on what "now" means.
//!
//! `#[serial_test::serial]` and plain `#[tokio::test]` (current-thread),
//! never `flavor = "multi_thread"`: `RenderCache::install`'s runtime and
//! the process-wide global middleware registry are process-global state,
//! and `TestContainer::fake()` writes a thread-local that a multi-thread
//! runtime could migrate away from between polls - see
//! `render_cache_middleware.rs`'s own module doc for the fuller version of
//! this reasoning, which this file's tests follow for the same reasons.
#![allow(dead_code)]

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::live::testing::{AdjustableTestClock, prepare_live_router_with_clock_for_test};
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::{
    FreshnessPolicy, RenderCache, RenderCachePolicy, RepresentationClass,
};
use suprnova::testing::TestContainer;
use suprnova::{
    CsrfMiddleware, MiddlewareRegistry, Router, SessionConfig, SessionMiddleware, StatusCode,
    handle_request,
};
use suprnova_live::clock::Clock;

use crate::live_dogfood_support::{
    DOCUMENT_PATH, LoginHeader, MemorySessionStore, PRIVATE_DOCUMENT_PATH, build_public_router,
    fixture,
};

/// The single RenderCache migration this harness needs; mirrors
/// `render_cache_middleware_support::MiddlewareMigrator`, which is private
/// to that module.
struct LiveRenderCacheMigrator;

#[async_trait::async_trait]
impl MigratorTrait for LiveRenderCacheMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// The Live runtime hardcodes every `LiveRuntime`'s public-seed promotion
/// deadline to this many milliseconds after mount time, in
/// `framework/src/live/runtime.rs`'s `assemble_runtime`. Every assembly
/// path, including this harness's own
/// `prepare_live_router_with_clock_for_test`, funnels through that one
/// function. There is no accessor that reads the value back off a built
/// runtime, so this harness mirrors the constant directly rather than
/// adding one only for this test.
const LIVE_RUNTIME_MAX_SEED_AGE_MS: u64 = 86_400_000;

/// Everything one test needs: the router and middleware registry to
/// dispatch through, and the clock both the Live runtime and the
/// RenderCache runtime read.
pub struct Harness {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    clock: Arc<AdjustableTestClock>,
    _conn: suprnova::database::DbConnection,
    _guard: suprnova::testing::TestContainerGuard,
    _tempdir: tempfile::TempDir,
}

/// Boots a fresh SQLite database with the RenderCache migration applied,
/// registers `live_dogfood_support`'s public-seed document
/// (`DOCUMENT_PATH`) and identity-bound document (`PRIVATE_DOCUMENT_PATH`)
/// with a generous shared `RenderCachePolicy`, installs RenderCache, and
/// prepares the Live runtime on the same router with the same clock.
pub async fn boot_with_render_cache_and_live() -> Arc<Harness> {
    static CRYPT_ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT_ONCE.get_or_init(|| {
        suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    });
    suprnova::App::init();
    suprnova::middleware::clear_global_middleware_for_test();

    let guard = TestContainer::fake();
    fixture();

    let tempdir = tempfile::tempdir().expect("tempdir for render cache live test database");
    let db_path = tempdir.path().join("render-cache-live.sqlite3");
    let config = suprnova::database::DatabaseConfig::builder()
        .url(format!("sqlite://{}", db_path.display()))
        .max_connections(4)
        .min_connections(1)
        .logging(false)
        .build();
    let conn = suprnova::database::DbConnection::connect(&config)
        .await
        .expect("connect sqlite");
    LiveRenderCacheMigrator::up(conn.inner(), None)
        .await
        .expect("apply render cache migration");
    TestContainer::singleton(conn.clone());

    let clock = Arc::new(AdjustableTestClock::new(1_000_000));

    // `fresh_ms` is chosen larger than the Live runtime's fixed 24-hour
    // seed lifetime so that ordinary freshness never governs the tests in
    // this file: the seed deadline is always the tighter bound, which is
    // exactly the mechanism under test.
    let public_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .build()
        .expect("public seed policy");
    let private_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .build()
        .expect("identity bound policy");

    let router: Router = build_public_router();
    let router = router
        .try_render_cache(DOCUMENT_PATH, public_policy)
        .expect("attach public seed render cache policy")
        .try_render_cache(PRIVATE_DOCUMENT_PATH, private_policy)
        .expect("attach identity bound render cache policy");

    let mut render_cache_config =
        RenderCacheConfig::from_env().with_clock_for_test(Arc::clone(&clock) as Arc<dyn Clock>);
    render_cache_config.enabled = true;
    render_cache_config.l1 = suprnova::render_cache::L1Config::Disabled;

    // Registered globally, and before `RenderCache::install`, for the same
    // ordering reason `render_cache_middleware_support::boot` documents:
    // `install` appends its own middleware rather than inserting at a fixed
    // position, so this is what makes the identity-bound document's
    // `AuthMiddleware::new()` guard see the session `LoginHeader`
    // establishes.
    let mut session_config = SessionConfig::default();
    session_config.cookie_secure = false;
    suprnova::middleware::register_global_middleware(SessionMiddleware::with_store(
        session_config,
        Arc::new(MemorySessionStore::default()),
    ));
    suprnova::middleware::register_global_middleware(CsrfMiddleware::new());
    suprnova::middleware::register_global_middleware(LoginHeader);

    let router = RenderCache::install(router, render_cache_config)
        .await
        .expect("install render cache");
    let router = Arc::new(router);
    prepare_live_router_with_clock_for_test(&router, Arc::clone(&clock))
        .expect("prepare Live runtime");

    let middleware = Arc::new(MiddlewareRegistry::from_global());

    Arc::new(Harness {
        router,
        middleware,
        clock,
        _conn: conn,
        _guard: guard,
        _tempdir: tempdir,
    })
}

/// The adjustable clock this harness shared between both runtimes.
pub fn clock(harness: &Harness) -> &Arc<AdjustableTestClock> {
    &harness.clock
}

/// The Live runtime's fixed public-seed lifetime in milliseconds.
pub fn public_seed_lifetime_ms(_harness: &Harness) -> u64 {
    LIVE_RUNTIME_MAX_SEED_AGE_MS
}

/// One dispatched response: status, an accessor for a header, and the body.
pub struct TestResponse {
    pub status: StatusCode,
    headers: hyper::HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    /// The first value of `name`, if present.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// The session cookie pair from this response, skipping the XSRF token
    /// cookie the CSRF middleware attaches alongside it - the same rule
    /// `live_dogfood_support::session_cookie` applies.
    #[must_use]
    pub fn session_cookie(&self) -> String {
        self.headers
            .get_all("set-cookie")
            .iter()
            .find_map(|value| {
                let pair = value.to_str().ok()?.split(';').next()?.to_owned();
                (!pair.starts_with("XSRF-TOKEN=")).then_some(pair)
            })
            .expect("session response must emit a session cookie")
    }
}

/// Dispatches one GET request with the given extra headers through the
/// harness's real HTTP path (a bound loopback listener, exactly like
/// `render_cache_middleware_support::dispatch_get` and
/// `live_dogfood_support::dispatch`), and returns the decoded response.
pub async fn dispatch_get(harness: &Harness, path: &str, headers: &[(&str, &str)]) -> TestResponse {
    let mut builder = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(path)
        .header("host", "127.0.0.1");
    for (name, value) in headers {
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
    let headers = response.headers().clone();
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
