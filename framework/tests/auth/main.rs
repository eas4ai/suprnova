//! Integration tests for the `auth` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[cfg(feature = "testing")]
#[path = "../support/magnetar_auth.rs"]
mod magnetar_auth;

pub mod bearer_token_without_session;
pub mod database_provider;
pub mod dummy_verify_timing;
pub mod eloquent_provider;
pub mod http_middleware;
pub mod providerless_fallback;
pub mod remember_me;
pub mod session_commit_boundary;
pub mod session_guard;
