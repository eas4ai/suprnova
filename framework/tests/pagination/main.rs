//! Integration tests for the `pagination` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod laravel_shape;
pub mod named_connection;
pub mod pagination;

// `Context::test_set_query` compiles with the `testing` feature alone.
#[cfg(feature = "testing")]
pub mod laravel_gaps;
