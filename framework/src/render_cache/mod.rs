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
//! middleware on the router. Task 15 adds the Live integration in
//! [`live`]: reading a seed promotion deadline out of a mounted document
//! for `EntryHeader`, and declining identity-bound documents. Task 16
//! (this one) adds operator control: [`RenderCache::store_inspection`] and
//! [`RenderCache::sweep`] alongside the epoch and body-free inspection
//! operators Task 14/15's fix rounds already added, plus the hidden
//! console commands in [`console`]. Task 16's own fix round 1 (R93-R95)
//! made retention a `RenderStore::publish` parameter bound to the Dead
//! edge in [`suprnova_live::render_cache::FreshnessPolicy::dead_after_ms`],
//! bounded [`file_store::FileRenderStore::sweep`] to a fixed number of
//! candidates per call, made [`RenderCache::advance_epoch`] clear L0, and
//! made the console commands honest about failure and the current epoch.

pub mod collector;
pub mod config;
/// `pub` only so its `#[doc(hidden)]` `_for_test` report builders are
/// reachable from integration tests outside this crate; it registers no
/// public command API of its own (see its own module doc).
pub mod console;
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
pub use file_store::SweepOutcome;
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

/// L0 occupancy, bounds, and the current authority epoch. See
/// [`RenderCache::store_inspection`]'s own doc for why the epoch travels
/// with it: the engine's own
/// [`suprnova_live::render_cache::store::StoreInspection`] has no concept
/// of an authority epoch (it is a store-level fact shared by any
/// [`suprnova_live::render_cache::store::RenderStore`] implementor,
/// including a generic in-process test double with no ledger at all), so
/// this framework-level type wraps it with the one framework-level fact
/// (the ledger's current epoch) an operator needs alongside it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreInspection {
    /// Entries held in L0.
    pub entries: usize,
    /// Bytes held in L0.
    pub bytes: usize,
    /// The current authority epoch.
    pub epoch: u64,
}

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
    /// - `APP_BUILD_ID` (`RenderCacheConfig::build_id`) does not satisfy the
    ///   bounded build identity grammar (final review, F6): the value is
    ///   parsed once here, before anything else, and an unparsable one is an
    ///   actionable error naming the variable and the accepted shape rather
    ///   than every deploy silently sharing one `default` namespace;
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
    ///
    /// A configuration whose [`RenderCacheConfig::enabled`] master switch is
    /// false (`RENDER_CACHE_ENABLED=false`) makes this a no-op: the router
    /// is returned untouched, no migration probe is issued, no runtime is
    /// assembled, no middleware is registered, and the write side's
    /// instrumentation gate stays shut. That is what an off switch has to
    /// mean at install time. An application that turns the cache off must
    /// not be made to carry the RenderCache migration, must not fail to
    /// boot on ruling R58's probe, and must not pay ledger SQL on its
    /// writes. [`middleware::RenderCacheMiddleware`] re-reads the same flag
    /// on every request. That check is redundant now: this is the only
    /// place a [`RenderCacheRuntime`] is built, so a runtime that exists at
    /// all was installed from a configuration whose switch was on. It is
    /// kept as defence in depth against a future second construction site,
    /// and for no other reason.
    pub async fn install(
        router: Router,
        config: RenderCacheConfig,
    ) -> Result<Router, FrameworkError> {
        if !config.enabled {
            return Ok(router);
        }
        // Parsed before the migration probe: this is pure configuration
        // validation and needs no database, so a misconfigured build id
        // fails the same way with or without one.
        let build = suprnova_live::identity::BuildId::parse(&config.build_id).map_err(|_| {
            FrameworkError::internal(format!(
                "RenderCache::install: APP_BUILD_ID {:?} is not a valid build identity. The \
                 value must be 1 to 64 bytes of ASCII letters, digits, '.', '_', or '-' (the \
                 same grammar as a crate version such as 1.3.7 or 1.3.7-rc.1); a '+' build \
                 metadata suffix, whitespace, or any other character is refused, because a \
                 build id that could not be parsed would otherwise collapse every deploy \
                 onto one shared cache namespace.",
                config.build_id
            ))
        })?;
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
            build,
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

    /// Advances the permission version. Call this when an application
    /// changes what a signed-in user is permitted to do (a role
    /// reassignment, a permission grant or revocation) - without it, a user
    /// whose permissions just changed keeps matching the cache key their
    /// prior permission set produced, and keeps being served whatever was
    /// cached under it. See ruling R54.
    ///
    /// The version is a persisted generation, not a process counter (final
    /// review, F3 / ruling R119): this advances
    /// [`collector::permission_version_identity`] through the same ledger
    /// path an ORM write's advance takes ([`orm`]'s own `advance`), so the
    /// bump joins the caller's ambient `DB::transaction` when there is one
    /// (a role change and its bump commit or roll back together) and opens
    /// its own otherwise, and is recorded in `suprnova_render_generations`
    /// and its change log. Every render whose key carries a resolved
    /// `Principal` observes that identity, so each such entry published
    /// before the bump fails its coherence check at its next lookup, in this
    /// process and in every later one sharing the database: a restart no
    /// longer resurrects a pre-bump entry from an L1 directory, which the
    /// earlier process-local counter allowed.
    ///
    /// A no-op that returns `Ok(())` when no RenderCache runtime is
    /// installed in this process, matching every other write-side advance:
    /// nothing is cached, so there is nothing to invalidate, and an
    /// application that never installs RenderCache pays no SQL here.
    ///
    /// # Errors
    ///
    /// Returns the ledger's own error when the advance cannot be committed
    /// (a database failure, or a missing `suprnova_render_epochs` table under
    /// the caller's own transaction).
    pub async fn bump_permission_version() -> Result<(), FrameworkError> {
        orm::advance(vec![collector::permission_version_identity()]).await
    }

    /// Emergency invalidation: advances the authority epoch, making every
    /// entry observed at the prior epoch unreachable at its next freshness
    /// check, and clears L0 immediately (fix round 1, R94/F11).
    ///
    /// L0 is cleared, not merely left to age out: every L0 key embeds the
    /// epoch it was derived under
    /// ([`suprnova_live::render_cache::key::RenderKey::derive`]), so the
    /// instant this returns, every existing L0 entry is unreachable to any
    /// future request - it names a key nothing can ever derive again. L0
    /// is in-process memory with no filesystem to reconcile, so a full,
    /// unconditional clear is both correct and free; unlike L1's
    /// [`Self::sweep`], there is no bounded-work concern to balance against
    /// an unbounded pause, since clearing a `BTreeMap` and a `VecDeque` is
    /// not I/O. L1 is not cleared here - it still holds every pre-epoch
    /// file until [`Self::sweep`] reclaims it, bounded per call; see that
    /// method's own doc.
    ///
    /// # Errors
    ///
    /// Returns [`RenderCacheError`] when no runtime is installed or the
    /// ledger's epoch update fails. L0 is cleared only after the ledger
    /// update itself succeeds.
    pub async fn advance_epoch() -> Result<(), RenderCacheError> {
        let runtime = Self::runtime()
            .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        // Fix round 2, item 7: uses the runtime's own ledger handle rather
        // than constructing an unconnected one, so a future ledger override
        // reaches the epoch-advance operator too. See `epoch_ledger`'s doc.
        runtime.epoch_ledger.advance_epoch().await?;
        runtime.l0.clear();
        Ok(())
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

    /// L0 occupancy, bounds, and the current authority epoch (fix round 1,
    /// R95/F10) - the epoch an operator needs to judge whether an entry
    /// [`Self::inspect`] shows (which carries its own, possibly older,
    /// epoch) is still current. Never exposes a body or a raw key.
    ///
    /// # Errors
    ///
    /// Returns [`RenderCacheError`] when no runtime is installed, the L0
    /// provider fails, or the ledger epoch read fails.
    pub async fn store_inspection() -> Result<StoreInspection, RenderCacheError> {
        let runtime = Self::runtime()
            .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        let raw = runtime.l0.inspect().await?;
        let epoch = runtime.ledger.epoch().await?;
        Ok(StoreInspection {
            entries: raw.entries,
            bytes: raw.bytes,
            epoch,
        })
    }

    /// Removes on-disk L1 entries that are dead by retention or by epoch
    /// (see [`file_store::FileRenderStore::sweep`]); returns how many were
    /// removed and whether more dead entries remain, bounded per call. A
    /// no-op returning `Ok(SweepOutcome { removed: 0, more_remain: false
    /// })` when no L1 provider is configured - disk hygiene has nothing to
    /// do in that case, not a misconfiguration. L0's unreachable-by-epoch
    /// entries are not this method's concern: [`Self::advance_epoch`]
    /// clears them immediately, since L0 has no filesystem to bound the
    /// work against (see that method's own doc).
    ///
    /// # Errors
    ///
    /// Returns [`RenderCacheError`] when no runtime is installed or the
    /// ledger epoch read fails.
    pub async fn sweep() -> Result<SweepOutcome, RenderCacheError> {
        let runtime = Self::runtime()
            .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        let Some(l1) = runtime.l1.as_ref() else {
            return Ok(SweepOutcome {
                removed: 0,
                more_remain: false,
            });
        };
        let epoch = runtime.ledger.epoch().await?;
        l1.sweep(runtime.now_ms(), epoch).await
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

#[cfg(test)]
mod tests {
    //! `RENDER_CACHE_ENABLED=false` has to be an off switch at install time,
    //! not only per request. An application that turns the cache off must
    //! not be made to carry the RenderCache migration, must not fail ruling
    //! R58's boot probe, and must not open the write side's instrumentation
    //! gate for a cache that will never serve anything. This test binary has
    //! no database configured at all, so a probe that still ran would fail
    //! loudly right here.
    use super::*;
    use crate::http::text;

    fn disabled_config() -> RenderCacheConfig {
        RenderCacheConfig {
            enabled: false,
            l0: L0Limits {
                max_entries: 1,
                max_bytes: 1024,
            },
            l1: L1Config::Disabled,
            failure: FailurePolicy::Open,
            build_id: "disabled-install-test".to_owned(),
            clock_override: None,
            coordinator_override: None,
        }
    }

    /// Final review, F6: an `APP_BUILD_ID` that does not satisfy `BuildId`'s
    /// grammar fails install with an error naming the variable and the
    /// accepted shape, before any database is consulted (this binary has
    /// none). Before the fix the key path silently fell back to `default`
    /// on every request instead.
    ///
    /// Proven by revert: with the parse in `install` restored to the old
    /// per-request `unwrap_or_else(.. "default" ..)` fallback, `install`
    /// reaches the migration probe and fails on the missing database with a
    /// message that never mentions `APP_BUILD_ID`, so the first assertion
    /// below fails.
    #[tokio::test]
    async fn an_unparsable_app_build_id_fails_install_with_an_actionable_error() {
        let router: Router = Router::new()
            .get("/unparsable-build-id-probe", |_req| async { text("probe") })
            .into();
        let mut config = disabled_config();
        config.enabled = true;
        config.build_id = "build 7".to_owned();

        let message = match RenderCache::install(router, config).await {
            Ok(_) => panic!("a build id with a space must be refused at install"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("APP_BUILD_ID") && message.contains("build 7"),
            "the error must name the variable and echo the rejected value: {message}"
        );
        assert!(
            message.contains("letters, digits"),
            "the error must state the accepted shape: {message}"
        );
        assert!(
            RenderCache::runtime().is_none(),
            "a refused install must leave no runtime behind"
        );

        // The shipped default must parse, or every install would now fail:
        // `RenderCacheConfig::from_env` falls back to the application's
        // `CARGO_PKG_VERSION`, which is plain dotted digits.
        let default_build_id = RenderCacheConfig::from_env().build_id;
        assert!(
            suprnova_live::identity::BuildId::parse(&default_build_id).is_ok(),
            "the default build id {default_build_id:?} must satisfy the grammar install enforces"
        );
    }

    #[tokio::test]
    async fn a_disabled_configuration_installs_nothing_and_probes_nothing() {
        let router: Router = Router::new()
            .get("/disabled-render-cache-probe", |_req| async {
                text("probe")
            })
            .into();

        let returned = RenderCache::install(router, disabled_config())
            .await
            .expect(
                "a disabled RenderCache must install without a database: the migration probe \
                 must not run at all",
            );

        assert!(
            returned
                .match_route(&hyper::Method::GET, "/disabled-render-cache-probe")
                .is_some(),
            "a disabled install must hand back the application's own router untouched",
        );
        assert!(
            RenderCache::runtime().is_none(),
            "a disabled install must assemble no runtime: with one installed the middleware \
             would be registered and every request would pay a lookup for a cache that is off",
        );
    }
}
