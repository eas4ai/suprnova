//! Integration tests for the `auth_flows` module: one binary per module,
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

pub mod brute_force;
pub mod email_verified_middleware;
pub mod email_verified_middleware_fail_closed;
#[cfg(feature = "testing")]
pub mod email_verify;
pub mod email_verify_engines;
pub mod login_throttle_backend_error;
pub mod password_reset;
pub mod password_reset_provider;
#[cfg(feature = "testing")]
pub mod scaffold_token_table;
#[cfg(feature = "testing")]
pub mod two_factor;
pub mod two_factor_brute_force_integration;
pub mod two_factor_challenge_flow;
pub mod two_factor_challenge_middleware;
#[cfg(feature = "testing")]
pub mod two_factor_engines;
