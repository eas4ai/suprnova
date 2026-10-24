//! The clock the framework reads.
//!
//! Every place in the framework that needs "the time now" - signed URL
//! expiry, session idle timeout, due scheduled tasks, rate-limit windows,
//! prunable models, queue availability - calls [`now`] instead of
//! `chrono::Utc::now()`. Production builds get the system clock and nothing
//! else. With the `testing` feature, a test can move the clock through
//! `suprnova::testing::TestClock`, so a test of expiry needs no sleep.
//! `tokio::time::pause` cannot do this: it moves the timers of Tokio, not
//! the wall-clock reads.

use chrono::{DateTime, Utc};

/// The current time.
///
/// Without the `testing` feature this is `Utc::now()`: no branch, no
/// task-local, no atomic. With it, an override installed by
/// `TestClock` on the current task or thread wins, and
/// the system clock answers when there is none.
#[cfg(not(any(test, feature = "testing")))]
#[inline]
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// The current time.
///
/// An override installed by [`TestClock`] answers first: a
/// task-local from `TestClock::scope`, then a thread-local from
/// `TestClock::freeze` or `TestClock::travel_to`. With neither, the system
/// clock answers. Neither layer allocates on read.
#[cfg(any(test, feature = "testing"))]
pub fn now() -> DateTime<Utc> {
    if let Some(at) = overridden::current() {
        return at;
    }
    Utc::now()
}

#[cfg(any(test, feature = "testing"))]
pub(crate) mod overridden {
    //! The two override layers. The thread-local serves a plain test body
    //! (a current-thread `#[tokio::test]` runs on the test's own thread);
    //! the task-local serves futures driven by `TestClock::scope`, which
    //! may hop between worker threads. There is no process-wide layer: a
    //! test that moved the time of the process would move it for every
    //! test running beside it.

    use chrono::{DateTime, Utc};
    use std::cell::Cell;
    use std::sync::{Arc, Mutex, PoisonError};

    thread_local! {
        static THREAD_NOW: Cell<Option<DateTime<Utc>>> = const { Cell::new(None) };
    }

    tokio::task_local! {
        static TASK_NOW: Arc<Mutex<DateTime<Utc>>>;
    }

    /// The override for this task or thread, if any. Task-local first.
    pub(crate) fn current() -> Option<DateTime<Utc>> {
        if let Ok(at) =
            TASK_NOW.try_with(|shared| *shared.lock().unwrap_or_else(PoisonError::into_inner))
        {
            return Some(at);
        }
        THREAD_NOW.try_with(Cell::get).ok().flatten()
    }

    /// Replace the thread's override and return the one it had.
    pub(crate) fn swap_thread(at: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
        THREAD_NOW.try_with(|cell| cell.replace(at)).ok().flatten()
    }

    /// Run `future` with the task-local override set to `shared`.
    pub(crate) async fn scope_task<F: std::future::Future>(
        shared: Arc<Mutex<DateTime<Utc>>>,
        future: F,
    ) -> F::Output {
        TASK_NOW.scope(shared, future).await
    }
}

/// Entry point for moving the clock in a test.
///
/// Time under a `TestClock` stands still until the test moves it with
/// [`TestClockGuard::advance`] or [`TestClockHandle::advance`]: a frozen
/// clock makes "exactly at the expiry" a state a test can hold, which a
/// ticking clock cannot.
///
/// Two layers, asked in this order by [`now`]:
///
/// - [`TestClock::scope`] sets a task-local for one future. It holds across
///   awaits and on a multi-thread runtime. A task started with
///   `tokio::spawn` inside the scope does not inherit it: run such a test
///   on a current-thread runtime with [`TestClock::freeze`] or
///   [`TestClock::travel_to`] (the spawned task then runs on the test's
///   thread and sees the thread-local), or spawn through a helper that
///   re-enters `TestClock::scope` with the same handle.
/// - [`TestClock::freeze`] and [`TestClock::travel_to`] set a thread-local
///   for the test's own thread and hand back a guard.
///
/// Nothing moves the clock of the whole process, so tests that run in
/// parallel do not see each other's time.
///
/// # Example
///
/// ```rust,no_run
/// use chrono::Duration;
/// use suprnova::testing::TestClock;
///
/// # fn my_test() {
/// let clock = TestClock::freeze();
/// let before = suprnova::clock::now();
/// clock.advance(Duration::minutes(30));
/// assert_eq!(suprnova::clock::now() - before, Duration::minutes(30));
/// # }
/// ```
#[cfg(any(test, feature = "testing"))]
pub struct TestClock;

#[cfg(any(test, feature = "testing"))]
impl TestClock {
    /// Stop the clock of this thread at the moment of the call.
    ///
    /// The returned guard gives the thread its previous clock back when it
    /// drops, also on a panic.
    pub fn freeze() -> TestClockGuard {
        Self::travel_to(now())
    }

    /// Set the clock of this thread to `at` and stand still there.
    ///
    /// Guards nest: an inner guard hands back the time of the outer one
    /// when it drops.
    pub fn travel_to(at: DateTime<Utc>) -> TestClockGuard {
        let previous = overridden::swap_thread(Some(at));
        TestClockGuard {
            previous,
            _not_send: std::marker::PhantomData,
        }
    }

    /// Run the future that `build(handle)` returns with the clock at `at`
    /// on this task. The closure itself runs before the scope opens, so a
    /// read in its body, outside the future, sees the clock of the thread.
    ///
    /// Inside the scope the scope wins: a [`TestClockGuard`] of the thread
    /// does not change what [`now`] returns there. Move the time with the
    /// handle.
    ///
    /// The closure receives a [`TestClockHandle`] to move the time from
    /// inside the future; the override is a task-local, so it survives
    /// awaits and worker-thread hops on a multi-thread runtime. A handle
    /// rather than a guard, because the clock belongs to the future and
    /// ends with it.
    pub async fn scope<F, Fut>(at: DateTime<Utc>, build: F) -> Fut::Output
    where
        F: FnOnce(TestClockHandle) -> Fut,
        Fut: std::future::Future,
    {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(at));
        let handle = TestClockHandle {
            shared: std::sync::Arc::clone(&shared),
        };
        overridden::scope_task(shared, build(handle)).await
    }
}

/// Holds the clock of one thread at a test-chosen time.
///
/// It is `!Send`: the override lives in a thread-local, so a guard dropped
/// on another thread would restore the wrong thread's clock.
#[cfg(any(test, feature = "testing"))]
#[must_use = "dropping the guard at once gives the system clock back"]
pub struct TestClockGuard {
    previous: Option<DateTime<Utc>>,
    _not_send: std::marker::PhantomData<*const ()>,
}

#[cfg(any(test, feature = "testing"))]
impl TestClockGuard {
    /// Move the clock of this thread forward (or back, for a negative
    /// `by`). A move past the range of `DateTime` leaves it where it was.
    pub fn advance(&self, by: chrono::Duration) {
        let at = self.now();
        self.set(at.checked_add_signed(by).unwrap_or(at));
    }

    /// Set the clock of this thread to `at`.
    pub fn set(&self, at: DateTime<Utc>) {
        overridden::swap_thread(Some(at));
    }

    /// The time the clock of this thread shows.
    pub fn now(&self) -> DateTime<Utc> {
        overridden::current().unwrap_or_else(Utc::now)
    }
}

#[cfg(any(test, feature = "testing"))]
impl Drop for TestClockGuard {
    fn drop(&mut self) {
        overridden::swap_thread(self.previous);
    }
}

/// Moves the clock inside a [`TestClock::scope`].
///
/// Cloneable and `Send`, so it can be moved into spawned tasks that
/// re-enter the scope.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone)]
pub struct TestClockHandle {
    shared: std::sync::Arc<std::sync::Mutex<DateTime<Utc>>>,
}

#[cfg(any(test, feature = "testing"))]
impl TestClockHandle {
    /// Move the scoped clock forward (or back, for a negative `by`). A move
    /// past the range of `DateTime` leaves it where it was.
    pub fn advance(&self, by: chrono::Duration) {
        let mut at = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *at = at.checked_add_signed(by).unwrap_or(*at);
    }

    /// Set the scoped clock to `at`.
    pub fn set(&self, at: DateTime<Utc>) {
        *self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = at;
    }

    /// The time the scoped clock shows.
    pub fn now(&self) -> DateTime<Utc> {
        *self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Run `future` with this handle's clock, for a task spawned inside a
    /// scope that would not inherit the task-local.
    pub async fn run<F: std::future::Future>(&self, future: F) -> F::Output {
        overridden::scope_task(std::sync::Arc::clone(&self.shared), future).await
    }
}
