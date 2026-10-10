//! Integration tests for the `middleware` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

// `own_process::run_alone` takes the environment lock before it starts a
// child.
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/http_wire.rs"]
mod http_wire;
// A test that reads the priority list of a fresh process runs alone in a
// child.
#[path = "../support/own_process.rs"]
mod own_process;

/// Tests observe the agreed infrastructure gaps.
pub mod laravel_infra_gaps;
pub mod maintenance_middleware;
pub mod named;
pub mod panic_safety;
pub mod priority;
