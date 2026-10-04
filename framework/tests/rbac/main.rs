//! Integration tests for the `rbac` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod gate_bridge;
pub mod migration;
pub mod postgres;
pub mod rbac;
#[cfg(feature = "testing")]
pub mod shipped_tables;
