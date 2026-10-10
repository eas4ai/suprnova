//! Integration tests for the `container` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

// `own_process` takes the environment lock before it starts a child.
#[path = "../support/env_lock.rs"]
mod env_lock;
// The `APP_ENV` tests of `#[service]` run alone in a child process. Public
// so the helpers this binary does not call are not dead code.
#[path = "../support/own_process.rs"]
pub mod own_process;

pub mod boot_idempotent;
pub mod dep_resolution;
pub mod injectable_override;
/// Tests observe the agreed infrastructure gaps.
pub mod laravel_infra_gaps;
pub mod laravel_named_aliases;
pub mod scoped_bindings;
pub mod test_scope_async_safe;
pub mod test_spawn_inheritance;
