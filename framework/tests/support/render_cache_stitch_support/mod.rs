//! Shared boot for `render_cache/stitch.rs`: a production-shaped Live router
//! whose routes are every declared as a stitched public shell
//! ([`RepresentationClass::PublicShellStitched`]), each behind the same route
//! chain a real application would use - `AuthMiddleware::new()` and
//! `LiveTenantMiddleware` - so a test can prove that a hit on one of these
//! routes still runs that chain instead of being answered by the global
//! RenderCache middleware before the guard ever sees the request.
//!
//! The three `OPTIONAL_*` routes are the deliberate exception: their guard is
//! `AuthMiddleware::optional()`, so a visitor the identity-bound island will
//! not accept still reaches the end of the chain. That is what separates a
//! refusal by the route from a refusal by one slot, and it is the only shape
//! in which the declared per-slot failure policies can be observed at all.
//!
//! Two counters per route, not one, because "the chain ran" and "the handler
//! ran" are different facts on a stitched route and the whole point of the
//! request path under test is that the first can happen without the second:
//! [`chain_reaches`] counts requests the cache middleware passed on to the
//! rest of the chain, and [`handler_renders`] counts requests the route's own
//! handler rendered.
//!
//! `#[serial_test::serial]` and plain `#[tokio::test]` (current-thread),
//! never `flavor = "multi_thread"`: `RenderCache::install`'s runtime and the
//! process-wide global middleware registry are process-global state, and
//! `TestContainer::fake()` writes a thread-local that a multi-thread runtime
//! could migrate away from between polls - the same reasoning
//! `render_cache_live_support` documents, followed here for the same reasons.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::live::testing::{AdjustableTestClock, prepare_live_router_with_clock_for_test};
use suprnova::live::{
    LiveBootstrapOptions, LiveDocument, LiveMount, LiveRegistry, LiveTenantMiddleware,
    LiveTenantResolver, StitchFailurePolicy,
};
use suprnova::middleware::{Middleware, Next};
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::{
    FreshnessPolicy, RenderCache, RenderCachePolicy, RepresentationClass,
};
use suprnova::testing::TestContainer;
use suprnova::view::{
    AssetSet, DocumentResponseIntent, TrustedHtml, TrustedMarkupReason, ViewName,
};
use suprnova::{
    Auth, AuthMiddleware, CsrfMiddleware, FrameworkError, HttpResponse, MiddlewareRegistry,
    Request, Response, Router, SessionConfig, SessionMiddleware, StatusCode, handle_request,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::Clock;
use suprnova_live::mount::MountFlags;
// Only `rewrite_stored_entry` names this type, and that helper is gated on
// the `testing` feature for the reason its own doc records.
#[cfg(feature = "testing")]
use suprnova_live::render_cache::composite::SegmentGraph;

use crate::live_dogfood_support::{
    DogfoodCounter, DogfoodDocument, LoginHeader, MemorySessionStore, build_public_router, fixture,
};

/// Askama filter resolution for [`StitchDocument`]: the template's
/// `trusted_html` filter is looked up in a `filters` module beside the
/// struct, exactly as `live_dogfood_support` exposes it for its own view.
pub mod filters {
    pub use suprnova::view::filters::trusted_html;
}

/// A stitched shell around one identity-bound island: the shape every other
/// route here varies from, and the only route whose document actually holds
/// per-principal markup.
pub const STITCHED_PATH: &str = "/stitch/dashboard";

/// A stitched shell around one public-seed island. Nothing in the document
/// is principal-specific, so the whole document is a Complete representation
/// even under the stitched class - the case that proves declaring the class
/// does not by itself force assembly.
pub const SEED_ONLY_PATH: &str = "/stitch/seed-only";

/// A stitched shell whose own handler reads the principal, so the shell -
/// not just its island - depends on who asked.
pub const SHELL_READS_PRINCIPAL_PATH: &str = "/stitch/shell-reads-principal";

/// A stitched shell whose route chain rewrites the body after the handler
/// returns ([`AppendFooter`]), so the bytes the client receives are not the
/// bytes the document render produced.
pub const POST_PROCESSED_PATH: &str = "/stitch/post-processed";

/// A stitched shell whose island declares [`StitchFailurePolicy::Omit`].
pub const OMIT_PATH: &str = "/stitch/omit";

/// A stitched shell whose island declares [`StitchFailurePolicy::Fallback`].
pub const FALLBACK_PATH: &str = "/stitch/fallback";

/// A stitched shell whose bootstrap stamps a fresh Content Security Policy
/// nonce per render and declares the matching `script-src` header.
pub const NONCE_PATH: &str = "/stitch/nonce";

/// [`NONCE_PATH`]'s shape around a public-seed island instead of an
/// identity-bound one: a fresh nonce per render with nothing
/// principal-specific in the document, so the shell has holes to cut but no
/// slots. The one route that separates "has islands to stitch" from "has
/// anything to cut out at all".
pub const SEED_ONLY_NONCE_PATH: &str = "/stitch/seed-only-nonce";

/// A stitched shell whose handler mounts its identity-bound island and then
/// hand-builds the response itself, never calling `LiveDocument::render`.
/// The mount facts are still captured - `LiveDocument::mount` records them -
/// but no document digest is, so nothing proves the bytes being sent are the
/// bytes a document rendered.
pub const NO_DIGEST_PATH: &str = "/stitch/no-digest";

/// A stitched shell whose handler renders *two* Live documents in one
/// request and returns the second. The captured mounts then belong to two
/// different bodies and no single shell can be cut from both, which is the
/// one thing `StitchCapture::invalid` records.
pub const TWICE_RENDERED_PATH: &str = "/stitch/twice-rendered";

/// A stitched shell whose route guard is `AuthMiddleware::optional()`
/// around an identity-bound island declaring the default
/// [`StitchFailurePolicy::FailDocument`].
///
/// The three `OPTIONAL_*` routes are the only ones here whose guard admits a
/// visitor the island itself will not accept, which is what separates a slot
/// reauthorization refusal from a route refusal: the guard says yes, the
/// chain runs to the end, and `validate_request_context` is what says no -
/// per slot, on the hit, for this request alone. Everywhere else in this
/// harness the guard turns such a visitor away first and the slot is never
/// reached.
pub const OPTIONAL_FAIL_PATH: &str = "/stitch/optional-fail";

/// [`OPTIONAL_FAIL_PATH`]'s shape with [`StitchFailurePolicy::Omit`].
pub const OPTIONAL_OMIT_PATH: &str = "/stitch/optional-omit";

/// [`OPTIONAL_FAIL_PATH`]'s shape with [`StitchFailurePolicy::Fallback`].
pub const OPTIONAL_FALLBACK_PATH: &str = "/stitch/optional-fallback";

/// Every stitched route this harness registers, in registration order.
pub const STITCH_PATHS: [&str; 13] = [
    STITCHED_PATH,
    SEED_ONLY_PATH,
    SHELL_READS_PRINCIPAL_PATH,
    POST_PROCESSED_PATH,
    OMIT_PATH,
    FALLBACK_PATH,
    NONCE_PATH,
    SEED_ONLY_NONCE_PATH,
    NO_DIGEST_PATH,
    TWICE_RENDERED_PATH,
    OPTIONAL_FAIL_PATH,
    OPTIONAL_OMIT_PATH,
    OPTIONAL_FALLBACK_PATH,
];

/// The fallback fragment `FALLBACK_PATH` declares.
pub const FALLBACK_HTML: &str = "<p>island unavailable</p>";

/// The footer `POST_PROCESSED_PATH`'s route middleware appends.
pub const FOOTER: &str = "<!-- footer -->";

/// Requests the RenderCache middleware handed on to the rest of the chain,
/// per route. Counted by a global middleware registered *after*
/// `RenderCache::install`, so this grows only when the cache middleware
/// called `next` - which, on a stitched route, it must do on a hit as well
/// as on a miss.
static CHAIN_REACHES: Mutex<BTreeMap<&'static str, usize>> = Mutex::new(BTreeMap::new());

/// Requests each route's own handler rendered, counted inside the handler
/// itself. A stitched hit that replays a stored Complete representation
/// leaves this untouched while [`CHAIN_REACHES`] still grows.
static HANDLER_RENDERS: Mutex<BTreeMap<&'static str, usize>> = Mutex::new(BTreeMap::new());

/// Whether [`Refusing`] resolves a tenant at all; see [`set_tenant_refusing`].
static TENANT_REFUSING: AtomicBool = AtomicBool::new(false);

/// Distinguishes one refusing resolution from the next.
static TENANT_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

/// Requests the cache middleware passed on to the rest of the chain for
/// `path` so far.
#[must_use]
pub fn chain_reaches(path: &str) -> usize {
    count(&CHAIN_REACHES, path)
}

/// Requests `path`'s own handler rendered so far.
#[must_use]
pub fn handler_renders(path: &str) -> usize {
    count(&HANDLER_RENDERS, path)
}

fn count(counter: &Mutex<BTreeMap<&'static str, usize>>, path: &str) -> usize {
    counter
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(path)
        .copied()
        .unwrap_or(0)
}

fn increment(counter: &Mutex<BTreeMap<&'static str, usize>>, path: &'static str) {
    *counter
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(path)
        .or_insert(0) += 1;
}

/// Makes [`Refusing`] resolve a *different* tenant on every request while
/// `refusing` is set. Reset by [`boot`].
///
/// This is a shifting tenant, not a refusal: a fresh mount succeeds under
/// whatever tenant this request resolved, and only the scope it is bound to
/// moves. It is therefore how a test observes that a stitched hit derives
/// the island's authority from the request in front of it rather than
/// replaying the authority the shell was published under - see
/// `stitch.rs`'s own test of that. A slot refusal needs a visitor the island
/// will not accept, which is what the `OPTIONAL_*` routes exist for.
pub fn set_tenant_refusing(refusing: bool) {
    TENANT_REFUSING.store(refusing, Ordering::SeqCst);
}

/// The tenant resolver every stitched route is guarded by.
///
/// Resolves nothing at all by default, exactly as `live_dogfood_support`'s
/// own `Tenantless` does, so the ordinary routes behave identically; with
/// [`set_tenant_refusing`] on it resolves a different tenant per request
/// instead. See that function for what a shifting tenant does and does not
/// prove.
pub struct Refusing;

#[async_trait::async_trait]
impl LiveTenantResolver for Refusing {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        if TENANT_REFUSING.load(Ordering::SeqCst) {
            let sequence = TENANT_SEQUENCE.fetch_add(1, Ordering::SeqCst);
            return Ok(Some(format!("tenant-{sequence}")));
        }
        Ok(None)
    }
}

/// Counts requests the RenderCache middleware passed on to the rest of the
/// chain. Registered globally and AFTER `RenderCache::install`, so it sits
/// behind the cache middleware: a request the cache answered on its own
/// never reaches it.
struct ChainCounter;

#[async_trait::async_trait]
impl Middleware for ChainCounter {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(path) = STITCH_PATHS.iter().find(|path| **path == request.path()) {
            increment(&CHAIN_REACHES, path);
        }
        next(request).await
    }
}

/// Rewrites `POST_PROCESSED_PATH`'s body after the handler returned, the way
/// an application's own route middleware may: the bytes the client receives
/// are then not the bytes the Live document render produced.
struct AppendFooter;

#[async_trait::async_trait]
impl Middleware for AppendFooter {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let (response, failed) = match next(request).await {
            Ok(response) => (response, false),
            Err(response) => (response, true),
        };
        if response.is_streaming() {
            return if failed { Err(response) } else { Ok(response) };
        }
        let mut body = response.body().to_vec();
        body.extend_from_slice(FOOTER.as_bytes());
        let headers: Vec<(String, String)> = response
            .headers()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        let content_type = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| "text/html; charset=utf-8".to_owned());
        let mut rebuilt = HttpResponse::bytes_body(Bytes::from(body), content_type)
            .status(response.status_code());
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("content-type") {
                continue;
            }
            rebuilt = rebuilt.header(name, value);
        }
        if failed { Err(rebuilt) } else { Ok(rebuilt) }
    }
}

/// The single RenderCache migration this harness needs; mirrors
/// `render_cache_live_support::LiveRenderCacheMigrator`, which is private to
/// that module.
struct StitchRenderCacheMigrator;

#[async_trait::async_trait]
impl MigratorTrait for StitchRenderCacheMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// Both RenderCache migrations, for [`Providers::Database`]: the tier tables
/// carry the SQL L1 rows and the fenced lease rows, and `RenderCache::install`
/// refuses a database tier without them.
struct StitchTierMigrator;

#[async_trait::async_trait]
impl MigratorTrait for StitchTierMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(suprnova::render_cache::migration::Migration),
            Box::new(suprnova::render_cache::migration::TierMigration),
        ]
    }
}

/// Which deployment profile a boot installs. Everything else about the
/// harness - the routes, the policies, the guards, the clock, the counters -
/// is identical whichever is chosen, which is the point: a profile changes
/// where entries and leases live, never what a route does.
enum Providers {
    /// Tier 0: no L1 provider and the in-process coordinator.
    Embedded,
    /// Tier 1: the SQL L1 store and the database-backed fenced lease
    /// coordinator, over a database carrying both migrations.
    Database,
    /// Tier 2: the Redis L1 store and the Redis-backed fenced lease
    /// coordinator, under the caller's own key namespace. Generation truth
    /// stays in the database, which therefore still carries the original
    /// migration - and only that one, because a Redis profile reaches no
    /// tier table.
    Redis(suprnova::render_cache::providers::RedisProviderConfig),
}

/// Everything one test needs: the router and middleware registry to dispatch
/// through, and the clock both the Live runtime and the RenderCache runtime
/// read.
pub struct Harness {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    clock: Arc<AdjustableTestClock>,
    _conn: suprnova::database::DbConnection,
    _guard: suprnova::testing::TestContainerGuard,
    /// The directory holding the SQLite file a [`Providers::Embedded`] boot
    /// created; `None` for the distributed profiles, whose database is in
    /// memory and has no directory to keep alive.
    _tempdir: Option<tempfile::TempDir>,
}

/// The adjustable clock this harness shares between both runtimes.
pub fn clock(harness: &Harness) -> &Arc<AdjustableTestClock> {
    &harness.clock
}

/// Edits the Composite entry already stored for `path` in place, through the
/// framework's own test seam, and leaves it published under the same key.
///
/// The only way a test can reach the drift a redeploy produces: a stored
/// slot whose component or contract digest the running registry no longer
/// has cannot arise inside one process, because the registry and the entry
/// are written by the same build. An `edit` that changes nothing is also how
/// a test observes the stored graph, which nothing else here exposes.
///
/// Gated on `testing`: the seam it calls,
/// `render_cache::testing::rewrite_composite_for_test`, is
/// `#[cfg(any(test, feature = "testing"))]` in the framework, and the
/// minimal profile checked by `scripts/check-feature-matrix.sh` leaves that
/// feature off. The gate is at item level because this support module is
/// shared with test modules that need none of it.
#[cfg(feature = "testing")]
pub async fn rewrite_stored_entry<F>(path: &str, edit: F)
where
    F: FnOnce(&mut SegmentGraph),
{
    suprnova::render_cache::testing::rewrite_composite_for_test(path, edit).await;
}

/// Boots a fresh SQLite database with the RenderCache migration applied,
/// registers every stitched route with a generous `PublicShellStitched`
/// policy, installs RenderCache, and prepares the Live runtime on the same
/// router with the same clock.
pub async fn boot() -> Arc<Harness> {
    boot_with_stitched_freshness(generous_freshness(), Providers::Embedded).await
}

/// [`boot`] on the Database deployment profile: the same routes, policies,
/// guards, and counters, over a database carrying both render cache
/// migrations, with `L1Config::Database` and `CoordinatorConfig::Database`
/// installed in place of a disabled L1 and the in-process coordinator.
///
/// Written here rather than as a second harness beside this one so that what
/// a profile changes is exactly one thing: a test that dispatches the same
/// request through the same route is comparing the providers and nothing
/// else. The database is `sqlite::memory:` with a **single** connection, the
/// same shape `TestDatabase::fresh` uses, which is also what makes this boot
/// prove something no larger pool could: `SqlLeaseStore` and `SqlRenderStore`
/// each open a transaction of their own rather than joining the caller's, so
/// a coordinator or publish call made inside the render's own read
/// transaction would deadlock here instead of merely being slower.
pub async fn boot_on_the_database_profile_for_test() -> Arc<Harness> {
    boot_with_stitched_freshness(generous_freshness(), Providers::Database).await
}

/// [`boot_on_the_database_profile_for_test`]'s Tier 2 twin: the same routes
/// over the Redis L1 store and the Redis rebuild coordinator, under
/// `config`'s own key namespace. Generation truth stays in the database, so
/// the boot still carries the original render cache migration; it carries no
/// tier tables, because a Redis profile reaches none.
///
/// Only ever called from an `#[ignore]`d live Redis test: `RenderCache::install`
/// pings the endpoint and refuses a boot that nothing answers.
pub async fn boot_on_the_redis_profile_for_test(
    config: suprnova::render_cache::providers::RedisProviderConfig,
) -> Arc<Harness> {
    boot_with_stitched_freshness(generous_freshness(), Providers::Redis(config)).await
}

/// [`boot`] with [`STITCHED_PATH`] given the named intervals instead of the
/// generous default, so a test can move the shared clock past the fresh
/// interval and observe the stale-servable path. Every other route keeps
/// the default, so only the route under test ever goes stale.
pub async fn boot_with_freshness(
    fresh_ms: u64,
    stale_servable_ms: u64,
    stale_on_error_ms: u64,
) -> Arc<Harness> {
    boot_with_stitched_freshness(
        FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)
            .expect("stitched freshness intervals"),
        Providers::Embedded,
    )
    .await
}

/// The shared body of [`boot`], [`boot_with_freshness`],
/// [`boot_on_the_database_profile_for_test`], and
/// [`boot_on_the_redis_profile_for_test`]: everything but the freshness
/// [`STITCHED_PATH`] is registered with and the providers `install` is given
/// is identical.
async fn boot_with_stitched_freshness(
    stitched_freshness: FreshnessPolicy,
    providers: Providers,
) -> Arc<Harness> {
    static CRYPT_ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT_ONCE.get_or_init(|| {
        suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    });
    suprnova::App::init();
    suprnova::middleware::clear_global_middleware_for_test();

    let guard = TestContainer::fake();
    fixture();
    suprnova::App::singleton(
        LiveRegistry::builder()
            .register::<DogfoodCounter>()
            .expect("register dogfood counter")
            .build(),
    );

    let (conn, tempdir) = match providers {
        Providers::Embedded => {
            let tempdir =
                tempfile::tempdir().expect("tempdir for render cache stitch test database");
            let db_path = tempdir.path().join("render-cache-stitch.sqlite3");
            let config = suprnova::database::DatabaseConfig::builder()
                .url(format!("sqlite://{}", db_path.display()))
                .max_connections(4)
                .min_connections(1)
                .logging(false)
                .build();
            let conn = suprnova::database::DbConnection::connect(&config)
                .await
                .expect("connect sqlite");
            StitchRenderCacheMigrator::up(conn.inner(), None)
                .await
                .expect("apply render cache migration");
            (conn, Some(tempdir))
        }
        Providers::Database | Providers::Redis(_) => {
            let config = suprnova::database::DatabaseConfig::builder()
                .url("sqlite::memory:")
                .max_connections(1)
                .min_connections(1)
                .logging(false)
                .build();
            let conn = suprnova::database::DbConnection::connect(&config)
                .await
                .expect("connect sqlite");
            if matches!(providers, Providers::Database) {
                StitchTierMigrator::up(conn.inner(), None)
                    .await
                    .expect("apply both render cache migrations");
            } else {
                StitchRenderCacheMigrator::up(conn.inner(), None)
                    .await
                    .expect("apply the render cache migration");
            }
            (conn, None)
        }
    };
    TestContainer::singleton(conn.clone());

    let clock = Arc::new(AdjustableTestClock::new(1_000_000));

    let seed_only =
        LiveMount::<DogfoodCounter>::public_seed(SEED_ONLY_PATH, "counter", "stitch-seed")
            .expect("declare seed-only mount");
    let stitched = identity_bound(STITCHED_PATH, "stitch-counter");
    let shell_reads_principal =
        identity_bound(SHELL_READS_PRINCIPAL_PATH, "stitch-shell-principal");
    let post_processed = identity_bound(POST_PROCESSED_PATH, "stitch-post-processed");
    let omit = identity_bound(OMIT_PATH, "stitch-omit")
        .on_stitch_failure(StitchFailurePolicy::Omit)
        .expect("declare omit failure policy");
    let fallback_reason =
        TrustedMarkupReason::new("stitch harness fallback").expect("fallback reason");
    let fallback_html =
        TrustedHtml::framework_static(FALLBACK_HTML, fallback_reason).expect("fallback markup");
    let fallback = identity_bound(FALLBACK_PATH, "stitch-fallback")
        .on_stitch_failure(StitchFailurePolicy::Fallback(fallback_html))
        .expect("declare fallback failure policy");
    let nonce = identity_bound(NONCE_PATH, "stitch-nonce");
    let seed_only_nonce = LiveMount::<DogfoodCounter>::public_seed(
        SEED_ONLY_NONCE_PATH,
        "counter",
        "stitch-seed-nonce",
    )
    .expect("declare seed-only nonce mount");
    let no_digest = identity_bound(NO_DIGEST_PATH, "stitch-no-digest");
    let twice_rendered = identity_bound(TWICE_RENDERED_PATH, "stitch-twice-rendered");
    let optional_fail = identity_bound(OPTIONAL_FAIL_PATH, "stitch-optional-fail");
    let optional_omit = identity_bound(OPTIONAL_OMIT_PATH, "stitch-optional-omit")
        .on_stitch_failure(StitchFailurePolicy::Omit)
        .expect("declare optional omit failure policy");
    let optional_fallback_html = TrustedHtml::framework_static(
        FALLBACK_HTML,
        TrustedMarkupReason::new("stitch harness fallback").expect("fallback reason"),
    )
    .expect("fallback markup");
    let optional_fallback = identity_bound(OPTIONAL_FALLBACK_PATH, "stitch-optional-fallback")
        .on_stitch_failure(StitchFailurePolicy::Fallback(optional_fallback_html))
        .expect("declare optional fallback failure policy");

    let mut router: Router = build_public_router();
    router = document_route(router, SEED_ONLY_PATH, &seed_only);
    router = document_route(router, STITCHED_PATH, &stitched);
    router = principal_route(router, &shell_reads_principal);
    router = post_processed_route(router, &post_processed);
    router = document_route(router, OMIT_PATH, &omit);
    router = document_route(router, FALLBACK_PATH, &fallback);
    router = nonce_route(router, NONCE_PATH, &nonce);
    router = nonce_route(router, SEED_ONLY_NONCE_PATH, &seed_only_nonce);
    router = handler_built_route(router, &no_digest);
    router = twice_rendered_route(router, &twice_rendered);
    router = optional_guard_route(router, OPTIONAL_FAIL_PATH, &optional_fail);
    router = optional_guard_route(router, OPTIONAL_OMIT_PATH, &optional_omit);
    router = optional_guard_route(router, OPTIONAL_FALLBACK_PATH, &optional_fallback);

    for path in STITCH_PATHS {
        let freshness = if path == STITCHED_PATH {
            stitched_freshness
        } else {
            generous_freshness()
        };
        router = router
            .try_render_cache(path, stitched_policy(freshness))
            .unwrap_or_else(|_| panic!("attach stitched render cache policy for {path}"));
    }

    let mut render_cache_config = RenderCacheConfig::from_env()
        .expect("the test environment configures a valid render cache")
        .with_clock_for_test(Arc::clone(&clock) as Arc<dyn Clock>);
    render_cache_config.enabled = true;
    // Pinned, never inherited: an ambient RENDER_CACHE_PROFILE,
    // RENDER_CACHE_L1, or RENDER_CACHE_COORDINATOR must not change which
    // providers this suite installs - the boot's own `Providers` is the only
    // thing that decides.
    match providers {
        Providers::Embedded => {
            render_cache_config.l1 = suprnova::render_cache::L1Config::Disabled;
            render_cache_config.coordinator = suprnova::render_cache::CoordinatorConfig::Local {
                lease_ms: 30_000,
                max_waiters: 128,
            };
        }
        Providers::Database => {
            render_cache_config.l1 = suprnova::render_cache::L1Config::Database {
                max_bytes: 4 * 1024 * 1024,
            };
            render_cache_config.coordinator = suprnova::render_cache::CoordinatorConfig::Database {
                lease_ms: 30_000,
                max_waiters: 128,
            };
        }
        Providers::Redis(ref redis) => {
            render_cache_config.l1 = suprnova::render_cache::L1Config::Redis {
                url: redis.url.clone(),
                prefix: redis.prefix.clone(),
                max_bytes: 4 * 1024 * 1024,
            };
            render_cache_config.coordinator = suprnova::render_cache::CoordinatorConfig::Redis {
                url: redis.url.clone(),
                prefix: redis.prefix.clone(),
                lease_ms: 30_000,
                max_waiters: 128,
            };
        }
    }

    // Registered globally, and before `RenderCache::install`, for the same
    // ordering reason `render_cache_live_support::boot_with_render_cache_and_live`
    // documents: `install` appends its own middleware rather than inserting
    // at a fixed position, so this is what makes each route's
    // `AuthMiddleware::new()` guard see the session `LoginHeader` establishes.
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
    // AFTER install, so it sits behind the cache middleware and grows only
    // when the cache handed the request on; reset before it is registered so
    // an earlier test's counts never leak into this one.
    CHAIN_REACHES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    HANDLER_RENDERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    set_tenant_refusing(false);
    suprnova::middleware::register_global_middleware(ChainCounter);
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

fn identity_bound(path: &str, document_key: &str) -> LiveMount<DogfoodCounter> {
    LiveMount::<DogfoodCounter>::identity_bound(path, "counter", document_key)
        .unwrap_or_else(|_| panic!("declare identity-bound mount for {path}"))
}

/// Fresh for longer than any test's clock ever advances, and never
/// stale-servable: the default every route here keeps unless a test asks
/// for something narrower.
fn generous_freshness() -> FreshnessPolicy {
    FreshnessPolicy::new(200_000_000, 0, 0).expect("freshness")
}

/// Every stitched route stores in both tiers. A no-op under
/// [`Providers::Embedded`], whose runtime has no L1 provider at all to
/// publish into; under the distributed profiles it is what sends a published
/// shell to the shared L1 as well as to L0.
fn stitched_policy(freshness: FreshnessPolicy) -> RenderCachePolicy {
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(freshness)
        .layers(suprnova::render_cache::StorageLayers::l0_and_l1())
        .build()
        .expect("stitched policy")
}

/// Registers one stitched route behind the production-shaped chain every
/// route here shares: the required auth guard first, then the tenant
/// middleware, then the Live completion middleware the framework appends.
fn document_route(router: Router, path: &'static str, mount: &LiveMount<DogfoodCounter>) -> Router {
    let handler_mount = mount.clone();
    let router: Router = router
        .get(path, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_stitch_document(request, mount, path).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Refusing)))
        .into();
    router
        .try_live_mount(mount)
        .unwrap_or_else(|_| panic!("register stitched mount for {path}"))
}

fn principal_route(router: Router, mount: &LiveMount<DogfoodCounter>) -> Router {
    let handler_mount = mount.clone();
    let router: Router = router
        .get(SHELL_READS_PRINCIPAL_PATH, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_principal_document(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Refusing)))
        .into();
    router
        .try_live_mount(mount)
        .expect("register shell-reads-principal mount")
}

fn post_processed_route(router: Router, mount: &LiveMount<DogfoodCounter>) -> Router {
    let handler_mount = mount.clone();
    let router: Router = router
        .get(POST_PROCESSED_PATH, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_stitch_document(request, mount, POST_PROCESSED_PATH).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Refusing)))
        .middleware(AppendFooter)
        .into();
    router
        .try_live_mount(mount)
        .expect("register post-processed mount")
}

/// One stitched route whose guard is `AuthMiddleware::optional()`: an
/// anonymous visitor reaches the handler and the Live completion middleware
/// instead of being turned away, so the identity-bound island - not the
/// route - is what refuses them.
fn optional_guard_route(
    router: Router,
    path: &'static str,
    mount: &LiveMount<DogfoodCounter>,
) -> Router {
    let handler_mount = mount.clone();
    let router: Router = router
        .get(path, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_stitch_document(request, mount, path).await }
        })
        .middleware(AuthMiddleware::optional())
        .middleware(LiveTenantMiddleware::new(Arc::new(Refusing)))
        .into();
    router
        .try_live_mount(mount)
        .unwrap_or_else(|_| panic!("register optional-guard mount for {path}"))
}

/// `NO_DIGEST_PATH`: the same chain as every other stitched route, behind a
/// handler that mounts its island and then builds the response by hand.
fn handler_built_route(router: Router, mount: &LiveMount<DogfoodCounter>) -> Router {
    let handler_mount = mount.clone();
    let router: Router = router
        .get(NO_DIGEST_PATH, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_without_a_document(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Refusing)))
        .into();
    router
        .try_live_mount(mount)
        .expect("register no-digest mount")
}

/// `TWICE_RENDERED_PATH`: the same chain, behind a handler that renders two
/// documents and returns the second.
fn twice_rendered_route(router: Router, mount: &LiveMount<DogfoodCounter>) -> Router {
    let handler_mount = mount.clone();
    let router: Router = router
        .get(TWICE_RENDERED_PATH, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_twice(request, mount).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Refusing)))
        .into();
    router
        .try_live_mount(mount)
        .expect("register twice-rendered mount")
}

fn nonce_route(router: Router, path: &'static str, mount: &LiveMount<DogfoodCounter>) -> Router {
    let handler_mount = mount.clone();
    let router: Router = router
        .get(path, move |request: Request| {
            let mount = handler_mount.clone();
            async move { render_nonce_document(request, mount, path).await }
        })
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Refusing)))
        .into();
    router
        .try_live_mount(mount)
        .unwrap_or_else(|_| panic!("register nonce mount for {path}"))
}

/// The document every plain stitched route renders: one island inside the
/// shared dogfood shell.
async fn render_stitch_document(
    request: Request,
    mount: LiveMount<DogfoodCounter>,
    path: &'static str,
) -> Result<HttpResponse, HttpResponse> {
    increment(&HANDLER_RENDERS, path);
    into_response(render_one_document(&request, &mount).await)
}

/// One Live document render, without touching [`HANDLER_RENDERS`]: the
/// counter belongs to the *request*, and [`render_twice`] renders two
/// documents inside one request.
async fn render_one_document(
    request: &Request,
    mount: &LiveMount<DogfoodCounter>,
) -> Result<HttpResponse, FrameworkError> {
    let mut document = LiveDocument::from_request(request)
        .map_err(|error| FrameworkError::internal(format!("from_request {error}")))?;
    let island = document
        .mount(
            mount,
            CanonicalValue::Object(std::collections::BTreeMap::new()),
            MountFlags::empty(),
        )
        .await
        .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
    let bootstrap = document
        .bootstrap(LiveBootstrapOptions::esm())
        .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
    document
        .render(
            dogfood_view()?,
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

/// The shared tail of every handler here: a framework error becomes the
/// same 500 the visitor would see in production.
fn into_response(
    result: Result<HttpResponse, FrameworkError>,
) -> Result<HttpResponse, HttpResponse> {
    result.map_err(|error| HttpResponse::text(format!("Live document failed: {error}")).status(500))
}

/// `NO_DIGEST_PATH`'s handler: mounts the island, builds the bootstrap, and
/// then writes the response itself instead of calling
/// `LiveDocument::render`. The mount facts are captured either way - they
/// are recorded at `mount` - but no document digest ever is, so the
/// publisher cannot prove the bytes it is being asked to cut a shell from
/// are the bytes a Live document produced.
async fn render_without_a_document(
    request: Request,
    mount: LiveMount<DogfoodCounter>,
) -> Result<HttpResponse, HttpResponse> {
    increment(&HANDLER_RENDERS, NO_DIGEST_PATH);
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)
            .map_err(|error| FrameworkError::internal(format!("from_request {error}")))?;
        let island = document
            .mount(
                &mount,
                CanonicalValue::Object(std::collections::BTreeMap::new()),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
        let bootstrap = document
            .bootstrap(LiveBootstrapOptions::esm())
            .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
        Ok(HttpResponse::html(format!(
            "<!doctype html><html><head>{}</head><body>{}</body></html>",
            bootstrap.html(),
            island.html()
        )))
    }
    .await;
    into_response(result)
}

/// `TWICE_RENDERED_PATH`'s handler: two whole Live documents in one request,
/// the second of which is returned. Both mounts are captured, but they
/// belong to two different bodies, and the capture says so.
async fn render_twice(
    request: Request,
    mount: LiveMount<DogfoodCounter>,
) -> Result<HttpResponse, HttpResponse> {
    increment(&HANDLER_RENDERS, TWICE_RENDERED_PATH);
    let result: Result<HttpResponse, FrameworkError> = async {
        let _discarded = render_one_document(&request, &mount).await?;
        render_one_document(&request, &mount).await
    }
    .await;
    into_response(result)
}

/// `SHELL_READS_PRINCIPAL_PATH`'s handler: identical to
/// [`render_stitch_document`] except that the shell itself reads the
/// principal and carries it in the document's title, so the bytes outside
/// the island already depend on who asked.
async fn render_principal_document(
    request: Request,
    mount: LiveMount<DogfoodCounter>,
) -> Result<HttpResponse, HttpResponse> {
    increment(&HANDLER_RENDERS, SHELL_READS_PRINCIPAL_PATH);
    let result: Result<HttpResponse, FrameworkError> = async {
        let title = Auth::id().unwrap_or_else(|| "anonymous".to_owned());
        let mut document = LiveDocument::from_request(&request)
            .map_err(|error| FrameworkError::internal(format!("from_request {error}")))?;
        let island = document
            .mount(
                &mount,
                CanonicalValue::Object(std::collections::BTreeMap::new()),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
        let bootstrap = document
            .bootstrap(LiveBootstrapOptions::esm())
            .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
        document
            .render(
                ViewName::parse("live/stitch-document.html")
                    .map_err(|_| FrameworkError::internal("view identity"))?,
                &StitchDocument {
                    title: &title,
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

/// The handler behind `NONCE_PATH` and `SEED_ONLY_NONCE_PATH`: a fresh
/// Content Security Policy nonce per render, stamped on the bootstrap's
/// script elements and declared in the document's own
/// `content-security-policy` header, so a stored shell and the header it was
/// stored with can never quietly disagree. Which of the two routes is being
/// served decides only whether the island it mounts is identity-bound; the
/// document is otherwise identical.
async fn render_nonce_document(
    request: Request,
    mount: LiveMount<DogfoodCounter>,
    path: &'static str,
) -> Result<HttpResponse, HttpResponse> {
    increment(&HANDLER_RENDERS, path);
    let result: Result<HttpResponse, FrameworkError> = async {
        let nonce = suprnova_live::render_cache::composite::fresh_nonce()
            .map_err(|_| FrameworkError::internal("fresh nonce"))?;
        let mut document = LiveDocument::from_request(&request)
            .map_err(|error| FrameworkError::internal(format!("from_request {error}")))?;
        let island = document
            .mount(
                &mount,
                CanonicalValue::Object(std::collections::BTreeMap::new()),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
        let bootstrap = document
            .bootstrap(LiveBootstrapOptions::esm().with_nonce(nonce.clone()))
            .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
        let intent = DocumentResponseIntent::html(StatusCode::OK)
            .map_err(|_| FrameworkError::internal("response intent"))?
            .with_header(
                hyper::header::HeaderName::from_static("content-security-policy"),
                hyper::header::HeaderValue::from_str(&format!("script-src 'nonce-{nonce}'"))
                    .map_err(|_| FrameworkError::internal("nonce header"))?,
            )
            .map_err(|_| FrameworkError::internal("nonce header"))?;
        document
            .render(
                dogfood_view()?,
                &DogfoodDocument {
                    bootstrap: bootstrap.html(),
                    island: island.html(),
                },
                intent,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|error| HttpResponse::text(format!("Live document failed: {error}")).status(500))
}

fn dogfood_view() -> Result<ViewName, FrameworkError> {
    ViewName::parse("live/dogfood-document.html")
        .map_err(|_| FrameworkError::internal("view identity"))
}

/// The dogfood shell with a title the handler controls; used only by
/// `SHELL_READS_PRINCIPAL_PATH`, whose whole point is a shell that carries
/// principal-specific bytes outside the island.
#[suprnova::view(path = "live/stitch-document.html")]
pub struct StitchDocument<'a> {
    /// Title text the handler resolved for this request.
    pub title: &'a str,
    /// Live bootstrap markup.
    pub bootstrap: &'a TrustedHtml,
    /// The mounted island's markup.
    pub island: &'a TrustedHtml,
}

/// The island-markup readers, defined once in
/// [`crate::render_cache_support`] and re-exported here: this suite and the
/// privacy suite held byte-identical copies until task 8, and separating an
/// island's per-principal markup from the shell bytes around it is exactly
/// the distinction an assembled stitched document has to get right.
pub use crate::render_cache_support::{decoded_snapshot, island_tag};
// `attribute` is used only by `the_stored_shell_holds_no_island_markup_and_no_signed_snapshot`,
// which is gated on the `testing` feature (it needs `RenderCache::shell_for_test`),
// so the re-export is gated the same way instead of going unused under the
// minimal profile.
#[cfg(feature = "testing")]
pub use crate::render_cache_support::attribute;

/// One dispatched response: status, an accessor for a header, and the body.
pub struct TestResponse {
    /// The response status.
    pub status: StatusCode,
    headers: hyper::HeaderMap,
    /// The response body bytes.
    pub body: Bytes,
}

impl TestResponse {
    /// The first value of `name`, if present.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// The body as UTF-8, for assertion messages.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The session cookie pair this response established, to present on a
    /// later request that has to be the *same* session. Without it every
    /// dispatch starts a fresh session, and the session fingerprint is one
    /// of the three identities a mount's scope is derived from.
    #[must_use]
    pub fn session_cookie(&self) -> String {
        crate::live_dogfood_support::session_cookie(&self.headers)
    }
}

/// Dispatches one request with the given extra headers through the harness's
/// real HTTP path (a bound loopback listener, exactly like
/// `render_cache_live_support::dispatch_get`), and returns the decoded
/// response. `method` is `GET` or `HEAD`.
pub async fn dispatch(
    harness: &Harness,
    method: hyper::Method,
    path: &str,
    headers: &[(&str, &str)],
) -> TestResponse {
    let mut builder = hyper::Request::builder()
        .method(method)
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
