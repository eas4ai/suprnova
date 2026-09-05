//! Integration tests for the `container` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod boot_idempotent;
pub mod boot_returns_err_on_missing_dep;
pub mod dep_resolution;
pub mod laravel_named_aliases;
pub mod test_scope_async_safe;
pub mod test_spawn_inheritance;
