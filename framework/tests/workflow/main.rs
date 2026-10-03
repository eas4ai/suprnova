//! Integration tests for the `workflow` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod claim_postgres;
pub mod migrations;
pub mod steps;
pub mod worker_config;
