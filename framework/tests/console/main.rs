//! Integration tests for the `console` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod command_macro;
pub mod console;
pub mod db_seed;
// Only the `testing`-feature tests of `db:seed` set the environment.
#[cfg(feature = "testing")]
#[path = "../support/env_lock.rs"]
mod env_lock;
#[cfg(feature = "testing")]
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
pub mod harness;
pub mod laravel_infra_gaps;
pub mod process_boot;
pub mod ssr;
pub mod typed;
