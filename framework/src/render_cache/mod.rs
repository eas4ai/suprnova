//! Suprnova's RenderCache: route and group policy, one global middleware
//! that serves proven Complete representations, a request-scoped dependency
//! collector, database-authoritative generations, and Tier 0 providers.
//!
//! Live is complete without it. Enabling it changes avoided work, never
//! application capability.
//!
//! This module starts small: Task 9 added configuration, route and group
//! policy registration, and effective policy resolution. Task 10 added the
//! request-scoped dependency collector. Task 11 added the
//! database-authoritative generation ledger and its migration. Task 12
//! added the write side: every supported ORM and query-builder write path
//! advances the generation of what it changed. Task 13 added the
//! file-backed L1 store. Task 14 added the middleware and
//! [`RenderCache::install`], which assembles the runtime and puts the
//! middleware on the router. Task 15 (this one) adds the Live integration
//! in [`live`]: reading a seed promotion deadline out of a mounted document
//! for `EntryHeader`, and declining identity-bound documents.

pub mod collector;
pub mod config;
pub mod file_store;
pub mod ledger;
pub mod live;
pub mod middleware;
pub mod migration;
pub mod orm;
pub mod registry;
pub mod telemetry;
#[doc(hidden)]
pub mod testing;

pub use config::{FailurePolicy, L0Limits, L1Config, RenderCacheConfig};
pub use middleware::{RenderCacheMiddleware, RenderCacheRuntime};
pub use suprnova_live::render_cache::entry::EntryInspection;
pub use suprnova_live::render_cache::generation::DependencyIdentity;
pub use suprnova_live::render_cache::{
    CoherenceMode, DeclineReason, Eligibility, FreshnessPolicy, PolicyPatch, QueryPolicy,
    RenderCachePolicy, RenderCachePolicyBuilder, RepresentationClass, SharedCachePolicy,
    StorageLayers, VarianceDimension,
};

use std::sync::{Arc, OnceLock, RwLock};

use suprnova_live::render_cache::entry::EntryLimits;
use suprnova_live::render_cache::singleflight::{LocalCoordinatorLimits, LocalRebuildCoordinator};
use suprnova_live::render_cache::store::{MemoryRenderStore, MemoryStoreLimits, RenderStore as _};
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};

use crate::{FrameworkError, Router};

/// The RenderCache facade: install, observe, inspect, and epoch control.
pub struct RenderCache;

/// Whether a RenderCache runtime has been installed for this process.
///
/// Starts `false`. An application that never installs RenderCache -
/// every existing application, and nearly every test database - must pay
/// zero RenderCache SQL on any write, not even a probe. Probing
/// unconditionally is unsafe: on Postgres, a failed statement poisons the
/// enclosing transaction, and `COMMIT` on a poisoned transaction returns
/// the ROLLBACK tag without raising, so a caller inside `DB::transaction`
/// on an unmigrated database would see its own write silently discarded
/// while being told it succeeded. See `orm::advance` and
/// `ledger::advance_in_current_transaction`, which both consult
/// [`is_installed`] before issuing any SQL.
static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True once a RenderCache runtime has been installed for this process.
#[must_use]
pub(crate) fn is_installed() -> bool {
    INSTALLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Marks a RenderCache runtime as installed for this process, opening the
/// write side's gate.
///
/// Called by [`RenderCache::install`]. Test setup that boots the RenderCache
/// migration directly - without a running `install` to call yet - calls
/// this too; there is no other way today to open the gate. Not part of the
/// public contract: doc-hidden.
#[doc(hidden)]
pub fn mark_installed() {
    INSTALLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The installed runtime, behind a lock rather than a set-once cell: a
/// process installs RenderCache once, but this crate's own test suite
/// (one process, many `#[tokio::test]` functions, serialized with
/// `#[serial_test::serial]` where a shared runtime's own state - the L0
/// store, the coordinator, the policy table - would otherwise leak across
/// tests) calls `install` once per test and needs each call to actually
/// replace what is there.
static RUNTIME: OnceLock<RwLock<Option<Arc<RenderCacheRuntime>>>> = OnceLock::new();

fn runtime_slot() -> &'static RwLock<Option<Arc<RenderCacheRuntime>>> {
    RUNTIME.get_or_init(|| RwLock::new(None))
}

impl RenderCache {
    /// Assembles the RenderCache runtime from `config` and the policies
    /// `router` has registered, appends the middleware to the process-wide
    /// global middleware chain via
    /// [`crate::middleware::register_global_middleware`], and returns the
    /// router unchanged.
    ///
    /// # Call this after registering locale, session, and auth middleware
    ///
    /// `register_global_middleware` appends to whatever is already
    /// registered; it does not insert at a fixed position. The middleware
    /// reads `Lang::locale()` and `Auth::id()` *before* calling the route
    /// handler (to build the lookup key's declared variance), and both are
    /// only meaningful once the middleware that establishes them - the
    /// locale middleware, session/auth middleware - has already run around
    /// it. Call `RenderCache::install` **after** every `global_middleware!`
    /// registration that establishes request-scoped locale or identity, so
    /// this middleware lands after them in the chain, not before. Calling
    /// it first would make every request collapse onto the default locale
    /// and see no principal, silently over-sharing exactly the kind of
    /// entry `middleware::key_used_different_values_than_the_render_saw`
    /// exists to catch (that guard is private, so this is a plain code
    /// span, not a doc link; its name has changed twice since this note
    /// was written - fix round 3's `key_omits_observed_privacy`, fix
    /// round 4's `key_omits_the_dimension_each_reason_requires`, fix
    /// round 6's current name - keep this reference current).
    ///
    /// An earlier version of this note claimed that check's failure mode
    /// was reduced rendering rather than a privacy leak. That was true only
    /// for a route declaring no matching variance dimension at all; it was
    /// false, and proven false, for a route that *does* declare `Principal`
    /// (or `Tenant`): before fix round 3, item 3, the guard tested whether a
    /// dimension was declared, not whether the key's resolved value for it
    /// actually partitioned anything. Installed too early, `Auth::id()` is
    /// still unset when the key is derived, so a declared `Principal`
    /// dimension resolves to `DimensionValue::Anonymous` - present in the
    /// map, but not a partition - while the handler, running later in the
    /// chain after auth has established identity, still observes a real
    /// principal. That combination published one shared, anonymous-keyed
    /// entry that every signed-in visitor received: a privacy leak, not
    /// merely reduced rendering. The guard now requires a *resolved*
    /// `DimensionValue::Private` value, not merely a declared dimension, so
    /// this specific ordering mistake is caught and declined rather than
    /// mis-keyed - but installing in the wrong order still costs whatever
    /// that route's variance depended on (locale collapsing to the default,
    /// principal-scoped routes losing their cache entirely to declines),
    /// which is still not what an application installing this in the wrong
    /// order would want.
    ///
    /// # Errors
    ///
    /// Fails when:
    /// - the RenderCache migration's tables are not present on the primary
    ///   connection (ruling R58) - an actionable error naming the
    ///   migration, rather than every request failing later against a
    ///   missing table;
    /// - the Live key material this process was configured with cannot be
    ///   read (see [`mod@crate::live`]'s own boot error for the underlying
    ///   cause - a missing or malformed application key);
    /// - the configured L1 directory cannot be created or listed.
    ///
    /// Sets the process-installed flag ([`mark_installed`]) before
    /// returning `Ok`, opening the write side's instrumentation gate
    /// (ruling R66) - without this, the cache installs and serves,
    /// generations never advance, and every entry is served stale forever
    /// with nothing failing.
    ///
    /// Never clears the global middleware registry: an application that
    /// registered its own logging, session, CSRF, or auth middleware before
    /// calling this must keep every one of them. A repeated `install()` in
    /// one process (this crate's own test suite calls it once per test) is
    /// a test concern, solved by the test harness clearing the registry
    /// itself before each call - never by this production path.
    pub async fn install(
        router: Router,
        config: RenderCacheConfig,
    ) -> Result<Router, FrameworkError> {
        if !ledger::migration_present().await? {
            return Err(FrameworkError::internal(
                "RenderCache::install: the suprnova_render_epochs table is missing. Add \
                 suprnova::render_cache::migration::Migration to your Migrator's migrations() \
                 list and apply migrations before calling RenderCache::install.",
            ));
        }
        let keys = crate::live::build_key_ring()?;
        let table = router.render_cache_policies().clone();
        let l0 = MemoryRenderStore::new(MemoryStoreLimits {
            max_entries: config.l0.max_entries,
            max_bytes: config.l0.max_bytes,
        });
        let l1 = match &config.l1 {
            L1Config::Disabled => None,
            L1Config::File {
                directory,
                max_bytes,
            } => Some(file_store::FileRenderStore::open(directory, *max_bytes)?),
        };
        let clock = config
            .clock_override
            .clone()
            .unwrap_or_else(|| Arc::new(suprnova_live::clock::SystemClock));
        let coordinator = config.coordinator_override.clone().unwrap_or_else(|| {
            Arc::new(LocalRebuildCoordinator::new(LocalCoordinatorLimits {
                lease_ms: 30_000,
                max_waiters: 128,
            }))
        });
        let epoch_ledger = ledger::SqlGenerationLedger::new();
        let runtime = Arc::new(RenderCacheRuntime {
            config,
            table,
            l0,
            l1,
            ledger: Arc::new(epoch_ledger),
            epoch_ledger,
            coordinator,
            keys,
            clock,
            limits: EntryLimits::default(),
            leases: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        });
        *runtime_slot().write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&runtime));
        // Appends, never clears: `register_global_middleware` is
        // idempotent per type on its own (a second `install()` call in the
        // same process registers nothing further, since a
        // `RenderCacheMiddleware` of some earlier install is already
        // present) - the fix round 1 review proved that an earlier draft's
        // `clear_global_middleware_for_test()` call here silently deleted
        // an application's own logging, session, CSRF, and auth middleware
        // on every install. A process that genuinely needs to replace an
        // already-installed runtime (this crate's own test suite: one
        // process, many `#[tokio::test]` functions) clears the registry
        // itself, in test-only code, before calling this - never here.
        crate::middleware::register_global_middleware(RenderCacheMiddleware);
        mark_installed();
        Ok(router)
    }

    /// The installed runtime, or `None` before [`Self::install`] has run.
    #[must_use]
    pub(crate) fn runtime() -> Option<Arc<RenderCacheRuntime>> {
        runtime_slot()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Bumps the process-wide permission version fed into `Principal`
    /// variance. Call this when an application changes what a signed-in
    /// user is permitted to do (a role reassignment, a permission grant or
    /// revocation) - without it, a user whose permissions just changed
    /// keeps matching the cache key their prior permission set produced,
    /// and keeps being served whatever was cached under it. See ruling R54.
    pub fn bump_permission_version() {
        collector::bump_permission_version();
    }

    /// Emergency invalidation: advances the authority epoch, making every
    /// entry observed at the prior epoch unreachable at its next freshness
    /// check.
    ///
    /// # Errors
    ///
    /// Returns [`RenderCacheError`] when no runtime is installed or the
    /// ledger's epoch update fails.
    pub async fn advance_epoch() -> Result<(), RenderCacheError> {
        let runtime = Self::runtime()
            .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        // Fix round 2, item 7: uses the runtime's own ledger handle rather
        // than constructing an unconnected one, so a future ledger override
        // reaches the epoch-advance operator too. See `epoch_ledger`'s doc.
        runtime.epoch_ledger.advance_epoch().await
    }

    /// Body-free inspection of a stored L0 entry by its encoded key text.
    ///
    /// # Errors
    ///
    /// Returns [`RenderCacheError`] when no runtime is installed, the key
    /// text is malformed, or the store provider fails.
    pub async fn inspect(key_text: &str) -> Result<Option<EntryInspection>, RenderCacheError> {
        let runtime = Self::runtime()
            .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        let key = suprnova_live::render_cache::key::RenderKey::from_base64url(key_text)?;
        let Some(stored) = runtime.l0.get(&key).await? else {
            return Ok(None);
        };
        Ok(Some(suprnova_live::render_cache::inspect(
            &stored.bytes,
            &runtime.limits,
        )?))
    }

    /// Test-only: the key text for a route with default variance and an
    /// optional login.
    #[doc(hidden)]
    #[must_use]
    pub fn key_for_route_for_test(
        pattern: &str,
        params: &[(&str, &str)],
        login: Option<&str>,
    ) -> String {
        let runtime = Self::runtime().expect("RenderCache installed");
        let policy = runtime.table.effective_policy(pattern).expect("policy");
        let input = middleware::key_input_for_test(&runtime, pattern, params, login, &policy);
        suprnova_live::render_cache::key::RenderKey::derive(&input, &runtime.keys)
            .expect("key")
            .to_base64url()
    }

    /// Test-only: L0 inspection of a route with empty params and anonymous
    /// variance.
    #[doc(hidden)]
    pub async fn inspect_route_for_test(pattern: &str) -> Option<EntryInspection> {
        let key = Self::key_for_route_for_test(pattern, &[], None);
        Self::inspect(&key).await.expect("inspect")
    }

    /// Test-only: L1 inspection of a route by pattern, params, and an
    /// optional login - the L1 counterpart of [`Self::inspect_route_for_test`]
    /// and [`Self::key_for_route_for_test`]. Added for fix round 2, item 5:
    /// nothing could inspect the file-backed tier directly by key before
    /// this, so no test could tell L0 and L1 state apart.
    ///
    /// # Panics
    ///
    /// Panics if no runtime is installed, no L1 provider is configured, the
    /// route has no effective policy, the L1 read fails, or a found entry
    /// fails to decode.
    #[doc(hidden)]
    pub async fn inspect_l1_for_test(
        pattern: &str,
        params: &[(&str, &str)],
        login: Option<&str>,
    ) -> Option<EntryInspection> {
        let runtime = Self::runtime().expect("RenderCache installed");
        let l1 = runtime.l1.as_ref().expect("L1 configured");
        let policy = runtime.table.effective_policy(pattern).expect("policy");
        let input = middleware::key_input_for_test(&runtime, pattern, params, login, &policy);
        let key = suprnova_live::render_cache::key::RenderKey::derive(&input, &runtime.keys)
            .expect("key");
        let stored = l1.get(&key).await.expect("l1 get")?;
        Some(suprnova_live::render_cache::inspect(&stored.bytes, &runtime.limits).expect("inspect"))
    }

    /// Test-only: the number of entries currently held in the
    /// [`CoherenceMode::Lease`] validation-lease map. Added for fix round 2,
    /// item 6, to observe the map's bound from outside the crate: nothing
    /// else exposes `RenderCacheRuntime::leases`'s size.
    ///
    /// # Panics
    ///
    /// Panics if no runtime is installed.
    #[doc(hidden)]
    #[must_use]
    pub fn lease_count_for_test() -> usize {
        let runtime = Self::runtime().expect("RenderCache installed");
        runtime
            .leases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}
