//! Integration tests for the `testing` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/common.rs"]
mod common;

pub mod assertable_inertia;
pub mod clock;
pub mod clock_reads;
pub mod registry_clears;
pub mod request_diagnostics;
pub mod test_database_helpers;
pub mod test_response;
