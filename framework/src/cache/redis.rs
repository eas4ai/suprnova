//! Redis-backed cache implementation

use async_trait::async_trait;
use redis::{AsyncCommands, aio::ConnectionManager};
use std::sync::{PoisonError, RwLock};
use std::time::Duration;

use super::config::CacheConfig;
use super::store::{CacheStore, ConditionalIncrement};
use crate::error::FrameworkError;
use crate::redis_facade::{Redis, RedisConnection};

/// How many forward-index members `flush_tags` pulls per `SSCAN` round.
///
/// A hint, not a guarantee - Redis may return more or fewer. It bounds the
/// per-round allocation and the size of one Lua invocation, which is the
/// whole point of scanning instead of `SMEMBERS`.
const TAG_SCAN_BATCH: usize = 256;

fn namespaced_key(prefix: &str, namespace: &str, key: &str) -> String {
    format!("{prefix}\0{namespace}:{key}")
}

/// Escape `literal` so a Redis glob pattern matches it character for
/// character.
///
/// `SCAN MATCH` reads `*`, `?`, `[`, `]` and `\` as pattern syntax. The
/// prefix is stored literally, so it has to be matched literally too: an
/// unescaped `[ab]` matches `a` or `b` and never the stored `[ab]`, and a
/// trailing `\` escapes the `*` that was meant to follow it.
fn glob_escape(literal: &str) -> String {
    let mut escaped = String::with_capacity(literal.len());
    for c in literal.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

fn data_key(prefix: &str, key: &str) -> String {
    if key.starts_with('\0') {
        namespaced_key(prefix, "data", key)
    } else {
        format!("{prefix}{key}")
    }
}

/// How many members of a tag's forward index each tagged write checks, and
/// prunes when they can no longer be reached.
///
/// A member stays in the forward index after its value expires, is
/// forgotten, or is overwritten untagged, because none of those touch the
/// index. Checking a fixed sample on every write to the tag keeps the index
/// close to its live size: each write adds one member and removes most of
/// the dead ones it samples, so a tag written often never grows without
/// bound. A tag whose index holds fewer members than this is checked whole.
const TAG_PRUNE_SAMPLE: usize = 16;

/// Prune a sample of a tag's forward index.
///
/// `KEYS[1]` is the tag index, `ARGV[1]` the tag, `ARGV[2]` the prefix of
/// every aux tag-membership key, and `ARGV[3]` the sample size. A member
/// whose aux set no longer names the tag is unreachable through this tag -
/// its value expired, was forgotten, or was overwritten without it - so it
/// is removed. The aux set is the same source of truth `flush_tags` uses, so
/// a member this removes is one a flush would have skipped.
const PRUNE_TAG_SAMPLE_LUA: &str = r#"
local sample = redis.call('SRANDMEMBER', KEYS[1], tonumber(ARGV[3]))
for _, member in ipairs(sample) do
    if redis.call('SISMEMBER', ARGV[2] .. member, ARGV[1]) == 0 then
        redis.call('SREM', KEYS[1], member)
    end
end
return #sample
"#;

/// Atomically settle one `SSCAN` batch of a tag's forward index.
///
/// `KEYS[1]` is the tag index; `ARGV[1]` is the tag; `ARGV[2]` is the
/// prefix of every tag index key; `ARGV[3..]` alternates `member, aux` (the
/// value key and its tag-membership set), computed by the caller so the
/// key formats live in exactly one place.
///
/// A deleted value is also removed from the indexes of its *other* tags.
/// Leaving it there would grow those indexes with a member nothing can
/// reach until each of those tags is flushed in turn.
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
local index_prefix = ARGV[2]
local flushed = 0
for i = 3, #ARGV, 2 do
    local member = ARGV[i]
    local aux = ARGV[i + 1]
    if redis.call('SISMEMBER', aux, tag) == 1 then
        for _, other in ipairs(redis.call('SMEMBERS', aux)) do
            if other ~= tag then
                redis.call('SREM', index_prefix .. other, member)
            end
        end
        redis.call('DEL', member, aux)
        flushed = flushed + 1
    end
    redis.call('SREM', KEYS[1], member)
end
return flushed
"#;

/// Atomically install a missing untagged value and clear stale tag metadata.
///
/// `KEYS[1]` is the value key, `KEYS[2]` is its auxiliary tag-membership set,
/// `ARGV[1]` is the value, and optional `ARGV[2]` is the TTL in milliseconds.
/// The aux key is removed only when this invocation wins `SET NX`, so a newer
/// tagged overwrite can never land between the conditional write and cleanup.
///
/// RedisCache uses redis-rs' single-node `ConnectionManager`, not its cluster
/// client. The two-key script therefore does not add a cross-slot limitation
/// to the supported driver surface.
const ADD_RAW_LUA: &str = r#"
-- suprnova_cache_add_raw_v1
local result
if ARGV[2] then
    result = redis.call('SET', KEYS[1], ARGV[1], 'NX', 'PX', ARGV[2])
else
    result = redis.call('SET', KEYS[1], ARGV[1], 'NX')
end
if result then
    redis.call('DEL', KEYS[2])
    return 1
end
return 0
"#;

/// Atomically add to a counter only while it is below a ceiling.
///
/// `KEYS[1]` is the counter key, `ARGV[1]` the amount and `ARGV[2]` the
/// ceiling. A missing key reads as 0. A value `INCRBY` would refuse is
/// refused the same way, before anything changes: Redis stores a counter as
/// a canonical decimal, an optional `-` and no leading zero. Below the
/// ceiling the script runs `INCRBY` and returns `{1, new}`; otherwise it
/// returns `{0, current}` and writes nothing. `INCRBY` keeps the key's TTL,
/// and the counter carries no tag record, as for `increment`.
///
/// The comparison is on the decimal strings, sign first, then length, then
/// digits byte by byte, because a Lua number is a double: near `i64::MAX`
/// both sides would round to 2^63 and compare equal. The values come back as
/// the strings `GET` returns, for the same reason.
const INCREMENT_IF_BELOW_LUA: &str = r#"
-- suprnova_cache_increment_if_below_v1
local function below(value, ceiling)
    local value_negative = string.sub(value, 1, 1) == '-'
    local ceiling_negative = string.sub(ceiling, 1, 1) == '-'
    if value_negative ~= ceiling_negative then
        return value_negative
    end
    if value == ceiling then
        return false
    end
    -- Whether the magnitude of value is the smaller one, byte by byte,
    -- so the server's collation locale plays no part.
    local smaller = #value < #ceiling
    if #value == #ceiling then
        for i = 1, #value do
            local value_byte, ceiling_byte = string.byte(value, i), string.byte(ceiling, i)
            if value_byte ~= ceiling_byte then
                smaller = value_byte < ceiling_byte
                break
            end
        end
    end
    if value_negative then
        return not smaller
    end
    return smaller
end
local current = redis.call('GET', KEYS[1])
if not current then
    current = '0'
elseif current ~= '0' and not string.match(current, '^%-?[1-9]%d*$') then
    return redis.error_reply('ERR value is not an integer or out of range')
end
if below(current, ARGV[2]) then
    redis.call('INCRBY', KEYS[1], ARGV[1])
    return {1, redis.call('GET', KEYS[1])}
end
return {0, current}
"#;

/// Extend a value's TTL and its tag-membership record together.
///
/// `KEYS[1]` is the value key, `KEYS[2]` its aux tag-membership set, and
/// `ARGV[1]` the TTL in milliseconds. A tagged write gives the aux set the
/// value's TTL, and `flush_tags` deletes a value only while the aux set
/// still names the tag. Extending the value alone let the aux set expire at
/// the old TTL, after which a tag flush skipped a value that was still live.
/// The aux set is extended only when the value exists, so a stale aux set
/// left behind by an expired value is never revived.
const TOUCH_LUA: &str = r#"
local touched = redis.call('PEXPIRE', KEYS[1], ARGV[1])
if touched == 1 then
    redis.call('PEXPIRE', KEYS[2], ARGV[1])
end
return touched
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
/// Runs its commands on one named connection of the [`Redis`] facade and
/// takes its locks on another, as Laravel's `RedisStore` does: the
/// bootstrap builds it with [`connect_named`](Self::connect_named) on the
/// connections `REDIS_CACHE_CONNECTION` and `REDIS_CACHE_LOCK_CONNECTION`
/// name, `cache` and `default` by default. Every key it writes is the
/// connection's prefix (`REDIS_PREFIX`), then the cache prefix, then the
/// key.
pub struct RedisCache {
    /// Where the commands go. [`CacheStore::set_connection`] replaces it.
    commands: RwLock<Endpoint>,
    /// Where the locks go.
    locks: Endpoint,
    /// The cache prefix, `CACHE_PREFIX`, kept to rebuild the commands'
    /// whole prefix when the connection changes.
    cache_prefix: String,
    default_ttl: Option<Duration>,
}

/// A connection and the whole prefix of the keys the store writes on it:
/// the connection's own prefix, then the cache prefix.
#[derive(Clone)]
struct Endpoint {
    connection: RedisConnection,
    prefix: String,
}

impl Endpoint {
    fn new(connection: RedisConnection, cache_prefix: &str) -> Self {
        let prefix = format!("{}{cache_prefix}", connection.prefix());
        Self { connection, prefix }
    }

    /// The connection for one operation. The facade's connection is bound
    /// to the runtime it was first used on and opened again on a new one,
    /// so it is taken per operation, never kept.
    fn target(&self) -> Result<Target, FrameworkError> {
        Ok(Target {
            conn: self.connection.client()?,
            prefix: self.prefix.clone(),
        })
    }
}

/// One operation's connection and key prefix.
struct Target {
    conn: ConnectionManager,
    prefix: String,
}

impl Target {
    fn prefixed_key(&self, key: &str) -> String {
        data_key(&self.prefix, key)
    }

    /// Distributed-lock keyspace key for `key`.
    ///
    /// Values and locks carry distinct namespace components before the
    /// caller-controlled key, so no user key can address a lock slot.
    fn locked_key(&self, key: &str) -> String {
        namespaced_key(&self.prefix, "lock", key)
    }

    /// Tag forward-index key (`tag -> set of value keys`).
    ///
    /// Hidden under the same NUL-byte sentinel as the lock keyspace so
    /// `Cache::forget("tag:users")` cannot drop the forward index for
    /// the `users` tag.
    fn tag_index_key(&self, tag: &str) -> String {
        namespaced_key(&self.prefix, "tag", tag)
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
        namespaced_key(&self.prefix, "key_tags", prefixed_key)
    }

    /// The part every aux tag-membership key shares, for the Lua scripts
    /// that derive an aux key from a member name.
    fn key_tags_prefix(&self) -> String {
        namespaced_key(&self.prefix, "key_tags", "")
    }

    /// The part every tag forward-index key shares, for the Lua scripts
    /// that derive an index key from a tag name.
    fn tag_index_prefix(&self) -> String {
        namespaced_key(&self.prefix, "tag", "")
    }
}

impl RedisCache {
    /// Build a store on a Redis server of its own: `config.url`, for its
    /// commands and its locks, with `config.prefix` as the whole key prefix.
    /// For code that builds a store by hand; the bootstrap uses
    /// [`connect_named`](Self::connect_named).
    ///
    /// # Errors
    ///
    /// When the URL is not usable, or the server does not answer the one
    /// command sent while the store is built.
    pub async fn connect(config: &CacheConfig) -> Result<Self, FrameworkError> {
        let client = crate::redis_client::open(config.url.as_str())
            .map_err(|e| FrameworkError::internal(format!("Redis connection error: {}", e)))?;
        // A connection of its own, with no connection prefix, so the keys
        // are the cache prefix and the key, as they always were.
        let connection = RedisConnection::new("cache store", client, String::new());
        Self::on(connection.clone(), connection, config).await
    }

    /// Build the store the bootstrap binds for `CACHE_DRIVER=redis`: its
    /// commands on the [`Redis`] connection `config.connection` names, its
    /// locks on the one `config.lock_connection` names, as Laravel's
    /// `createRedisDriver` builds its store on `connection` and
    /// `lock_connection`.
    ///
    /// # Errors
    ///
    /// When either connection is not defined, or the command connection
    /// does not answer the one command sent while the store is built: a
    /// cache that cannot reach its server fails the boot instead of every
    /// request.
    pub async fn connect_named(config: &CacheConfig) -> Result<Self, FrameworkError> {
        let commands = Redis::connection(&config.connection)?;
        let locks = Redis::connection(&config.lock_connection)?;
        Self::on(commands, locks, config).await
    }

    /// The store on `commands` and `locks`, once `commands` has answered a
    /// `PING`.
    async fn on(
        commands: RedisConnection,
        locks: RedisConnection,
        config: &CacheConfig,
    ) -> Result<Self, FrameworkError> {
        // The facade's connections open lazily, so a server that cannot
        // answer would otherwise surface on the first read. Each attempt is
        // bounded (2s to connect, at most 3 retries 500ms apart), so an
        // unreachable host fails the boot within seconds instead of hanging
        // it.
        let mut conn = commands.client()?;
        redis::cmd("PING")
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "the Redis connection '{}' at {} did not answer: {e}",
                    commands.name(),
                    commands.endpoint()
                ))
            })?;

        let default_ttl = if config.default_ttl > 0 {
            Some(Duration::from_secs(config.default_ttl))
        } else {
            None
        };

        Ok(Self {
            commands: RwLock::new(Endpoint::new(commands, &config.prefix)),
            locks: Endpoint::new(locks, &config.prefix),
            cache_prefix: config.prefix.clone(),
            default_ttl,
        })
    }

    /// The connection and prefix for one command.
    fn target(&self) -> Result<Target, FrameworkError> {
        self.commands
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .target()
    }

    /// The connection and prefix for one lock operation.
    fn lock_target(&self) -> Result<Target, FrameworkError> {
        self.locks.target()
    }
}

#[async_trait]
impl CacheStore for RedisCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        let target = self.target()?;
        let key = target.prefixed_key(key);

        // GET is a pure read: running it twice returns the same answer, so a
        // connection that died under the first attempt costs a reconnect, not
        // a failed cache read.
        let value: Option<String> = crate::redis_retry::retry_read("cache GET", || {
            let mut conn = target.conn.clone();
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
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let pkey = target.prefixed_key(key);
        let aux = target.key_tags_set(&pkey);

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

    fn locks_are_shared(&self) -> bool {
        // The lock key lives in Redis, which every process shares.
        true
    }

    fn name(&self) -> &str {
        "redis"
    }

    fn set_connection(&self, name: &str) -> Result<(), FrameworkError> {
        let endpoint = Endpoint::new(Redis::connection(name)?, &self.cache_prefix);
        *self
            .commands
            .write()
            .unwrap_or_else(PoisonError::into_inner) = endpoint;
        Ok(())
    }

    async fn add_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<bool, FrameworkError> {
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let pkey = target.prefixed_key(key);
        let aux = target.key_tags_set(&pkey);
        let mut script = redis::cmd("EVAL");
        script
            .arg(ADD_RAW_LUA)
            .arg(2)
            .arg(&pkey)
            .arg(&aux)
            .arg(value);
        if let Some(duration) = ttl {
            script.arg(redis_ttl_ms(duration));
        }
        let added: i64 = script
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache add error: {e}")))?;
        Ok(added == 1)
    }

    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        let target = self.target()?;
        let key = target.prefixed_key(key);

        // EXISTS, like GET, answers the same way however many times it runs.
        let exists: bool = crate::redis_retry::retry_read("cache EXISTS", || {
            let mut conn = target.conn.clone();
            let key = key.clone();
            async move { conn.exists(&key).await }
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("Cache exists error: {}", e)))?;

        Ok(exists)
    }

    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let pkey = target.prefixed_key(key);

        // Drop the value AND its tag aux set. The forward `tag:{t}` set
        // may still list this key; that's harmless - flush_tags validates
        // membership via the aux set and skips a key whose aux set says
        // "no longer tagged with t" (or no longer exists at all).
        let aux = target.key_tags_set(&pkey);
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
        let target = self.target()?;
        let mut conn = target.conn.clone();

        // SCAN beats KEYS for production: incremental cursor iteration
        // avoids blocking the Redis server on a single O(N) pass. We
        // batch DEL per page so very large keyspaces don't build one
        // giant argument list. The MATCH glob is anchored to the whole
        // prefix, the connection's and then the cache's, escaped so it
        // matches literally, so we never touch other applications' keys
        // or the connection's keys outside the cache. An empty prefix has
        // nothing to anchor to and matches the whole database.
        let pattern = format!("{}*", glob_escape(&target.prefix));
        let mut cursor: u64 = 0;
        loop {
            // SCAN is a pure read; the DEL below is not, and is deliberately
            // left un-retried.
            let (next_cursor, batch): (u64, Vec<String>) =
                crate::redis_retry::retry_read("cache SCAN", || {
                    let mut conn = target.conn.clone();
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
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let key = target.prefixed_key(key);

        let value: i64 = conn
            .incr(&key, amount)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache increment error: {}", e)))?;

        Ok(value)
    }

    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let key = target.prefixed_key(key);

        let value: i64 = conn
            .decr(&key, amount)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache decrement error: {}", e)))?;

        Ok(value)
    }

    async fn increment_if_below(
        &self,
        key: &str,
        amount: i64,
        ceiling: i64,
    ) -> Result<ConditionalIncrement, FrameworkError> {
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let key = target.prefixed_key(key);
        // Not retried: a reply lost after the script ran would count twice.
        let (incremented, value): (i64, String) = redis::cmd("EVAL")
            .arg(INCREMENT_IF_BELOW_LUA)
            .arg(1)
            .arg(&key)
            .arg(amount)
            .arg(ceiling)
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache increment error: {e}")))?;
        // `INCRBY` refuses a value outside i64 below the ceiling; one at or
        // above it comes back unchanged and is refused here.
        let value = value.parse::<i64>().map_err(|_| {
            FrameworkError::internal(
                "Cache increment error: stored value is not a signed 64-bit integer",
            )
        })?;
        Ok(if incremented == 1 {
            ConditionalIncrement::Incremented(value)
        } else {
            ConditionalIncrement::Unchanged(value)
        })
    }

    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let pkey = target.prefixed_key(key);
        let aux = target.key_tags_set(&pkey);

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
        let aux_prefix = target.key_tags_prefix();
        for t in tags {
            let tag_key = target.tag_index_key(t);
            pipe.cmd("SADD").arg(&tag_key).arg(&pkey).ignore();
            // Prune a sample of the same index in the same transaction, so
            // a tag that is written often but rarely flushed stays near its
            // live size. See `TAG_PRUNE_SAMPLE`.
            pipe.cmd("EVAL")
                .arg(PRUNE_TAG_SAMPLE_LUA)
                .arg(1)
                .arg(&tag_key)
                .arg(*t)
                .arg(&aux_prefix)
                .arg(TAG_PRUNE_SAMPLE)
                .ignore();
        }
        pipe.query_async::<()>(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache tagged set: {e}")))?;
        Ok(())
    }

    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let index_prefix = target.tag_index_prefix();
        for t in tags {
            let tag_key = target.tag_index_key(t);
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
                        let mut conn = target.conn.clone();
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
                    script
                        .arg(FLUSH_TAG_BATCH_LUA)
                        .arg(1)
                        .arg(&tag_key)
                        .arg(*t)
                        .arg(&index_prefix);
                    for member in &members {
                        script.arg(member).arg(target.key_tags_set(member));
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
        let target = self.lock_target()?;
        let mut conn = target.conn.clone();
        let pkey = target.locked_key(key);
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
        let target = self.lock_target()?;
        let mut conn = target.conn.clone();
        let pkey = target.locked_key(key);
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
        let target = self.lock_target()?;
        let mut conn = target.conn.clone();
        let pkey = target.locked_key(key);
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
        let target = self.target()?;
        let mut conn = target.conn.clone();
        let pkey = target.prefixed_key(key);
        let aux = target.key_tags_set(&pkey);
        // PEXPIRE returns 1 if the TTL was set, 0 if the key does not
        // exist. PEXPIRE preserves sub-second precision; EXPIRE would
        // truncate a sub-second ttl to 0 and delete the key. The script
        // carries the tag record along; see `TOUCH_LUA`.
        let ok: i64 = redis::Script::new(TOUCH_LUA)
            .key(&pkey)
            .key(&aux)
            .arg(redis_ttl_ms(ttl))
            .invoke_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("Cache touch: {e}")))?;
        Ok(ok == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_keys_cannot_enter_redis_internal_namespaces() {
        let prefix = "t:";
        let lock_key = namespaced_key(prefix, "lock", "job");
        let tag_key = namespaced_key(prefix, "tag", "users");
        let key_tags_key = namespaced_key(prefix, "key_tags", "stored-key");

        assert_eq!(data_key(prefix, "ordinary"), "t:ordinary");
        assert_ne!(data_key(prefix, "\0lock:job"), lock_key);
        assert_ne!(data_key(prefix, "\0tag:users"), tag_key);
        assert_ne!(data_key(prefix, "\0key_tags:stored-key"), key_tags_key);
    }

    #[test]
    fn glob_escape_quotes_every_pattern_character() {
        assert_eq!(glob_escape("plain:"), "plain:");
        assert_eq!(glob_escape("app[1]:*?\\"), "app\\[1\\]:\\*\\?\\\\");
    }

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
