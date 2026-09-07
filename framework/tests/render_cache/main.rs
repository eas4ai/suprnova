//! Integration tests for the `render_cache` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/live_dogfood_support/mod.rs"]
mod live_dogfood_support;
#[path = "../support/render_cache_live_support/mod.rs"]
mod render_cache_live_support;
#[path = "../support/render_cache_middleware_support/mod.rs"]
mod render_cache_middleware_support;
#[path = "../support/render_cache_operations_support/mod.rs"]
mod render_cache_operations_support;
#[path = "../support/render_cache_privacy_support/mod.rs"]
mod render_cache_privacy_support;
#[path = "../support/render_cache_stitch_support/mod.rs"]
mod render_cache_stitch_support;
#[path = "../support/render_cache_support/mod.rs"]
mod render_cache_support;
#[path = "../support/render_cache_tiers_support/mod.rs"]
mod render_cache_tiers_support;

pub mod bypass;
pub mod collector;
pub mod file_store;
pub mod ledger;
pub mod live;
pub mod middleware;
pub mod no_delays;
pub mod operations;
pub mod orm;
pub mod privacy;
pub mod races;
pub mod registry;
pub mod stitch;
pub mod store_conformance;
pub mod tiers;
