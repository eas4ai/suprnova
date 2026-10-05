//! Integration tests for the `magnetar_integration` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
#[cfg(feature = "testing")]
#[path = "../support/magnetar_auth.rs"]
mod magnetar_auth;
#[path = "../support/own_process.rs"]
mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;

pub mod abuse_limiter;
#[cfg(feature = "testing")]
pub mod api_starter_users;
pub mod atomic_install;
pub mod binding;
#[cfg(feature = "testing")]
pub mod ceremony_store;
pub mod default_engine;
pub mod factor_completion;
pub mod framework_login;
pub mod host_engine;
pub mod integration;
pub mod missing_engine_diagnostics;
pub mod oauth_avatar;
pub mod oauth_factor_install;
pub mod oauth_only;
pub mod oauth_reqwest_transport;
pub mod remember_middleware;
