//! Integration tests for the `auth_flows` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[cfg(feature = "testing")]
#[path = "../support/magnetar_auth.rs"]
mod magnetar_auth;

pub mod brute_force;
pub mod email_verified_middleware;
pub mod email_verified_middleware_fail_closed;
pub mod email_verify;
pub mod login_throttle_backend_error;
pub mod password_reset;
pub mod password_reset_provider;
pub mod two_factor;
pub mod two_factor_brute_force_integration;
pub mod two_factor_challenge_flow;
pub mod two_factor_challenge_middleware;
