//! Integration tests for the `container` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod boot_idempotent;
pub mod dep_resolution;
pub mod injectable_override;
pub mod laravel_named_aliases;
pub mod scoped_bindings;
pub mod test_scope_async_safe;
pub mod test_spawn_inheritance;
