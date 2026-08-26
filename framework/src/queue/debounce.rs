//! Debounce locks: collapse a burst of dispatches into one delayed run.
//!
//! Suprnova's [`Queue::push_unique`](crate::queue::Queue::push_unique)
//! suppresses a duplicate and keeps the **first** dispatch. Debouncing keeps
//! the **last**: every dispatch overwrites the owner token and re-arms the
//! delay, so twenty events in ten seconds become one run, one window after the
//! twentieth. `max_wait` bounds that, so a continuous burst cannot defer the
//! work forever.
//!
//! Ports `Illuminate\Bus\DebounceLock`.
//!
//! # Why this is not a `Cache::lock`
//!
//! [`Cache::lock`](crate::cache::Cache::lock) is mutual exclusion: on Redis it
//! is `SET NX` in a separate lock keyspace, so a second acquire fails. That is
//! the opposite of what a debounce needs. Here the newest dispatch **must**
//! overwrite the previous owner - last-writer-wins is the entire mechanism by
//! which an older, still-queued envelope learns it has been superseded. So the
//! token lives in the ordinary cache keyspace behind
//! [`Cache::put`](crate::cache::Cache::put), and nothing here is a lock in the
//! mutual-exclusion sense.

use crate::cache::Cache;
use crate::error::FrameworkError;
use std::time::Duration;

/// The result of arming a debounce window.
#[derive(Debug, Clone)]
pub struct Debounced {
    /// Token identifying this dispatch as the current owner of the window.
    ///
    /// Stamped on the envelope and compared at run time: an envelope whose
    /// token is no longer the stored one was superseded by a newer dispatch and
    /// is dropped instead of run.
    pub owner: String,
    /// Whether this dispatch hit the configured maximum wait.
    ///
    /// `true` means the burst has been deferring the work for at least
    /// `max_wait`, so this dispatch is queued with no delay at all rather than
    /// waiting out another window.
    pub max_wait_exceeded: bool,
}

/// Per-dispatch debounce settings, for
/// [`Queue::push_debounced`](crate::queue::Queue::push_debounced) and
/// [`DebouncedListener`](crate::events::DebouncedListener).
///
/// The declarative form is [`Job::debounce_for`](crate::queue::Job::debounce_for)
/// and friends; reach for this when the window is a property of the *call site*
/// rather than of the job - which is what Laravel's `#[DebounceFor]` attribute
/// on a listener expresses.
#[derive(Debug, Clone)]
pub struct DebounceOptions {
    /// How long to wait after the most recent dispatch before running.
    pub window: Duration,
    /// Longest the burst may defer the run. `None` means no bound.
    pub max_wait: Option<Duration>,
    /// Debounce id, scoping the window to one entity. `None` debounces every
    /// dispatch of the job together.
    pub id: Option<String>,
}

impl DebounceOptions {
    /// Debounce with `window` and no maximum wait, keyed on the job alone.
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            max_wait: None,
            id: None,
        }
    }

    /// Bound how long a continuous burst may defer the run.
    pub fn max_wait(mut self, max_wait: Duration) -> Self {
        self.max_wait = Some(max_wait);
        self
    }

    /// Scope the window to one entity, so bursts for different ids debounce
    /// independently.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// How long the owner token and its timestamp key live.
///
/// `max(window * 10, 300s)`, matching `DebounceLock::acquire`. Deliberately
/// generous: the token must outlive the delayed envelope that carries it, and
/// an expired token only fails open - the worker runs the job - where a token
/// that expired too early would silently make a supersession invisible.
/// Saturating arithmetic, so an absurd window cannot overflow into a short TTL.
pub(crate) fn lock_ttl(window: Duration) -> Duration {
    let scaled = window.as_secs().saturating_mul(10);
    Duration::from_secs(scaled.max(300))
}

/// The companion key holding the unix timestamp of the burst's first dispatch.
pub(crate) fn first_dispatched_key(key: &str) -> String {
    format!("{key}:first_dispatched_at")
}

/// Arm (or re-arm) the debounce window for `key`, returning the new owner token.
///
/// The write **overwrites** any existing token: that is the mechanism, not an
/// oversight - see the module docs. Returns `max_wait_exceeded == true` when
/// the burst has been deferring the run for at least `max_wait`, in which case
/// the caller queues the job with no delay at all.
pub(crate) async fn acquire(
    key: &str,
    window: Duration,
    max_wait: Option<Duration>,
) -> Result<Debounced, FrameworkError> {
    let ttl = lock_ttl(window);
    let owner = uuid::Uuid::new_v4().to_string();
    Cache::put(key, &owner, Some(ttl)).await?;
    let max_wait_exceeded = max_wait_exceeded(key, ttl, max_wait).await?;
    Ok(Debounced {
        owner,
        max_wait_exceeded,
    })
}

/// Whether the burst owning `key` has been deferring its run for `max_wait`.
///
/// Stamps the first-dispatch timestamp when there is none (and answers `false`,
/// because a burst that just started has not been waiting). Clears the stamp on
/// the branch that answers `true`, so the forced run starts a fresh window.
/// Ports `DebounceLock::maxWaitExceeded`.
async fn max_wait_exceeded(
    key: &str,
    ttl: Duration,
    max_wait: Option<Duration>,
) -> Result<bool, FrameworkError> {
    let Some(max_wait) = max_wait else {
        return Ok(false);
    };
    let stamp_key = first_dispatched_key(key);
    let now = chrono::Utc::now().timestamp();
    let Some(first) = Cache::get::<i64>(&stamp_key).await? else {
        Cache::put(&stamp_key, &now, Some(ttl)).await?;
        return Ok(false);
    };
    if now.saturating_sub(first) >= max_wait.as_secs() as i64 {
        Cache::forget(&stamp_key).await?;
        return Ok(true);
    }
    Ok(false)
}

/// The token currently owning `key`, or `None` when the window has lapsed.
pub(crate) async fn current_owner(key: &str) -> Result<Option<String>, FrameworkError> {
    Cache::get::<String>(key).await
}

/// Drop the debounce window for `key` outright, owner token included.
///
/// Called when a push armed the window and then failed to enqueue its
/// envelope. Leaving the token behind would name an owner that does not exist,
/// and the worker would read every earlier envelope in the burst as superseded
/// and drop it - losing work whose own push reported success. Forgetting the
/// token instead makes the window lapse, and a lapsed window
/// [fails open](current_owner): whatever is still queued runs.
pub(crate) async fn abandon(key: &str) -> Result<(), FrameworkError> {
    Cache::forget(key).await?;
    Cache::forget(&first_dispatched_key(key)).await?;
    Ok(())
}

/// Start a fresh max-wait window for `key`, leaving the owner token alone.
///
/// Called at the start of every actual run (Laravel #61281). Before that fix,
/// the timestamp key was cleared only on the branch where max wait had actually
/// fired, so a job that reached the worker by the ordinary debounce path left
/// the original stamp in place - and the *next* burst measured its max-wait
/// window from a first dispatch that belonged to the previous burst, which
/// could make its very first dispatch look overdue and fire immediately.
pub(crate) async fn release_max_wait(key: &str) -> Result<(), FrameworkError> {
    Cache::forget(&first_dispatched_key(key)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ttl_is_generous_relative_to_the_window() {
        // Laravel: max(debounceFor * 10, 300). The token has to outlive the
        // delayed envelope it belongs to, and an over-long TTL only fails open.
        assert_eq!(lock_ttl(Duration::from_secs(5)), Duration::from_secs(300));
        assert_eq!(lock_ttl(Duration::from_secs(30)), Duration::from_secs(300));
        assert_eq!(lock_ttl(Duration::from_secs(60)), Duration::from_secs(600));
        assert_eq!(
            lock_ttl(Duration::from_secs(u64::MAX / 4)),
            Duration::from_secs(u64::MAX),
            "a preposterous window saturates instead of overflowing"
        );
    }

    #[test]
    fn the_timestamp_key_hangs_off_the_owner_key() {
        assert_eq!(
            first_dispatched_key("queue-debounce:SyncOrder:42"),
            "queue-debounce:SyncOrder:42:first_dispatched_at"
        );
    }
}
