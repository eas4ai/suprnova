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
//! Arming reserves a dispatch its place in the burst from an atomic counter,
//! one past every place handed out before it; the owner token is written only
//! after the driver has accepted the envelope. A dispatch that fails, or is
//! cancelled, between the two never names the owner, so it cannot make an
//! earlier, successfully queued envelope look superseded and get it dropped.
//! The token carries the place, and the worker drops an envelope only for a
//! token from a *later* place: an envelope that runs before its own claim
//! lands still runs. Because places are reserved, not read off the last
//! claim, two dispatches that overlap never share a place, and the one that
//! armed later always outranks the other, whichever claims last. Every race
//! between dispatches then costs at worst a duplicate run, never a lost one,
//! and a failed dispatch has nothing to hand back.
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

/// The companion key counting the places handed out for `key`.
///
/// A prefix, where the stamp above takes a suffix: a debounce id is free
/// text, so `{key}:place` could be another id's owner key, and an owner token
/// written over the counter would fail every later increment for that id.
pub(crate) fn place_key(key: &str) -> String {
    format!("queue-debounce-place:{key}")
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
/// The place is reserved here, so jobs armed one after another in one
/// [`Queue::bulk`](crate::queue::Queue::bulk) call are placed in order
/// although none of them has claimed the window yet.
///
/// `forced` says that max wait already fired for this key earlier in the
/// same call. `Queue::bulk` claims each window with its last job, so the jobs
/// after the one max wait fired on belong to that forced run: they go out at
/// once too, and leave alone the stamp it cleared. Otherwise the next job
/// stamped a new burst, took the ordinary delay and claimed the window, the
/// forced run was dropped as superseded, and a stream of bulks could defer
/// the work forever.
pub(crate) async fn acquire(
    key: &str,
    window: Duration,
    max_wait: Option<Duration>,
    forced: bool,
) -> Result<Debounced, FrameworkError> {
    let ttl = lock_ttl(window);
    let place = reserve_place(key, ttl).await?;
    let max_wait_exceeded = forced || max_wait_exceeded(key, ttl, max_wait).await?;
    Ok(Debounced {
        owner: format!("{place}:{}", uuid::Uuid::new_v4()),
        max_wait_exceeded,
    })
}

/// Reserve this dispatch's place in the burst: one past every place handed
/// out for `key` before it, whether or not those dispatches have claimed the
/// window yet.
///
/// The place comes from an atomic increment, so no two dispatches share one
/// and the dispatch that arms later holds the later place. Reading it off
/// the current claim instead let overlapping dispatches read the same claim:
/// two pushes tied and their random ids picked the survivor, and an older
/// bulk that claimed last outranked a newer push that had claimed already.
///
/// The counter can restart below a claim that is still live, when it expired
/// first or the claim predates counters. The reservation then moves the
/// counter past that claim with a second increment, so every place is still
/// the result of an atomic increment, and still unique.
async fn reserve_place(key: &str, ttl: Duration) -> Result<u64, FrameworkError> {
    let counter = place_key(key);
    let mut reserved = Cache::increment(&counter, 1).await?;
    let claimed = current_owner(key).await?.as_deref().and_then(place);
    if let Some(claimed) = claimed.and_then(|claimed| i64::try_from(claimed).ok())
        && reserved <= claimed
    {
        let behind = claimed
            .checked_sub(reserved)
            .and_then(|gap| gap.checked_add(1))
            .ok_or_else(|| FrameworkError::internal("debounce place counter overflow"))?;
        reserved = Cache::increment(&counter, behind).await?;
    }
    Cache::touch(&counter, ttl).await?;
    u64::try_from(reserved)
        .map_err(|_| FrameworkError::internal("debounce place counter went below zero"))
}

/// Claim the window for `owner`, whose envelope the driver has accepted,
/// unless a dispatch from a later place has claimed it already.
///
/// The check and the write are two steps, so a later claim that lands
/// between them is overwritten. That costs a duplicate run, never a lost
/// one: the later dispatch's envelope is not superseded by an earlier place,
/// so it still runs. The check only spares the common case, a dispatch that
/// stalled before its claim and finds a newer one already there.
pub(crate) async fn claim(key: &str, owner: &str, window: Duration) -> Result<(), FrameworkError> {
    if let Some(current) = current_owner(key).await?
        && supersedes(&current, owner)
    {
        return Ok(());
    }
    Cache::put(key, &owner, Some(lock_ttl(window))).await
}

/// Whether the window's token `current` supersedes an envelope stamped with
/// `envelope`: true only for a token from a later place in the burst. The id
/// breaks a tie, which only tokens from before places were reserved can
/// produce.
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
