//! Integration tests for the `Process` facade (PAR-021 to PAR-025).
//!
//! The real processes are POSIX tools (`sh`, `sleep`, `sort`), so the
//! binary is Unix only. The fake is process-global while its guard lives,
//! so every test here is `#[serial]`: a real run beside a faked one would
//! be faked.

#![cfg(unix)]

// `Process::fake` exists with the `testing` feature only.
#[cfg(feature = "testing")]
pub mod fake;
pub mod pool;
pub mod run;
pub mod support;
pub mod timeouts;
