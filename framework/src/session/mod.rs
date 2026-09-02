//! Session management for suprnova framework
//!
//! Provides Laravel-like session handling with database storage.
//!
//! # Features
//!
//! - Secure session cookies (HttpOnly, Secure, SameSite)
//! - Database-backed storage for scalability
//! - CSRF token generation per session
//! - Flash messages for one-time notifications
//! - Session data stored as JSON
//!
//! # Example
//!
//! ```rust,no_run
//! use suprnova::session::{session, session_mut};
//!
//! // Read from session
//! if let Some(s) = session() {
//!     let name: Option<String> = s.get("name");
//! }
//!
//! // Write to session
//! session_mut(|s| {
//!     s.put("name", "John");
//!     s.flash("success", "Item saved!");
//! });
//! ```
//!
//! # Setup
//!
//! Add the `SessionMiddleware` to your bootstrap:
//!
//! ```rust,no_run
//! use suprnova::{global_middleware, SessionMiddleware, SessionConfig};
//!
//! pub async fn register() {
//!     let config = SessionConfig::from_env();
//!     global_middleware!(SessionMiddleware::install(config).await);
//! }
//! ```

pub mod config;
pub mod driver;
pub mod middleware;
pub mod store;

pub use config::SessionConfig;
pub use driver::DatabaseSessionDriver;
pub use middleware::{
    SessionGcMetrics, SessionGcSupervisor, SessionMiddleware, auth_user_id, clear_auth_user,
    clear_two_factor_pending, clear_two_factor_pending_remember, generate_csrf_token,
    generate_session_id, get_csrf_token, invalidate_session, is_authenticated,
    regenerate_csrf_token, regenerate_session_id, session, session_gc_metrics, session_mut,
    set_auth_user, set_two_factor_pending, set_two_factor_pending_remember,
    two_factor_pending_remember, two_factor_pending_user_id,
};
pub use store::{SessionData, SessionStore, is_valid_session_id};

/// Destroy every session belonging to `user_id`. Returns the number of
/// session rows deleted.
///
/// Called after security-state transitions where a credential rotation
/// must not leave stale sessions valid:
/// - [`crate::auth_flows::PasswordReset::complete`] - password rotated,
///   stolen sessions revoked.
/// - Future hooks for 2FA disable, account recovery, admin-forced
///   logout.
///
/// # Security - SEC-02(b)
///
/// This used to construct a fresh [`DatabaseSessionDriver`]
/// unconditionally, regardless of what store the app actually
/// configured `SessionMiddleware` with - so an app running a custom
/// [`SessionStore`] (Redis, for instance) had every revocation call
/// silently no-op against the wrong backend while still reporting
/// success. `SessionMiddleware::new` / `with_store` now register the
/// store they were built with via [`crate::container::App::bind_if_absent`]
/// (see `session::middleware::register_configured_store`), so this
/// resolves the *actually configured* store from the container first.
/// The `DatabaseSessionDriver` construction below only fires when
/// nothing is registered - e.g. a test or embedder that drives session
/// state without ever constructing a `SessionMiddleware` - preserving
/// the original default-driver behaviour for that case.
pub async fn destroy_all_for_user(user_id: &str) -> Result<u64, crate::error::FrameworkError> {
    if let Some(store) = crate::container::App::make::<dyn SessionStore>() {
        return store.destroy_for_user(user_id).await;
    }
    let driver = driver::DatabaseSessionDriver::new(std::time::Duration::from_secs(0));
    driver.destroy_for_user(user_id).await
}

// Test helpers - these mirror the per-request session scope that
// `SessionMiddleware` sets up at runtime. Crates outside the framework
// shouldn't need these; they exist for unit/integration tests that
// drive code paths reading the session without booting a full server.
#[doc(hidden)]
pub fn new_session_slot_for_test() -> std::sync::Arc<std::sync::Mutex<Option<SessionData>>> {
    std::sync::Arc::new(std::sync::Mutex::new(Some(SessionData::new(
        "test_session".to_string(),
        "test_csrf_token".to_string(),
    ))))
}

#[doc(hidden)]
pub async fn session_scope_for_test<F: std::future::Future>(
    slot: std::sync::Arc<std::sync::Mutex<Option<SessionData>>>,
    fut: F,
) -> F::Output {
    middleware::SESSION_CONTEXT.scope(slot, fut).await
}

/// Test-only: install every request scope required for session binding.
#[doc(hidden)]
#[cfg(feature = "testing")]
pub async fn session_bind_scopes_for_test<F: std::future::Future>(
    slot: std::sync::Arc<std::sync::Mutex<Option<SessionData>>>,
    future: F,
) -> F::Output {
    middleware::session_bind_scopes_for_test(slot, future).await
}

/// Test-only: a fresh pending-cookies slot. Use with
/// [`pending_cookies_scope_for_test`] to drive `Auth::login_remember`
/// and friends without booting `SessionMiddleware`.
#[doc(hidden)]
pub fn new_pending_cookies_slot_for_test()
-> std::sync::Arc<std::sync::Mutex<Vec<crate::http::cookie::Cookie>>> {
    std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))
}

#[doc(hidden)]
pub async fn pending_cookies_scope_for_test<F: std::future::Future>(
    slot: std::sync::Arc<std::sync::Mutex<Vec<crate::http::cookie::Cookie>>>,
    fut: F,
) -> F::Output {
    middleware::PENDING_COOKIES.scope(slot, fut).await
}
