//! Integration tests for the `inertia` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod error_page;
pub mod error_page_placement;
pub mod flash_commit_boundary;
pub mod inertia;
pub mod merge_paths;
pub mod middleware;
pub mod production_fail_closed;
pub mod prop_composition;
pub mod shared_ergonomics;
pub mod try_serialize;
pub mod validation_redirect;
