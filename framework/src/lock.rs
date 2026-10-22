//! Small helpers to handle poisoned locks consistently across the framework.
//!
//! Policy:
//! - A poisoned lock is never a panic. One bad request must not take down
//!   an entire subsystem.
//! - [`read`], [`write`] and [`lock`] treat a poisoned lock as an internal
//!   error and return a `FrameworkError`. This is the rule.
//! - [`recover`] goes on with the value of a poisoned lock. It is for two
//!   kinds of value, where an error would fail a clean-up that has to run:
//!   a value that is whole after any panic, and a value that code of the
//!   application changes under the guard, when the code that goes on reads
//!   it and never stores it. The session of a request is the second kind.
//!   The documentation of [`recover`] says more.
//!
//! Each helper takes a `context` label naming the subsystem that owns the
//! lock (e.g. `"connection registry"`, `"payments registry"`, `"db event
//! listeners"`). The label is embedded in the resulting `FrameworkError`
//! message so dev-only `debug_message` payloads and operator logs can tell
//! which lock poisoned without forcing every caller to wrap the result with
//! its own context.

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::error::FrameworkError;

/// Acquire a read guard on an `RwLock`, returning a `FrameworkError` on poison.
///
/// `context` identifies the lock's owning subsystem (e.g. `"connection
/// registry"`). It is woven into the error message so the poison source is
/// visible without the caller having to wrap the error.
pub(crate) fn read<'a, T>(
    lock: &'a RwLock<T>,
    context: &'static str,
) -> Result<RwLockReadGuard<'a, T>, FrameworkError> {
    lock.read()
        .map_err(|_| FrameworkError::internal(format!("{context} lock poisoned")))
}

/// Acquire a write guard on an `RwLock`, returning a `FrameworkError` on poison.
///
/// `context` identifies the lock's owning subsystem (e.g. `"connection
/// registry"`). It is woven into the error message so the poison source is
/// visible without the caller having to wrap the error.
pub(crate) fn write<'a, T>(
    lock: &'a RwLock<T>,
    context: &'static str,
) -> Result<RwLockWriteGuard<'a, T>, FrameworkError> {
    lock.write()
        .map_err(|_| FrameworkError::internal(format!("{context} lock poisoned")))
}

/// Acquire a guard on a `Mutex`, returning a `FrameworkError` on poison.
///
/// `context` identifies the lock's owning subsystem. It is woven into the
/// error message so the poison source is visible without the caller having
/// to wrap the error.
pub(crate) fn lock<'a, T>(
    lock: &'a Mutex<T>,
    context: &'static str,
) -> Result<MutexGuard<'a, T>, FrameworkError> {
    lock.lock()
        .map_err(|_| FrameworkError::internal(format!("{context} lock poisoned")))
}

/// Acquire a guard on a `Mutex`, and keep going when the lock is poisoned.
///
/// A lock is poisoned when a thread panicked while it held the guard. That
/// matters where the panic can have left the value between two states that
/// the code after it must not see. It does not matter for a value that is
/// whole at every point where its critical sections can panic: a list that
/// is pushed to and drained, or a value that is replaced as one. Such a
/// value is as good after the panic as it was before it, and an error for
/// every later use of it would make one panic into many.
///
/// A value is of that kind only when every critical section of it is code
/// of this crate: a closure of the application that runs under the guard
/// can panic between any two of its writes.
///
/// A value that such a closure changes is recovered on one condition: the
/// code that goes on with it reads it and must not keep what it finds. The
/// session middleware is the example. It reads a session whose lock is
/// poisoned, so that the code that answers for the panic can run, and it
/// does not store that session.
///
/// Use [`lock`] for a value that is neither.
pub(crate) fn recover<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    /// Poison a fresh `RwLock` by panicking while holding its write guard.
    fn poison_rw<T: Send + Sync + 'static>(rw: &Arc<RwLock<T>>) {
        let clone = Arc::clone(rw);
        let _ = thread::spawn(move || {
            let _g = clone.write().unwrap();
            panic!("intentional poison");
        })
        .join();
        assert!(rw.is_poisoned(), "test setup: RwLock must be poisoned");
    }

    /// Propagate policy: a poisoned lock surfaces as a `FrameworkError`
    /// rather than panicking the whole subsystem. Notifications and mail
    /// registries `?` these helpers.
    #[test]
    fn helpers_return_err_on_poison_instead_of_panicking() {
        let rw = Arc::new(RwLock::new(0u32));
        poison_rw(&rw);
        assert!(
            read(&rw, "test registry").is_err(),
            "lock::read must return Err on poison",
        );
        assert!(
            write(&rw, "test registry").is_err(),
            "lock::write must return Err on poison",
        );

        let mtx = Arc::new(Mutex::new(0u32));
        let mtx_clone = Arc::clone(&mtx);
        let _ = thread::spawn(move || {
            let _g = mtx_clone.lock().unwrap();
            panic!("intentional poison");
        })
        .join();
        assert!(mtx.is_poisoned(), "test setup: Mutex must be poisoned");
        assert!(
            lock(&mtx, "test mutex").is_err(),
            "lock::lock must return Err on poison",
        );
    }

    /// Each helper's `FrameworkError::internal` message includes the
    /// caller-supplied context so logs / dev `debug_message` payloads can
    /// tell `connection registry` poison from `payments registry` poison
    /// without the caller having to wrap the error.
    #[test]
    fn error_message_includes_context_label() {
        let rw = Arc::new(RwLock::new(0u32));
        poison_rw(&rw);

        let err = read(&rw, "connection registry").expect_err("expected poison Err");
        let msg = err.to_string();
        assert!(
            msg.contains("connection registry"),
            "read err must name subsystem, got: {msg}",
        );

        let err = write(&rw, "payments registry").expect_err("expected poison Err");
        let msg = err.to_string();
        assert!(
            msg.contains("payments registry"),
            "write err must name subsystem, got: {msg}",
        );

        let mtx = Arc::new(Mutex::new(0u32));
        let mtx_clone = Arc::clone(&mtx);
        let _ = thread::spawn(move || {
            let _g = mtx_clone.lock().unwrap();
            panic!("intentional poison");
        })
        .join();
        let err = lock(&mtx, "db event listeners").expect_err("expected poison Err");
        let msg = err.to_string();
        assert!(
            msg.contains("db event listeners"),
            "lock err must name subsystem, got: {msg}",
        );
    }

    /// Recover-in-place policy (`data::registry` hot-path reads + init
    /// writes use `raw.write()/read().unwrap_or_else(|e| e.into_inner())`):
    /// the registry keeps working through a poisoned lock rather than
    /// panicking, and the recovered guard is fully usable.
    #[test]
    fn into_inner_recovers_a_poisoned_lock() {
        let rw = Arc::new(RwLock::new(vec![1u32, 2, 3]));
        poison_rw(&rw);
        rw.write().unwrap_or_else(|e| e.into_inner()).push(4);
        let snapshot = rw.read().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(snapshot, vec![1, 2, 3, 4], "recovered guard must be usable");
    }

    /// A panic while the guard is held, which is what poisons the lock.
    fn poisoned(values: Vec<u8>) -> Arc<Mutex<Vec<u8>>> {
        let shared = Arc::new(Mutex::new(values));
        let held = Arc::clone(&shared);
        let ended = thread::spawn(move || {
            let _guard = held.lock().expect("the lock is whole");
            panic!("a panic while the guard is held");
        })
        .join();
        assert!(ended.is_err(), "the thread panicked");
        assert!(shared.is_poisoned());
        shared
    }

    #[test]
    fn recover_gives_the_value_of_a_poisoned_lock() {
        let shared = poisoned(vec![1, 2, 3]);

        assert_eq!(*recover(&shared), [1, 2, 3]);
        recover(&shared).push(4);
        assert_eq!(*recover(&shared), [1, 2, 3, 4]);
    }
}
