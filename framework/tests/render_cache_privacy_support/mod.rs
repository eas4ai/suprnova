//! Shared boot for the Task 18 privacy-leak suite in
//! `render_cache_privacy.rs`.
//!
//! Deliberately self-contained and separate from
//! `render_cache_middleware_support`, for two reasons. The first is
//! ruling R91: Task 17 extends that module on a concurrent branch, and two
//! branches appending to one file conflict at merge. The second is the
//! point of this suite at all: every attack it replays was proven against
//! the middleware's own guard, and a suite that shared the guard's own
//! harness could be made green by a change to that harness rather than by
//! the guard actually holding. Nothing here imports from the middleware
//! suite; the routes, policies, middleware, and dispatch loop below are a
//! minimal re-derivation of the same shapes.
//!
//! Every route is registered under `/privacy/...` so nothing here can be
//! confused with a route of the same purpose in another suite.
//!
//! Two boots, because one attack is about middleware order:
//!
//! - [`boot_with_render_cache`] registers the identity, tenant, locale and
//!   feature middleware **before** `RenderCache::install`, which is the
//!   only ordering a real deployment can produce for a global middleware
//!   (`install` appends). [`ImpersonationMiddleware`] and
//!   [`LateLocaleMiddleware`] are registered **after** it, standing in for
//!   per-route middleware, which always compose closer to the handler than
//!   any global one.
//! - [`boot_with_cache_installed_before_the_auth_middleware`] does the
//!   opposite for the identity middleware alone, so `RenderCacheMiddleware`
//!   derives its key before any identity exists and a declared `Principal`
//!   dimension resolves `Anonymous` while the render observes a real
//!   principal.
#![allow(
    dead_code,
    reason = "the framework's test-support modules are shared by test binaries \
              that each use a subset of the harness; five of the nine \
              pre-existing ones carry a bare allow for the same reason, and \
              this one names it"
)]

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
    FreshnessPolicy, L1Config, RenderCache, RenderCachePolicy, RepresentationClass,
    VarianceDimension,
};
use suprnova::testing::TestContainer;
use suprnova::{
    App, Auth, ConnectionTrait, Crypt, EncryptionKey, FrameworkError, HttpResponse,
    MiddlewareRegistry, Next, Request, Response, Router, handle_request, scope_locale,
};
use suprnova::{Lang, Locale};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::identity::UnixMillis;

struct PrivacyMigrator;

#[async_trait::async_trait]
impl MigratorTrait for PrivacyMigrator {
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

/// Stands in for the application's sign-in on the default guard: a request
/// carrying `x-test-login: <id>` is that authenticated user for the rest of
/// the request.
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

/// The name of the non-default guard this suite signs in on. Scoped to this
/// file so it cannot collide with a guard another test binary registers.
const NAMED_GUARD: &str = "privacy-suite-admin-guard";

/// A `UserProvider` whose `retrieve_by_id` is never exercised: the named
/// guard's user is set directly through `set_user`, which the guard's own
/// per-request cache serves back without a provider lookup.
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

/// Signs a request in on [`NAMED_GUARD`] specifically, from
/// `x-test-named-login: <id>`. `SessionGuard::set_user` mirrors into the
/// generic `Auth` facade slot only for the configured default guard, so
/// `Auth::id()` stays `None` for a request only this middleware touched.
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

/// Resolves the Live tenant from `x-test-tenant`, through the real
/// `LiveTenantMiddleware` rather than by setting `Request::live_tenant`
/// directly (that setter is crate-private).
pub struct TestTenantResolver;

#[async_trait]
impl suprnova::live::LiveTenantResolver for TestTenantResolver {
    async fn resolve(&self, request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(request.header("x-test-tenant").map(str::to_owned))
    }
}

/// Opens the per-request locale scope, the job the real `LocaleMiddleware`
/// does once a translator is bound. Reads `x-test-locale` (default `en`) so
/// a test can drive two requests through two declared locales and observe
/// that the key genuinely partitions by locale. Registered before
/// `RenderCache::install`, so the scope is already open when
/// `RenderCacheMiddleware` reads `Lang::locale()` to build the key.
pub struct TestLocaleMiddleware;

#[async_trait]
impl suprnova::Middleware for TestLocaleMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let locale = request
            .header("x-test-locale")
            .and_then(|tag| Locale::parse(tag).ok())
            .unwrap_or_else(|| Locale::parse("en").expect("en is a valid locale"));
        scope_locale(locale, next(request)).await
    }
}

/// Stands in for a per-route impersonation middleware, which the framework
/// explicitly supports. Registered *after* `RenderCache::install`, so it
/// runs after `RenderCacheMiddleware` and therefore after the key has
/// already been derived from whatever [`LoginHeader`] established.
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

/// Stands in for a per-route locale middleware. A per-route middleware
/// always composes closer to the handler than a global one, so it always
/// runs after `RenderCacheMiddleware` no matter how it was registered, and
/// its `scope_locale` pops the instant its own `next(request)` resolves -
/// before any post-render re-read of the same task-local could look.
/// Gated on `x-test-late-locale` so it changes nothing for any other test.
pub struct LateLocaleMiddleware;

#[async_trait]
impl suprnova::Middleware for LateLocaleMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let Some(tag) = request.header("x-test-late-locale") else {
            return next(request).await;
        };
        let Ok(locale) = Locale::parse(tag) else {
            return next(request).await;
        };
        scope_locale(locale, next(request)).await
    }
}

/// A clock that never moves. Deliberately fixed and with no way to advance
/// it: every entry this suite publishes stays fresh for its whole window, so
/// a second render is always a guard decision and never an expiry. A test
/// that wanted to observe an expiry would be testing freshness, not privacy.
pub struct FixedTestClock {
    millis: AtomicU64,
}

impl FixedTestClock {
    fn new(start_ms: u64) -> Self {
        Self {
            millis: AtomicU64::new(start_ms),
        }
    }
}

impl Clock for FixedTestClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Ok(UnixMillis::new(self.millis.load(Ordering::SeqCst)))
    }
}

/// Render counting. Every proof in this suite rests on it: a leak is a
/// request that was served without its handler running, so the count is
/// what distinguishes "the guard declined" from "the guard published and
/// the next visitor got someone else's page".
pub mod counting_route {
    use super::{AtomicU64, Ordering};

    static RENDERS: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn reset() {
        RENDERS.store(0, Ordering::SeqCst);
    }

    /// Total number of times a handler in this file has actually run.
    pub fn renders() -> u64 {
        RENDERS.load(Ordering::SeqCst)
    }

    pub(crate) fn record() -> u64 {
        RENDERS.fetch_add(1, Ordering::SeqCst) + 1
    }
}

// ── Handlers ───────────────────────────────────────────────────────────

/// Touches nothing observable: no identity, no tenant, no locale, no
/// session, no cookie, no flag. The negative direction of the whole suite
/// depends on this staying that way.
async fn plain_handler(request: Request) -> Response {
    let n = counting_route::record();
    let id = request.param("id").unwrap_or("0");
    Ok(HttpResponse::html(format!("plain render {n} for {id}")))
}

/// Reads the identity through `Auth::id()` and puts it in the body.
async fn reads_auth_id_handler(_request: Request) -> Response {
    let n = counting_route::record();
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "auth-id render {n} for {identity}"
    )))
}

/// Reads the identity through the crate-root `suprnova::auth_user_id()`,
/// which consults request state before the session-backed path, so a
/// bearer-token or remember-me identity is read without `Auth::id()`'s own
/// explicit observation ever running.
async fn reads_crate_root_auth_user_id_handler(_request: Request) -> Response {
    let n = counting_route::record();
    let identity = suprnova::auth_user_id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "crate-root-auth-user-id render {n} for {identity}"
    )))
}

/// Drives the body entirely from a `Gate::allows` decision, touching no
/// identity accessor at all.
async fn authz_driven_handler(request: Request) -> Response {
    let n = counting_route::record();
    let is_admin = request.header("x-test-role") == Some("admin");
    let allowed = suprnova::Gate::allows::<bool, bool>(ROLE_GATE, &is_admin, &true);
    Ok(HttpResponse::html(format!(
        "authz render {n} allowed={allowed}"
    )))
}

/// Reads the tenant *and* the identity: a tenant-keyed route whose body
/// still varies per user inside one tenant.
async fn reads_tenant_and_identity_handler(request: Request) -> Response {
    let n = counting_route::record();
    let tenant = request.live_tenant().unwrap_or("no-tenant").to_owned();
    let identity = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "tenant-and-identity render {n} tenant={tenant} for {identity}"
    )))
}

/// Reads only the tenant, for the "a declared dimension actually
/// partitions" direction.
async fn reads_tenant_handler(request: Request) -> Response {
    let n = counting_route::record();
    let tenant = request.live_tenant().unwrap_or("no-tenant").to_owned();
    Ok(HttpResponse::html(format!(
        "tenant render {n} tenant={tenant}"
    )))
}

/// Reads the identity through the non-default [`NAMED_GUARD`] only.
/// `Auth::id()` yields nothing for such a request, so the key's `Principal`
/// dimension resolves `Anonymous` while the body is a specific person's.
async fn reads_named_guard_handler(_request: Request) -> Response {
    let n = counting_route::record();
    let guard = SessionGuard::named(NAMED_GUARD, Arc::new(NamedGuardDummyProvider));
    let identity = guard
        .id()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "named-guard render {n} for {identity}"
    )))
}

/// Builds the body from the named guard's identity and *then* touches the
/// default accessor for an unrelated check whose result the body ignores.
/// A record that keeps one slot per dimension keeps only the second value,
/// which is the one the key was built from, so the comparison passes while
/// the body came from the first.
async fn reads_named_guard_then_touches_default_handler(_request: Request) -> Response {
    let n = counting_route::record();
    let guard = SessionGuard::named(NAMED_GUARD, Arc::new(NamedGuardDummyProvider));
    let named_identity = guard
        .id()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "anonymous".to_owned());
    // The unrelated later touch: an audit or feature check whose own
    // result this body never uses.
    let _ = Auth::id();
    Ok(HttpResponse::html(format!(
        "named-then-default render {n} for {named_identity}"
    )))
}

/// Reads session state through `session_mut`, the idiomatic read-and-touch
/// accessor, rather than through `session()`.
async fn reads_session_mut_handler(_request: Request) -> Response {
    let n = counting_route::record();
    let _ = suprnova::session::session_mut(|session| session.get::<String>("anything"));
    Ok(HttpResponse::html(format!("session-mut render {n}")))
}

/// Reads a cookie and nothing else. A cookie read produces no
/// classification reason of its own; it has to be counted as a session
/// read or it costs the guard nothing.
async fn reads_cookie_handler(request: Request) -> Response {
    let n = counting_route::record();
    let _ = request.cookie("session");
    Ok(HttpResponse::html(format!("cookie render {n}")))
}

/// Switches the locale mid-render through `Lang::set_locale`, which the
/// framework documents as supported, after the key has already been fixed
/// at the pre-switch locale. The target comes from a header so two
/// requests that derive the *same* key render two different bodies.
async fn locale_switching_handler(request: Request) -> Response {
    let n = counting_route::record();
    let before = Lang::locale().as_str();
    if let Some(locale) = request
        .header("x-test-switch-to")
        .and_then(|tag| Locale::parse(tag).ok())
    {
        Lang::set_locale(locale);
    }
    let after = Lang::locale().as_str();
    Ok(HttpResponse::html(format!(
        "locale-switch render {n} before={before} after={after}"
    )))
}

/// Renders the whole body inside a *nested* `scope_locale`, the framework's
/// own documented API for a mid-render locale switch. The nested scope pops
/// the instant its future resolves, before the handler returns, so nothing
/// outside it can re-read what the body was rendered in.
async fn nested_scope_locale_handler(request: Request) -> Response {
    let n = counting_route::record();
    let target = request
        .header("x-test-nested-locale")
        .and_then(|tag| Locale::parse(tag).ok())
        .unwrap_or_else(|| Locale::parse("en").expect("en is a valid locale"));
    let body = scope_locale(target, async move {
        format!("nested-scope render {n} locale={}", Lang::locale().as_str())
    })
    .await;
    Ok(HttpResponse::html(body))
}

/// Reads `Lang::locale()` plainly. [`LateLocaleMiddleware`] is what
/// actually supplies the switched locale, in a scope that pops before its
/// own `next(request)` returns.
async fn reads_locale_handler(_request: Request) -> Response {
    let n = counting_route::record();
    let locale = Lang::locale().as_str();
    Ok(HttpResponse::html(format!(
        "locale render {n} locale={locale}"
    )))
}

/// Reads a **user-scoped** feature flag ambiently through `is_enabled!`.
/// `FeatureMiddleware` resolved the identity into the featureflag context
/// before the render started, so the render itself touches no instrumented
/// identity accessor: the evaluator's own read is the only place the
/// dependency can be seen.
async fn reads_user_scoped_flag_handler(_request: Request) -> Response {
    let n = counting_route::record();
    // The literal is repeated rather than referenced through
    // [`USER_SCOPED_FLAG`] because `is_enabled!` matches a string literal,
    // not an expression. A divergence between the two would show up
    // immediately as the flag's sanity assertion in the test (alice gets
    // `enabled=true`) failing.
    let enabled = suprnova::is_enabled!("privacy-user-scoped-flag", false);
    Ok(HttpResponse::html(format!(
        "user-scoped-flag render {n} enabled={enabled}"
    )))
}

/// Reads a **globally** scoped flag: the same answer for every visitor, no
/// identity anywhere in the decision. Reading it must cost the cache
/// nothing, even for a signed-in visitor whose id `FeatureMiddleware` has
/// already put in the ambient context. R103's positive control for the two
/// flag leak tests: without it, a change that made every flag read
/// uncacheable would leave both of them green while disabling the cache for
/// every page in an application that checks any flag.
async fn reads_global_flag_handler(_request: Request) -> Response {
    let n = counting_route::record();
    // See [`reads_user_scoped_flag_handler`] for why the literal is
    // repeated here rather than referenced through [`GLOBAL_FLAG`].
    let enabled = suprnova::is_enabled!("privacy-global-flag", false);
    Ok(HttpResponse::html(format!(
        "global-flag render {n} enabled={enabled}"
    )))
}

/// Builds its body from `Request::auth_user_id()` - a `pub` accessor with no
/// collector instrumentation at all - alongside the instrumented one, so a
/// test can assert both halves of the claim the R79 sweep rests on: that the
/// field is stamped only on the WebSocket-upgrade path, and that the
/// identity the render actually sees is the observed one.
async fn reads_request_auth_user_id_handler(request: Request) -> Response {
    let n = counting_route::record();
    let stamped = request.auth_user_id().unwrap_or("none").to_owned();
    let observed = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
    Ok(HttpResponse::html(format!(
        "request-auth-user-id render {n} stamped={stamped} observed={observed}"
    )))
}

/// Reads a flag whose only identity rule belongs to *someone else*. A
/// reader who carries no id at all falls through to the global rule, gets
/// an answer the override's owner would not get, and must not publish that
/// answer under a key the owner also hits.
async fn reads_another_users_override_flag_handler(_request: Request) -> Response {
    let n = counting_route::record();
    // See [`reads_user_scoped_flag_handler`] for why the literal is
    // repeated here rather than referenced through [`OVERRIDE_FLAG`].
    let enabled = suprnova::is_enabled!("privacy-override-flag", false);
    Ok(HttpResponse::html(format!(
        "override-flag render {n} enabled={enabled}"
    )))
}

// ── Gates and flags ────────────────────────────────────────────────────

/// A gate keyed by a plain role flag, so a body can depend on an
/// authorization decision without any identity accessor being touched.
const ROLE_GATE: &str = "privacy-suite-role-gate";

/// Registers [`ROLE_GATE`] exactly once per process. `Gate::allows` on an
/// undefined ability always denies, so without this the handler's body
/// would not actually vary.
pub fn ensure_role_gate() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        suprnova::Gate::define::<bool, bool>(ROLE_GATE, |is_admin: &bool, _resource| *is_admin);
    });
}

/// A flag whose only rule is at one specific user: `alice` gets `true`,
/// everyone else falls through to no rule at all and takes the default.
const USER_SCOPED_FLAG: &str = "privacy-user-scoped-flag";

/// A flag with one global rule and no identity rule at all: its answer does
/// not depend on the reader, so reading it must narrow nothing.
const GLOBAL_FLAG: &str = "privacy-global-flag";

/// A flag with a global rule *and* an override belonging to `bob`. The
/// case that distinguishes "record by flag scope" from "record by the
/// scope key that happened to match this reader", and (for a reader with
/// no id) "record a bare read" from "record nothing".
const OVERRIDE_FLAG: &str = "privacy-override-flag";

/// The evaluator stack, installed process-globally exactly once, the way
/// `features::bootstrap_database_cached` does in a real application.
///
/// A [`CachedEvaluator`](suprnova::features::CachedEvaluator) in front of a
/// [`DatabaseEvaluator`](suprnova::features::DatabaseEvaluator) deliberately,
/// so one installed stack exercises both halves of the feature-flag attack:
/// the miss path reaches the database evaluator's own identity record, and
/// the second read of the same flag by the same context is a cache hit that
/// never reaches it and must replay what the miss consulted.
static FEATURE_EVALUATOR: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn install_feature_evaluator() {
    FEATURE_EVALUATOR
        .get_or_init(|| async {
            let database = suprnova::features::DatabaseEvaluator::new_in_memory()
                .await
                .expect("in-memory feature evaluator");
            database
                .set_flag(USER_SCOPED_FLAG, "user:alice", true)
                .await
                .expect("seed the user-scoped flag");
            database
                .set_flag(GLOBAL_FLAG, "", true)
                .await
                .expect("seed the globally scoped flag");
            database
                .set_flag(OVERRIDE_FLAG, "", false)
                .await
                .expect("seed the global rule of the override flag");
            database
                .set_flag(OVERRIDE_FLAG, "user:bob", true)
                .await
                .expect("seed bob's override");
            let cached = suprnova::features::CachedEvaluator::new(
                Arc::new(database),
                // Far longer than any test in this file runs, so a second
                // read of one flag by one context is always a hit and never
                // an expiry.
                Duration::from_secs(3_600),
            );
            suprnova::features::install_evaluator(Arc::new(cached));
        })
        .await;
}

// ── Harness ────────────────────────────────────────────────────────────

/// Everything one test needs: the router and middleware registry to
/// dispatch through, plus the clock `install` was configured with.
pub struct Harness {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    _conn: suprnova::database::DbConnection,
    _guard: suprnova::testing::TestContainerGuard,
    _tempdir: tempfile::TempDir,
}

/// The ordinary boot: every identity, tenant, locale and feature
/// middleware registered before `RenderCache::install`, impersonation and
/// the per-route locale middleware after it.
pub async fn boot_with_render_cache() -> Arc<Harness> {
    boot(true).await
}

/// The same routes and policies, with the identity middleware registered
/// **after** `RenderCache::install`, so `RenderCacheMiddleware` derives its
/// key before any identity exists. A route declaring `Principal` then
/// resolves that dimension to `Anonymous` and partitions nothing, while the
/// render goes on to observe a real principal.
pub async fn boot_with_cache_installed_before_the_auth_middleware() -> Arc<Harness> {
    boot(false).await
}

async fn boot(auth_before_install: bool) -> Arc<Harness> {
    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    counting_route::reset();
    suprnova::middleware::clear_global_middleware_for_test();

    let guard = TestContainer::fake();
    let tempdir = tempfile::tempdir().expect("tempdir for render cache privacy test database");
    let db_path = tempdir.path().join("render-cache-privacy.sqlite3");
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
    PrivacyMigrator::up(conn.inner(), None)
        .await
        .expect("apply render cache migration");
    TestContainer::singleton(conn.clone());

    let clock = Arc::new(FixedTestClock::new(1_000_000));

    let no_variance = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("no variance policy");
    let principal_declared = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("principal declared policy");
    let tenant_declared = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Tenant)
        .build()
        .expect("tenant declared policy");
    let locale_declared = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Locale)
        .build()
        .expect("locale declared policy");
    let private_declared = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("private declared policy");

    let router: Router = Router::new().get(PLAIN_ROUTE, plain_handler).into();
    let router: Router = router
        .get(READS_AUTH_ID_ROUTE, reads_auth_id_handler)
        .into();
    let router: Router = router
        .get(
            READS_CRATE_ROOT_AUTH_USER_ID_ROUTE,
            reads_crate_root_auth_user_id_handler,
        )
        .into();
    let router: Router = router.get(AUTHZ_DRIVEN_ROUTE, authz_driven_handler).into();
    let router: Router = router
        .get(
            TENANT_DECLARED_READS_IDENTITY_ROUTE,
            reads_tenant_and_identity_handler,
        )
        .into();
    let router: Router = router.get(TENANT_VARIES_ROUTE, reads_tenant_handler).into();
    let router: Router = router
        .get(NAMED_GUARD_ONLY_ROUTE, reads_named_guard_handler)
        .into();
    let router: Router = router
        .get(
            NAMED_THEN_DEFAULT_ROUTE,
            reads_named_guard_then_touches_default_handler,
        )
        .into();
    let router: Router = router
        .get(READS_SESSION_MUT_ROUTE, reads_session_mut_handler)
        .into();
    let router: Router = router.get(READS_COOKIE_ROUTE, reads_cookie_handler).into();
    let router: Router = router.get(IMPERSONATED_ROUTE, reads_auth_id_handler).into();
    let router: Router = router
        .get(
            PRINCIPAL_DECLARED_READS_IDENTITY_ROUTE,
            reads_auth_id_handler,
        )
        .into();
    let router: Router = router.get(PRIVATE_ROUTE, reads_auth_id_handler).into();
    let router: Router = router
        .get(LOCALE_SWITCHES_ROUTE, locale_switching_handler)
        .into();
    let router: Router = router
        .get(LOCALE_NESTED_SCOPE_ROUTE, nested_scope_locale_handler)
        .into();
    let router: Router = router
        .get(LOCALE_LATE_MIDDLEWARE_ROUTE, reads_locale_handler)
        .into();
    let router: Router = router.get(LOCALE_VARIES_ROUTE, reads_locale_handler).into();
    let router: Router = router
        .get(UNDECLARED_LOCALE_ROUTE, reads_locale_handler)
        .into();
    let router: Router = router
        .get(READS_USER_SCOPED_FLAG_ROUTE, reads_user_scoped_flag_handler)
        .into();
    let router: Router = router
        .get(
            READS_OVERRIDE_FLAG_ROUTE,
            reads_another_users_override_flag_handler,
        )
        .into();
    let router: Router = router
        .get(READS_GLOBAL_FLAG_ROUTE, reads_global_flag_handler)
        .into();
    let router: Router = router
        .get(PRINCIPAL_DECLARED_AUTHZ_ROUTE, authz_driven_handler)
        .into();
    let router: Router = router
        .get(
            REQUEST_AUTH_USER_ID_ROUTE,
            reads_request_auth_user_id_handler,
        )
        .into();

    let router = router
        .try_render_cache(PLAIN_ROUTE, GroupPolicy::from(no_variance.clone()))
        .expect("attach plain policy")
        .try_render_cache(READS_AUTH_ID_ROUTE, GroupPolicy::from(no_variance.clone()))
        .expect("attach reads-auth-id policy")
        .try_render_cache(
            READS_CRATE_ROOT_AUTH_USER_ID_ROUTE,
            GroupPolicy::from(no_variance.clone()),
        )
        .expect("attach crate-root auth-user-id policy")
        .try_render_cache(AUTHZ_DRIVEN_ROUTE, GroupPolicy::from(no_variance.clone()))
        .expect("attach authz-driven policy")
        .try_render_cache(
            TENANT_DECLARED_READS_IDENTITY_ROUTE,
            GroupPolicy::from(tenant_declared.clone()),
        )
        .expect("attach tenant-declared reads-identity policy")
        .try_render_cache(TENANT_VARIES_ROUTE, GroupPolicy::from(tenant_declared))
        .expect("attach tenant-varies policy")
        .try_render_cache(
            NAMED_GUARD_ONLY_ROUTE,
            GroupPolicy::from(principal_declared.clone()),
        )
        .expect("attach named-guard-only policy")
        .try_render_cache(
            NAMED_THEN_DEFAULT_ROUTE,
            GroupPolicy::from(principal_declared.clone()),
        )
        .expect("attach named-then-default policy")
        .try_render_cache(
            READS_SESSION_MUT_ROUTE,
            GroupPolicy::from(no_variance.clone()),
        )
        .expect("attach session-mut policy")
        .try_render_cache(READS_COOKIE_ROUTE, GroupPolicy::from(no_variance.clone()))
        .expect("attach cookie policy")
        .try_render_cache(
            IMPERSONATED_ROUTE,
            GroupPolicy::from(principal_declared.clone()),
        )
        .expect("attach impersonated policy")
        .try_render_cache(
            PRINCIPAL_DECLARED_READS_IDENTITY_ROUTE,
            GroupPolicy::from(principal_declared.clone()),
        )
        .expect("attach principal-declared reads-identity policy")
        .try_render_cache(PRIVATE_ROUTE, GroupPolicy::from(private_declared))
        .expect("attach private policy")
        .try_render_cache(
            LOCALE_SWITCHES_ROUTE,
            GroupPolicy::from(locale_declared.clone()),
        )
        .expect("attach locale-switches policy")
        .try_render_cache(
            LOCALE_NESTED_SCOPE_ROUTE,
            GroupPolicy::from(locale_declared.clone()),
        )
        .expect("attach locale-nested-scope policy")
        .try_render_cache(
            LOCALE_LATE_MIDDLEWARE_ROUTE,
            GroupPolicy::from(locale_declared.clone()),
        )
        .expect("attach locale-late-middleware policy")
        .try_render_cache(LOCALE_VARIES_ROUTE, GroupPolicy::from(locale_declared))
        .expect("attach locale-varies policy")
        .try_render_cache(
            UNDECLARED_LOCALE_ROUTE,
            GroupPolicy::from(no_variance.clone()),
        )
        .expect("attach undeclared-locale policy")
        .try_render_cache(
            READS_USER_SCOPED_FLAG_ROUTE,
            GroupPolicy::from(no_variance.clone()),
        )
        .expect("attach user-scoped-flag policy")
        .try_render_cache(
            READS_OVERRIDE_FLAG_ROUTE,
            GroupPolicy::from(no_variance.clone()),
        )
        .expect("attach override-flag policy")
        .try_render_cache(READS_GLOBAL_FLAG_ROUTE, GroupPolicy::from(no_variance))
        .expect("attach global-flag policy")
        .try_render_cache(
            PRINCIPAL_DECLARED_AUTHZ_ROUTE,
            GroupPolicy::from(principal_declared.clone()),
        )
        .expect("attach principal-declared authz policy")
        .try_render_cache(
            REQUEST_AUTH_USER_ID_ROUTE,
            GroupPolicy::from(principal_declared),
        )
        .expect("attach request-auth-user-id policy");

    let mut config =
        RenderCacheConfig::from_env().with_clock_for_test(Arc::clone(&clock) as Arc<dyn Clock>);
    config.enabled = true;
    config.l1 = L1Config::Disabled;

    install_feature_evaluator().await;

    if auth_before_install {
        // The production ordering: `RenderCache::install` appends to the
        // global registry, so anything the middleware needs already
        // resolved (identity, tenant, locale, the feature context) has to
        // be registered before it.
        suprnova::middleware::register_global_middleware(LoginHeader);
    }
    suprnova::middleware::register_global_middleware(NamedGuardLoginHeader);
    suprnova::middleware::register_global_middleware(suprnova::live::LiveTenantMiddleware::new(
        Arc::new(TestTenantResolver),
    ));
    suprnova::middleware::register_global_middleware(TestLocaleMiddleware);
    suprnova::middleware::register_global_middleware(
        suprnova::features::FeatureMiddleware::new().with_team_from_header("x-test-team"),
    );
    let router = RenderCache::install(router, config)
        .await
        .expect("install render cache");
    if !auth_before_install {
        // The attacked ordering: the cache is installed first, so
        // `RenderCacheMiddleware` runs before any identity exists.
        suprnova::middleware::register_global_middleware(LoginHeader);
    }
    // Both stand in for per-route middleware, which always compose closer
    // to the handler than any global middleware and therefore always run
    // after `RenderCacheMiddleware` has derived the key.
    suprnova::middleware::register_global_middleware(ImpersonationMiddleware);
    suprnova::middleware::register_global_middleware(LateLocaleMiddleware);

    let middleware = Arc::new(MiddlewareRegistry::from_global());

    Arc::new(Harness {
        router: Arc::new(router),
        middleware,
        _conn: conn,
        _guard: guard,
        _tempdir: tempdir,
    })
}

/// Whether `pattern` resolves to a render-cache policy on the router this
/// harness installed (R103).
///
/// Every leak test that attacks a route which can never be stored - the
/// undeclared-dimension declines, and the session and cookie reads - asserts
/// this about its own route, because no behavioural signal can tell "under a
/// policy and correctly declining" from "never registered at all": both
/// return the raw handler response with no validators. Without it, deleting
/// one route's `.try_render_cache(...)` opt-in makes that route's leak test
/// assert nothing while staying green, which is exactly the vacuity the
/// review measured.
///
/// Reads the production policy table (`RenderCachePolicyTable::effective_policy`,
/// through the framework's own `render_cache::testing::policy_table` seam),
/// not a copy this module keeps.
///
/// Requires the route's own effective policy to carry a class other than
/// `Uncacheable` (final review, F13 / ruling R104): a future patch to
/// `Uncacheable` would otherwise keep the attachment assertion green while
/// the route could never store anything, which is the same vacuity from a
/// different direction.
pub fn route_is_under_a_policy(harness: &Harness, pattern: &str) -> bool {
    suprnova::render_cache::testing::policy_table(&harness.router)
        .effective_policy(pattern)
        .is_some_and(|policy| policy.class() != RepresentationClass::Uncacheable)
}

// ── Route patterns ─────────────────────────────────────────────────────

/// Declares nothing, observes nothing. The negative direction.
pub const PLAIN_ROUTE: &str = "/privacy/plain/{id}";
/// Declares nothing, reads `Auth::id()`.
pub const READS_AUTH_ID_ROUTE: &str = "/privacy/reads-auth-id";
/// Declares nothing, reads `suprnova::auth_user_id()`.
pub const READS_CRATE_ROOT_AUTH_USER_ID_ROUTE: &str = "/privacy/reads-crate-root-auth-user-id";
/// Declares nothing, body driven by `Gate::allows` alone.
pub const AUTHZ_DRIVEN_ROUTE: &str = "/privacy/authz-driven";
/// Declares `Tenant` only, reads the tenant *and* the identity.
pub const TENANT_DECLARED_READS_IDENTITY_ROUTE: &str =
    "/privacy/tenant-declared-reads-identity/{id}";
/// Declares `Tenant`, reads only the tenant.
pub const TENANT_VARIES_ROUTE: &str = "/privacy/tenant-varies/{id}";
/// Declares `Principal`, reads the identity through the non-default guard.
pub const NAMED_GUARD_ONLY_ROUTE: &str = "/privacy/named-guard-only/{id}";
/// Declares `Principal`, builds the body from the named guard then touches
/// the default accessor.
pub const NAMED_THEN_DEFAULT_ROUTE: &str = "/privacy/named-then-default/{id}";
/// Declares nothing, reads session state through `session_mut`.
pub const READS_SESSION_MUT_ROUTE: &str = "/privacy/reads-session-mut";
/// Declares nothing, reads a cookie.
pub const READS_COOKIE_ROUTE: &str = "/privacy/reads-cookie";
/// Declares `Principal`, reads `Auth::id()`; driven with
/// `x-test-impersonate` so the identity changes after key derivation.
pub const IMPERSONATED_ROUTE: &str = "/privacy/impersonated/{id}";
/// Declares `Principal`, reads `Auth::id()`; used under the boot whose
/// auth middleware runs after the cache.
pub const PRINCIPAL_DECLARED_READS_IDENTITY_ROUTE: &str =
    "/privacy/principal-declared-reads-identity/{id}";
/// `PrivateCached`, declares `Principal`, reads `Auth::id()`.
pub const PRIVATE_ROUTE: &str = "/privacy/private/{id}";
/// Declares `Locale`, switches locale mid-render.
pub const LOCALE_SWITCHES_ROUTE: &str = "/privacy/locale-switches/{id}";
/// Declares `Locale`, renders inside a nested `scope_locale`.
pub const LOCALE_NESTED_SCOPE_ROUTE: &str = "/privacy/locale-nested-scope/{id}";
/// Declares `Locale`; the locale is supplied by a middleware installed
/// after the cache.
pub const LOCALE_LATE_MIDDLEWARE_ROUTE: &str = "/privacy/locale-late-middleware/{id}";
/// Declares `Locale`, reads the locale; the positive control.
pub const LOCALE_VARIES_ROUTE: &str = "/privacy/locale-varies/{id}";
/// Declares nothing, reads the locale.
pub const UNDECLARED_LOCALE_ROUTE: &str = "/privacy/undeclared-locale";
/// Declares nothing, reads a user-scoped flag.
pub const READS_USER_SCOPED_FLAG_ROUTE: &str = "/privacy/reads-user-scoped-flag/{id}";
/// Declares nothing, reads a flag whose only override belongs to bob.
pub const READS_OVERRIDE_FLAG_ROUTE: &str = "/privacy/reads-override-flag/{id}";
/// Declares nothing, reads a globally scoped flag; the flag tests' positive
/// control, because a global flag's answer does not depend on the reader.
pub const READS_GLOBAL_FLAG_ROUTE: &str = "/privacy/reads-global-flag/{id}";
/// Declares `Principal`, body driven by `Gate::allows`; the authorization
/// test's positive control, since `AuthorizationRead` requires exactly that
/// dimension.
pub const PRINCIPAL_DECLARED_AUTHZ_ROUTE: &str = "/privacy/principal-declared-authz/{id}";
/// Declares `Principal`, reads the uninstrumented `Request::auth_user_id()`
/// beside the instrumented accessor.
pub const REQUEST_AUTH_USER_ID_ROUTE: &str = "/privacy/reads-request-auth-user-id/{id}";

// ── Dispatch ───────────────────────────────────────────────────────────

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

    /// The body as text. Every body this suite renders is ASCII HTML.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Dispatches a `GET` to `path` with `extra_headers` over a real loopback
/// HTTP connection, so the whole middleware chain runs exactly as it does
/// in production.
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
