//! Integration tests for the `config` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod debug_gating;
pub mod env_loading;
#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod typed_config;
