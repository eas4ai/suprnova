//! Integration tests for the `payments` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod dto;
pub mod migration;
pub mod mock_discriminator;
pub mod money;
pub mod registry;
pub mod webhook_hydration;
pub mod webhook_idempotency;
pub mod webhook_remote_addr;
