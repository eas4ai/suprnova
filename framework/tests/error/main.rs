//! Integration tests for the `error` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod debug_error_page;
pub mod domain_error_macro;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
pub mod external;
pub mod panic_response_contract;
pub mod responses;
