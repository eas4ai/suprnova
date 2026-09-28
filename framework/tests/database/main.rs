//! Integration tests for the `database` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod facade;
pub mod identifier_validation;
pub mod medium_audit;
pub mod multiconnection;
pub mod observability;
pub mod pool_liveness;
pub mod query_binary_comparison;
pub mod raw_helpers;
pub mod sea_orm_aliases;
pub mod transactions;
pub mod tx_leak_diagnostic;
pub mod url_prod_validation;
