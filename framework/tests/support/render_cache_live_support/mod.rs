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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::live::testing::{AdjustableTestClock, prepare_live_router_with_clock_for_test};
use suprnova::live::{
    LiveBootstrapOptions, LiveComponent, LiveDocument, LiveMount, LiveRegistry,
    LiveTenantMiddleware, live,
};
use suprnova::middleware::{Middleware, Next};
use suprnova::render_cache::collector::{self, CollectorReport};
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::{
    FreshnessPolicy, RenderCache, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
    VarianceDimension,
};
use suprnova::testing::TestContainer;
use suprnova::view::{AssetSet, DocumentResponseIntent, ViewName};
use suprnova::{
    Auth, AuthMiddleware, CsrfMiddleware, FrameworkError, HttpResponse, MiddlewareRegistry,
    Request, Response, Router, SessionConfig, SessionMiddleware, StatusCode, handle_request,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::Clock;
use suprnova_live::mount::MountFlags;

use crate::live_dogfood_support::{
    DOCUMENT_PATH, DogfoodCounter, DogfoodDocument, LoginHeader, MemorySessionStore,
    PRIVATE_DOCUMENT_PATH, Tenantless, build_public_router, fixture,
};

/// An identity-bound island mounted on a route whose handler never calls
/// `LiveDocument::render`: the island markup goes straight into a
/// hand-built response, exactly as `MountedIsland::html()`'s public
/// `Display` allows. Proves the mount-time recording (R87) declines this
/// even though `render_cache::live::record_document_intent` never runs.
pub const RAW_PATH: &str = "/dogfood/private-raw";

/// An identity-bound island whose handler renders a whole document and then
/// stores the collector report it can see at that moment into
/// [`last_report`]. Reading the report from inside the handler is what makes
/// the mount-time stitch facts observable: the render cache middleware folds
/// and consumes its own copy after the handler returns, so a test that only
/// looked at what the cache did could never see the capture itself.
pub const CAPTURE_PATH: &str = "/dogfood/private-capture";

/// The document mount key `CAPTURE_PATH` declares.
pub const CAPTURE_DOCUMENT_KEY: &str = "dogfood-capture";

/// The same whole-document render as [`CAPTURE_PATH`], on a route carrying
/// no render cache policy at all, so nothing ever wraps its handler in a
/// collector scope.
///
/// `LiveDocument` guards its capture work on `collector::is_active`, so this
/// route is what proves those guards leave the served document exactly as it
/// was and record nothing: the handler stores the report it can see into
/// [`last_report`] the same way `CAPTURE_PATH`'s does, and outside a scope
/// there is none to see.
pub const UNCACHED_CAPTURE_PATH: &str = "/dogfood/private-capture-uncached";

/// The document mount key `UNCACHED_CAPTURE_PATH` declares.
pub const UNCACHED_CAPTURE_DOCUMENT_KEY: &str = "dogfood-capture-uncached";

/// An identity-bound island on a route carrying no authentication
/// middleware at all, so an anonymous request's mount fails and the
/// handler still answers `200` with a document that has no island in it.
///
/// This is the one residual path of the slot-read exemption in
/// `collector::mark_incomplete`: `LiveDocument::mount` calls
/// `render_cache::live::record_mount` *after* the mount, so a mount that
/// fails propagates with `?` before the fact is recorded, and
/// `live::document_declines` never sees an identity-bound island to
/// decline the route for. The response is therefore storable - and safe,
/// because the mount that failed produced no island markup for the stored
/// shell to carry. `FAILED_MOUNT_FALLBACK` is what the handler renders
/// instead, so a test can name what the shell does contain as well as what
/// it does not.
pub const FAILED_MOUNT_PATH: &str = "/dogfood/failed-mount";

/// The document mount key `FAILED_MOUNT_PATH` declares.
pub const FAILED_MOUNT_DOCUMENT_KEY: &str = "dogfood-failed-mount";

/// The only body text `FAILED_MOUNT_PATH`'s handler renders once its mount
/// has failed. See that constant's own doc.
pub const FAILED_MOUNT_FALLBACK: &str = "island unavailable";

/// The Content Security Policy nonce `CAPTURE_PATH`'s bootstrap stamps, so
/// the recorded bootstrap nonce has one exact expected value.
pub const CAPTURE_NONCE: &str = "c4ptur3n0nce";

/// The component `CAPTURE_PATH` mounts: the same shape as
/// [`DogfoodCounter`], with its own view, and a mount hook that reads the
/// request's principal.
///
/// That read is the point: an identity-bound mount runs inside
/// `collector::slot_scope`, so a read it performs is counted into
/// `CollectorReport::slot_reads` and recorded in no bucket at all. Without a
/// component that actually reads something during its mount, the capture
/// test could not tell a mount that runs inside a slot scope from one that
/// does not. `DogfoodCounter` itself must not gain this hook: its
/// public-seed mount runs outside any slot scope, so the same read would
/// land in a recorded bucket and narrow that document's class.
#[derive(LiveComponent)]
#[live(
    name = "tests.capture-counter",
    view = "live/tests/capture-counter.html"
)]
pub struct CaptureCounter {
    #[public]
    count: u64,
}

#[live]
impl CaptureCounter {
    #[mount]
    pub fn mount() -> Self {
        let _ = Auth::id();
        Self { count: 0 }
    }
}

/// The report `CAPTURE_PATH`'s handler saw at the moment it finished
/// rendering its document. Process-global like every other counter here,
/// and reset by `boot_with_render_cache_and_live`; the tests that read it
/// are `#[serial_test::serial]`.
static LAST_REPORT: Mutex<Option<CollectorReport>> = Mutex::new(None);

/// The collector report `CAPTURE_PATH`'s handler last stored.
#[must_use]
pub fn last_report() -> Option<CollectorReport> {
    LAST_REPORT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The report `FAILED_MOUNT_PATH`'s handler saw after its mount failed.
/// Kept apart from [`LAST_REPORT`] so neither route can overwrite the
/// other's, which two `#[serial_test::serial]` tests reading one shared
/// slot would otherwise do.
static LAST_FAILED_MOUNT_REPORT: Mutex<Option<CollectorReport>> = Mutex::new(None);

/// The collector report `FAILED_MOUNT_PATH`'s handler last stored.
#[must_use]
pub fn last_failed_mount_report() -> Option<CollectorReport> {
    LAST_FAILED_MOUNT_REPORT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Fix round 2 (R89): a route declared `RepresentationClass::PrivateCached`
/// with `Principal` variance, whose handler reads no identity at all.
/// `classify` starts from the declared class and only narrows further, so
/// this always produces `(PrivateCached, [])` - a shape `is_unreasoned_private_class`
/// must not decline, since the declared class already required `Principal`
/// variance and Task 14 cached this correctly. No Live document involved:
/// this exercises `middleware.rs`'s classification path directly.
pub const UNREASONED_PATH: &str = "/unreasoned-private-cached";

/// Fix round 2 (finding 8), policy updated in round 3 (R90): a route
/// declared `PublicShared` with `Principal` variance, whose handler reads
/// an identity (attaching `ClassificationReason::PrincipalObserved`) and
/// then calls the test-only `strip_classification_reasons_for_test` seam.
/// Since the seam only ever strips a *copy* used for the invariant check
/// (R90), the value guard sees the real, unstripped reason and is
/// satisfied by the declared `Principal` variance - it cannot be what
/// declines a second request from the same user. Only
/// `is_unreasoned_private_class`, seeing the copy narrowed to
/// `PrivateCached` with the reason stripped away, can still decline this
/// shape, reached here through `lead_render`'s real control flow since
/// `classify` itself cannot produce it unaided.
pub const STRIP_PATH: &str = "/strip-classification-reason";

/// Fix round 3 (R90, finding 10): declared `PrivateCached` with `Tenant`
/// variance only - no `Principal` dimension in the key. The handler reads
/// an identity (attaching `ClassificationReason::PrincipalObserved`) and
/// then calls the test-only seam. Without the fix, stripping the *real*
/// classification also blanks `key_used_different_values_than_the_render_saw`'s
/// input, and R89 exempts the declared-`PrivateCached` class from the
/// invariant, so the two combine into a cross-user serve. Paired with
/// `SEAM_CONTROL_PATH`, identical except it never calls the seam, to prove
/// the value guard alone already declines this shape.
pub const SEAM_LEAK_PATH: &str = "/seam-leak";
/// See `SEAM_LEAK_PATH`'s own doc.
pub const SEAM_CONTROL_PATH: &str = "/seam-control";

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
            PRIVATE_DOCUMENT_PATH | RAW_PATH | CAPTURE_PATH | UNCACHED_CAPTURE_PATH => {
                PRIVATE_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            UNREASONED_PATH => {
                UNREASONED_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            STRIP_PATH => {
                STRIP_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            SEAM_LEAK_PATH => {
                SEAM_LEAK_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            SEAM_CONTROL_PATH => {
                SEAM_CONTROL_RENDERS.fetch_add(1, Ordering::SeqCst);
            }
            FAILED_MOUNT_PATH => {
                FAILED_MOUNT_RENDERS.fetch_add(1, Ordering::SeqCst);
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
static SEAM_LEAK_RENDERS: AtomicUsize = AtomicUsize::new(0);
static SEAM_CONTROL_RENDERS: AtomicUsize = AtomicUsize::new(0);
static FAILED_MOUNT_RENDERS: AtomicUsize = AtomicUsize::new(0);

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
    // `fixture` registers only `DogfoodCounter`; `CAPTURE_PATH` mounts its
    // own component, so this harness replaces the registry with one that
    // holds both. Every other route keeps the component it always had.
    suprnova::App::singleton(
        LiveRegistry::builder()
            .register::<DogfoodCounter>()
            .expect("register dogfood counter")
            .register::<CaptureCounter>()
            .expect("register capture counter")
            .build(),
    );

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
    // A policy at all is what puts `CAPTURE_PATH`'s handler inside a
    // collector scope, which is what makes `current_report()` answer there.
    let capture_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("capture identity bound policy");
    // R89's own shape: declared `PrivateCached` already requires `Principal`
    // or `Tenant` variance to build at all (Task 14 round 6), so this is a
    // legitimately cacheable route whose handler simply never happens to
    // read an identity.
    let unreasoned_policy = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("unreasoned private cached policy");
    // Finding 8's shape: declared `PublicShared` with `Principal` variance
    // declared. R90 means the seam only ever touches a *copy* used for the
    // invariant check, so the value guard sees the render's real,
    // unstripped `PrincipalObserved` reason - declaring `Principal`
    // satisfies that guard for two requests from the same signed-in user,
    // so it cannot be what declines the second one. Only the invariant,
    // seeing the copy narrowed to `PrivateCached` with the reason stripped
    // away, can still decline this shape; without its call site running,
    // nothing else would.
    let strip_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("strip classification reason policy");
    // Shared by SEAM_LEAK_PATH and SEAM_CONTROL_PATH: declared PrivateCached
    // (so R89 exempts it from the invariant) with only Tenant variance, so
    // a PrincipalObserved reason has no Principal dimension in the key.
    let seam_policy = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Tenant)
        .build()
        .expect("seam policy");
    // Declared `PublicShared` with `Principal` variance, exactly like
    // `capture_policy`: nothing about the policy declines this route, so
    // whether its render is stored is decided by the Live document facts
    // alone - which is the point of `FAILED_MOUNT_PATH`.
    let failed_mount_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("failed mount policy");

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
    let capture =
        LiveMount::<CaptureCounter>::identity_bound(CAPTURE_PATH, "counter", CAPTURE_DOCUMENT_KEY)
            .expect("declare capture identity-bound mount");
    let capture_handler = capture.clone();
    let router: Router = router
        .get(CAPTURE_PATH, move |request: Request| {
            let mount = capture_handler.clone();
            async move { render_capture_document(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .into();
    let router = router
        .try_live_mount(&capture)
        .expect("register capture identity-bound mount");
    // No `try_render_cache` for this one, deliberately: a route with no
    // policy is never wrapped in a collector scope, which is the state the
    // capture guards have to leave the document unchanged in.
    let uncached = LiveMount::<CaptureCounter>::identity_bound(
        UNCACHED_CAPTURE_PATH,
        "counter",
        UNCACHED_CAPTURE_DOCUMENT_KEY,
    )
    .expect("declare uncached capture identity-bound mount");
    let uncached_handler = uncached.clone();
    let router: Router = router
        .get(UNCACHED_CAPTURE_PATH, move |request: Request| {
            let mount = uncached_handler.clone();
            async move { render_capture_document(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .into();
    let router = router
        .try_live_mount(&uncached)
        .expect("register uncached capture identity-bound mount");
    // No `AuthMiddleware` here, deliberately, and that is the whole
    // mechanism: without one nothing records principal evidence on the
    // request, so `mount_private_component` refuses the identity-bound
    // mount and the handler takes its failure branch.
    let failed_mount = LiveMount::<CaptureCounter>::identity_bound(
        FAILED_MOUNT_PATH,
        "counter",
        FAILED_MOUNT_DOCUMENT_KEY,
    )
    .expect("declare failed-mount identity-bound mount");
    let failed_mount_handler = failed_mount.clone();
    let router: Router = router
        .get(FAILED_MOUNT_PATH, move |request: Request| {
            let mount = failed_mount_handler.clone();
            async move { render_failed_mount_document(request, mount).await }
        })
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .into();
    let router = router
        .try_live_mount(&failed_mount)
        .expect("register failed-mount identity-bound mount");
    let router: Router = router.get(UNREASONED_PATH, unreasoned_handler).into();
    // Gated with their handlers, for the reason `strip_handler` records.
    // The policies below stay attached unconditionally: a policy attached
    // to a pattern no route serves is inert.
    #[cfg(feature = "testing")]
    let router: Router = router.get(STRIP_PATH, strip_handler).into();
    #[cfg(feature = "testing")]
    let router: Router = router.get(SEAM_LEAK_PATH, seam_leak_handler).into();
    let router: Router = router.get(SEAM_CONTROL_PATH, seam_control_handler).into();
    let router = router
        .try_render_cache(DOCUMENT_PATH, public_policy)
        .expect("attach public seed render cache policy")
        .try_render_cache(PRIVATE_DOCUMENT_PATH, private_policy)
        .expect("attach identity bound render cache policy")
        .try_render_cache(RAW_PATH, raw_policy)
        .expect("attach raw render cache policy")
        .try_render_cache(CAPTURE_PATH, capture_policy)
        .expect("attach capture render cache policy")
        .try_render_cache(UNREASONED_PATH, unreasoned_policy)
        .expect("attach unreasoned private cached policy")
        .try_render_cache(STRIP_PATH, strip_policy)
        .expect("attach strip classification reason policy")
        .try_render_cache(SEAM_LEAK_PATH, seam_policy.clone())
        .expect("attach seam leak policy")
        .try_render_cache(SEAM_CONTROL_PATH, seam_policy)
        .expect("attach seam control policy")
        .try_render_cache(FAILED_MOUNT_PATH, failed_mount_policy)
        .expect("attach failed mount policy");

    let mut render_cache_config = RenderCacheConfig::from_env()
        .expect("the test environment configures a valid render cache")
        .with_clock_for_test(Arc::clone(&clock) as Arc<dyn Clock>);
    render_cache_config.enabled = true;
    render_cache_config.l1 = suprnova::render_cache::L1Config::Disabled;
    // Pinned alongside the L1 tier, and for the same reason: an ambient
    // RENDER_CACHE_PROFILE or RENDER_CACHE_COORDINATOR must not change which
    // providers this suite installs.
    render_cache_config.coordinator = suprnova::render_cache::CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };

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
    SEAM_LEAK_RENDERS.store(0, Ordering::SeqCst);
    SEAM_CONTROL_RENDERS.store(0, Ordering::SeqCst);
    FAILED_MOUNT_RENDERS.store(0, Ordering::SeqCst);
    *LAST_REPORT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    *LAST_FAILED_MOUNT_REPORT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
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

/// `CAPTURE_PATH`'s handler, and `UNCACHED_CAPTURE_PATH`'s: the same
/// whole-document render `live_dogfood_support` performs for
/// `PRIVATE_DOCUMENT_PATH`, plus one extra step - the collector report is
/// stored into [`last_report`] right after `render` returns, while every
/// mount-time fact and the rendered document's digest are already recorded
/// and the scope is still open. On `UNCACHED_CAPTURE_PATH` there is no
/// scope, so the stored report is `None`.
async fn render_capture_document(
    request: Request,
    mount: LiveMount<CaptureCounter>,
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
            .bootstrap(LiveBootstrapOptions::esm().with_nonce(CAPTURE_NONCE))
            .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
        let response = document
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
            .map_err(FrameworkError::from)?;
        *LAST_REPORT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = collector::current_report();
        Ok(response)
    }
    .await;
    result.map_err(|error| HttpResponse::text(format!("Live document failed: {error}")).status(500))
}

/// `FAILED_MOUNT_PATH`'s handler: attempts an identity-bound mount that is
/// expected to fail, then answers `200` with a document built by hand,
/// carrying [`FAILED_MOUNT_FALLBACK`] and no island markup at all.
///
/// A mount that unexpectedly succeeds is reported as a `500` rather than
/// quietly rendering something else: the route exists only to produce the
/// failing path, and a test that read `200` from a successful mount would
/// be asserting about a different path than the one it names.
///
/// The report is stored while the collector scope is still open, the same
/// way `render_capture_document` stores its own, so a test can see that
/// `record_mount` never ran.
async fn render_failed_mount_document(
    request: Request,
    mount: LiveMount<CaptureCounter>,
) -> Result<HttpResponse, HttpResponse> {
    let mut document = LiveDocument::from_request(&request)
        .map_err(|error| HttpResponse::text(format!("from_request {error}")).status(500))?;
    if document
        .mount(
            &mount,
            CanonicalValue::Object(BTreeMap::new()),
            MountFlags::empty(),
        )
        .await
        .is_ok()
    {
        return Err(
            HttpResponse::text("the identity-bound mount was expected to fail").status(500),
        );
    }
    *LAST_FAILED_MOUNT_REPORT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = collector::current_report();
    Ok(HttpResponse::html(format!(
        "<!doctype html><html><body><p>{FAILED_MOUNT_FALLBACK}</p></body></html>"
    )))
}

/// R89's shape: reads no identity at all. `UNREASONED_PATH`'s policy
/// already declares `Principal` variance, so `key_input` derives an
/// identity-scoped key without this handler's help.
async fn unreasoned_handler(_request: Request) -> Response {
    Ok(HttpResponse::html("unreasoned private cached"))
}

/// Finding 8's shape: reads an identity (attaching
/// `ClassificationReason::PrincipalObserved` inside `classify`), then calls
/// the test-only seam, which strips that reason only from the copy
/// `is_unreasoned_private_class` checks (R90) - the real classification the
/// value guard and `entry_header` see keeps the reason.
/// Gated on `testing`: `strip_classification_reasons_for_test` is
/// `#[cfg(any(test, feature = "testing"))]` in the framework, and the
/// minimal profile checked by `scripts/check-feature-matrix.sh` leaves that
/// feature off. The gate is at item level because this support module is
/// shared with test modules that need none of it.
#[cfg(feature = "testing")]
async fn strip_handler(_request: Request) -> Response {
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    suprnova::render_cache::collector::strip_classification_reasons_for_test();
    Ok(HttpResponse::html(format!(
        "strip classification reason render for {identity}"
    )))
}

/// R90's own regression shape: reads an identity and calls the seam, on a
/// route whose key does not carry `Principal` material. If the seam ever
/// touched the real classification again, this would store under a key
/// that does not partition by principal and serve `user-9` the body
/// rendered for `user-7`.
#[cfg(feature = "testing")]
async fn seam_leak_handler(_request: Request) -> Response {
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    suprnova::render_cache::collector::strip_classification_reasons_for_test();
    Ok(HttpResponse::html(format!(
        "seam leak render for {identity}"
    )))
}

/// Identical to `seam_leak_handler` except it never calls the seam: proves
/// the value guard alone already declines this shape (a `PrincipalObserved`
/// reason with no `Principal` dimension in the key), so `SEAM_LEAK_PATH`'s
/// behavior is genuinely attributable to the seam and not to some other
/// difference between the two routes.
async fn seam_control_handler(_request: Request) -> Response {
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "seam control render for {identity}"
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

/// Renders reaching `SEAM_LEAK_PATH`'s handler so far.
pub fn seam_leak_renders() -> usize {
    SEAM_LEAK_RENDERS.load(Ordering::SeqCst)
}

/// Renders reaching `SEAM_CONTROL_PATH`'s handler so far.
pub fn seam_control_renders() -> usize {
    SEAM_CONTROL_RENDERS.load(Ordering::SeqCst)
}

/// Renders reaching `FAILED_MOUNT_PATH`'s handler so far.
pub fn failed_mount_renders() -> usize {
    FAILED_MOUNT_RENDERS.load(Ordering::SeqCst)
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
