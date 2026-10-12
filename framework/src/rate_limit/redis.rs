//! Redis ZSET-backed sliding window. ZADD on acquire, ZREMRANGEBYSCORE
//! to evict, ZCOUNT to count. Atomic via a single Lua eval per call.
//!
//! One key can serve quotas with different windows (a per-minute and a
//! per-hour limit on the same account, or a limit whose window changes
//! between deploys). Like the in-memory driver, this one keeps hits for
//! the longest window any caller has used on the key, recorded beside the
//! hit set, and counts each caller's own window inside that history. A
//! shorter quota therefore never prunes, or expires, hits a longer quota
//! still counts.

use crate::error::FrameworkError;
use crate::rate_limit::{RateLimiterDriver, SlidingWindowConfig};
use async_trait::async_trait;
use redis::Script;
use redis::aio::ConnectionManager;
use std::time::Duration;
use uuid::Uuid;

/// Redis-backed sliding-window rate limiter. Stores hit timestamps in a
/// ZSET per key and prunes them atomically via a single Lua eval.
pub struct RedisRateLimiter {
    conn: ConnectionManager,
    prefix: String,
}

impl RedisRateLimiter {
    /// Open a connection to `url` and return a limiter that scopes
    /// every key under `prefix`.
    pub async fn connect(url: &str, prefix: &str) -> Result<Self, FrameworkError> {
        let client = crate::redis_client::open(url)
            .map_err(|e| FrameworkError::internal(format!("redis open: {e}")))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis conn: {e}")))?;
        Ok(Self {
            conn,
            prefix: prefix.into(),
        })
    }
}

/// Where the longest window used on `key` is recorded, in milliseconds.
///
/// A sibling of the hit set rather than a member of it, so it never
/// counts as a hit. The `rlw:` namespace cannot collide with any `rl:` hit
/// set, whatever the caller's key contains.
fn retention_key(prefix: &str, key: &str) -> String {
    format!("{prefix}rlw:{key}")
}

#[async_trait]
impl RateLimiterDriver for RedisRateLimiter {
    async fn try_acquire(
        &self,
        key: &str,
        config: &SlidingWindowConfig,
    ) -> Result<bool, FrameworkError> {
        let zkey = format!("{}rl:{}", self.prefix, key);
        let wkey = retention_key(&self.prefix, key);
        let now_ms = crate::clock::now().timestamp_millis();
        let window_ms = config.window.as_millis() as i64;
        let member = Uuid::new_v4().to_string();

        // `retention` is the longest window seen on this key. Hits are
        // pruned and the key expires by it, never by the caller's own
        // window, which only bounds what this caller counts. A raised
        // retention is recorded even on a refusal, as the in-memory
        // driver records it, so a refused long quota is still protected
        // from the next short one.
        let script = Script::new(
            r"
            local zkey = KEYS[1]
            local wkey = KEYS[2]
            local now = tonumber(ARGV[1])
            local window = tonumber(ARGV[2])
            local max = tonumber(ARGV[3])
            local member = ARGV[4]
            local stored = tonumber(redis.call('GET', wkey)) or 0
            local retention = stored
            if window > retention then retention = window end
            redis.call('ZREMRANGEBYSCORE', zkey, '-inf', now - retention)
            local count = redis.call('ZCOUNT', zkey, '(' .. (now - window), '+inf')
            local allowed = count < max
            if allowed then
                redis.call('ZADD', zkey, now, member)
            end
            if allowed or retention > stored then
                redis.call('PEXPIRE', zkey, retention)
                redis.call('SET', wkey, retention, 'PX', retention)
            end
            if allowed then
                return 1
            end
            return 0
        ",
        );
        // Deliberately not retried: the script `ZADD`s a fresh UUID member, so
        // a retry after a socket drop whose command the server did execute
        // consumes two slots from the window and throttles a caller early.
        let mut conn = self.conn.clone();
        let ok: i64 = script
            .key(&zkey)
            .key(&wkey)
            .arg(now_ms)
            .arg(window_ms)
            .arg(config.max_requests as i64)
            .arg(member)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("rate limit script: {e}")))?;
        Ok(ok == 1)
    }

    async fn retry_after(
        &self,
        key: &str,
        config: &SlidingWindowConfig,
    ) -> Result<Option<Duration>, FrameworkError> {
        let zkey = format!("{}rl:{}", self.prefix, key);
        let wkey = retention_key(&self.prefix, key);
        let now_ms = crate::clock::now().timestamp_millis();
        let window_ms = config.window.as_millis() as i64;

        // Single Lua block so the evict / count / boundary-score reads
        // observe the same snapshot. As three separate round-trips a
        // concurrent `try_acquire` (which is itself atomic) could
        // ZADD between our ZCARD and our ZRANGE - count says "at
        // limit" then ZRANGE returns a *newer* member's score,
        // shrinking the computed Retry-After well below the real
        // remaining window. Returns -1 for "under limit, no header
        // needed" and a non-negative ms count for "still throttled,
        // header should be at least this long."
        //
        // Pruning goes by the key's retention, as in `try_acquire`. The
        // set can then hold older hits than this caller's window, and,
        // when a longer quota shares the key, more hits inside it than
        // `max`. The wait is until the hit `max` places from the newest
        // leaves the window, which is the in-memory driver's answer too.
        let script = Script::new(
            r"
            local zkey = KEYS[1]
            local wkey = KEYS[2]
            local now = tonumber(ARGV[1])
            local window = tonumber(ARGV[2])
            local max = tonumber(ARGV[3])
            local retention = tonumber(redis.call('GET', wkey)) or 0
            if window > retention then retention = window end
            redis.call('ZREMRANGEBYSCORE', zkey, '-inf', now - retention)
            local floor = '(' .. (now - window)
            local count = redis.call('ZCOUNT', zkey, floor, '+inf')
            if count < max then
                return -1
            end
            local boundary
            if max > 0 then
                boundary = redis.call('ZRANGE', zkey, -max, -max, 'WITHSCORES')
            else
                boundary = redis.call('ZRANGEBYSCORE', zkey, floor, '+inf', 'WITHSCORES', 'LIMIT', 0, 1)
            end
            if #boundary < 2 then
                return 0
            end
            local oldest_score = tonumber(boundary[2])
            local elapsed = now - oldest_score
            if elapsed < 0 then elapsed = 0 end
            local remaining = window - elapsed
            if remaining < 0 then remaining = 0 end
            return remaining
            ",
        );
        // Retried on a transient failure, unlike `try_acquire` above it. This
        // script's only write is `ZREMRANGEBYSCORE ... -inf now-retention`,
        // which removes members already outside every window the key serves -
        // running it twice has the same effect as running it once, and it adds
        // nothing. A dropped
        // connection here would otherwise cost the response its `Retry-After`
        // header for no reason. `now_ms` is captured before the first attempt,
        // so a retried computation is stale by however long the reconnect
        // took - up to this driver's reconnect budget, about a second and a
        // half with the defaults; that under-reports the wait by at most that
        // much on a header measured in whole seconds.
        let max_requests = config.max_requests as i64;
        let remaining_ms: i64 = crate::redis_retry::retry_read("rl retry_after script", || {
            let mut conn = self.conn.clone();
            let script = &script;
            let zkey = &zkey;
            let wkey = &wkey;
            async move {
                script
                    .key(zkey)
                    .key(wkey)
                    .arg(now_ms)
                    .arg(window_ms)
                    .arg(max_requests)
                    .invoke_async(&mut conn)
                    .await
            }
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("rl retry_after script: {e}")))?;
        if remaining_ms < 0 {
            return Ok(None);
        }
        Ok(Some(Duration::from_millis(remaining_ms as u64)))
    }
}
