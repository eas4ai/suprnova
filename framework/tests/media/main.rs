//! Integration tests for the `media` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod decode_budget;
#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod image_magick_driver;
pub mod image_processing;
pub mod images;
pub mod interop;
pub mod laravel_infra_gaps;
pub mod magick_coder;
// The stand-in `magick` binaries that use it are unix shell scripts.
#[cfg(all(feature = "media", unix))]
#[path = "../support/magick_policy.rs"]
mod magick_policy;
#[path = "../support/own_process.rs"]
mod own_process;
