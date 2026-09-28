//! Integration tests for the `data` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod allowlist_module_isolation;
pub mod field;
pub mod form_request;
pub mod form_request_async_validation;
pub mod form_request_custom_hooks;
pub mod generic;
pub mod include_set;
pub mod integration;
pub mod lazy_flavors;
pub mod lazy_resolution;
pub mod max_body_bytes_override;
pub mod middleware;
pub mod partial_data_composition;
pub mod registry;
pub mod route_params;
pub mod route_params_lifecycle;
