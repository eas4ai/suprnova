//! Integration tests for the `queue` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod after_commit;
pub mod batch_repository;
pub mod batches;
pub mod bootstrap;
pub mod chains;
pub mod database;
pub mod database_postgres;
pub mod debounce;
pub mod delayed;
pub mod dispatch;
pub mod drivers_sync_null;
pub mod envelope;
pub mod events;
pub mod failed_store;
pub mod failover;
pub mod fake;
pub mod fault_injection;
pub mod inspection_api;
pub mod introspection;
pub mod memory;
pub mod middleware_pipeline;
pub mod panic_isolation;
pub mod pause;
pub mod reclaim_attempts;
pub mod redis;
pub mod restart;
pub mod retry;
pub mod routing;
pub mod unique;
pub mod unique_until_processing;
pub mod worker;
pub mod worker_postgres;
