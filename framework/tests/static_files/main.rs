//! Integration tests for the `static_files` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[cfg(unix)]
#[path = "../support/tree_racer.rs"]
mod tree_racer;

pub mod static_files;
