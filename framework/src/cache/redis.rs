//! Redis-backed cache implementation

use async_trait::async_trait;
use redis::{
    AsyncCommands, Client,
    aio::{ConnectionManager, ConnectionManagerConfig},
};
use std::time::Duration;

use super::config::CacheConfig;
use super::store::CacheStore;
use crate::error::FrameworkError;

/// How many forward-index members `flush_tags` pulls per `SSCAN` round.
///
/// A hint, not a guarantee - Redis may return more or fewer. It bounds the
/// per-round allocation and the size of one Lua invocation, which is the
/// whole point of scanning instead of `SMEMBERS`.
const TAG_SCAN_BATCH: usize = 256;

/// Atomically settle one `SSCAN` batch of a tag's forward index.
///
/// `KEYS[1]` is the tag index; `ARGV[1]` is the tag; `ARGV[2..]` alternates
/// `member, aux` (the value key and its tag-membership set), computed by
/// the caller so the aux-key format lives in exactly one place.
///
/// Why a script rather than the SISMEMBER-then-DEL it replaces: those were
/// two round trips with a gap between them. A concurrent untagged
/// `put_raw` landing in that gap dropped the aux entry, and the flush went
/// on to delete a value that was no longer tagged - silent data loss, and
/// only under load, which is the worst way to find it.
///
/// The `SREM` is per observed member instead of a `DEL` of the whole index
/// for the mirror-image reason: a `tagged_put_raw` that added a key while
/// the scan was running would have had its membership erased by the wider
/// delete, leaving a live tagged value that no future flush would ever
/// find. Empty sets disappear on their own in Redis, so the index still
/// goes away once the last member is removed.
const FLUSH_TAG_BATCH_LUA: &str = r#"
local tag = ARGV[1]
local flushed = 0
for i = 2, #ARGV, 2 do
    local member = ARGV[i]
    local aux = ARGV[i + 1]
    if redis.call('SISMEMBER', aux, tag) == 1 then
        redis.call('DEL', member, aux)
        flushed = flushed + 1
    end
    redis.call('SREM', KEYS[1], member)
end
return flushed
"#;

/// Convert a `Duration` into a Redis-millisecond TTL argument.
///
/// Redis sub-second TTLs are expressed via `PX` (set) and `PEXPIRE`
/// (extend). Sub-second durations passed as `EX`/`EXPIRE` truncate to 0
/// seconds, which Redis rejects for `SET ... EX 0` and, worse, treats as
/// "delete the key" for `EXPIRE key 0`. Routing every Redis TTL through
/// `PX`/`PEXPIRE` (Redis 2.6+, 2012) avoids both pitfalls.
///
/// `Duration::ZERO` is clamped to 1 ms so neither `PX 0` (rejected) nor
/// `PEXPIRE 0` (key-delete) can sneak through. Caller-side `Duration`s
/// outside u64 ms (≈ 584 million years) saturate to `u64::MAX`; Redis
/// will reject that as an invalid expire on its own.
#[inline]
fn redis_ttl_ms(d: Duration) -> u64 {
    let ms = d.as_millis();
    if ms == 0 {
        1
    } else if ms > u64::MAX as u128 {
        u64::MAX
    } else {
        ms as u64
    }
}

/// Redis cache implementation
///
/// Uses redis-rs with async/tokio runtime for high-performance caching.
pub struct RedisCache {
    conn: ConnectionManager,
    prefix: String,
    default_ttl: Option<Duration>,
}

impl RedisCache {
    /// Create a new Redis cache connection
    pub async fn connect(config: &CacheConfig) -> Result<Self, FrameworkError> {
        let client = Client::open(config.url.as_str())
            .map_err(|e| FrameworkError::internal(format!("Redis connection error: {}", e)))?;

        // Bound the initial-connect budget so an unreachable Redis fails
        // CLOSED promptly instead of hanging. The redis-rs
        // defaults are 6 reconnect retries with an UNCAPPED exponential
        // backoff (max_delay = None), so against a down/unreachable host the
        // connect future can take well over 10s to resolve with an error -
        // blocking `Cache::bootstrap` at startup for that whole window.
        //
        // We cap it: at most 3 retries, =<500ms between them, each connection
        // and command attempt bounded by an explicit timeout. A refused or
        // unreachable host now errors in under two seconds, while a healthy
        // Redis (sub-second on localhost/LAN) is unaffected.
        let cm_config = ConnectionManagerConfig::new()
            .set_connection_timeout(Some(Duration::from_secs(2)))
            .set_response_timeout(Some(Duration::from_secs(5)))
            .set_number_of_retries(3)
            .set_max_delay(Duration::from_millis(500));
        let conn = ConnectionManager::new_with_config(client, cm_config)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!("Redis connection manager error: {e}"))
            })?;

        let default_ttl = if config.default_ttl > 0 {
            Some(Duration::from_secs(config.default_ttl))
        } else {
            None
        };

        Ok(Self {
            conn,
            prefix: config.prefix.clone(),
            default_ttl,
        })
    }

    fn prefixed_key(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key)
    }

    /// Distributed-lock keyspace key for `key`.
    ///
    /// Locks live under a NUL-byte sentinel after the configured prefix
    /// so they cannot collide with any user-supplied cache key. User
    /// keys are always passed through `prefixed_key(...)` which does not
    /// inject the sentinel, so a caller doing `Cache::forget("lock:foo")`
    /// targets `<prefix>lock:foo` - distinct from the lock's
    /// `<prefix>\0lock:foo` slot. This prevents a regular `forget` /
    /// `put` from releasing or overwriting a held distributed lock.
    fn locked_key(&self, key: &str) -> String {
        format!("{}\0lock:{}", self.prefix, key)
    }

    /// Tag forward-index key (`tag -> set of value keys`).
    ///
    /// Hidden under the same NUL-byte sentinel as the lock keyspace so
    /// `Cache::forget("tag:users")` cannot drop the forward index for
    /// the `users` tag.
    fn tag_index_key(&self, tag: &str) -> String {
        format!("{}\0tag:{}", self.prefix, tag)
    }

    /// Aux SET that records the tag memberships for a value key.
    ///
    /// This lets `flush_tags` validate "is this key STILL tagged with `t`"
    /// at the moment of deletion, so an untagged overwrite of a previously
    /// tagged key is not silently deleted by a later `flush_tags(t)`.
    ///
    /// The aux set carries the same TTL as the value key, so an expired
    /// value's tag entries age out together rather than accumulating
    /// forever in the forward `tag:{t}` set.
    ///
    /// Stored under the same NUL-byte sentinel as the lock and tag
    /// forward index so the bookkeeping is unreachable from caller-side
    /// `Cache::put/forget/get`.
    fn key_tags_set(&self, prefixed_key: &str) -> String {
        format!("{}\0key_tags:{}", self.prefix, prefixed_key)
    }
}

#[async_trait]
impl CacheStore for RedisCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        let key = self.prefixed_key(key);

        // GET is a pure read: running it twice returns the same answer, so a
        // connection that died under the first attempt costs a reconnect, not
        // a failed cache read.
        let value: Option<String> = crate::redis_retry::retry_read("cache GET", || {
            let mut conn = self.conn.clone();
            let key = key.clone();
            async move { conn.get(&key).await }
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("Cache get error: {}", e)))?;

        Ok(value)
    }

    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.prefixed_key(key);
        let aux = self.key_tags_set(&pkey);

        // Drop any prior tag aux set so a later tagged_put_raw does not
        // resurrect stale tag memberships AND a later flush_tags cannot
        // delete this untagged value (the aux set is the source of truth
        // for "is this key still tagged with t?" at flush time). Pipelined
        // with the SET so an untagged write is still one round trip.
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.cmd("DEL").arg(&aux).ignore();
        // `None` ttl means **no expiration** per the CacheStore contract.
        // The facade resolves any configured default before calling this
        // method - otherwise `Cache::forever` would not be forever on
        // Redis.
        if let Some(duration) = ttl {
            pipe.cmd("SET")
                .arg(&pkey)
                .arg(value)
                .arg("PX")
                .arg(redis_ttl_ms(duration))
                .ignore();
        } else {
            pipe.cmd("SET").arg(&pkey).arg(value).ignore();
        }
        pipe.query_async::<()>(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache set error: {}", e)))?;
        Ok(())
    }

    fn default_ttl(&self) -> Option<Duration> {
        self.default_ttl
    }

    async fn add_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<bool, FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.prefixed_key(key);

        // Atomic via SET NX [PX ttl] - Redis writes the value only when
        // the key does not exist. Returns the string "OK" on success and
        // nil (Option::None) on contention.
        let res: Option<String> = if let Some(d) = ttl {
            redis::cmd("SET")
                .arg(&pkey)
                .arg(value)
                .arg("NX")
                .arg("PX")
                .arg(redis_ttl_ms(d))
                .query_async(&mut conn)
                .await
                .map_err(|e| FrameworkError::internal(format!("Cache add error: {e}")))?
        } else {
            redis::cmd("SET")
                .arg(&pkey)
                .arg(value)
                .arg("NX")
                .query_async(&mut conn)
                .await
                .map_err(|e| FrameworkError::internal(format!("Cache add error: {e}")))?
        };

        // If we wrote a fresh untagged value, drop any leftover tag aux
        // set so a stale flush_tags cannot delete it.
        if res.is_some() {
            let aux = self.key_tags_set(&pkey);
            redis::cmd("DEL")
                .arg(&aux)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| FrameworkError::internal(format!("Cache aux drop: {e}")))?;
        }

        Ok(res.is_some())
    }

    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        let key = self.prefixed_key(key);

        // EXISTS, like GET, answers the same way however many times it runs.
        let exists: bool = crate::redis_retry::retry_read("cache EXISTS", || {
            let mut conn = self.conn.clone();
            let key = key.clone();
            async move { conn.exists(&key).await }
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("Cache exists error: {}", e)))?;

        Ok(exists)
    }

    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.prefixed_key(key);

        // Drop the value AND its tag aux set. The forward `tag:{t}` set
        // may still list this key; that's harmless - flush_tags validates
        // membership via the aux set and skips a key whose aux set says
        // "no longer tagged with t" (or no longer exists at all).
        let aux = self.key_tags_set(&pkey);
        // `DEL key aux` returns the count of ALL keys removed, so deleting the
        // value and its aux bookkeeping key together would report `true` even
        // when only the aux key survived (e.g. the value expired first while
        // its aux entry lagged). Delete both in one pipeline but report
        // existence based on the VALUE key's own DEL result - the aux delete is
        // ignored so it doesn't inflate the count.
        let (value_deleted,): (i64,) = redis::pipe()
            .atomic()
            .cmd("DEL")
            .arg(&pkey)
            .cmd("DEL")
            .arg(&aux)
            .ignore()
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache delete error: {}", e)))?;

        Ok(value_deleted > 0)
    }

    async fn flush(&self) -> Result<(), FrameworkError> {
        let mut conn = self.conn.clone();

        // SCAN beats KEYS for production: incremental cursor iteration
        // avoids blocking the Redis server on a single O(N) pass. We
        // batch DEL per page so very large keyspaces don't build one
        // giant argument list. The MATCH glob is anchored to our prefix
        // so we never touch other applications' keys.
        let pattern = format!("{}*", self.prefix);
        let mut cursor: u64 = 0;
        loop {
            // SCAN is a pure read; the DEL below is not, and is deliberately
            // left un-retried.
            let (next_cursor, batch): (u64, Vec<String>) =
                crate::redis_retry::retry_read("cache SCAN", || {
                    let mut conn = self.conn.clone();
                    let pattern = pattern.clone();
                    async move {
                        redis::cmd("SCAN")
                            .arg(cursor)
                            .arg("MATCH")
                            .arg(&pattern)
                            .arg("COUNT")
                            .arg(500)
                            .query_async(&mut conn)
                            .await
                    }
                })
                .await
                .map_err(|e| FrameworkError::internal(format!("Cache flush scan error: {}", e)))?;
            if !batch.is_empty() {
                conn.del::<_, ()>(batch).await.map_err(|e| {
                    FrameworkError::internal(format!("Cache flush delete error: {}", e))
                })?;
            }
            if next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }

        Ok(())
    }

    async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        let mut conn = self.conn.clone();
        let key = self.prefixed_key(key);

        let value: i64 = conn
            .incr(&key, amount)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache increment error: {}", e)))?;

        Ok(value)
    }

    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        let mut conn = self.conn.clone();
        let key = self.prefixed_key(key);

        let value: i64 = conn
            .decr(&key, amount)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache decrement error: {}", e)))?;

        Ok(value)
    }

    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.prefixed_key(key);
        let aux = self.key_tags_set(&pkey);

        let mut pipe = redis::pipe();
        pipe.atomic();
        // Rewrite the aux set from scratch - replaces (not unions with)
        // any prior tag memberships. This is what protects a tagged
        // overwrite from carrying old tags.
        pipe.cmd("DEL").arg(&aux).ignore();
        // `None` ttl honoured literally - see put_raw for rationale.
        if let Some(d) = ttl {
            let pxms = redis_ttl_ms(d);
            pipe.cmd("SET")
                .arg(&pkey)
                .arg(value)
                .arg("PX")
                .arg(pxms)
                .ignore();
            // Aux set rides the same TTL so the bookkeeping ages out with
            // the value rather than accumulating forever.
            if !tags.is_empty() {
                let mut sadd = redis::cmd("SADD");
                sadd.arg(&aux);
                for t in tags {
                    sadd.arg(*t);
                }
                pipe.add_command(sadd).ignore();
                pipe.cmd("PEXPIRE").arg(&aux).arg(pxms).ignore();
            }
        } else {
            pipe.cmd("SET").arg(&pkey).arg(value).ignore();
            if !tags.is_empty() {
                let mut sadd = redis::cmd("SADD");
                sadd.arg(&aux);
                for t in tags {
                    sadd.arg(*t);
                }
                pipe.add_command(sadd).ignore();
            }
        }
        // Forward index: tag -> set of value keys. Used as the candidate
        // list by flush_tags; the aux set is the source of truth for
        // "is this key still tagged with t" at deletion time.
        for t in tags {
            let tag_key = self.tag_index_key(t);
            pipe.cmd("SADD").arg(&tag_key).arg(&pkey).ignore();
        }
        pipe.query_async::<()>(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache tagged set: {e}")))?;
        Ok(())
    }

    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        let mut conn = self.conn.clone();
        for t in tags {
            let tag_key = self.tag_index_key(t);
            let mut cursor: u64 = 0;
            loop {
                // SSCAN, not SMEMBERS. A tag's forward index is unbounded -
                // it grows with every key ever written under that tag - and
                // SMEMBERS materialises all of it, in Redis and again in this
                // process. On a large tag that is a multi-megabyte allocation
                // behind a command that blocks the whole server while it
                // serialises. SSCAN bounds both.
                //
                // Removing members while scanning is safe: SSCAN guarantees
                // every element present for the full scan is returned at
                // least once, and elements removed mid-scan are exactly the
                // ones already handled.
                // SSCAN is a pure read and its cursor contract already
                // tolerates a page being served twice. The EVAL below deletes
                // values, so it is never retried.
                let (next, members): (u64, Vec<String>) =
                    crate::redis_retry::retry_read("cache SSCAN", || {
                        let mut conn = self.conn.clone();
                        let tag_key = tag_key.clone();
                        async move {
                            redis::cmd("SSCAN")
                                .arg(&tag_key)
                                .arg(cursor)
                                .arg("COUNT")
                                .arg(TAG_SCAN_BATCH)
                                .query_async(&mut conn)
                                .await
                        }
                    })
                    .await
                    .map_err(|e| FrameworkError::internal(format!("Cache tag scan: {e}")))?;

                if !members.is_empty() {
                    let mut script = redis::cmd("EVAL");
                    script.arg(FLUSH_TAG_BATCH_LUA).arg(1).arg(&tag_key).arg(*t);
                    for member in &members {
                        script.arg(member).arg(self.key_tags_set(member));
                    }
                    script
                        .query_async::<i64>(&mut conn)
                        .await
                        .map_err(|e| FrameworkError::internal(format!("Cache tag flush: {e}")))?;
                }

                cursor = next;
                if cursor == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.locked_key(key);
        let token = uuid::Uuid::new_v4().to_string();

        // SET key token NX PX ttl_ms - atomic: only sets if key does not
        // exist. PX preserves sub-second precision (EX truncates and a
        // sub-second TTL would round to 0, which Redis rejects).
        let res: Option<String> = redis::cmd("SET")
            .arg(&pkey)
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(redis_ttl_ms(ttl))
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Lock acquire: {e}")))?;

        // Redis returns "OK" string on success, nil (None) on contention
        Ok(res.map(|_ok| token))
    }

    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.locked_key(key);
        // Atomically: if GET key == token then DEL key, else return 0
        let script = redis::Script::new(
            "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end",
        );
        let removed: i64 = script
            .key(&pkey)
            .arg(token)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Lock release: {e}")))?;
        Ok(removed == 1)
    }

    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.locked_key(key);
        // Atomically: if GET key == token then PEXPIRE key ttl_ms, else
        // return 0. PEXPIRE preserves sub-second precision - EXPIRE
        // would truncate, and `EXPIRE key 0` deletes the key, which
        // would silently release the lock on a sub-second refresh.
        let script = redis::Script::new(
            "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('PEXPIRE', KEYS[1], ARGV[2]) else return 0 end",
        );
        let ok: i64 = script
            .key(&pkey)
            .arg(token)
            .arg(redis_ttl_ms(ttl) as i64)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Lock refresh: {e}")))?;
        Ok(ok == 1)
    }

    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        let mut conn = self.conn.clone();
        let pkey = self.prefixed_key(key);
        // PEXPIRE returns 1 if the TTL was set, 0 if the key does not
        // exist. PEXPIRE preserves sub-second precision; EXPIRE would
        // truncate a sub-second ttl to 0 and delete the key.
        let ok: i64 = redis::cmd("PEXPIRE")
            .arg(&pkey)
            .arg(redis_ttl_ms(ttl))
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache touch: {e}")))?;
        Ok(ok == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_ttl_ms_preserves_millisecond_resolution() {
        assert_eq!(redis_ttl_ms(Duration::from_millis(1)), 1);
        assert_eq!(redis_ttl_ms(Duration::from_millis(50)), 50);
        assert_eq!(redis_ttl_ms(Duration::from_millis(999)), 999);
        assert_eq!(redis_ttl_ms(Duration::from_secs(1)), 1_000);
        assert_eq!(redis_ttl_ms(Duration::from_secs(60)), 60_000);
    }

    #[test]
    fn redis_ttl_ms_clamps_zero_to_one_ms() {
        // Redis rejects PX 0 and PEXPIRE key 0 deletes the key - clamp
        // to 1 ms so neither failure mode is reachable from this layer.
        assert_eq!(redis_ttl_ms(Duration::ZERO), 1);
    }

    #[test]
    fn redis_ttl_ms_handles_large_durations_safely() {
        // 1 year in ms fits comfortably in u64; verify the path.
        let one_year_ms = 365u64 * 24 * 60 * 60 * 1000;
        assert_eq!(
            redis_ttl_ms(Duration::from_secs(365 * 24 * 60 * 60)),
            one_year_ms
        );
        // u64::MAX milliseconds is a hard ceiling - anything past it
        // saturates rather than wrapping or panicking.
        assert_eq!(redis_ttl_ms(Duration::MAX), u64::MAX);
    }

    #[test]
    fn redis_ttl_ms_subsecond_does_not_round_to_zero() {
        // The bug we're fixing: `as_secs()` of any sub-second Duration is
        // 0. Verify the replacement preserves precision instead.
        let half_sec = Duration::from_millis(500);
        assert_eq!(half_sec.as_secs(), 0, "control: as_secs truncates");
        assert_eq!(redis_ttl_ms(half_sec), 500, "as_millis preserves");
    }
}
