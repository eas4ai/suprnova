//! Integration tests for the `error` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod domain_error_macro;
pub mod external;
pub mod panic_response_contract;
pub mod responses;
