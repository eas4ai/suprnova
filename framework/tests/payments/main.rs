//! Integration tests for the `payments` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod dto;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
pub mod migration;
pub mod mock_discriminator;
pub mod mock_environment;
pub mod money;
// Its tests move the framework clock with `TestClock`, which exists with
// the `testing` feature only.
#[cfg(feature = "testing")]
pub mod native_engines;
#[path = "../support/own_process.rs"]
mod own_process;
pub mod public_surface;
pub mod registry;
pub mod webhook_hydration;
pub mod webhook_idempotency;
pub mod webhook_remote_addr;
