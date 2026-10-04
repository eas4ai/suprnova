//! Integration tests for the `broadcasting` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod channel;
pub mod e2e;
#[cfg(feature = "broadcasting-fanout")]
#[path = "../support/env_lock.rs"]
mod env_lock;
#[cfg(feature = "broadcasting-fanout")]
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
pub mod event_integration;
pub mod fanout;
pub mod hub;
pub mod middleware;
#[cfg(feature = "broadcasting-fanout")]
#[path = "../support/own_process.rs"]
mod own_process;
pub mod presence;
pub mod protocol;
pub mod pusher;
#[cfg(feature = "broadcasting-fanout")]
#[path = "../support/redis_server.rs"]
mod redis_server;
pub mod teardown;
