//! Integration tests for the `server` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod header_read_timeout;
pub mod health_error_redaction;
pub mod health_readiness_gate;
#[path = "../support/own_process.rs"]
pub mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;
pub mod shutdown;
