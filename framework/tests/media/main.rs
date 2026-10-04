//! Integration tests for the `media` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod decode_budget;
#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod image_magick_driver;
pub mod image_processing;
pub mod interop;
#[path = "../support/own_process.rs"]
mod own_process;
