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
//! # The window is claimed only once the envelope is queued
//!
//! Arming reserves a dispatch its place in the burst, one past the dispatch
//! that last claimed the window; the owner token is written only after the
//! driver has accepted the envelope. A dispatch that fails, or is cancelled,
//! between the two never names the owner, so it cannot make an earlier,
//! successfully queued envelope look superseded and get it dropped. The token
//! carries the place, and the worker drops an envelope only for a token from a
//! *later* place: an envelope that runs before its own claim lands still runs.
//! Every race between dispatches then costs at worst a duplicate run, never a
//! lost one, and a failed dispatch has nothing to hand back.
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
    /// Token this dispatch claims the window with once its envelope is on the
    /// queue: `"{place}:{id}"`, its place in the burst, then a unique id.
    ///
    /// Stamped on the envelope and compared at run time: an envelope is
    /// dropped instead of run when the window has since been claimed by a
    /// dispatch from a later place in its burst.
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

/// Arm (or re-arm) the debounce window for `key`, returning the token this
/// dispatch will claim it with.
///
/// Nothing is written for the owner here: the caller claims the window with
/// [`claim`] once its envelope is on the queue - see the module docs. The
/// claim **overwrites** any earlier token, and that is the mechanism, not an
/// oversight. Returns `max_wait_exceeded == true` when the burst has been
/// deferring the run for at least `max_wait`, in which case the caller queues
/// the job with no delay at all.
///
/// `after` is the place an earlier dispatch in the same call reserved for
/// this key. [`Queue::bulk`](crate::queue::Queue::bulk) claims its windows
/// together after one write, so its later jobs have to be placed after its
/// earlier ones rather than after a claim that has not landed yet.
pub(crate) async fn acquire(
    key: &str,
    window: Duration,
    max_wait: Option<Duration>,
    after: Option<u64>,
) -> Result<Debounced, FrameworkError> {
    let ttl = lock_ttl(window);
    let claimed = current_owner(key).await?;
    let place = claimed
        .as_deref()
        .and_then(place)
        .unwrap_or(0)
        .max(after.unwrap_or(0));
    let max_wait_exceeded = max_wait_exceeded(key, ttl, max_wait).await?;
    Ok(Debounced {
        owner: format!("{}:{}", place.saturating_add(1), uuid::Uuid::new_v4()),
        max_wait_exceeded,
    })
}

/// Claim the window for `owner`, whose envelope the driver has accepted.
pub(crate) async fn claim(key: &str, owner: &str, window: Duration) -> Result<(), FrameworkError> {
    Cache::put(key, &owner, Some(lock_ttl(window))).await
}

/// Whether the window's token `current` supersedes an envelope stamped with
/// `envelope`: true only for a token from a later place in the burst, the id
/// breaking a tie between two dispatches that armed from the same claim.
///
/// A token from before places existed carries only an id. An envelope
/// stamped with one is judged as it always was, superseded by any other
/// token. A placed envelope is never superseded by an unplaced token, which
/// can only belong to a dispatch from before it.
pub(crate) fn supersedes(current: &str, envelope: &str) -> bool {
    match (parse(current), parse(envelope)) {
        (Some(current), Some(envelope)) => current > envelope,
        (None, Some(_)) => false,
        (_, None) => current != envelope,
    }
}

/// A token's place and id, or `None` for a token from before places.
fn parse(token: &str) -> Option<(u64, &str)> {
    let (place, id) = token.split_once(':')?;
    Some((place.parse().ok()?, id))
}

/// A token's place in its burst, or `None` for a token from before places.
pub(crate) fn place(token: &str) -> Option<u64> {
    parse(token).map(|(place, _)| place)
}

/// Whether the burst owning `key` has been deferring its run for `max_wait`.
///
/// Stamps the first-dispatch timestamp when there is none (and answers `false`,
/// because a burst that just started has not been waiting). Clears the stamp on
/// the branch that answers `true`, so the forced run starts a fresh window.
/// Ports `DebounceLock::maxWaitExceeded`.
///
/// The stamp lives `max_wait` past the owner token's TTL. It is never
/// refreshed, while the owner token is renewed by every dispatch, so a stamp
/// living only the token's TTL expired before a long `max_wait` came due: the
/// next dispatch re-stamped the burst as just started, and a continuous burst
/// was deferred forever.
async fn max_wait_exceeded(
    key: &str,
    ttl: Duration,
    max_wait: Option<Duration>,
) -> Result<bool, FrameworkError> {
    let Some(max_wait) = max_wait else {
        return Ok(false);
    };
    let stamp_key = first_dispatched_key(key);
    let now = crate::clock::now().timestamp();
    let Some(first) = Cache::get::<i64>(&stamp_key).await? else {
        Cache::put(&stamp_key, &now, Some(ttl.saturating_add(max_wait))).await?;
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
    fn only_a_later_place_supersedes() {
        assert!(supersedes("3:b", "2:a"), "a later place supersedes");
        assert!(!supersedes("2:a", "3:b"), "an earlier claim never does");
        assert!(
            !supersedes("2:a", "2:a"),
            "nor does the envelope's own claim"
        );
        assert!(
            supersedes("2:b", "2:a") != supersedes("2:a", "2:b"),
            "a tie breaks one way"
        );
        assert!(
            !supersedes("legacy", "1:a"),
            "an unplaced token predates every placed one"
        );
        assert!(
            supersedes("1:a", "legacy"),
            "an unplaced envelope keeps the equality rule"
        );
        assert!(!supersedes("legacy", "legacy"));
    }

    #[test]
    fn the_timestamp_key_hangs_off_the_owner_key() {
        assert_eq!(
            first_dispatched_key("queue-debounce:SyncOrder:42"),
            "queue-debounce:SyncOrder:42:first_dispatched_at"
        );
    }
}
