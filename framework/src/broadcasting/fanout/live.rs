//! Redis-backed cursor and coalescing state for Inertia Live signals.

use crate::FrameworkError;
use crate::broadcasting::live::{LIVE_TOPIC_EVENT, live_topic_channel};
use redis::Script;
use redis::aio::ConnectionManager;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;
use uuid::Uuid;

const LIVE_KEY_SEGMENT: &str = "__live";
const LIVE_FLUSH_POLL: Duration = Duration::from_millis(100);
const LIVE_FLUSH_ERROR_BACKOFF: Duration = Duration::from_secs(1);
/// The browser's `RegisteredTopicChange` envelope bound.
const MAX_REGISTERED_TOPIC_CHANGE_PROPS: usize = 256;

/// Atomically accepts a signal, advancing its cursor and either writing an
/// immediate stream entry or merging the signal into the pending union.
const ACCEPT_SCRIPT: &str = r#"
local function encode_signal(envelope, empty_props)
    local encoded = cjson.encode(envelope)
    if empty_props then
        encoded = string.gsub(encoded, '"props":{}', '"props":[]', 1)
    end
    return encoded
end

local meta_type = redis.call('TYPE', KEYS[1]).ok
local props_type = redis.call('TYPE', KEYS[2]).ok
local schedule_type = redis.call('TYPE', KEYS[3]).ok
local stream_type = redis.call('TYPE', KEYS[4]).ok
if meta_type ~= 'none' and meta_type ~= 'hash' then
    return redis.error_reply('live topic metadata key has the wrong type')
end
if props_type ~= 'none' and props_type ~= 'set' then
    return redis.error_reply('live topic props key has the wrong type')
end
if schedule_type ~= 'none' and schedule_type ~= 'zset' then
    return redis.error_reply('live topic schedule key has the wrong type')
end
if stream_type ~= 'none' and stream_type ~= 'stream' then
    return redis.error_reply('broadcast stream key has the wrong type')
end

local time = redis.call('TIME')
local now = (tonumber(time[1]) * 1000) + math.floor(tonumber(time[2]) / 1000)
local cursor = redis.call('HINCRBY', KEYS[1], 'cursor', 1)
local previous_next_at = redis.call('HGET', KEYS[1], 'next_at')
local next_at = tonumber(previous_next_at or '0')
local pending = redis.call('HGET', KEYS[1], 'pending') == '1'
local interval = tonumber(ARGV[1])
local props = cjson.decode(ARGV[8])
local max_props = tonumber(ARGV[10])

if not pending and now >= next_at then
    local envelope = {
        instance_id = ARGV[2],
        channel = ARGV[3],
        event = ARGV[4],
        data = { topic = ARGV[5], cursor = cursor, props = props }
    }
    if ARGV[6] == '1' then
        envelope.except = ARGV[7]
    end
    redis.call('HSET', KEYS[1], 'next_at', now + interval)
    local written = redis.pcall('XADD', KEYS[4], '*', ARGV[9], encode_signal(envelope, #props == 0))
    if type(written) == 'table' and written.err then
        redis.call('HINCRBY', KEYS[1], 'cursor', -1)
        if previous_next_at then
            redis.call('HSET', KEYS[1], 'next_at', previous_next_at)
        else
            redis.call('HDEL', KEYS[1], 'next_at')
        end
        return redis.error_reply(written.err)
    end
    return {0, cursor, 0}
end

local first = not pending
redis.call('HSET', KEYS[1], 'pending', 1, 'topic', ARGV[5], 'interval', interval)
if #props == 0 then
    redis.call('HSET', KEYS[1], 'pending_all', 1)
    redis.call('DEL', KEYS[2])
elseif redis.call('HGET', KEYS[1], 'pending_all') ~= '1' then
    for _, prop in ipairs(props) do
        redis.call('SADD', KEYS[2], prop)
    end
    if redis.call('SCARD', KEYS[2]) > max_props then
        redis.call('HSET', KEYS[1], 'pending_all', 1)
        redis.call('DEL', KEYS[2])
    end
end

local incoming_mode = 'none'
if ARGV[6] == '1' then incoming_mode = 'some' end
if first then
    redis.call('HSET', KEYS[1], 'except_mode', incoming_mode, 'except_value', ARGV[7])
else
    local mode = redis.call('HGET', KEYS[1], 'except_mode') or 'mixed'
    local value = redis.call('HGET', KEYS[1], 'except_value') or ''
    if mode ~= incoming_mode or (mode == 'some' and value ~= ARGV[7]) then
        redis.call('HSET', KEYS[1], 'except_mode', 'mixed', 'except_value', '')
    end
end

if next_at == 0 then next_at = now + interval end
redis.call('ZADD', KEYS[3], next_at, KEYS[1])
return {1, cursor, next_at}
"#;

/// Atomically turns a due pending union into one Redis stream entry.
const FLUSH_SCRIPT: &str = r#"
local function encode_signal(envelope, empty_props)
    local encoded = cjson.encode(envelope)
    if empty_props then
        encoded = string.gsub(encoded, '"props":{}', '"props":[]', 1)
    end
    return encoded
end

local time = redis.call('TIME')
local now = (tonumber(time[1]) * 1000) + math.floor(tonumber(time[2]) / 1000)
local next_at = tonumber(redis.call('HGET', KEYS[1], 'next_at') or '0')
if redis.call('HGET', KEYS[1], 'pending') ~= '1' or next_at > now then
    if redis.call('HGET', KEYS[1], 'pending') ~= '1' then
        redis.call('ZREM', KEYS[3], KEYS[1])
    end
    return 0
end

local topic = redis.call('HGET', KEYS[1], 'topic')
local cursor = tonumber(redis.call('HGET', KEYS[1], 'cursor'))
local interval = tonumber(redis.call('HGET', KEYS[1], 'interval'))
local props
if redis.call('HGET', KEYS[1], 'pending_all') == '1' then
    props = cjson.decode('[]')
else
    props = redis.call('SMEMBERS', KEYS[2])
    table.sort(props)
end
local envelope = {
    instance_id = ARGV[1],
    channel = ARGV[2] .. topic,
    event = ARGV[3],
    data = { topic = topic, cursor = cursor, props = props }
}
if redis.call('HGET', KEYS[1], 'except_mode') == 'some' then
    envelope.except = redis.call('HGET', KEYS[1], 'except_value')
end

redis.call('XADD', KEYS[4], '*', ARGV[4], encode_signal(envelope, #props == 0))
redis.call('HDEL', KEYS[1], 'pending', 'pending_all', 'topic', 'interval', 'except_mode', 'except_value')
redis.call('DEL', KEYS[2])
redis.call('ZREM', KEYS[3], KEYS[1])
redis.call('HSET', KEYS[1], 'next_at', now + interval)
return 1
"#;

/// Shared Redis publication state for one stream-backed hub.
#[derive(Clone)]
pub(super) struct RedisLiveTopics {
    connection: ConnectionManager,
    stream: String,
    schedule: String,
}

impl RedisLiveTopics {
    /// Create publication state on an existing Redis connection manager.
    pub(super) fn new(connection: ConnectionManager, stream: &str) -> Self {
        Self {
            connection,
            stream: stream.to_owned(),
            schedule: format!("{stream}:{LIVE_KEY_SEGMENT}:schedule"),
        }
    }

    /// Accept and persist one publication atomically.
    pub(super) async fn publish(
        &self,
        instance_id: Uuid,
        topic: &str,
        props: &[String],
        except: Option<String>,
        interval: Duration,
    ) -> Result<u64, FrameworkError> {
        let (meta, prop_set) = self.topic_keys(topic);
        let interval_ms = duration_millis(interval);
        let mut props = props.to_vec();
        props.sort();
        props.dedup();
        if props.len() > MAX_REGISTERED_TOPIC_CHANGE_PROPS {
            props.clear();
        }
        let props_json = serde_json::to_string(&props).map_err(|error| {
            FrameworkError::internal(format!(
                "SeaStreamerBroadcastHub: failed to serialize live signal props: {error}"
            ))
        })?;
        let result: (u8, u64, u64) = Script::new(ACCEPT_SCRIPT)
            .key(meta)
            .key(prop_set)
            .key(&self.schedule)
            .key(&self.stream)
            .arg(interval_ms)
            .arg(instance_id.to_string())
            .arg(live_topic_channel(topic))
            .arg(LIVE_TOPIC_EVENT)
            .arg(topic)
            .arg(if except.is_some() { "1" } else { "0" })
            .arg(except.unwrap_or_default())
            .arg(props_json)
            .arg(super::sea_streamer::MESSAGE_FIELD)
            .arg(MAX_REGISTERED_TOPIC_CHANGE_PROPS)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: live topic publish failed: {error}"
                ))
            })?;
        Ok(result.1)
    }

    /// Read the shared cursor for `topic`.
    pub(super) async fn cursor(&self, topic: &str) -> Result<u64, FrameworkError> {
        let (meta, _) = self.topic_keys(topic);
        redis::cmd("HGET")
            .arg(meta)
            .arg("cursor")
            .query_async::<Option<u64>>(&mut self.connection.clone())
            .await
            .map(|cursor| cursor.unwrap_or(0))
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: live topic cursor read failed: {error}"
                ))
            })
    }

    /// A single Redis command captures all requested family and static baselines.
    pub(super) async fn cursors(&self, topics: &[String]) -> Result<Vec<u64>, FrameworkError> {
        if topics.is_empty() {
            return Ok(Vec::new());
        }
        let script = Script::new(
            "local values = {} for i, key in ipairs(KEYS) do values[i] = tonumber(redis.call('HGET', key, 'cursor') or '0') end return values",
        );
        let mut invocation = script.prepare_invoke();
        for topic in topics {
            invocation.key(self.topic_keys(topic).0);
        }
        invocation
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: live topic baseline read failed: {error}"
                ))
            })
    }

    /// Start a worker that atomically flushes pending unions into the stream.
    pub(super) fn spawn_flush_worker(&self, instance_id: Uuid) -> JoinHandle<()> {
        let backend = self.clone();
        tokio::spawn(async move { backend.flush_worker(instance_id).await })
    }

    async fn flush_worker(self, instance_id: Uuid) {
        loop {
            match self.next_pending().await {
                Ok(Some((meta, due))) => {
                    let now = unix_millis();
                    if due > now {
                        tokio::time::sleep(Duration::from_millis(due - now).min(LIVE_FLUSH_POLL))
                            .await;
                        continue;
                    }
                    if let Err(error) = self.flush(&meta, instance_id).await {
                        tracing::error!(
                            error = %error,
                            "SeaStreamerBroadcastHub: live topic flush failed; retrying"
                        );
                        tokio::time::sleep(LIVE_FLUSH_ERROR_BACKOFF).await;
                    }
                }
                Ok(None) => tokio::time::sleep(LIVE_FLUSH_POLL).await,
                Err(error) => {
                    tracing::error!(
                        error = %error,
                        "SeaStreamerBroadcastHub: live topic schedule read failed; retrying"
                    );
                    tokio::time::sleep(LIVE_FLUSH_ERROR_BACKOFF).await;
                }
            }
        }
    }

    async fn next_pending(&self) -> Result<Option<(String, u64)>, FrameworkError> {
        let entries: Vec<(String, u64)> = redis::cmd("ZRANGE")
            .arg(&self.schedule)
            .arg(0)
            .arg(0)
            .arg("WITHSCORES")
            .query_async(&mut self.connection.clone())
            .await
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: live topic schedule read failed: {error}"
                ))
            })?;
        Ok(entries.into_iter().next())
    }

    async fn flush(&self, meta: &str, instance_id: Uuid) -> Result<(), FrameworkError> {
        let prop_set = format!("{meta}:props");
        Script::new(FLUSH_SCRIPT)
            .key(meta)
            .key(prop_set)
            .key(&self.schedule)
            .key(&self.stream)
            .arg(instance_id.to_string())
            .arg(crate::broadcasting::live::LIVE_TOPIC_CHANNEL_PREFIX)
            .arg(LIVE_TOPIC_EVENT)
            .arg(super::sea_streamer::MESSAGE_FIELD)
            .invoke_async::<u8>(&mut self.connection.clone())
            .await
            .map(|_| ())
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: live topic flush failed: {error}"
                ))
            })
    }

    fn topic_keys(&self, topic: &str) -> (String, String) {
        let identity = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{}\0{topic}", self.stream).as_bytes(),
        );
        let meta = format!("{}:{LIVE_KEY_SEGMENT}:topic:{identity}", self.stream);
        let prop_set = format!("{meta}:props");
        (meta, prop_set)
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().clamp(1, u64::MAX as u128) as u64
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
