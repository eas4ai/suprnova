//! Integration tests for the `live` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/live_async_support/mod.rs"]
mod live_async_support;
#[path = "../support/live_dogfood_support/mod.rs"]
mod live_dogfood_support;

pub mod assets;
pub mod async_backpressure;
pub mod async_routes;
pub mod async_security;
pub mod boot;
pub mod boot_async_route_order;
pub mod dependency_topology;
pub mod document_routes;
pub mod dogfood;
pub mod dogfood_server;
pub mod external_authoring;
pub mod facade_contract;
pub mod hostile_adapter;
pub mod macro_expansion;
pub mod multi_stream_root;
pub mod public_api;
pub mod public_seed_actions;
pub mod routes;
pub mod tooling_protocol;
pub mod trusted_context;
pub mod upload_policy;
pub mod upload_providers;
pub mod upload_routes;
pub mod upload_security;
pub mod view_contract;
