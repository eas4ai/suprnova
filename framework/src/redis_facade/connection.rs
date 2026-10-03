//! One named connection: the commands, the escape hatch, pipelines,
//! subscriptions and blocking commands.

use super::events::{self, RedisCommandExecuted, RedisCommandFailed};
use super::pipeline::{RedisPipeline, with_key};
use super::subscription::{self, RedisSubscription};
use super::value::RedisValue;
use crate::error::FrameworkError;
use crate::redis_retry::retry_read;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use redis::{AsyncConnectionConfig, FromRedisValue};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long opening a connection may take, per attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the shared connection waits for a reply. Blocking commands run
/// on connections of their own, without this limit.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The reads among the commands Laravel's
/// `PhpRedisConnection::RETRYABLE_COMMANDS` lists, which `command` sends
/// again after a lost connection. Laravel's list also holds three writes,
/// `MSET`, `HMSET` and `SET` with no options; they are left out, since the
/// server may have applied a write whose reply was lost, and a second one
/// would undo a write another client made in between.
const RETRYABLE: &[&str] = &[
    "BITCOUNT",
    "BITPOS",
    "DBSIZE",
    "DUMP",
    "EXISTS",
    "GEODIST",
    "GEOHASH",
    "GEOPOS",
    "GEOSEARCH",
    "GET",
    "GETBIT",
    "GETRANGE",
    "HEXISTS",
    "HGET",
    "HGETALL",
    "HKEYS",
    "HLEN",
    "HMGET",
    "HSTRLEN",
    "HVALS",
    "KEYS",
    "LINDEX",
    "LLEN",
    "LPOS",
    "LRANGE",
    "MGET",
    "PING",
    "PTTL",
    "RANDOMKEY",
    "SCARD",
    "SDIFF",
    "SINTER",
    "SISMEMBER",
    "SMEMBERS",
    "SMISMEMBER",
    "SRANDMEMBER",
    "STRLEN",
    "SUNION",
    "TIME",
    "TTL",
    "TYPE",
    "XINFO",
    "XLEN",
    "XPENDING",
    "XRANGE",
    "XREVRANGE",
    "ZCARD",
    "ZCOUNT",
    "ZLEXCOUNT",
    "ZMSCORE",
    "ZRANGE",
    "ZRANK",
    "ZREVRANK",
    "ZSCORE",
];

/// The blocking commands, which `command` sends on a connection of their
/// own. `XREAD` and `XREADGROUP` block too when given `BLOCK`. `WAIT` and
/// `WAITAOF` are not among them: they wait for the writes their own
/// connection made, so they run on the shared one.
const BLOCKING: &[&str] = &[
    "BLPOP",
    "BRPOP",
    "BLMOVE",
    "BRPOPLPUSH",
    "BZPOPMIN",
    "BZPOPMAX",
    "BLMPOP",
    "BZMPOP",
];

/// Whether `command` with `args` blocks the connection it runs on.
fn blocks(command: &str, args: &[&[u8]]) -> bool {
    BLOCKING.contains(&command)
        || (matches!(command, "XREAD" | "XREADGROUP")
            && args.iter().any(|arg| arg.eq_ignore_ascii_case(b"BLOCK")))
}

/// Why `command`, a pipeline and a transaction refuse a command that would
/// change the shared connection for every task that uses it, and what to
/// use instead.
fn refused(command: &str, args: &[&[u8]]) -> Option<&'static str> {
    let subcommand = |name: &[u8]| {
        args.first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case(name))
    };
    match command {
        "CLIENT" if subcommand(b"REPLY") || subcommand(b"TRACKING") => Some(
            "it changes which replies the shared connection gets: open a connection of your own \
             with suprnova::redis::Client",
        ),
        "SUBSCRIBE" | "PSUBSCRIBE" | "SSUBSCRIBE" | "UNSUBSCRIBE" | "PUNSUBSCRIBE"
        | "SUNSUBSCRIBE" => {
            Some("use subscribe or psubscribe, which open a connection of their own")
        }
        "MULTI" | "EXEC" | "DISCARD" => Some("use transaction"),
        "WATCH" | "UNWATCH" | "MONITOR" | "SYNC" | "PSYNC" => Some(
            "it needs a connection no other task shares: open one with suprnova::redis::Client",
        ),
        "SELECT" => Some("name the database in the connection's URL"),
        "AUTH" | "HELLO" | "RESET" | "QUIT" => {
            Some("the connection's URL sets its credentials and protocol")
        }
        _ => None,
    }
}

/// Which end of a list [`blmove`](RedisConnection::blmove) takes from or
/// puts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedisSide {
    /// The head: `LEFT`.
    Left,
    /// The tail: `RIGHT`.
    Right,
}

impl RedisSide {
    fn as_str(self) -> &'static str {
        match self {
            RedisSide::Left => "LEFT",
            RedisSide::Right => "RIGHT",
        }
    }
}

/// A named connection to a Redis server, from
/// [`Redis::connection`](super::Redis::connection). Cloning it is cheap and
/// every clone shares the one connection.
///
/// The connection opens on the first command and opens again after it is
/// lost. Subscriptions and blocking commands each open a connection of
/// their own, so they never hold this one up.
#[derive(Clone)]
pub struct RedisConnection {
    inner: Arc<Inner>,
}

struct Inner {
    name: String,
    client: redis::Client,
    manager: Mutex<Option<Bound>>,
}

/// The connection, and the runtime it was opened on. The connection's task
/// runs on that runtime; once the runtime is gone, as each
/// `#[tokio::test]`'s is when the test ends, the connection is dead, so the
/// next command opens a new one on its own runtime. `runtime_alive` is
/// closed when the task holding its receiver, spawned on that runtime, is
/// dropped with it.
struct Bound {
    manager: ConnectionManager,
    runtime_alive: tokio::sync::oneshot::Sender<()>,
}

impl std::fmt::Debug for RedisConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisConnection")
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

/// Text for an argument, as the events report it.
fn printable(arg: &[u8]) -> String {
    String::from_utf8_lossy(arg).into_owned()
}

/// A duration as Redis's timeout argument: seconds, with a fraction.
fn seconds(timeout: Duration) -> String {
    timeout.as_secs_f64().to_string()
}

impl RedisConnection {
    pub(crate) fn new(name: &str, client: redis::Client) -> Self {
        Self {
            inner: Arc::new(Inner {
                name: name.to_owned(),
                client,
                manager: Mutex::new(None),
            }),
        }
    }

    /// The connection's name.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// The underlying `redis` client's connection, for what the facade does
    /// not cover. It is the connection this one's commands use.
    ///
    /// # Errors
    ///
    /// Outside a Tokio runtime, where the connection cannot run.
    pub fn client(&self) -> Result<ConnectionManager, FrameworkError> {
        let mut slot = self
            .inner
            .manager
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(bound) = slot.as_ref()
            && !bound.runtime_alive.is_closed()
        {
            return Ok(bound.manager.clone());
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(FrameworkError::internal(format!(
                "the Redis connection '{}' is used outside a Tokio runtime",
                self.inner.name
            )));
        }
        let config = ConnectionManagerConfig::new()
            .set_connection_timeout(Some(CONNECT_TIMEOUT))
            .set_response_timeout(Some(RESPONSE_TIMEOUT))
            .set_number_of_retries(3)
            .set_max_delay(Duration::from_millis(500));
        let manager = ConnectionManager::new_lazy_with_config(self.inner.client.clone(), config)
            .map_err(|error| self.error("connect", error))?;
        let (runtime_alive, on_runtime) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = on_runtime.await;
        });
        *slot = Some(Bound {
            manager: manager.clone(),
            runtime_alive,
        });
        Ok(manager)
    }

    fn error(&self, command: &str, error: redis::RedisError) -> FrameworkError {
        FrameworkError::from_external_with(
            format!(
                "the Redis connection '{}' failed {command}: {error}",
                self.inner.name
            ),
            error,
        )
    }

    fn refusal(&self, command: &str, instead: &str) -> FrameworkError {
        FrameworkError::internal(format!(
            "the Redis connection '{}' does not send {command} through command, a pipeline or a \
             transaction: {instead}",
            self.inner.name
        ))
    }

    /// Send `command` with `args`, on a connection of its own when
    /// `blocking`, and on the shared one otherwise, again after a lost
    /// connection when `retry` says it is safe. Nothing is reported yet; the
    /// outer error, outside a Tokio runtime, is from before anything was
    /// sent.
    async fn exchange(
        &self,
        command: &str,
        args: &[&[u8]],
        retry: bool,
        blocking: bool,
    ) -> Result<(redis::RedisResult<redis::Value>, Duration), FrameworkError> {
        let shared = if blocking { None } else { Some(self.client()?) };
        let mut cmd = redis::cmd(command);
        for arg in args {
            cmd.arg(*arg);
        }
        let start = Instant::now();
        let result = match shared {
            // A connection of its own, which waits for as long as the
            // command does.
            None => {
                let config = AsyncConnectionConfig::new()
                    .set_connection_timeout(Some(CONNECT_TIMEOUT))
                    .set_response_timeout(None);
                match self
                    .inner
                    .client
                    .get_multiplexed_async_connection_with_config(&config)
                    .await
                {
                    Ok(mut connection) => cmd.query_async::<redis::Value>(&mut connection).await,
                    Err(error) => Err(error),
                }
            }
            Some(manager) if retry => {
                retry_read(command, || {
                    let mut manager = manager.clone();
                    let cmd = cmd.clone();
                    async move { cmd.query_async::<redis::Value>(&mut manager).await }
                })
                .await
            }
            Some(mut manager) => cmd.query_async::<redis::Value>(&mut manager).await,
        };
        Ok((result, start.elapsed()))
    }

    /// Tell the listeners how `command` went, while events are enabled.
    fn report(&self, command: &str, args: &[&[u8]], duration: Duration, error: Option<String>) {
        if !events::enabled() {
            return;
        }
        let arguments = args.iter().map(|arg| printable(arg)).collect();
        match error {
            None => events::executed(&RedisCommandExecuted {
                connection: self.inner.name.clone(),
                command: command.to_owned(),
                arguments,
                duration,
            }),
            Some(error) => events::failed(&RedisCommandFailed {
                connection: self.inner.name.clone(),
                command: command.to_owned(),
                arguments,
                error,
                duration,
            }),
        }
    }

    /// Send a command, report it, and give its reply as it came.
    async fn run(
        &self,
        command: &str,
        args: &[&[u8]],
        retry: bool,
        blocking: bool,
    ) -> Result<RedisValue, FrameworkError> {
        let (result, duration) = self.exchange(command, args, retry, blocking).await?;
        self.report(
            command,
            args,
            duration,
            result.as_ref().err().map(ToString::to_string),
        );
        result
            .map(Into::into)
            .map_err(|error| self.error(command, error))
    }

    /// Send a command, convert its reply, and report it: a reply that does
    /// not convert is a failure, as the caller sees it.
    async fn typed_on<T: FromRedisValue>(
        &self,
        command: &str,
        args: &[&str],
        retry: bool,
        blocking: bool,
    ) -> Result<T, FrameworkError> {
        let bytes: Vec<&[u8]> = args.iter().map(|arg| arg.as_bytes()).collect();
        let (result, duration) = self.exchange(command, &bytes, retry, blocking).await?;
        let converted = result
            .and_then(|value| redis::from_redis_value::<T>(value).map_err(redis::RedisError::from));
        self.report(
            command,
            &bytes,
            duration,
            converted.as_ref().err().map(ToString::to_string),
        );
        converted.map_err(|error| self.error(command, error))
    }

    async fn typed<T: FromRedisValue>(
        &self,
        command: &str,
        args: &[&str],
        retry: bool,
    ) -> Result<T, FrameworkError> {
        self.typed_on(command, args, retry, false).await
    }

    async fn typed_blocking<T: FromRedisValue>(
        &self,
        command: &str,
        args: &[&str],
    ) -> Result<T, FrameworkError> {
        self.typed_on(command, args, false, true).await
    }

    /// Run any command and return its reply. A read Laravel lists as
    /// retryable (`GET`, `LRANGE`, `HGETALL` and the rest) is sent again
    /// after a lost connection; any other command is not, since it may
    /// already have been applied. A blocking command (`BLPOP`, `XREAD` with
    /// `BLOCK` and the rest) runs on a connection of its own.
    ///
    /// # Errors
    ///
    /// When the server rejects the command or cannot be reached, and, before
    /// anything is sent, for a command that would change the connection
    /// every other command shares: `SUBSCRIBE` and the rest of its family,
    /// `MULTI`, `EXEC`, `DISCARD`, `WATCH`, `UNWATCH`, `MONITOR`, `SELECT`,
    /// `AUTH`, `HELLO`, `RESET`, `QUIT`, and `CLIENT REPLY` and
    /// `CLIENT TRACKING`. The error names what to use instead.
    pub async fn command<A: AsRef<[u8]>>(
        &self,
        name: &str,
        args: &[A],
    ) -> Result<RedisValue, FrameworkError> {
        let command = name.to_ascii_uppercase();
        let bytes: Vec<&[u8]> = args.iter().map(AsRef::as_ref).collect();
        if let Some(instead) = refused(&command, &bytes) {
            return Err(self.refusal(&command, instead));
        }
        let blocking = blocks(&command, &bytes);
        let retry = !blocking && RETRYABLE.contains(&command.as_str());
        self.run(&command, &bytes, retry, blocking).await
    }

    /// `GET`: the value, or `None` when the key does not exist.
    ///
    /// # Errors
    ///
    /// When the server rejects the command or cannot be reached, here and
    /// for every command below.
    pub async fn get(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        self.typed("GET", &[key], true).await
    }

    /// `SET`. A write, so it is not sent again after a lost connection.
    pub async fn set(&self, key: &str, value: &str) -> Result<(), FrameworkError> {
        self.typed("SET", &[key, value], false).await
    }

    /// `SETEX`: set the value to expire after `seconds`.
    pub async fn set_ex(&self, key: &str, value: &str, seconds: u64) -> Result<(), FrameworkError> {
        self.typed("SETEX", &[key, &seconds.to_string(), value], false)
            .await
    }

    /// `DEL`: how many of the keys existed.
    pub async fn del(&self, keys: &[&str]) -> Result<u64, FrameworkError> {
        self.typed("DEL", keys, false).await
    }

    /// `EXISTS`: whether the key exists.
    pub async fn exists(&self, key: &str) -> Result<bool, FrameworkError> {
        self.typed("EXISTS", &[key], true).await
    }

    /// `INCRBY`: the value after adding `by`.
    pub async fn incr(&self, key: &str, by: i64) -> Result<i64, FrameworkError> {
        self.typed("INCRBY", &[key, &by.to_string()], false).await
    }

    /// `DECRBY`: the value after taking away `by`.
    pub async fn decr(&self, key: &str, by: i64) -> Result<i64, FrameworkError> {
        self.typed("DECRBY", &[key, &by.to_string()], false).await
    }

    /// `EXPIRE`: whether the key exists and now expires after `seconds`.
    pub async fn expire(&self, key: &str, seconds: i64) -> Result<bool, FrameworkError> {
        self.typed("EXPIRE", &[key, &seconds.to_string()], false)
            .await
    }

    /// `TTL`: the seconds left; -1 for a key with no expiry, -2 for none.
    pub async fn ttl(&self, key: &str) -> Result<i64, FrameworkError> {
        self.typed("TTL", &[key], true).await
    }

    /// `MGET`: each key's value, `None` where it does not exist.
    pub async fn mget(&self, keys: &[&str]) -> Result<Vec<Option<String>>, FrameworkError> {
        self.typed("MGET", keys, true).await
    }

    /// `HSET`: 1 when the field is new, 0 when it was replaced.
    pub async fn hset(&self, key: &str, field: &str, value: &str) -> Result<u64, FrameworkError> {
        self.typed("HSET", &[key, field, value], false).await
    }

    /// `HGET`: the field's value.
    pub async fn hget(&self, key: &str, field: &str) -> Result<Option<String>, FrameworkError> {
        self.typed("HGET", &[key, field], true).await
    }

    /// `HGETALL`: every field and value.
    pub async fn hgetall(&self, key: &str) -> Result<BTreeMap<String, String>, FrameworkError> {
        self.typed("HGETALL", &[key], true).await
    }

    /// `HDEL`: how many of the fields existed.
    pub async fn hdel(&self, key: &str, fields: &[&str]) -> Result<u64, FrameworkError> {
        self.typed("HDEL", &with_key(key, fields), false).await
    }

    /// `LPUSH`: the list's length after.
    pub async fn lpush(&self, key: &str, values: &[&str]) -> Result<u64, FrameworkError> {
        self.typed("LPUSH", &with_key(key, values), false).await
    }

    /// `RPUSH`: the list's length after.
    pub async fn rpush(&self, key: &str, values: &[&str]) -> Result<u64, FrameworkError> {
        self.typed("RPUSH", &with_key(key, values), false).await
    }

    /// `LPOP`: the head, or `None` for an empty list.
    pub async fn lpop(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        self.typed("LPOP", &[key], false).await
    }

    /// `RPOP`: the tail, or `None` for an empty list.
    pub async fn rpop(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        self.typed("RPOP", &[key], false).await
    }

    /// `LRANGE`: the elements from `start` to `stop`, both included;
    /// negative indexes count from the tail.
    pub async fn lrange(
        &self,
        key: &str,
        start: i64,
        stop: i64,
    ) -> Result<Vec<String>, FrameworkError> {
        self.typed(
            "LRANGE",
            &[key, &start.to_string(), &stop.to_string()],
            true,
        )
        .await
    }

    /// `SADD`: how many members were new.
    pub async fn sadd(&self, key: &str, members: &[&str]) -> Result<u64, FrameworkError> {
        self.typed("SADD", &with_key(key, members), false).await
    }

    /// `SREM`: how many members were there.
    pub async fn srem(&self, key: &str, members: &[&str]) -> Result<u64, FrameworkError> {
        self.typed("SREM", &with_key(key, members), false).await
    }

    /// `SMEMBERS`: the set.
    pub async fn smembers(&self, key: &str) -> Result<BTreeSet<String>, FrameworkError> {
        self.typed("SMEMBERS", &[key], true).await
    }

    /// `ZADD`: 1 when the member is new, 0 when its score was updated.
    pub async fn zadd(&self, key: &str, member: &str, score: f64) -> Result<u64, FrameworkError> {
        self.typed("ZADD", &[key, &score.to_string(), member], false)
            .await
    }

    /// `ZRANGE`: the members from rank `start` to `stop`, lowest score
    /// first.
    pub async fn zrange(
        &self,
        key: &str,
        start: i64,
        stop: i64,
    ) -> Result<Vec<String>, FrameworkError> {
        self.typed(
            "ZRANGE",
            &[key, &start.to_string(), &stop.to_string()],
            true,
        )
        .await
    }

    /// `ZRANGEBYSCORE`: the members scored from `min` to `max`, both
    /// included. `f64::INFINITY` and `f64::NEG_INFINITY` leave a side open.
    pub async fn zrangebyscore(
        &self,
        key: &str,
        min: f64,
        max: f64,
    ) -> Result<Vec<String>, FrameworkError> {
        self.typed(
            "ZRANGEBYSCORE",
            &[key, &min.to_string(), &max.to_string()],
            true,
        )
        .await
    }

    /// `PUBLISH`: how many subscribers received the message.
    pub async fn publish(&self, channel: &str, message: &str) -> Result<u64, FrameworkError> {
        self.typed("PUBLISH", &[channel, message], false).await
    }

    /// `EVAL`: run a Lua script with `keys` as `KEYS` and `args` as `ARGV`.
    pub async fn eval(
        &self,
        script: &str,
        keys: &[&str],
        args: &[&str],
    ) -> Result<RedisValue, FrameworkError> {
        let count = keys.len().to_string();
        let mut all = vec![script, count.as_str()];
        all.extend_from_slice(keys);
        all.extend_from_slice(args);
        let bytes: Vec<&[u8]> = all.iter().map(|arg| arg.as_bytes()).collect();
        self.run("EVAL", &bytes, false, false).await
    }

    /// Every key matching `pattern`, by `SCAN` from the first cursor to the
    /// last. Unlike `KEYS`, the server is never blocked for the whole walk.
    pub async fn scan(&self, pattern: &str) -> Result<Vec<String>, FrameworkError> {
        let mut keys = Vec::new();
        let mut cursor = "0".to_owned();
        loop {
            let (next, batch): (String, Vec<String>) = self
                .typed(
                    "SCAN",
                    &[cursor.as_str(), "MATCH", pattern, "COUNT", "100"],
                    true,
                )
                .await?;
            keys.extend(batch);
            if next == "0" {
                return Ok(keys);
            }
            cursor = next;
        }
    }

    /// Send every command `queue` adds before reading a reply, and return
    /// the replies in order. A pipeline is not atomic: other clients'
    /// commands may run between its own; use
    /// [`transaction`](Self::transaction) for that. Its commands are not
    /// reported to the command listeners.
    ///
    /// # Errors
    ///
    /// When any command fails, or the server cannot be reached.
    pub async fn pipeline(
        &self,
        queue: impl FnOnce(&mut RedisPipeline),
    ) -> Result<Vec<RedisValue>, FrameworkError> {
        self.run_pipeline(queue, false, "a pipeline").await
    }

    /// As [`pipeline`](Self::pipeline), inside `MULTI` and `EXEC`: the
    /// commands run together with nothing between them, and when Redis
    /// rejects one as it is queued, none runs.
    ///
    /// # Errors
    ///
    /// When Redis aborts the transaction, a command fails, or the server
    /// cannot be reached.
    pub async fn transaction(
        &self,
        queue: impl FnOnce(&mut RedisPipeline),
    ) -> Result<Vec<RedisValue>, FrameworkError> {
        self.run_pipeline(queue, true, "a transaction").await
    }

    async fn run_pipeline(
        &self,
        queue: impl FnOnce(&mut RedisPipeline),
        atomic: bool,
        what: &str,
    ) -> Result<Vec<RedisValue>, FrameworkError> {
        let mut queued = RedisPipeline::default();
        queue(&mut queued);
        if queued.commands.is_empty() {
            return Ok(Vec::new());
        }
        // Checked before anything is sent, as command checks them: these
        // would change the connection every task shares. A blocking command
        // would hold it up, except inside MULTI, where Redis never blocks.
        for (name, args) in &queued.commands {
            let args: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
            if let Some(instead) = refused(name, &args) {
                return Err(self.refusal(name, instead));
            }
            if !atomic && blocks(name, &args) {
                return Err(self.refusal(
                    name,
                    "a blocking command would hold up the shared connection: run it with \
                     command, or inside a transaction, where it does not block",
                ));
            }
        }
        let mut pipe = redis::pipe();
        if atomic {
            pipe.atomic();
        }
        for (name, args) in &queued.commands {
            let cmd = pipe.cmd(name);
            for arg in args {
                cmd.arg(arg.as_slice());
            }
        }
        let mut manager = self.client()?;
        let replies: Vec<redis::Value> = pipe
            .query_async(&mut manager)
            .await
            .map_err(|error| self.error(what, error))?;
        Ok(replies.into_iter().map(Into::into).collect())
    }

    /// Subscribe to `channels` on a connection of its own.
    ///
    /// # Errors
    ///
    /// When the server cannot be reached or refuses the subscription.
    pub async fn subscribe(&self, channels: &[&str]) -> Result<RedisSubscription, FrameworkError> {
        self.open_subscription(channels, false).await
    }

    /// Subscribe to the channels matching `patterns`, such as `orders.*`,
    /// on a connection of its own.
    ///
    /// # Errors
    ///
    /// When the server cannot be reached or refuses the subscription.
    pub async fn psubscribe(&self, patterns: &[&str]) -> Result<RedisSubscription, FrameworkError> {
        self.open_subscription(patterns, true).await
    }

    async fn open_subscription(
        &self,
        names: &[&str],
        patterns: bool,
    ) -> Result<RedisSubscription, FrameworkError> {
        let what = if patterns { "PSUBSCRIBE" } else { "SUBSCRIBE" };
        let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        let start = Instant::now();
        let opened = subscription::open(&self.inner.client, &names, patterns).await;
        let bytes: Vec<&[u8]> = names.iter().map(String::as_bytes).collect();
        self.report(
            what,
            &bytes,
            start.elapsed(),
            opened.as_ref().err().map(ToString::to_string),
        );
        let stream = opened.map_err(|error| self.error(what, error))?;
        Ok(RedisSubscription {
            stream,
            client: self.inner.client.clone(),
            names,
            patterns,
        })
    }

    /// `BLPOP`: wait up to `timeout` for the head of the first non-empty
    /// list, as the list's key and the element; `None` when the time runs
    /// out. A zero timeout waits for ever. Runs on a connection of its own.
    pub async fn blpop(
        &self,
        keys: &[&str],
        timeout: Duration,
    ) -> Result<Option<(String, String)>, FrameworkError> {
        let mut args = keys.to_vec();
        let timeout = seconds(timeout);
        args.push(&timeout);
        self.typed_blocking("BLPOP", &args).await
    }

    /// `BRPOP`: as [`blpop`](Self::blpop), from the tail.
    pub async fn brpop(
        &self,
        keys: &[&str],
        timeout: Duration,
    ) -> Result<Option<(String, String)>, FrameworkError> {
        let mut args = keys.to_vec();
        let timeout = seconds(timeout);
        args.push(&timeout);
        self.typed_blocking("BRPOP", &args).await
    }

    /// `BLMOVE`: wait up to `timeout` to move an element from one end of
    /// `source` to one end of `destination`, and return it. Runs on a
    /// connection of its own.
    pub async fn blmove(
        &self,
        source: &str,
        destination: &str,
        from: RedisSide,
        to: RedisSide,
        timeout: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        self.typed_blocking(
            "BLMOVE",
            &[
                source,
                destination,
                from.as_str(),
                to.as_str(),
                &seconds(timeout),
            ],
        )
        .await
    }

    /// `BRPOPLPUSH`: wait up to `timeout` to move the tail of `source` to
    /// the head of `destination`, and return it. Runs on a connection of
    /// its own.
    pub async fn brpoplpush(
        &self,
        source: &str,
        destination: &str,
        timeout: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        self.typed_blocking("BRPOPLPUSH", &[source, destination, &seconds(timeout)])
            .await
    }

    /// `BZPOPMIN`: wait up to `timeout` for the lowest-scored member of the
    /// first non-empty sorted set, as its key, the member and its score.
    /// Runs on a connection of its own.
    pub async fn bzpopmin(
        &self,
        keys: &[&str],
        timeout: Duration,
    ) -> Result<Option<(String, String, f64)>, FrameworkError> {
        let mut args = keys.to_vec();
        let timeout = seconds(timeout);
        args.push(&timeout);
        self.typed_blocking("BZPOPMIN", &args).await
    }

    /// `BZPOPMAX`: as [`bzpopmin`](Self::bzpopmin), the highest-scored.
    pub async fn bzpopmax(
        &self,
        keys: &[&str],
        timeout: Duration,
    ) -> Result<Option<(String, String, f64)>, FrameworkError> {
        let mut args = keys.to_vec();
        let timeout = seconds(timeout);
        args.push(&timeout);
        self.typed_blocking("BZPOPMAX", &args).await
    }
}
