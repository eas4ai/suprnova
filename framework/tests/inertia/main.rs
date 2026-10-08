//! Integration tests for the `inertia` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
pub mod error_page;
pub mod error_page_placement;
pub mod exceptions;
pub mod facade_runtime;
pub mod flash_commit_boundary;
pub mod flash_session;
#[path = "../support/http_wire.rs"]
mod http_wire;
pub mod inertia;
pub mod location;
pub mod merge_paths;
pub mod middleware;
pub mod middleware_hooks;
#[path = "../support/own_process.rs"]
pub mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;
pub mod production_fail_closed;
pub mod prop_composition;
pub mod props_types;
pub mod protocol_core;
pub mod protocol_harness;
pub mod redirect_back;
pub mod root_template;
pub mod shared_ergonomics;
pub mod ssr_controls;
pub mod try_serialize;
pub mod validation_redirect;
