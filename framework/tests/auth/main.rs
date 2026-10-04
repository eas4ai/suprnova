//! Integration tests for the `auth` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
#[path = "../support/http_wire.rs"]
mod http_wire;
#[cfg(feature = "testing")]
#[path = "../support/magnetar_auth.rs"]
mod magnetar_auth;
#[path = "../support/own_process.rs"]
mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;

pub mod bearer_token_without_session;
pub mod custom_guard;
pub mod database_provider;
pub mod dummy_verify_timing;
pub mod eloquent_provider;
pub mod http_middleware;
pub mod providerless_fallback;
pub mod remember_me;
#[cfg(feature = "testing")]
pub mod scaffold_tables;
pub mod session_commit_boundary;
pub mod session_guard;
