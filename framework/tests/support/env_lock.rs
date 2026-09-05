//! One process-wide lock for tests that mutate process environment.
//!
//! Under nextest every test is its own process and this lock is redundant.
//! Under plain `cargo test` a module binary runs its former files on shared
//! threads, so two tests that set the same variable must not overlap.
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Hold the returned guard for the whole test body that mutates environment.
pub fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
