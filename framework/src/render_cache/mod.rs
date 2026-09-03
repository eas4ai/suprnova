//! Suprnova's RenderCache: route and group policy, one global middleware
//! that serves proven Complete representations, a request-scoped dependency
//! collector, database-authoritative generations, and Tier 0 providers.
//!
//! Live is complete without it. Enabling it changes avoided work, never
//! application capability.
//!
//! This module starts small: Task 9 added configuration, route and group
//! policy registration, and effective policy resolution. Task 10 added the
//! request-scoped dependency collector. Task 11 (this one) adds the
//! database-authoritative generation ledger and its migration. The
//! remaining submodules (the file store, the Live integration, the
//! middleware, and the ORM entities) are added by later tasks in the same
//! iteration.

pub mod collector;
pub mod config;
pub mod ledger;
pub mod migration;
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
