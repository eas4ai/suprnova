//! Environment mutation helpers for tests that hold the shared env lock.
//!
//! Every test that calls [`set_env`] or drops an [`EnvSnapshot`] first takes
//! `crate::env_lock::lock_env` (or `lock_env_async`), so the process
//! environment is never written from two test threads at once.

/// Restores the captured environment variables when dropped, so a failing
/// assertion cannot leak a changed variable into the next test.
pub struct EnvSnapshot {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvSnapshot {
    /// Records the current value of each key, or its absence.
    pub fn capture(keys: &[&'static str]) -> Self {
        Self {
            keys: keys.iter().map(|k| (*k, std::env::var(k).ok())).collect(),
        }
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        for (k, v) in &self.keys {
            set_env(k, v.as_deref());
        }
    }
}

/// Sets `key` to `value`, or removes it when `value` is `None`.
///
/// The caller must hold the shared env lock for as long as the variable is
/// in use.
pub fn set_env(key: &str, value: Option<&str>) {
    // SAFETY: the caller holds the shared env lock, which serialises every
    // environment write within this test binary. Each integration test
    // module is its own binary, so no other binary shares this process.
    unsafe {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}
