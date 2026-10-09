//! Integration tests for the `routing` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/http_wire.rs"]
mod http_wire;

pub mod group_names;
pub mod inertia;
mod laravel_delta;
pub mod method_names;
pub mod params;
pub mod root_group_redirect;
pub mod route_binding;
pub mod router_middleware_keying;
pub mod try_register;
pub mod verbs;
