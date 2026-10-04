//! Integration tests for the `broadcasting` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod channel;
pub mod e2e;
pub mod event_integration;
pub mod fanout;
pub mod hub;
pub mod middleware;
pub mod presence;
pub mod protocol;
pub mod pusher;
#[cfg(feature = "broadcasting-fanout")]
#[path = "../support/redis_server.rs"]
mod redis_server;
