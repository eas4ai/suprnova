//! Integration tests for the `routing` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/http_wire.rs"]
mod http_wire;

pub mod group_names;
pub mod inertia;
pub mod params;
pub mod root_group_redirect;
pub mod route_binding_route_param_scoped;
pub mod route_param_destructured;
pub mod router_middleware_keying;
pub mod try_register;
pub mod verbs;
