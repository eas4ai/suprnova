//! Built-in typed config providers shipped with the framework.
//!
//! [`AppConfig`] covers the cross-cutting app metadata (name,
//! environment, debug flag); [`ServerConfig`] covers HTTP server bind +
//! TLS settings. Application crates register additional providers via
//! [`Config::register`](crate::Config::register).

mod app;
mod server;

pub(crate) use app::DEFAULT_URL as DEFAULT_APP_URL;
pub use app::{AppConfig, AppConfigBuilder};
pub use server::{
    DEFAULT_BODY_READ_TIMEOUT_SECS, DEFAULT_BODY_TOTAL_TIMEOUT_SECS,
    DEFAULT_HEADER_READ_TIMEOUT_SECS, DEFAULT_MAX_CONNECTIONS,
    DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION, ServerConfig, ServerConfigBuilder,
};
