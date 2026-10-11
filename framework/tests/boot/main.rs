//! Integration tests for the `boot` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod env_after_runtime;
#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod main_bootstrap_boundaries;
pub mod wiring;
