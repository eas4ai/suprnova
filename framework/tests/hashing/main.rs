//! Integration tests for the `hashing` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod argon2_and_driver_swap;
pub mod async_and_truncation;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/own_process.rs"]
mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;
pub mod rtc_gate;
