//! CSRF protection for suprnova framework
//!
//! Provides Laravel-like CSRF protection using per-session tokens.
//!
//! # How it works
//!
//! 1. Each session has a unique CSRF token
//! 2. The token is included in HTML responses via a meta tag
//! 3. The frontend (Inertia.js) reads the token and sends it with requests
//! 4. The middleware validates the token on state-changing requests
//!
//! # Setup
//!
//! Add the middleware after SessionMiddleware, and hand it the same
//! `SessionConfig`:
//!
//! ```rust,no_run
//! use suprnova::{global_middleware, SessionMiddleware, CsrfMiddleware, SessionConfig};
//!
//! pub async fn register() {
//!     let config = SessionConfig::from_env();
//!     global_middleware!(SessionMiddleware::new(config.clone()));
//!     global_middleware!(CsrfMiddleware::new().with_session_config(&config));
//! }
//! ```
//!
//! [`CsrfMiddleware::with_session_config`] copies the session cookie's
//! `Secure`, `SameSite`, `Domain`, `Path` and lifetime onto the JS-readable
//! `XSRF-TOKEN` cookie, so the two cannot drift apart. Without it that
//! cookie is always `Secure`; a browser will neither store nor return it
//! over `http://localhost`, so a development server running with
//! `SESSION_SECURE=false` refuses every state-changing request with
//! `419 CSRF token mismatch`.
//!
//! # Frontend Integration
//!
//! Add the CSRF meta tag to your HTML:
//!
//! ```html
//! <meta name="csrf-token" content="{{ csrf_token() }}">
//! ```
//!
//! Do not forward the token by hand on every Inertia visit. The Inertia
//! client sends its visits over XMLHttpRequest and, whenever the
//! `XSRF-TOKEN` cookie this middleware sets is present, echoes it back in
//! the `X-XSRF-TOKEN` header itself, once per request. Both header names
//! are accepted here, so nothing further is needed.
//!
//! Reading `<meta name="csrf-token">` once at module load and pinning that
//! value into a header is the pattern to avoid: logging in rotates the
//! session, the captured token goes stale, and the next visit - the logout
//! - is refused with a `419`. A form that submits a `_token` field, or any
//! code that reads the meta tag *per request* rather than once, is
//! unaffected.

pub mod middleware;

pub use middleware::{CsrfMiddleware, OriginPolicy};

use crate::session::get_csrf_token;

/// Get the current CSRF token
///
/// Returns None if no session is active.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::csrf::csrf_token;
///
/// if let Some(token) = csrf_token() {
///     // Use token in response
/// }
/// ```
pub fn csrf_token() -> Option<String> {
    get_csrf_token()
}

/// Generate a CSRF meta tag for HTML responses
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::csrf::csrf_meta_tag;
///
/// let meta = csrf_meta_tag();
/// // Returns: <meta name="csrf-token" content="...">
/// ```
pub fn csrf_meta_tag() -> String {
    csrf_token()
        .map(|token| format!(r#"<meta name="csrf-token" content="{}">"#, token))
        .unwrap_or_default()
}

/// Generate a hidden CSRF input field for forms
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::csrf::csrf_field;
///
/// let field = csrf_field();
/// // Returns: <input type="hidden" name="_token" value="...">
/// ```
pub fn csrf_field() -> String {
    csrf_token()
        .map(|token| format!(r#"<input type="hidden" name="_token" value="{}">"#, token))
        .unwrap_or_default()
}
