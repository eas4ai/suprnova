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
//! file-backed L1 store. Task 14 (this one) adds the middleware and
//! [`RenderCache::install`], which assembles the runtime and puts the
//! middleware on the router. The Live integration - reading a seed
//! promotion deadline out of a mounted document for `EntryHeader` - is a
//! later task in the same iteration.

pub mod collector;
pub mod config;
pub mod file_store;
pub mod ledger;
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
    /// `router` has registered, installs the middleware, and returns the
    /// router unchanged (the middleware itself is added to the process-wide
    /// global middleware chain - see the note on
    /// [`crate::middleware::register_global_middleware`] about snapshotting
    /// it into a [`crate::middleware::MiddlewareRegistry`] before serving).
    ///
    /// # Errors
    ///
    /// Fails when:
    /// - the RenderCache migration's tables are not present on the primary
    ///   connection (ruling R58) - an actionable error naming the
    ///   migration, rather than every request failing later against a
    ///   missing table;
    /// - the Live key material this process was configured with cannot be
    ///   read (see [`crate::live`]'s own boot error for the underlying
    ///   cause - a missing or malformed application key);
    /// - the configured L1 directory cannot be created or listed.
    ///
    /// Sets the process-installed flag ([`mark_installed`]) before
    /// returning `Ok`, opening the write side's instrumentation gate
    /// (ruling R66) - without this, the cache installs and serves,
    /// generations never advance, and every entry is served stale forever
    /// with nothing failing.
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
        let runtime = Arc::new(RenderCacheRuntime {
            config,
            table,
            l0,
            l1,
            ledger: Arc::new(ledger::SqlGenerationLedger::new()),
            coordinator,
            keys,
            clock,
            limits: EntryLimits::default(),
            leases: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        });
        *runtime_slot().write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&runtime));
        // Idempotent-per-type registration (see `register_global_middleware`'s
        // own doc) means a second `install()` in the same process - every run
        // of this crate's own middleware test suite, one process, many
        // `#[tokio::test]` functions - would otherwise keep dispatching to the
        // FIRST call's now-stale runtime forever. Clearing first is safe in
        // production too: `install` is called once, at boot, before any other
        // global middleware would have had a request to run against.
        crate::middleware::clear_global_middleware_for_test();
        crate::middleware::register_global_middleware(RenderCacheMiddleware::new(runtime));
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
        if Self::runtime().is_none() {
            return Err(RenderCacheError::new(
                RenderCacheErrorKind::ProviderUnavailable,
            ));
        }
        ledger::SqlGenerationLedger::new().advance_epoch().await
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
}
