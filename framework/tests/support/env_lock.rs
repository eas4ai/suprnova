//! One process-wide lock for tests that mutate process environment.
//!
//! Under nextest every test is its own process and this lock is redundant.
//! Under plain `cargo test` a module binary runs its former files on shared
//! threads, so two tests that set the same variable must not overlap. The
//! lock is a tokio mutex so an async test can hold it across awaits; sync
//! tests take it through the blocking entry point.

use tokio::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

/// Sync tests: hold the returned guard for the whole test body.
///
/// Must not be called from inside an async runtime; async tests use
/// [`lock_env_async`].
#[allow(
    dead_code,
    reason = "this file is shared via #[path] across the test binaries that declare it; a binary whose tests are all async never calls the sync entry point"
)]
pub fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.blocking_lock()
}

/// Async tests: `let _env = crate::env_lock::lock_env_async().await;` as the
/// first statement, so the guard covers every await in the body.
#[allow(
    dead_code,
    reason = "this file is shared via #[path] across the test binaries that declare it; a binary whose tests are all sync never calls the async entry point"
)]
pub async fn lock_env_async() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}
