//! Integration tests for the `render_cache` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

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

pub mod collector;
pub mod file_store;
pub mod ledger;
pub mod live;
pub mod middleware;
pub mod operations;
pub mod orm;
pub mod privacy;
pub mod races;
pub mod registry;
pub mod stitch;
