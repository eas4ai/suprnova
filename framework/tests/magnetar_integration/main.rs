//! Integration tests for the `magnetar_integration` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[cfg(feature = "testing")]
#[path = "../support/magnetar_auth.rs"]
mod magnetar_auth;

pub mod abuse_limiter;
pub mod atomic_install;
pub mod binding;
pub mod default_engine;
pub mod factor_completion;
pub mod host_engine;
pub mod integration;
pub mod missing_engine_diagnostics;
pub mod oauth_factor_install;
pub mod oauth_only;
pub mod oauth_reqwest_transport;
pub mod remember_middleware;
