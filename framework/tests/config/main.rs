//! Integration tests for the `config` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod debug_gating;
pub mod env_loading;
pub mod typed_config;
