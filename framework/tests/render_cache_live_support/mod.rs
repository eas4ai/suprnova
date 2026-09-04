//! Shared boot for `render_cache_live.rs`: the same production-shaped Live
//! router `live_dogfood_support` builds (a public-seed document and an
//! identity-bound one), plus a route that mounts an identity-bound island
//! and never calls `LiveDocument::render` at all, with RenderCache's
//! middleware installed on top and a single [`AdjustableTestClock`] shared
//! between the Live runtime and the RenderCache runtime, so a seed's
//! promotion deadline and the cache entry's publication time agree on what
//! "now" means.
//!
//! `#[serial_test::serial]` and plain `#[tokio::test]` (current-thread),
//! never `flavor = "multi_thread"`: `RenderCache::install`'s runtime and
//! the process-wide global middleware registry are process-global state,
//! and `TestContainer::fake()` writes a thread-local that a multi-thread
//! runtime could migrate away from between polls - see
//! `render_cache_middleware.rs`'s own module doc for the fuller version of
//! this reasoning, which this file's tests follow for the same reasons.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::live::testing::{AdjustableTestClock, prepare_live_router_with_clock_for_test};
use suprnova::live::{LiveDocument, LiveMount, LiveTenantMiddleware};
use suprnova::middleware::{Middleware, Next};
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::{
    FreshnessPolicy, RenderCache, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
    VarianceDimension,
};
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthMiddleware, CsrfMiddleware, HttpResponse, MiddlewareRegistry, Request, Response,
    Router, SessionConfig, SessionMiddleware, StatusCode, handle_request,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::Clock;
use suprnova_live::mount::MountFlags;

use crate::live_dogfood_support::{
    DOCUMENT_PATH, DogfoodCounter, LoginHeader, MemorySessionStore, PRIVATE_DOCUMENT_PATH,
    Tenantless, build_public_router, fixture,
};

/// An identity-bound island mounted on a route whose handler never calls
/// `LiveDocument::render`: the island markup goes straight into a
/// hand-built response, exactly as `MountedIsland::html()`'s public
/// `Display` allows. Proves the mount-time recording (R87) declines this
/// even though `render_cache::live::record_document_intent` never runs.
pub const RAW_PATH: &str = "/dogfood/private-raw";

/// Fix round 2 (R89): a route declared `RepresentationClass::PrivateCached`
/// with `Principal` variance, whose handler reads no identity at all.
/// `classify` starts from the declared class and only narrows further, so
/// this always produces `(PrivateCached, [])` - a shape `is_unreasoned_private_class`
/// must not decline, since the declared class already required `Principal`
/// variance and Task 14 cached this correctly. No Live document involved:
/// this exercises `middleware.rs`'s classification path directly.
pub const UNREASONED_PATH: &str = "/unreasoned-private-cached";

/// Fix round 2 (finding 8): a route declared `PublicShared` with no
/// variance, whose handler reads an identity (attaching
/// `ClassificationReason::PrincipalObserved`) and then calls the test-only
/// `strip_classification_reasons_for_test` seam, simulating a class
/// `classify` genuinely narrowed to `PrivateCached` with the reason
/// stripped away - the shape `is_unreasoned_private_class`'s call site
/// exists to catch, reached here through `lead_render`'s real control flow
/// since `classify` itself cannot produce it unaided.
pub const STRIP_PATH: &str = "/strip-classification-reason";

/// Counts renders reaching the handler side of the RenderCache middleware,
/// split by which route was hit so the probe routes do not share a
/// counter. Registered AFTER `RenderCache::install` (see the harness's own
/// doc), so a request served from the cache never increments any of them -
/// only an actual render does.
struct RenderCounter;

#[async_trait::async_trait]
impl Middleware for RenderCounter {
    async fn handle(&self, request: Request, next: Next) -> Response {
        match request.path() {
            PRIVATE_DOCUMENT_PATH | RAW_PATH => {
                PRIVATE_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            UNREASONED_PATH => {
                UNREASONED_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            STRIP_PATH => {
                STRIP_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            _ => {
                PUBLIC_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
        }
        next(request).await
    }
}

static PUBLIC_RENDERS: AtomicUsize = AtomicUsize::new(0);
static PRIVATE_RENDERS: AtomicUsize = AtomicUsize::new(0);
static UNREASONED_RENDERS: AtomicUsize = AtomicUsize::new(0);
static STRIP_RENDERS: AtomicUsize = AtomicUsize::new(0);

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
/// (`DOCUMENT_PATH`), identity-bound document (`PRIVATE_DOCUMENT_PATH`),
/// and the render-bypassing identity-bound route (`RAW_PATH`) with a
/// generous shared `RenderCachePolicy` each, installs RenderCache, and
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

    // `fresh_ms` is chosen larger than the Live runtime's fixed public-seed
    // lifetime so that ordinary freshness never governs the tests in this
    // file: the seed deadline is always the tighter bound, which is
    // exactly the mechanism under test. `shared(SMaxAge)` makes the served
    // `Cache-Control` actually say `public, ...` when the class really is
    // `PublicShared` - proving R86's fix, since the pre-fix `Private`
    // narrowing demoted this exact route to `private, ...` regardless.
    let public_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .shared(SharedCachePolicy::SMaxAge { seconds: 200_000 })
        .build()
        .expect("public seed policy");
    // Unlike an earlier draft of this harness, this declares `Principal`
    // variance: `classify`'s own key/value guard is satisfied, and cannot
    // be what declines the entry (see finding 4). Whatever declines it is
    // `document_declines`'s identity-bound branch and nothing else.
    let private_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("identity bound policy");
    let raw_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("raw identity bound policy");
    // R89's own shape: declared `PrivateCached` already requires `Principal`
    // or `Tenant` variance to build at all (Task 14 round 6), so this is a
    // legitimately cacheable route whose handler simply never happens to
    // read an identity.
    let unreasoned_policy = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("unreasoned private cached policy");
    // Finding 8's shape: declared `PublicShared`, no variance at all - the
    // point is that nothing partitions the key, so if the invariant's call
    // site did not run, the reason-stripped render would be stored and
    // served to every caller regardless of identity.
    let strip_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .build()
        .expect("strip classification reason policy");

    let raw = LiveMount::<DogfoodCounter>::identity_bound(RAW_PATH, "counter", "dogfood-raw")
        .expect("declare raw identity-bound mount");
    let raw_handler = raw.clone();
    let router: Router = build_public_router();
    let router: Router = router
        .get(RAW_PATH, move |request: Request| {
            let mount = raw_handler.clone();
            async move { render_raw_document(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .into();
    let router = router
        .try_live_mount(&raw)
        .expect("register raw identity-bound mount");
    let router: Router = router.get(UNREASONED_PATH, unreasoned_handler).into();
    let router: Router = router.get(STRIP_PATH, strip_handler).into();
    let router = router
        .try_render_cache(DOCUMENT_PATH, public_policy)
        .expect("attach public seed render cache policy")
        .try_render_cache(PRIVATE_DOCUMENT_PATH, private_policy)
        .expect("attach identity bound render cache policy")
        .try_render_cache(RAW_PATH, raw_policy)
        .expect("attach raw render cache policy")
        .try_render_cache(UNREASONED_PATH, unreasoned_policy)
        .expect("attach unreasoned private cached policy")
        .try_render_cache(STRIP_PATH, strip_policy)
        .expect("attach strip classification reason policy");

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
    // AFTER install, so it sits behind the cache middleware and is reached
    // only when the cache decides to render; reset before it is registered
    // so an earlier test's counts never leak into this one.
    PUBLIC_RENDERS.store(0, Ordering::SeqCst);
    PRIVATE_RENDERS.store(0, Ordering::SeqCst);
    UNREASONED_RENDERS.store(0, Ordering::SeqCst);
    STRIP_RENDERS.store(0, Ordering::SeqCst);
    suprnova::middleware::register_global_middleware(RenderCounter);
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

async fn render_raw_document(
    request: Request,
    mount: LiveMount<DogfoodCounter>,
) -> Result<HttpResponse, HttpResponse> {
    let mut document = LiveDocument::from_request(&request)
        .map_err(|error| HttpResponse::text(format!("from_request {error}")).status(500))?;
    let island = document
        .mount(
            &mount,
            CanonicalValue::Object(BTreeMap::new()),
            MountFlags::empty(),
        )
        .await
        .map_err(|error| HttpResponse::text(format!("mount {error}")).status(500))?;
    // Deliberately NOT `document.render(..)`: the island markup goes
    // straight into a hand-built response.
    Ok(HttpResponse::html(format!(
        "<!doctype html><html><body>{}</body></html>",
        island.html()
    )))
}

/// R89's shape: reads no identity at all. `UNREASONED_PATH`'s policy
/// already declares `Principal` variance, so `key_input` derives an
/// identity-scoped key without this handler's help.
async fn unreasoned_handler(_request: Request) -> Response {
    Ok(HttpResponse::html("unreasoned private cached"))
}

/// Finding 8's shape: reads an identity (attaching
/// `ClassificationReason::PrincipalObserved` inside `classify`), then
/// immediately strips that reason via the test-only seam, simulating a
/// class `classify` narrowed to `PrivateCached` with no reason surviving.
async fn strip_handler(_request: Request) -> Response {
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    suprnova::render_cache::collector::strip_classification_reasons_for_test();
    Ok(HttpResponse::html(format!(
        "strip classification reason render for {identity}"
    )))
}

/// The adjustable clock this harness shared between both runtimes.
pub fn clock(harness: &Harness) -> &Arc<AdjustableTestClock> {
    &harness.clock
}

/// The Live runtime's fixed public-seed lifetime in milliseconds - the one
/// constant `assemble_runtime` uses for every `LiveRuntime`, exposed
/// through `suprnova::live::testing` rather than mirrored here.
pub fn public_seed_lifetime_ms(_harness: &Harness) -> u64 {
    suprnova::live::testing::PUBLIC_SEED_MAX_AGE_MS
}

/// Renders reaching `DOCUMENT_PATH`'s handler so far.
pub fn public_renders() -> usize {
    PUBLIC_RENDERS.load(Ordering::SeqCst)
}

/// Renders reaching `PRIVATE_DOCUMENT_PATH`'s or `RAW_PATH`'s handler so far.
pub fn private_renders() -> usize {
    PRIVATE_RENDERS.load(Ordering::SeqCst)
}

/// Renders reaching `UNREASONED_PATH`'s handler so far.
pub fn unreasoned_renders() -> usize {
    UNREASONED_RENDERS.load(Ordering::SeqCst)
}

/// Renders reaching `STRIP_PATH`'s handler so far.
pub fn strip_renders() -> usize {
    STRIP_RENDERS.load(Ordering::SeqCst)
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
