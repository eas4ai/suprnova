//! Integration tests for the `session` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod config_knobs;
pub mod cookie_name_bound_aad;
pub mod cookie_prefix_roundtrip;
pub mod cookie_queue;
pub mod destroy_for_user;
pub mod facade;
pub mod gc_loop;
pub mod id_shape_validation;
pub mod lazy_persistence;
pub mod persistence_fail_closed;
pub mod previous_url_open_redirect;
pub mod store_container_binding;
