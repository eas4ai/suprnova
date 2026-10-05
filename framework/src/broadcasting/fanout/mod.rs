//! Cross-process broadcasting fanout over Redis Streams.
//!
//! This module is only compiled when the `broadcasting-fanout` feature is
//! enabled. Apps that don't need multi-process fanout depend on `suprnova`
//! without this feature and do not compile it.
//!
//! # Usage
//!
//! ```toml
//! # Cargo.toml
//! suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v3.2.0", features = ["broadcasting-fanout"] }
//! ```
//!
//! ```rust,no_run
//! use suprnova::broadcasting::fanout::SeaStreamerBroadcastHub;
//! use suprnova::broadcasting::BroadcastHub;
//! use std::sync::Arc;
//!
//! # async fn ex() {
//! let hub = Arc::new(
//!     SeaStreamerBroadcastHub::new("redis://127.0.0.1:6379", "my-app-broadcast")
//!         .await
//!         .expect("connect"),
//! );
//! // Register hub in the container so handlers receive it via injection.
//! # }
//! ```

mod sea_streamer;
pub use sea_streamer::SeaStreamerBroadcastHub;
