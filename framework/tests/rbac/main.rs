//! Integration tests for the `rbac` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod concurrent_grants;
pub mod gate_bridge;
pub mod migration;
// Its tests move the framework clock with `TestClock`, which exists with
// the `testing` feature only.
#[cfg(feature = "testing")]
pub mod native_engines;
pub mod postgres;
pub mod rbac;
