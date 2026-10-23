//! The Laravel HTTP API gaps (docs/spec/laravel-parity.md, "Laravel API
//! gaps: HTTP") that the http module owns, one module per topic. The
//! `par-laravel-gaps-http` mechanism runs every test under this module.

pub mod requests;
// `Request::for_test` compiles with the `testing` feature alone.
#[cfg(feature = "testing")]
pub mod urls;
