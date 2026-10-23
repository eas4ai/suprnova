//! Integration tests for the `cors` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/http_wire.rs"]
mod http_wire;
pub mod middleware;
