//! Integration tests for the `rate_limit` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod default_key;
pub mod identity_key;
pub mod middleware;
pub mod production_fail_closed;
pub mod rate_limit;
pub mod throttle;
