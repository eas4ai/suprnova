//! Redis-backed cursor and coalescing state for Inertia Live signals.

use crate::FrameworkError;
use crate::broadcasting::live::{LIVE_TOPIC_EVENT, LiveTopicRetention, live_topic_channel};
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
    if empty_props then encoded = string.gsub(encoded, '"props":{}', '"props":[]', 1) end
    return encoded
end
for i, expected in ipairs({'hash', 'set', 'zset', 'stream', 'hash'}) do
    local actual = redis.call('TYPE', KEYS[i]).ok
    if actual ~= 'none' and actual ~= expected then
        return redis.error_reply('live topic key has the wrong type')
    end
end
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local interval = tonumber(ARGV[1])
local retention = tonumber(ARGV[11])
local props = cjson.decode(ARGV[8])
local max_props = tonumber(ARGV[10])
local previous_member = redis.call('HGET', KEYS[1], 'pending_member')
local pending_hash = redis.call('EXISTS', KEYS[5]) == 1
local pending = pending_hash or previous_member ~= false
if redis.call('HEXISTS', KEYS[1], 'generation') == 0 then
    local generation = redis.call('HGET', KEYS[5], 'generation')
    if not generation then
        if retention > 0 then generation = now else generation = ARGV[12] end
    end
    redis.call('HSET', KEYS[1], 'generation', generation, 'cursor', redis.call('HGET', KEYS[5], 'cursor') or 0)
end
local generation = tonumber(redis.call('HGET', KEYS[1], 'generation'))
local cursor = redis.call('HINCRBY', KEYS[1], 'cursor', 1)
local previous_next_at = redis.call('HGET', KEYS[1], 'next_at')
local next_at = tonumber(previous_next_at or redis.call('HGET', KEYS[5], 'next_at') or '0')
local function idle()
    if retention > 0 then redis.call('PEXPIRE', KEYS[1], retention) end
end
redis.call('HSET', KEYS[1], 'retention_ms', retention)
if not pending and now >= next_at then
    local envelope = {
        instance_id = ARGV[2], channel = ARGV[3], event = ARGV[4],
        data = { topic = ARGV[5], generation = generation, cursor = cursor, props = props }
    }
    if ARGV[6] == '1' then
        local exclusion = cjson.decode(ARGV[7])
        envelope.except = exclusion.socket
        envelope.data.exclusion = exclusion
    end
    redis.call('HSET', KEYS[1], 'next_at', now + interval)
    local written = redis.pcall('XADD', KEYS[4], '*', ARGV[9], encode_signal(envelope, #props == 0))
    if type(written) == 'table' and written.err then
        redis.call('HINCRBY', KEYS[1], 'cursor', -1)
        if previous_next_at then redis.call('HSET', KEYS[1], 'next_at', previous_next_at)
        else redis.call('HDEL', KEYS[1], 'next_at') end
        idle()
        return redis.error_reply(written.err)
    end
    idle()
    return {0, cursor, 0}
end
local first = not pending
if next_at == 0 then next_at = now + interval end
redis.call('HSET', KEYS[5], 'generation', generation, 'cursor', cursor, 'next_at', next_at)
if (pending and not pending_hash) or #props == 0 then
    redis.call('HSET', KEYS[5], 'pending_all', 1)
    redis.call('DEL', KEYS[2])
elseif redis.call('HGET', KEYS[5], 'pending_all') ~= '1' then
    for _, prop in ipairs(props) do redis.call('SADD', KEYS[2], prop) end
    if redis.call('SCARD', KEYS[2]) > max_props then
        redis.call('HSET', KEYS[5], 'pending_all', 1)
        redis.call('DEL', KEYS[2])
    end
end
local incoming_mode = 'none'
if ARGV[6] == '1' then incoming_mode = 'some' end
if first then
    redis.call('HSET', KEYS[5], 'except_mode', incoming_mode, 'except_value', ARGV[7])
else
    local mode = redis.call('HGET', KEYS[5], 'except_mode') or 'mixed'
    local value = redis.call('HGET', KEYS[5], 'except_value') or ''
    if mode ~= incoming_mode or (mode == 'some' and value ~= ARGV[7]) then
        redis.call('HSET', KEYS[5], 'except_mode', 'mixed', 'except_value', '')
    end
end
local member = cjson.encode({topic = ARGV[5], interval_ms = interval, retention_ms = retention})
if previous_member and previous_member ~= member then redis.call('ZREM', KEYS[3], previous_member) end
redis.call('ZADD', KEYS[3], next_at, member)
redis.call('HSET', KEYS[1], 'pending_member', member)
redis.call('PERSIST', KEYS[1])
redis.call('PERSIST', KEYS[2])
redis.call('PERSIST', KEYS[5])
return {1, cursor, next_at}
"#;

/// Pending work is removed only after XADD succeeds. The schedule retains routing
/// metadata so a missing pending hash still produces a full recovery signal.
const FLUSH_SCRIPT: &str = r#"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local due = redis.call('ZSCORE', KEYS[3], ARGV[5])
if not due or tonumber(due) > now then return 0 end
local pending = redis.call('EXISTS', KEYS[5]) == 1
local generation = redis.call('HGET', KEYS[1], 'generation') or redis.call('HGET', KEYS[5], 'generation')
local cursor = redis.call('HGET', KEYS[1], 'cursor') or redis.call('HGET', KEYS[5], 'cursor') or 0
local retention = tonumber(ARGV[7])
if not generation then
    if retention > 0 then generation = now else generation = ARGV[9] end
end
redis.call('HSET', KEYS[1], 'generation', generation, 'cursor', cursor, 'retention_ms', retention)
redis.call('PERSIST', KEYS[1])
local props = {}
if pending and redis.call('HGET', KEYS[5], 'pending_all') ~= '1' then
    props = redis.call('SMEMBERS', KEYS[2])
    table.sort(props)
end
local envelope = {
    instance_id = ARGV[1], channel = ARGV[2] .. ARGV[6], event = ARGV[3],
    data = { topic = ARGV[6], generation = tonumber(generation), cursor = tonumber(cursor), props = props }
}
if pending and redis.call('HGET', KEYS[5], 'except_mode') == 'some' then
    local exclusion = cjson.decode(redis.call('HGET', KEYS[5], 'except_value'))
    envelope.except = exclusion.socket
    envelope.data.exclusion = exclusion
end
local encoded = cjson.encode(envelope)
if #props == 0 then encoded = string.gsub(encoded, '"props":{}', '"props":[]', 1) end
redis.call('XADD', KEYS[4], '*', ARGV[4], encoded)
redis.call('DEL', KEYS[2], KEYS[5])
redis.call('ZREM', KEYS[3], ARGV[5])
redis.call('HDEL', KEYS[1], 'pending_member')
redis.call('HSET', KEYS[1], 'next_at', now + tonumber(ARGV[8]))
if retention > 0 then redis.call('PEXPIRE', KEYS[1], retention) end
return 1
"#;

#[derive(serde::Deserialize)]
struct TopicSchedule {
    topic: String,
    interval_ms: u64,
    retention_ms: u64,
}

/// Shared Redis publication state for one stream-backed hub.
#[derive(Clone)]
pub(super) struct RedisLiveTopics {
    connection: ConnectionManager,
    stream: String,
    schedule: String,
    retention: LiveTopicRetention,
    generation: u64,
}

impl RedisLiveTopics {
    /// Create publication state on an existing Redis connection manager.
    pub(super) async fn new(
        connection: ConnectionManager,
        stream: &str,
    ) -> Result<Self, FrameworkError> {
        let generation: u64 = Script::new("local value = redis.call('GET', KEYS[1]); if not value then local t = redis.call('TIME'); value = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000); redis.call('SET', KEYS[1], value) end return tonumber(value)")
            .key(format!("{stream}:{LIVE_KEY_SEGMENT}:generation"))
            .invoke_async(&mut connection.clone()).await.map_err(|error| FrameworkError::internal(format!("SeaStreamerBroadcastHub: topic generation initialization failed: {error}")))?;
        Ok(Self {
            connection,
            stream: stream.to_owned(),
            schedule: format!("{stream}:{LIVE_KEY_SEGMENT}:schedule"),
            retention: LiveTopicRetention::default(),
            generation,
        })
    }

    /// Accept and persist one publication atomically.
    pub(super) async fn publish(
        &self,
        instance_id: Uuid,
        topic: &str,
        props: &[String],
        except: Option<crate::broadcasting::LiveTopicExclusion>,
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
        let exclusion_json = except
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: failed to serialize Live sender: {error}"
                ))
            })?
            .unwrap_or_default();
        let result: (u8, u64, u64) = Script::new(ACCEPT_SCRIPT)
            .key(&meta)
            .key(prop_set)
            .key(&self.schedule)
            .key(&self.stream)
            .key(format!("{meta}:pending"))
            .arg(interval_ms)
            .arg(instance_id.to_string())
            .arg(live_topic_channel(topic))
            .arg(LIVE_TOPIC_EVENT)
            .arg(topic)
            .arg(if except.is_some() { "1" } else { "0" })
            .arg(exclusion_json)
            .arg(props_json)
            .arg(super::sea_streamer::MESSAGE_FIELD)
            .arg(MAX_REGISTERED_TOPIC_CHANGE_PROPS)
            .arg(self.retention.window(topic).map_or(0, duration_millis))
            .arg(self.generation)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: live topic publish failed: {error}"
                ))
            })?;
        Ok(result.1)
    }

    pub(super) fn configure_family(
        &self,
        family: &str,
        retention: Duration,
    ) -> Result<(), FrameworkError> {
        self.retention.configure(family, retention)
    }

    /// Baseline reads renew the instance TTL through the same bulk command.
    pub(super) async fn cursor(&self, topic: &str) -> Result<u64, FrameworkError> {
        Ok(self.cursors(&[topic.to_owned()]).await?[0])
    }

    pub(super) async fn cursors(&self, topics: &[String]) -> Result<Vec<u64>, FrameworkError> {
        Ok(self
            .positions(topics)
            .await?
            .into_iter()
            .map(|position| position.cursor)
            .collect())
    }

    /// Captures generation and cursor together, renewing idle state in one command.
    pub(super) async fn positions(
        &self,
        topics: &[String],
    ) -> Result<Vec<crate::LiveTopicPosition>, FrameworkError> {
        if topics.is_empty() {
            return Ok(Vec::new());
        }
        let script = Script::new(
            r#"
local values = {}
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
for i, key in ipairs(KEYS) do
    local retention = tonumber(ARGV[i + 1])
    if redis.call('HEXISTS', key, 'generation') == 0 then
        local generation = redis.call('HGET', key .. ':pending', 'generation')
        if not generation then
            if retention > 0 then generation = now else generation = ARGV[1] end
        end
        redis.call('HSET', key, 'generation', generation, 'cursor', redis.call('HGET', key .. ':pending', 'cursor') or 0)
    end
    values[i] = {tonumber(redis.call('HGET', key, 'generation')), tonumber(redis.call('HGET', key, 'cursor') or '0')}
    redis.call('HSET', key, 'retention_ms', retention)
    if redis.call('HEXISTS', key, 'pending_member') == 1 or redis.call('EXISTS', key .. ':pending') == 1 then
        redis.call('PERSIST', key)
    elseif retention > 0 then redis.call('PEXPIRE', key, retention) end
end
return values
"#,
        );
        let mut invocation = script.prepare_invoke();
        for topic in topics {
            invocation.key(self.topic_keys(topic).0);
        }
        invocation.arg(self.generation);
        for topic in topics {
            invocation.arg(self.retention.window(topic).map_or(0, duration_millis));
        }
        let values: Vec<(u64, u64)> = invocation
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "SeaStreamerBroadcastHub: live topic baseline read failed: {error}"
                ))
            })?;
        Ok(values
            .into_iter()
            .map(|(generation, cursor)| crate::LiveTopicPosition { generation, cursor })
            .collect())
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

    async fn flush(&self, member: &str, instance_id: Uuid) -> Result<(), FrameworkError> {
        let schedule: TopicSchedule = serde_json::from_str(member).map_err(|error| {
            FrameworkError::internal(format!(
                "SeaStreamerBroadcastHub: invalid pending schedule: {error}"
            ))
        })?;
        let (meta, prop_set) = self.topic_keys(&schedule.topic);
        Script::new(FLUSH_SCRIPT)
            .key(&meta)
            .key(prop_set)
            .key(&self.schedule)
            .key(&self.stream)
            .key(format!("{meta}:pending"))
            .arg(instance_id.to_string())
            .arg(crate::broadcasting::live::LIVE_TOPIC_CHANNEL_PREFIX)
            .arg(LIVE_TOPIC_EVENT)
            .arg(super::sea_streamer::MESSAGE_FIELD)
            .arg(member)
            .arg(&schedule.topic)
            .arg(schedule.retention_ms)
            .arg(schedule.interval_ms)
            .arg(self.generation)
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
