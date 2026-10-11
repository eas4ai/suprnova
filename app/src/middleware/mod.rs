//! Application middleware
//!
//! Each middleware has its own dedicated file following the framework convention.

mod auth;
mod logging;

pub use auth::AuthMiddleware;
pub use logging::LoggingMiddleware;
