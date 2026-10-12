//! The Laravel infrastructure gaps (docs/spec/laravel-parity.md, "Laravel
//! API gaps: infrastructure") that the http module owns, one module per
//! topic. The `par-laravel-gaps-infra` mechanism runs every test under
//! this module.

pub mod cookies;
pub mod precognition;
