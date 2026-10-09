//! Integration tests for the `cache` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod bootstrap_driver;
pub mod forever_default_ttl;
pub mod laravel_facade;
pub mod locks;
pub mod redis_integration;
pub mod redis_retry;
pub mod tags;
pub mod touch;
