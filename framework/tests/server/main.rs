//! Integration tests for the `server` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod header_read_timeout;
pub mod health_error_redaction;
pub mod health_readiness_gate;
pub mod shutdown;
