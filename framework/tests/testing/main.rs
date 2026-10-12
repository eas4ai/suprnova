//! Integration tests for the `testing` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/common.rs"]
mod common;
// `own_process_async` takes the environment lock before it starts a child.
#[path = "../support/env_lock.rs"]
mod env_lock;
// The application convention `test_database!()` resolves against.
mod migrations;
// A test that registers a process-wide `AppConfig` runs alone in a child.
#[path = "../support/own_process.rs"]
pub mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;

pub mod assertable_inertia;
pub mod clock;
pub mod clock_reads;
/// Tests observe the agreed infrastructure gaps.
pub mod laravel_infra_gaps;
/// The test client and test response gaps, observed by the `par-laravel-gaps-testing` mechanism.
pub mod laravel_testing_gaps;
pub mod precognition;
pub mod registry_clears;
pub mod request_diagnostics;
pub mod test_client;
pub mod test_database_helpers;
pub mod test_response;
