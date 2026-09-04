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
use suprnova::auth::{Authenticatable, Guard, SessionGuard, UserProvider};
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::registry::GroupPolicy;
use suprnova::render_cache::{
    FreshnessPolicy, QueryPolicy, RenderCache, RenderCachePolicy, RepresentationClass,
    VarianceDimension,
};
use suprnova::testing::TestContainer;
use suprnova::{
    App, Auth, ConnectionTrait, Crypt, EncryptionKey, FrameworkError, HttpResponse, Lang, Locale,
    MiddlewareRegistry, Model, Next, Request, Response, Router, attrs, handle_request,
    scope_locale,
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

/// Resolves the Live tenant from an `x-test-tenant` header, for fix round
/// 4's tenant-partitioning tests. Wired through the real
/// `suprnova::live::LiveTenantMiddleware`, exactly like a production tenant
/// resolver, rather than setting `Request::live_tenant` directly - that
/// setter is crate-private, reachable only through this middleware.
pub struct TestTenantResolver;

#[async_trait]
impl suprnova::live::LiveTenantResolver for TestTenantResolver {
    async fn resolve(&self, request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(request.header("x-test-tenant").map(str::to_owned))
    }
}

/// Installs a per-request locale scope starting at `"en"`, the same job
/// the real `LocaleMiddleware` does (via `scope_locale`) once a translator
/// is bound - this harness has none, so it calls `scope_locale` directly
/// instead, matching that function's own doc ("tests... can use it
/// directly"). Registered before `RenderCache::install`, for the same
/// ordering reason as `LoginHeader`.
pub struct TestLocaleMiddleware;

#[async_trait]
impl suprnova::Middleware for TestLocaleMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        scope_locale(
            Locale::parse("en").expect("en is a valid locale"),
            next(request),
        )
        .await
    }
}

/// Fix round 5, Leak 1 (first reproduction): stands in for a per-route
/// impersonation middleware, which the framework explicitly supports.
/// Registered *after* `RenderCache::install` (see [`boot`]'s own comment at
/// the registration site), so it runs after `RenderCacheMiddleware` in the
/// chain and therefore after the key has already been derived from
/// whatever `LoginHeader` established - exactly the shape the reviewer
/// proved over real HTTP.
pub struct ImpersonationMiddleware;

#[async_trait]
impl suprnova::Middleware for ImpersonationMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(target) = request.header("x-test-impersonate") {
            Auth::set_user(Arc::new(Principal(target.to_owned())));
        }
        next(request).await
    }
}

/// A `UserProvider` whose `retrieve_by_id` is never actually exercised in
/// the fix round 4 Leak B reproduction: the test sets the named guard's
/// user directly via `set_user`, which the guard's own per-request cache
/// (`request_state::guard_user`) serves back without a provider lookup.
struct NamedGuardDummyProvider;

#[async_trait]
impl UserProvider for NamedGuardDummyProvider {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

/// The name a fix round 4 Leak B route resolves its identity through - a
/// guard other than the configured default - so `Auth::id()` (the default
/// guard's own slot) never reflects an identity this middleware sets.
const NAMED_GUARD: &str = "admin-guard-round-4";

/// Stands in for a non-default guard's own sign-in, the same shape
/// [`LoginHeader`] provides for the default guard: a request carrying
/// `x-test-named-login: <id>` is signed in on [`NAMED_GUARD`] specifically.
/// `SessionGuard::set_user` mirrors into the generic `Auth`-facade slot
/// only when the guard's name matches the configured default guard (see
/// `auth::request_state::set_guard_user`'s own doc), so `Auth::id()` stays
/// `None` for a request that only this middleware touched - exactly the
/// shape that defeated round 3's re-read-based classification (fix round
/// 4, Leak B).
pub struct NamedGuardLoginHeader;

#[async_trait]
impl suprnova::Middleware for NamedGuardLoginHeader {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(id) = request.header("x-test-named-login") {
            let guard = SessionGuard::named(NAMED_GUARD, Arc::new(NamedGuardDummyProvider));
            guard.set_user(Arc::new(Principal(id.to_owned()))).await;
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
    /// Held only for its `Drop` (removes the directory on disk); `None`
    /// unless booted through [`boot_with_render_cache_and_l1_for_test`].
    _l1_tempdir: Option<tempfile::TempDir>,
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
    boot(true, false).await
}

/// Test-only for the fix round 1, item 2 regression test: boots exactly
/// like [`boot_with_render_cache`] but does **not** clear the global
/// middleware registry first, so a caller can register its own marker
/// middleware beforehand and then observe whether `RenderCache::install`
/// preserved it. Production `install` never clears the registry (see its
/// own doc); this seam exists only so a test can arrange "an application
/// already registered its own middleware" without also fighting this
/// harness's own test-isolation clear.
pub async fn boot_with_render_cache_preserving_global_middleware_for_test() -> Arc<Harness> {
    boot(false, false).await
}

/// Test-only for fix round 2, item 5: boots exactly like
/// [`boot_with_render_cache`], except the runtime is configured with a real
/// file-backed L1 provider (a fresh temp directory) and an L0 capped at a
/// single entry, so a second publish deterministically evicts the first from
/// L0 while L1 - sized generously - keeps both. `/l1-cached/{id}` is the only
/// route registered with `StorageLayers::l0_and_l1()`; every other route in
/// this harness stays L0-only, matching every other test in this file, so
/// this is the first and only place L1 actually runs together with the
/// middleware.
pub async fn boot_with_render_cache_and_l1_for_test() -> Arc<Harness> {
    boot(true, true).await
}

async fn boot(clear_global_middleware: bool, l1_enabled: bool) -> Arc<Harness> {
    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    counting_route::reset();
    if clear_global_middleware {
        suprnova::middleware::clear_global_middleware_for_test();
    }

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
    // Fix round 1, item 1: deliberately declares no `Principal` variance,
    // matching the reviewer's proven shape exactly.
    let leaky_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("leaky policy");
    // Fix round 2, item 4: `Principal` variance with real stale windows.
    // Deliberately `PublicShared`, not `PrivateCached` like `/private/{id}`
    // above: `evaluate_freshness` never serves a `PrivateCached` entry
    // stale at all (see `stale_service_is_policy_driven_bounded_and_never_private`),
    // which would make this route unable to reach StaleServable and so
    // unable to exercise the background-rebuild skip this policy exists to
    // test. Paired with `cached_handler` below rather than `private_handler`:
    // a handler that itself reads `Auth::id()` would make `classify` narrow
    // the *served* class to `PrivateCached` regardless of what this policy
    // declares (see `variance::classify`), defeating the point of choosing
    // `PublicShared` here. Declaring `Principal` variance is enough on its
    // own to make `key_input` derive an identity-scoped key - the render
    // itself does not need to read the identity for that to happen.
    let stale_principal_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 60_000, 120_000).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("stale principal policy");
    // Fix round 2, item 6: the only route in this harness using
    // `CoherenceMode::Lease` rather than the default `Authority`.
    let leased_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .coherence(suprnova::render_cache::CoherenceMode::Lease { max_age_ms: 60_000 })
        .build()
        .expect("leased policy");
    // Fix round 2, item 5: the only route in this harness declaring
    // `StorageLayers::l0_and_l1()` - every other policy above defaults to
    // L0-only, so this is the one that actually exercises L1 together with
    // the middleware when booted through `boot_with_render_cache_and_l1_for_test`.
    let l1_cached_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .layers(suprnova::render_cache::StorageLayers::l0_and_l1())
        .build()
        .expect("l1 cached policy");
    // Fix round 3, item 1: same shape as `leaky_policy` - no declared
    // `Principal` variance - paired with a handler that reads identity
    // through a different, previously-uninstrumented accessor.
    let leaky_via_request_state_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .build()
            .expect("leaky via request_state policy");
    // Fix round 3, item 2: no declared variance at all - the shape the
    // reviewer's `Gate::allows`-driven attack needs, since the point is
    // that nothing partitions the key by which role was checked.
    let authz_driven_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("authz driven policy");
    // Fix round 4, Leak B: no declared variance at all, matching `/leaky`'s
    // shape - the point is that classification must narrow (and this guard
    // must then decline, since nothing partitions) regardless of which
    // accessor observed the identity.
    let leaky_via_named_guard_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .build()
            .expect("leaky via named guard policy");
    // Fix round 4, Leak C: no declared variance; the point is that a
    // `session_mut` read alone must force Uncacheable.
    let session_mut_reading_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("session mut reading policy");
    // Fix round 5, Leak 3: declares Locale correctly - the point is that a
    // mid-render `Lang::set_locale` call must still be caught even though
    // the declared dimension matches what a render *usually* uses.
    let locale_declared_switches_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Locale)
            .build()
            .expect("locale declared switches policy");
    // Fix round 5, Leak 2: no declared variance; the point is that a
    // cookie read alone must force Uncacheable.
    let cookie_reading_policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("cookie reading policy");
    // Fix round 4: one pair of policies per classification reason - the
    // wrong dimension declared for that reason, and the matching one -
    // parameterising the leak shape instead of pinning it to one remembered
    // route. `PrincipalObserved`'s pair:
    let tenant_declared_reads_principal_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Tenant)
            .build()
            .expect("tenant declared reads principal policy");
    let principal_declared_reads_principal_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Principal)
            .build()
            .expect("principal declared reads principal policy");
    // `TenantObserved`'s pair:
    let principal_declared_reads_tenant_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Principal)
            .build()
            .expect("principal declared reads tenant policy");
    let tenant_declared_reads_tenant_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Tenant)
            .build()
            .expect("tenant declared reads tenant policy");
    // `AuthorizationRead`'s pair (requires `Principal`, per that reason's
    // own "the decision is per-user" rule, not `Tenant`):
    let tenant_declared_reads_authz_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Tenant)
            .build()
            .expect("tenant declared reads authz policy");
    let principal_declared_reads_authz_policy =
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Principal)
            .build()
            .expect("principal declared reads authz policy");

    let router: Router = Router::new().get("/cached/{id}", cached_handler).into();
    let router: Router = router.get("/stale/{id}", stale_handler).into();
    let router: Router = router.get("/private/{id}", private_handler).into();
    let router: Router = router.get("/sets-cookie", sets_cookie_handler).into();
    let router: Router = router.get("/overflow", overflow_handler).into();
    let router: Router = router.get("/leaky", leaky_handler).into();
    let router: Router = router.get("/stale-principal/{id}", cached_handler).into();
    let router: Router = router.get("/leased/{id}", cached_handler).into();
    let router: Router = router.get("/l1-cached/{id}", cached_handler).into();
    let router: Router = router
        .get("/leaky-via-request-state", leaky_handler_via_request_state)
        .into();
    let router: Router = router.get("/authz-driven", authz_driven_handler).into();
    let router: Router = router
        .get(
            "/tenant-declared-reads-principal/{id}",
            reads_principal_leaky_handler,
        )
        .into();
    let router: Router = router
        .get(
            "/principal-declared-reads-principal/{id}",
            reads_principal_leaky_handler,
        )
        .into();
    let router: Router = router
        .get(
            "/principal-declared-reads-tenant/{id}",
            reads_tenant_leaky_handler,
        )
        .into();
    let router: Router = router
        .get(
            "/tenant-declared-reads-tenant/{id}",
            reads_tenant_leaky_handler,
        )
        .into();
    let router: Router = router
        .get(
            "/tenant-declared-reads-authz/{id}",
            reads_authz_by_principal_leaky_handler,
        )
        .into();
    let router: Router = router
        .get(
            "/principal-declared-reads-authz/{id}",
            reads_authz_by_principal_leaky_handler,
        )
        .into();
    let router: Router = router
        .get(
            "/leaky-via-named-guard",
            reads_via_named_guard_leaky_handler,
        )
        .into();
    let router: Router = router
        .get("/session-mut-reading", session_mut_reading_handler)
        .into();
    let router: Router = router
        .get(
            "/locale-declared-switches-mid-render/{id}",
            locale_switching_handler,
        )
        .into();
    let router: Router = router.get("/cookie-reading", cookie_reading_handler).into();
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
        .expect("attach overflow policy")
        .try_render_cache("/leaky", GroupPolicy::from(leaky_policy))
        .expect("attach leaky policy")
        .try_render_cache(
            "/stale-principal/{id}",
            GroupPolicy::from(stale_principal_policy),
        )
        .expect("attach stale principal policy")
        .try_render_cache("/leased/{id}", GroupPolicy::from(leased_policy))
        .expect("attach leased policy")
        .try_render_cache("/l1-cached/{id}", GroupPolicy::from(l1_cached_policy))
        .expect("attach l1 cached policy")
        .try_render_cache(
            "/leaky-via-request-state",
            GroupPolicy::from(leaky_via_request_state_policy),
        )
        .expect("attach leaky via request_state policy")
        .try_render_cache("/authz-driven", GroupPolicy::from(authz_driven_policy))
        .expect("attach authz driven policy")
        .try_render_cache(
            "/tenant-declared-reads-principal/{id}",
            GroupPolicy::from(tenant_declared_reads_principal_policy),
        )
        .expect("attach tenant declared reads principal policy")
        .try_render_cache(
            "/principal-declared-reads-principal/{id}",
            GroupPolicy::from(principal_declared_reads_principal_policy),
        )
        .expect("attach principal declared reads principal policy")
        .try_render_cache(
            "/principal-declared-reads-tenant/{id}",
            GroupPolicy::from(principal_declared_reads_tenant_policy),
        )
        .expect("attach principal declared reads tenant policy")
        .try_render_cache(
            "/tenant-declared-reads-tenant/{id}",
            GroupPolicy::from(tenant_declared_reads_tenant_policy),
        )
        .expect("attach tenant declared reads tenant policy")
        .try_render_cache(
            "/tenant-declared-reads-authz/{id}",
            GroupPolicy::from(tenant_declared_reads_authz_policy),
        )
        .expect("attach tenant declared reads authz policy")
        .try_render_cache(
            "/principal-declared-reads-authz/{id}",
            GroupPolicy::from(principal_declared_reads_authz_policy),
        )
        .expect("attach principal declared reads authz policy")
        .try_render_cache(
            "/leaky-via-named-guard",
            GroupPolicy::from(leaky_via_named_guard_policy),
        )
        .expect("attach leaky via named guard policy")
        .try_render_cache(
            "/session-mut-reading",
            GroupPolicy::from(session_mut_reading_policy),
        )
        .expect("attach session mut reading policy")
        .try_render_cache(
            "/locale-declared-switches-mid-render/{id}",
            GroupPolicy::from(locale_declared_switches_policy),
        )
        .expect("attach locale declared switches policy")
        .try_render_cache("/cookie-reading", GroupPolicy::from(cookie_reading_policy))
        .expect("attach cookie reading policy");

    let config = RenderCacheConfig::from_env()
        .with_clock_for_test(Arc::clone(&clock) as Arc<dyn Clock>)
        .with_coordinator_for_test(Arc::clone(&waiting) as Arc<dyn RebuildCoordinator>);
    let mut config = config;
    config.enabled = true;
    let l1_tempdir = if l1_enabled {
        let dir = tempfile::tempdir().expect("l1 tempdir");
        config.l1 = suprnova::render_cache::L1Config::File {
            directory: dir.path().to_path_buf(),
            max_bytes: 16 * 1024 * 1024,
        };
        // Forces a second publish to evict the first from L0 (see this
        // function's own doc), while L1's byte budget above comfortably
        // holds both of this suite's tiny bodies.
        config.l0.max_entries = 1;
        Some(dir)
    } else {
        config.l1 = suprnova::render_cache::L1Config::Disabled;
        None
    };

    // Fix round 1, item 3: register the identity-establishing middleware
    // globally, and do it *before* `RenderCache::install`, so the ordering
    // matches production exactly - `RenderCache::install` appends to
    // whatever is already registered (see its own doc), never inserts at a
    // fixed position, so calling it after `LoginHeader` is what makes the
    // cache middleware see `Auth::id()` as `LoginHeader` set it, the same
    // way a real deployment's locale/session/auth middleware would have to
    // be registered before this call for the same reason. An earlier draft
    // built the registry with `MiddlewareRegistry::from_global().prepend(LoginHeader)`
    // instead - a *local* prepend that put `LoginHeader` first regardless of
    // global registration order, which is an ordering no production
    // deployment can produce and which would have hidden exactly the
    // ordering bug fix round 1 found.
    suprnova::middleware::register_global_middleware(LoginHeader);
    // Fix round 4, Leak B: the non-default-guard sign-in, same ordering
    // requirement as `LoginHeader` above.
    suprnova::middleware::register_global_middleware(NamedGuardLoginHeader);
    // Fix round 4: the tenant resolver, same ordering requirement as
    // `LoginHeader` above and for the same reason - `RenderCacheMiddleware`
    // reads `Request::live_tenant()` while building declared `Tenant`
    // variance, which is only meaningful once this has already run.
    suprnova::middleware::register_global_middleware(suprnova::live::LiveTenantMiddleware::new(
        Arc::new(TestTenantResolver),
    ));
    // Fix round 5: the per-request locale scope, same ordering requirement
    // as `LoginHeader` above - `RenderCacheMiddleware` reads `Lang::locale()`
    // while building declared `Locale` variance.
    suprnova::middleware::register_global_middleware(TestLocaleMiddleware);
    let router = RenderCache::install(router, config)
        .await
        .expect("install render cache");
    // Fix round 5, Leak 1 (first reproduction): registered *after*
    // `RenderCache::install`, so it runs *after* `RenderCacheMiddleware` in
    // the chain - `register_global_middleware` appends (see `install`'s own
    // doc), it never inserts at a fixed position. This is what makes it a
    // faithful stand-in for a per-route impersonation middleware, which the
    // framework explicitly supports and which necessarily runs closer to
    // the handler than a middleware registered globally before install.
    suprnova::middleware::register_global_middleware(ImpersonationMiddleware);

    let middleware = Arc::new(MiddlewareRegistry::from_global());

    Arc::new(Harness {
        router: Arc::new(router),
        middleware,
        clock,
        waiting,
        _conn: conn,
        _guard: guard,
        _tempdir: tempdir,
        _l1_tempdir: l1_tempdir,
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
    if counting_route::should_fail_next_render() {
        return Ok(HttpResponse::text("boom").status(500));
    }
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

/// Fix round 1, item 1: declares `PublicShared` with **no** `Principal`
/// variance, yet reads an identity held in `auth::request_state` (via
/// `Auth::id()`, the same mechanism `LoginHeader` writes through
/// `Auth::set_user` - bearer-token or remember-me shaped authentication,
/// not a session read, which would already force `Uncacheable` through
/// `session_read`). Without `key_omits_observed_privacy`, this is exactly
/// the shape that stores one identity's render under a principal-free key
/// and serves it back to a different identity.
async fn leaky_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!("leaky render for {identity}")))
}

/// Fix round 3, item 1: the reviewer's exact proof - same route, same
/// policy as [`leaky_handler`], one line changed: reads the identity
/// through `suprnova::auth_user_id()` (the seam `request_state::read_state`
/// now instruments) instead of `Auth::id()` (which always called
/// `observe_principal_read()` explicitly, even before this round).
async fn leaky_handler_via_request_state(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let identity = suprnova::auth_user_id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "leaky render via request_state for {identity}"
    )))
}

/// Fix round 3, item 2: drives the served body entirely from a `Gate::allows`
/// decision - `x-test-role: admin` gets a different body than any other
/// value - without reading `Auth::id`, `auth_user_id`, or any other
/// identity accessor. `Gate::inspect` (which `allows` routes through)
/// already calls `observe_authorization_read()`, narrowing the served class
/// to `PrivateCached` via `classify`'s `AuthorizationRead` reason - but the
/// route below declares no variance dimension at all, so nothing partitions
/// the key by which role was checked.
async fn authz_driven_handler(request: Request) -> Response {
    counting_route::on_render_start().await;
    let is_admin = request.header("x-test-role") == Some("admin");
    let allowed = suprnova::Gate::allows::<bool, bool>(ROUND3_AUTHZ_GATE, &is_admin, &true);
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!(
        "authz render {n}: allowed={allowed}"
    )))
}

/// Registered once per process by [`ensure_round3_authz_gate`]; action name
/// scoped to this fix round so it cannot collide with a gate any other test
/// file registers.
const ROUND3_AUTHZ_GATE: &str = "fix-round-3-item-2-authz-gate";

/// Registers [`ROUND3_AUTHZ_GATE`] exactly once for the process:
/// `Gate::allows` on an undefined gate always denies (see its own doc), so
/// [`authz_driven_handler`] needs this registered before it can produce a
/// body that actually varies with `is_admin`. `Gate`'s registry is
/// independent of this harness's own per-test reset, so registering once
/// per process (not per test) is correct and sufficient.
pub fn ensure_round3_authz_gate() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        suprnova::Gate::define::<bool, bool>(ROUND3_AUTHZ_GATE, |is_admin: &bool, _resource| {
            *is_admin
        });
    });
}

/// Fix round 4: reads identity through `Auth::id()` and includes it in the
/// body, used across three routes with different declared variance so the
/// same reason (`PrincipalObserved`) can be tested against a route that
/// declares the wrong dimension, the right one, and (for `AuthorizationRead`,
/// below) as the input to a per-user gate decision.
async fn reads_principal_leaky_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "principal-reading render for {identity}"
    )))
}

/// Fix round 4: reads the Live tenant through `Request::live_tenant()`
/// (which now records a `tenant_read` observation on every call) and
/// includes it in the body.
async fn reads_tenant_leaky_handler(request: Request) -> Response {
    counting_route::on_render_start().await;
    let tenant = request.live_tenant().unwrap_or("no-tenant").to_owned();
    Ok(HttpResponse::html(format!(
        "tenant-reading render for {tenant}"
    )))
}

/// Registered once per process by [`ensure_round4_per_user_authz_gate`].
/// Deliberately keyed by the caller's own id (a `String`), not a bare
/// `bool` like [`ROUND3_AUTHZ_GATE`]: the point of this gate is that the
/// decision genuinely varies *by principal* ("admin" allowed, anyone else
/// denied), which is what makes `Principal` the dimension `AuthorizationRead`
/// must require - not merely a role flag carried on the request.
const ROUND4_PER_USER_AUTHZ_GATE: &str = "fix-round-4-per-user-authz-gate";

/// Registers [`ROUND4_PER_USER_AUTHZ_GATE`] exactly once for the process.
/// See [`ensure_round3_authz_gate`]'s own doc for why once-per-process is
/// correct and sufficient here too.
pub fn ensure_round4_per_user_authz_gate() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        suprnova::Gate::define::<String, bool>(
            ROUND4_PER_USER_AUTHZ_GATE,
            |user: &String, _resource| user == "admin",
        );
    });
}

/// Fix round 4: drives the served body from a per-user `Gate::allows`
/// decision - reading identity through `Auth::id()` to decide the
/// decision, so `principal_read` is set (via `Auth::id()`'s own explicit
/// observation) *and* `authorization_read` is set (via `Gate::inspect`'s),
/// exactly the shape `AuthorizationRead` names `Principal` as the required
/// dimension for.
async fn reads_authz_by_principal_leaky_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let user = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    let allowed = suprnova::Gate::allows::<String, bool>(ROUND4_PER_USER_AUTHZ_GATE, &user, &true);
    Ok(HttpResponse::html(format!(
        "authz-by-principal render for {user}: allowed={allowed}"
    )))
}

/// Fix round 4, Leak B (proven): reads identity through the named,
/// non-default guard [`NAMED_GUARD`] rather than `Auth::id()`. Round 3's
/// seam makes `SessionGuard::id`'s underlying `guard_auth_user_id` read
/// record a `principal_read` observation regardless; the leak was that
/// classification then re-read `Auth::id()` specifically to build the
/// observed value, and that accessor returns `None` for an identity this
/// guard alone holds.
async fn reads_via_named_guard_leaky_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let guard = SessionGuard::named(NAMED_GUARD, Arc::new(NamedGuardDummyProvider));
    let identity = guard
        .id()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "named-guard render for {identity}"
    )))
}

/// Fix round 4, Leak C (proven): reads session state through `session_mut`
/// rather than `session()`. Before this round, `session_mut` recorded no
/// observation at all, so a render depending on session state through this
/// idiomatic read-and-mutate accessor was never forced `Uncacheable` the
/// way an equivalent `session()` read already was.
async fn session_mut_reading_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let _ = suprnova::session::session_mut(|session| session.get::<String>("anything"));
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!("session-mut render {n}")))
}

/// Fix round 5, Leak 2: reads a cookie and nothing else. Cookies produce no
/// `ClassificationReason` on their own; `Request::cookies` (which
/// `Request::cookie` delegates to) now records a session read instead,
/// treating a cookie read the same as a session read.
async fn cookie_reading_handler(request: Request) -> Response {
    counting_route::on_render_start().await;
    let _ = request.cookie("session");
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!("cookie render {n}")))
}

/// Fix round 5, Leak 3 (proven): the key is derived from `Lang::locale()`
/// before this handler runs; this then calls `Lang::set_locale`, which the
/// framework documents as supported mid-request, and renders in the new
/// locale. The key was already fixed at the old one.
async fn locale_switching_handler(_request: Request) -> Response {
    counting_route::on_render_start().await;
    let before = Lang::locale().as_str().to_owned();
    Lang::set_locale(Locale::parse("fr").expect("fr is a valid locale"));
    let after = Lang::locale().as_str().to_owned();
    let n = counting_route::renders();
    Ok(HttpResponse::html(format!(
        "locale render {n} before={before} after={after}"
    )))
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
    static FAIL_NEXT: AtomicBool = AtomicBool::new(false);

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
        FAIL_NEXT.store(false, Ordering::SeqCst);
    }

    /// Arms the next render to return a 500 instead of its ordinary body -
    /// the shape a handler-level failure takes, as opposed to a provider
    /// failure before the handler ever runs. Used to exercise
    /// stale-on-error for the failure mode it is actually named for. See
    /// fix round 2, item 3.
    pub fn fail_next_render(_harness: &super::Harness) {
        FAIL_NEXT.store(true, Ordering::SeqCst);
    }

    /// Consumes the arm-once flag set by [`fail_next_render`].
    pub(crate) fn should_fail_next_render() -> bool {
        FAIL_NEXT.swap(false, Ordering::SeqCst)
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

    /// Releases a render blocked by [`hold_next_render`].
    ///
    /// The blocked render must already be waiting on `release_notify()` by
    /// the time a caller reaches this - `notify_waiters` stores no permit,
    /// so a release that fires before the held render's own
    /// `.notified().await` call is registered is lost forever, and that
    /// render (and any singleflight waiter parked behind it) hangs with no
    /// CPU and no output, not a red test. That guarantee holds only when
    /// the caller waited on [`wait_until_rendering_count`] for the *correct*
    /// count first - see that function's own doc for why "any render has
    /// started" is not the same guarantee, and was the bug fix round 3,
    /// item 4 found and fixed.
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

    /// Waits until at least `n` renders have started (called
    /// [`on_render_start`]) since the harness booted. Race-free: the notify
    /// handle is captured before the condition is checked, so a
    /// notification that fires in between is never missed.
    ///
    /// Takes an explicit count, not "any render has started" (fix round 3,
    /// item 4): `RENDER_STARTED` is cumulative across the whole test, never
    /// reset between renders, so a caller that arms [`hold_next_render`]
    /// *after* an earlier render already ran must wait for the *next* one
    /// specifically - passing the count of renders that will have happened
    /// by the time the held one starts (prior renders, plus one). An
    /// earlier version of this function checked only `> 0`, which was
    /// already satisfied by a prior render before `hold_next_render` was
    /// even armed, so it returned immediately without the held render ever
    /// having started - and [`release_render`]'s guarantee, which depends
    /// on this having actually waited for it, did not hold. That produced a
    /// real, if intermittent, hang: three hangs in forty isolated runs of
    /// the singleflight test this exact race affected, per the fix round 3
    /// review, not a background-task capture artifact as an earlier version
    /// of this project's report claimed.
    pub async fn wait_until_rendering_count(_harness: &super::Harness, n: u64) {
        loop {
            let notified = rendering_notify().notified();
            if RENDER_STARTED.load(Ordering::SeqCst) >= n {
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
