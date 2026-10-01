//! Integration tests for the `filesystem` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/common.rs"]
mod common;

pub mod atomic_writes;
pub mod copy_atomicity;
pub mod disk_ext;
pub mod disk_responses;
pub mod filesystem;
pub mod path_traversal;
pub mod public_url;
pub mod read_through;
pub mod read_through_options;
