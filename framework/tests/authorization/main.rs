//! Integration tests for the `authorization` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod authorization;
pub mod handler_authorize;
/// The Laravel authorization gaps, observed by the `par-laravel-gaps-auth` mechanism.
pub mod laravel_auth_gaps;
