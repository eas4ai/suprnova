//! Shared retry policy for read-shaped Redis commands.
//!
//! `redis::aio::ConnectionManager` reconnects in the background but returns
//! the error for the command that hit the dead socket, so a single transient
//! drop surfaces to the caller as a failed `Cache::get`, a failed queue
//! introspection read, or a missing `Retry-After` header. Laravel closed the
//! same hole in `PhpRedisConnection::command()` by retrying an allowlist of
//! read-only commands once after rebuilding the client.
//!
//! Suprnova has no generic `command(name, args)` dispatch, so the allowlist
//! is a per-call-site decision instead of a table: a driver opts one command
//! into the retry by wrapping it in [`retry_read`]. Nothing else retries, at
//! any configured budget - see the module's `is_transient` docs for why a
//! "retry everything" switch is deliberately not offered.

use std::time::Duration;

/// How long to wait before a retry.
///
/// This is a courtesy delay, not a correctness requirement. `ConnectionManager`
/// installs the replacement connection future synchronously before it returns
/// the failing command's error (`ConnectionManager::reconnect` builds the new
/// `SharedRedisFuture` and compare-and-swaps it in), and every
/// `send_packed_command` reloads that slot, so the next attempt already awaits
/// the replacement rather than the dead socket. What the pause buys is the
/// server's side of the problem: a Redis that just dropped us is usually
/// restarting or overloaded, and hammering it the microsecond its socket closed
/// helps neither party. Fifty milliseconds is small next to the reconnect the
/// next attempt is about to await anyway - see [`attempts_from_raw`] for what
/// that reconnect actually costs.
pub(crate) const RETRY_BACKOFF: Duration = Duration::from_millis(50);

/// Total attempts a read-shaped command gets, including the first.
///
/// Two by default: the try plus one retry, matching Laravel's
/// `max(isRetryable(...) ? 1 : 0, command_retries)` for an allowlisted read.
/// `REDIS_COMMAND_RETRIES` adds further retries on top.
pub(crate) fn read_attempts() -> u32 {
    attempts_from_raw(std::env::var("REDIS_COMMAND_RETRIES").ok().as_deref())
}

/// Parse `REDIS_COMMAND_RETRIES` into a total attempt count.
///
/// Split out from [`read_attempts`] so the policy is testable without mutating
/// process-global environment state. An unparseable value degrades to the
/// default instead of failing boot: this knob tunes resilience, and refusing
/// to start over a typo in it would be a worse outage than ignoring it.
///
/// # The clamp bounds attempts, not seconds
///
/// Budget this in seconds per attempt, not in the 50 ms of [`RETRY_BACKOFF`].
/// When a connection has dropped, the next attempt awaits the replacement
/// connection future, so it pays the driver's whole connect budget before it
/// can even send the command, and then its response timeout:
///
/// - The cache driver configures up to 3 connect retries, at most 500 ms
///   apart, each capped by a 2 s connect timeout, with a 5 s response timeout.
/// - The queue and rate-limiter drivers use `ConnectionManager::new`, i.e. the
///   redis-rs defaults: up to 6 connect retries with an *uncapped* exponential
///   delay from 100 ms, each capped by a 1 s connect timeout, with a 500 ms
///   response timeout.
///
/// So one retry against a down Redis costs seconds, and the clamp of 10 extra
/// retries bounds a single wrapped read at 12 attempts, not at any wall-clock
/// figure - which is tens of seconds to minutes on one call. A stall counts
/// too: [`is_transient`] treats a timeout as retryable, so a merely slow server
/// makes every wrapped read issue up to `attempts` commands instead of one.
/// Raise this only when a caller can afford to wait that long.
pub(crate) fn attempts_from_raw(raw: Option<&str>) -> u32 {
    let extra = raw
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
        .min(10);
    2 + extra
}

/// Whether `e` is worth retrying: the connection failed, not the command.
///
/// Laravel's post-#61175 match also accepts `RedisClusterException` and
/// `'Error processing response from Redis node'`. Both are cluster-only, no
/// Suprnova driver speaks Redis Cluster (the `redis` dependency is built
/// without the `cluster` features), and redis-rs reports a cluster
/// redirection through `RedisError::is_cluster_error` rather than a message
/// string - so there is nothing to translate.
///
/// A `READONLY` reply is excluded on purpose. Laravel matches that string
/// because a phpredis client can be pointed at another node; a Suprnova driver
/// holds one connection to one endpoint, so retrying a replica that just told
/// us it is read-only produces the same answer, more slowly. redis-rs agrees:
/// its own `retry_method` returns `NoRetry` for that kind.
///
/// `LOADING`, `TRYAGAIN`, and `MASTERDOWN` are excluded too, and this one is a
/// judgment call rather than an echo of redis-rs, whose `retry_method` marks
/// all three `WaitAndRetry`. They are server-authored replies about server
/// state, not connection failures: the server received the command, understood
/// it, and asked for time. Answering that on a 50 ms cadence adds load to a
/// node that is already recovering, and the wait these need is measured in
/// seconds to minutes - far outside a per-command budget. Surfacing them lets
/// the caller decide.
pub(crate) fn is_transient(e: &redis::RedisError) -> bool {
    e.is_connection_dropped() || e.is_io_error() || e.is_connection_refusal() || e.is_timeout()
}

/// Run an idempotent Redis command, retrying it after a transient failure.
///
/// `make` is called once per attempt and must build a *fresh* future each
/// time. Cloning the driver's `ConnectionManager` inside the closure is how the
/// call sites do that, and the reason is ownership, not connection routing: a
/// clone per attempt keeps the closure a plain `FnMut` with nothing borrowed
/// across the await. A `ConnectionManager` clone shares one `Arc` of internal
/// state and reloads the live connection slot on every command, so a clone and
/// the original see the same replacement connection after a drop - the clone
/// is not what makes the retry reach a healthy socket.
///
/// # Only wrap commands that are safe to run twice
///
/// There is no allowlist here to protect you. A transient error means the
/// connection failed, not that the server declined the command - the server
/// may well have executed it before the socket died. Wrapping a `ZADD`, an
/// `INCR`, a `SET NX`, or a queue pop therefore risks a second execution, and
/// `REDIS_COMMAND_RETRIES` deliberately cannot opt those in either. Every call
/// site that uses this carries a comment saying why its command is idempotent.
///
/// `op` names the command for the retry log line only.
pub(crate) async fn retry_read<T, F, Fut>(op: &'static str, mut make: F) -> redis::RedisResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = redis::RedisResult<T>>,
{
    let attempts = read_attempts();
    let mut attempt = 1;
    loop {
        match make().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt < attempts && is_transient(&e) => {
                tracing::warn!(
                    op,
                    attempt,
                    attempts,
                    error = %e,
                    "transient redis failure on a read-shaped command; retrying"
                );
                attempt += 1;
                tokio::time::sleep(RETRY_BACKOFF).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::{ErrorKind, RedisError};
    use std::cell::Cell;

    fn transient() -> RedisError {
        RedisError::from((ErrorKind::Io, "connection reset by peer"))
    }

    fn permanent() -> RedisError {
        RedisError::from((ErrorKind::Parse, "unparseable reply"))
    }

    #[test]
    fn classification_separates_transient_from_permanent() {
        assert!(is_transient(&transient()), "an IO error is transient");
        assert!(
            !is_transient(&permanent()),
            "a parse failure is the server answering; retrying cannot change it"
        );
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_and_the_second_attempt_wins() {
        let calls = Cell::new(0u32);
        let got: u64 = retry_read("unit GET", || {
            let n = calls.get() + 1;
            calls.set(n);
            async move { if n == 1 { Err(transient()) } else { Ok(7u64) } }
        })
        .await
        .expect("the retry must succeed");
        assert_eq!(got, 7);
        assert_eq!(calls.get(), 2, "exactly one retry, not a loop");
    }

    #[tokio::test]
    async fn a_permanent_failure_is_not_retried() {
        let calls = Cell::new(0u32);
        let err = retry_read("unit GET", || {
            calls.set(calls.get() + 1);
            async move { Err::<u64, _>(permanent()) }
        })
        .await
        .expect_err("a permanent error must surface");
        assert_eq!(err.kind(), redis::ErrorKind::Parse);
        assert_eq!(
            calls.get(),
            1,
            "retrying a server-side rejection only doubles the latency"
        );
    }

    #[tokio::test]
    async fn the_budget_is_finite_and_the_last_error_is_returned() {
        let calls = Cell::new(0u32);
        let err = retry_read("unit GET", || {
            calls.set(calls.get() + 1);
            async move { Err::<u64, _>(transient()) }
        })
        .await
        .expect_err("an unreachable server must still fail");
        assert!(err.is_io_error(), "the caller sees the last real error");
        assert_eq!(
            calls.get(),
            read_attempts(),
            "a permanently down Redis must not spin"
        );
    }

    #[test]
    fn the_configured_budget_is_parsed_and_clamped() {
        assert_eq!(
            attempts_from_raw(None),
            2,
            "one try plus the built-in retry"
        );
        assert_eq!(attempts_from_raw(Some("0")), 2);
        assert_eq!(attempts_from_raw(Some("3")), 5);
        assert_eq!(
            attempts_from_raw(Some("nonsense")),
            2,
            "an unparseable value falls back to the default rather than failing boot"
        );
        assert_eq!(
            attempts_from_raw(Some("9999")),
            12,
            "clamped at 10 extra retries so a typo cannot turn an outage into a hang"
        );
    }
}
