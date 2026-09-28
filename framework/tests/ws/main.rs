//! Integration tests for the `ws` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod e2e;
pub mod global_middleware;
pub mod heartbeat;
pub mod origin_policy;
pub mod per_route_config;
pub mod router;
pub mod router_macro;
pub mod unit;
