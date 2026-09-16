//! Session blocking: per-session request serialization (SESS-001).
//!
//! Two requests that carry one session cookie load the same row, and
//! without coordination the one that writes last wins, so a flash the
//! first request set can vanish before any later request reads it. With
//! blocking enabled, the session middleware acquires a lock for the
//! session id through the cache lock driver before it loads the session,
//! holds it through the handler and the write, and releases it after, so
//! the second request loads what the first one persisted.
//!
//! Blocking is off by default and enabled either globally through
//! [`crate::session::SessionConfig::block`] or per route through
//! `block_session` on the route and group builders. A route-level block
//! takes precedence over the global one, so one route can hold the lock
//! longer than the rest of the application. Mirrors Laravel's
//! `Route::block($lockSeconds, $waitSeconds)`.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use hyper::Method;

/// How long a request holds the session lock and how long it waits to get
/// it.
///
/// Both bounds are deliberate: the hold is the lock's TTL, so a handler
/// that outruns it does not wedge the session for every later request,
/// and the wait is how long a request queues before it answers `503`
/// instead of holding a connection open indefinitely. The defaults are
/// ten seconds each, Laravel's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionBlock {
    lock: Duration,
    wait: Duration,
}

impl Default for SessionBlock {
    fn default() -> Self {
        Self {
            lock: Duration::from_secs(10),
            wait: Duration::from_secs(10),
        }
    }
}

impl SessionBlock {
    /// A block that holds the session lock for `lock` and waits up to
    /// `wait` to acquire it. A zero hold would release the lock before the
    /// session loads, so the hold is at least one second.
    pub fn new(lock: Duration, wait: Duration) -> Self {
        Self {
            lock: lock.max(Duration::from_secs(1)),
            wait,
        }
    }

    /// The lock's TTL: the longest one request keeps the session to itself.
    pub fn lock_for(&self) -> Duration {
        self.lock
    }

    /// The longest a request queues behind another before it answers `503`.
    pub fn wait_for(&self) -> Duration {
        self.wait
    }
}

/// Route-level blocks, keyed by `(method, pattern)` exactly like the
/// router's middleware map, so a block on `POST /orders` never serializes
/// the sibling `GET /orders`. Process-global for the same reason route
/// names are: the session middleware runs before the router hands the
/// request to route-level middleware, so it has to find the route's block
/// by the pattern the server stamped on the request.
static ROUTE_BLOCKS: OnceLock<RwLock<HashMap<(Method, String), SessionBlock>>> = OnceLock::new();

fn route_blocks() -> &'static RwLock<HashMap<(Method, String), SessionBlock>> {
    ROUTE_BLOCKS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Record that requests matched to `pattern` under `method` are
/// serialized with `block`. Called by the route and group builders at
/// registration time. Re-registering replaces the earlier block.
pub(crate) fn register_route_block(method: &Method, pattern: &str, block: SessionBlock) {
    let mut blocks = match route_blocks().write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    blocks.insert((method.clone(), pattern.to_owned()), block);
}

/// The block registered for `pattern` under `method`, if any. A `HEAD`
/// request that matched a `GET` route reads the `GET` block, matching the
/// server's middleware lookup for the same fallback.
pub(crate) fn route_block(method: &Method, pattern: &str) -> Option<SessionBlock> {
    let blocks = match route_blocks().read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    blocks
        .get(&(method.clone(), pattern.to_owned()))
        .copied()
        .or_else(|| {
            (*method == Method::HEAD)
                .then(|| blocks.get(&(Method::GET, pattern.to_owned())).copied())
                .flatten()
        })
}

/// The cache key that serializes requests on `session_id`.
pub(crate) fn lock_key(session_id: &str) -> String {
    format!("session:block:{session_id}")
}

/// How long a waiting request sleeps between attempts to take the lock.
const RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// Take the session lock for `session_id`, waiting up to the block's wait
/// bound. A request that cannot get the lock in time answers `503` with a
/// `Retry-After`, a decisive outcome instead of an open connection; a
/// cache the framework cannot reach answers `500`, because blocking that
/// silently ran without its lock would be the race with a false promise.
pub(crate) async fn acquire(
    session_id: &str,
    block: SessionBlock,
) -> Result<HeldSessionLock, crate::http::HttpResponse> {
    let key = lock_key(session_id);
    let deadline = tokio::time::Instant::now() + block.wait_for();
    loop {
        match crate::cache::Cache::lock(&key, block.lock_for()).await {
            Ok(Some(guard)) => return Ok(HeldSessionLock(Some(guard))),
            Ok(None) => {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    tracing::debug!(
                        wait_ms = block.wait_for().as_millis() as u64,
                        "session lock not acquired within the wait bound; answering 503"
                    );
                    return Err(crate::http::HttpResponse::text(
                        "Service Unavailable: another request on this session is still running",
                    )
                    .status(503)
                    .header("Retry-After", "1"));
                }
                tokio::time::sleep(RETRY_INTERVAL.min(deadline - now)).await;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "session blocking is enabled but the cache lock driver failed; failing closed"
                );
                return Err(crate::http::HttpResponse::text(
                    "Internal Server Error: session blocking needs a reachable cache store",
                )
                .status(500));
            }
        }
    }
}

/// A session lock one request holds.
///
/// The server drops a request's future when the client goes away, a
/// navigation that cancels an in-flight fetch or stream for example, and a
/// future dropped at an await point never reaches the release after the
/// write. Without a release on drop the session stayed locked for the whole
/// hold bound, and every other request on the session queued behind a lock
/// nobody held until its own wait bound answered 503. Dropping a lock that
/// was not released hands the release to the runtime instead.
pub(crate) struct HeldSessionLock(Option<crate::cache::LockGuard>);

impl HeldSessionLock {
    /// Release the session lock after the write.
    pub(crate) async fn release(mut self) {
        if let Some(guard) = self.0.take() {
            release_guard(guard).await;
        }
    }
}

impl Drop for HeldSessionLock {
    fn drop(&mut self) {
        let Some(guard) = self.0.take() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(release_guard(guard));
            }
            // With no runtime left to release on, the hold bound is what
            // frees the session, as it is for a crashed process.
            Err(_) => tracing::warn!(
                "session lock dropped outside a runtime; it frees when its hold bound expires"
            ),
        }
    }
}

/// A guard whose TTL already ran out cannot be released, which means the
/// handler outran the hold bound and later requests were no longer
/// serialized behind it; that is logged, not raised, because the response
/// itself is complete.
async fn release_guard(guard: crate::cache::LockGuard) {
    match guard.release().await {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            "session lock expired before the request finished; raise the block's hold bound"
        ),
        Err(error) => tracing::warn!(%error, "session lock release failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_block_is_found_by_its_method_and_pattern() {
        let block = SessionBlock::new(Duration::from_secs(3), Duration::from_secs(2));
        register_route_block(&Method::POST, "/blocking/unit/{id}", block);

        assert_eq!(
            route_block(&Method::POST, "/blocking/unit/{id}"),
            Some(block)
        );
        assert_eq!(route_block(&Method::GET, "/blocking/unit/{id}"), None);
        assert_eq!(route_block(&Method::POST, "/blocking/other"), None);
    }

    #[test]
    fn a_head_request_reads_the_get_block() {
        let block = SessionBlock::default();
        register_route_block(&Method::GET, "/blocking/unit/head", block);

        assert_eq!(
            route_block(&Method::HEAD, "/blocking/unit/head"),
            Some(block)
        );
    }

    #[test]
    fn the_hold_is_at_least_one_second() {
        let block = SessionBlock::new(Duration::ZERO, Duration::ZERO);
        assert_eq!(block.lock_for(), Duration::from_secs(1));
        assert_eq!(block.wait_for(), Duration::ZERO);
    }
}
