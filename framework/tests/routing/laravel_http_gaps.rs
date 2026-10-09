//! The Laravel HTTP API gaps (docs/spec/laravel-parity.md, "Laravel API
//! gaps: HTTP") that the routing module owns, one module per topic. The
//! `par-laravel-gaps-http` mechanism runs every test under this module.
//! The Laravel HTTP API gaps the routing suite owns (PAR-111 to PAR-120),
//! one module per topic, which the `par-laravel-gaps-http` mechanism runs.

pub mod controller_middleware;
pub mod matching;
pub mod signatures;
