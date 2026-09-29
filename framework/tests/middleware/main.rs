//! Integration tests for the `middleware` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/http_wire.rs"]
mod http_wire;
pub mod maintenance_middleware;
pub mod named;
pub mod panic_safety;
pub mod priority;
