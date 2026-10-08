//! Logging - structured `tracing`-based output with env-driven config,
//! written to log channels.
//!
//! `tracing`'s macros write to the default channel, the one `LOG_CHANNEL`
//! names (`stdout` unless set). [`Log`] reaches the others: files that
//! rotate, syslog, stacks, and the drivers an application adds.

pub mod channel;
pub mod config;
mod facade;
pub mod init;
pub mod layer;
pub mod request_id;
mod sinks;

pub use channel::{LogChannel, LogLevel, LogRecord, LogSink};
pub use config::{LogConfig, LogFormat};
pub use facade::{Log, Logger};
pub use init::init_subscriber;
pub use layer::{build_subscriber, check_channels};
pub use request_id::{
    REQUEST_ID, RequestId, RequestIdMiddleware, current_request_id, spawn_with_request_id,
};
