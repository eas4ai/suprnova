//! Integration tests for the `logging` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod channels;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
#[path = "../support/http_wire.rs"]
mod http_wire;
pub mod logging;
pub mod request_id_e2e;
