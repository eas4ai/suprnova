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
//! advances the generation of what it changed. Task 13 (this one) adds the
//! file-backed L1 store. The remaining submodules (the Live integration and
//! the middleware) are added by later tasks in the same iteration.

pub mod collector;
pub mod config;
pub mod file_store;
pub mod ledger;
pub mod migration;
pub mod orm;
pub mod registry;
pub mod telemetry;
#[doc(hidden)]
pub mod testing;

pub use config::{FailurePolicy, L0Limits, L1Config, RenderCacheConfig};
pub use suprnova_live::render_cache::generation::DependencyIdentity;
pub use suprnova_live::render_cache::{
    CoherenceMode, DeclineReason, Eligibility, FreshnessPolicy, PolicyPatch, QueryPolicy,
    RenderCachePolicy, RenderCachePolicyBuilder, RepresentationClass, SharedCachePolicy,
    StorageLayers, VarianceDimension,
};

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
/// Called by `RenderCache::install` (a later task in this iteration).
/// Test setup that boots the RenderCache migration directly - without a
/// running `install` to call yet - calls this too; there is no other way
/// today to open the gate. Not part of the public contract: doc-hidden.
#[doc(hidden)]
pub fn mark_installed() {
    INSTALLED.store(true, std::sync::atomic::Ordering::Relaxed);
}
