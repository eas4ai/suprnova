//! Integration tests for the `seed` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
pub mod fake_crate;
pub mod seeders;
pub mod without_events_integration;

pub mod laravel_gaps;
