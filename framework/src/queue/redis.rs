//! Redis-backed queue driver via sea-streamer-redis consumer groups.
//!
//! # Design
//!
//! Messages are stored in a Redis Stream and consumed via consumer groups
//! (`XREADGROUP` / `XACK`). Each `pop` call uses `XREADGROUP` to deliver one
//! message to this consumer; the message stays in the PEL (pending-entry list)
//! until `ack` is called.
//!
//! ## Delivery semantics
//!
//! This driver provides **at-least-once delivery**. After `ack` returns
//! `Ok(())`, the actual `XACK` may not yet be committed to Redis
//! (sea-streamer batches commits under `AutoCommit::Disabled`); if the
//! process crashes before the next flush, the message re-enters the
//! pending entries list and is re-delivered. Idempotency belongs at the
//! job level - see `framework/src/idempotency/mod.rs`.
//!
//! Similarly, `nack` performs two non-atomic Redis commands (XADD +
//! XACK). If XACK fails after XADD succeeds, the original message stays
//! in the PEL and is re-delivered via XAUTOCLAIM, while the
//! freshly-published copy carries `attempts + 1`. Job handlers MUST be
//! idempotent.
//!
//! ## Attempt accounting across redeliveries
//!
//! A stream entry is immutable, so the `attempts` it carries is whatever
//! was true when it was published. That is not the same as how many times
//! the job has been handed to a worker: a worker that dies mid-handler
//! settles nothing, and XAUTOCLAIM redelivers the identical entry.
//!
//! Redis's own per-entry delivery counter is the only record of those
//! redeliveries, and sea-streamer does not carry it through - it merges
//! XREADGROUP and XAUTOCLAIM into one message stream with no redelivery
//! flag. So `pop` asks `XPENDING` for the entry's delivery count and adds
//! `count - 1` to the envelope before handing it to the worker.
//!
//! Without that, a job which kills its worker could never exhaust
//! `max_tries` and so could never be dead-lettered: it would kill each
//! worker that claimed it, be redelivered unchanged, and kill the next
//! one. The database and memory drivers charge the same event in their own
//! reclaim paths - the semantics have to match, because swapping
//! `QUEUE_DRIVER` must not change whether a poison job can be stopped.
//!
//! ## Reclaim latency
//!
//! Two clocks, not one. `visibility_timeout` sets XAUTOCLAIM's *idle
//! threshold* - how long an entry must sit unacked before it qualifies -
//! while a separate interval governs how often a consumer looks. The
//! driver ties the second to the first (clamped to 1s..=30s), so a lost
//! job is redelivered within roughly `2 x visibility_timeout` rather than
//! `visibility_timeout + 30s`, which is what the unset sea-streamer
//! default produced regardless of how short the timeout was set.
//!
//! The clamp is asymmetric on purpose. The floor stops a one-second
//! timeout becoming an XAUTOCLAIM storm; the ceiling is sea-streamer's own
//! default, so raising the timeout can only make reclaim faster than it
//! used to be, never slower.
//!
//! ## Visibility timeout
//!
//! `auto_claim_idle` is configured once at construction time (via the
//! `visibility_timeout` argument to `connect`). Messages not acknowledged within
//! that window will be re-claimed by this consumer (or another in the group) on
//! the next poll cycle via Redis `XAUTOCLAIM`.
//!
//! The `visibility_timeout: Duration` parameter on `QueueDriver::pop` is
//! **ignored** for this driver; the per-connection value governs. This is a
//! documented divergence from the trait contract imposed by Redis Streams'
//! construction-time-only idle window.
//!
//! ## Delayed jobs (ZSET-backed)
//!
//! Redis Streams has no native scheduling. Envelopes whose `available_at` is in
//! the future (`Queue::later`, `Queue::push_later`, or a `nack` with non-zero
//! `requeue_delay`) are NOT published to the stream immediately - they go into
//! a companion sorted set `<stream>:delayed` keyed by `available_at` (unix
//! seconds). On every `pop`, the driver runs a Lua script under `EVAL` that
//! atomically claims all ZSET entries with `score <= now`, `XADD`s them onto
//! the stream (field `msg`, matching sea-streamer-redis's payload encoding),
//! and `ZREM`s them. The script iterates `ZREM` by member rather than using
//! `ZREMRANGEBYSCORE` so a brand-new same-score entry that lands between the
//! `ZRANGEBYSCORE` and the cleanup isn't accidentally removed.
//!
//! The promotion uses Lua because three round-trips (`ZRANGEBYSCORE` → `XADD`
//! per entry → `ZREM`) from Rust would let two parallel consumers double-claim
//! the same delayed entry; `EVAL` runs the whole sequence atomically on the
//! Redis server.
//!
//! ## nack semantics
//!
//! Redis Streams has no native nack-with-delay. `nack` is implemented as an
//! atomic two-step:
//! 1. Re-publish the envelope (with `attempts` incremented and `available_at`
//!    advanced by `requeue_delay`). A zero delay re-publishes directly to the
//!    stream via `XADD`; a non-zero delay goes to the `<stream>:delayed` ZSET
//!    and is promoted by the next `pop`.
//! 2. Acknowledge the original message via `XACK` so it leaves the PEL.
//!
//! ## AutoCommit::Disabled
//!
//! The consumer is created with `AutoCommit::Disabled` so no implicit ack
//! ever fires. The caller drives all acknowledgements through `ack`/`nack`.
//!
//! ## Connection topology
//!
//! `RedisQueueDriver` maintains **two** connection pools at the same Redis
//! endpoint: the sea-streamer pool (consumer-group `XREADGROUP`/`XACK` and
//! producer `XADD` for immediate publishes) and a `redis::aio::Connection-
//! Manager` (ZSET `ZADD` and `EVAL` for the delayed-job promotion script).
//! Both layers are needed because sea-streamer's API doesn't expose ZSET or
//! `EVAL` and the `redis` crate doesn't speak the consumer-group protocol at
//! the abstraction sea-streamer provides. Size connection pools accordingly
//! (each driver instance opens both).

use crate::error::FrameworkError;
use crate::lock;
use crate::queue::driver::{QueueDriver, Reservation, ReservationToken};
use crate::queue::envelope::{Envelope, queue_filter, queue_matches};
use crate::queue::inspect::InspectedJob;
use async_trait::async_trait;
use chrono::Utc;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use sea_streamer::ConsumerMode;
use sea_streamer::{
    Buffer, Consumer, ConsumerOptions, Message, Producer, StreamKey, Streamer, StreamerUri,
};
use sea_streamer::{ConsumerGroup, ConsumerId};
use sea_streamer_redis::{
    AutoCommit, AutoStreamReset, RedisConsumer, RedisConsumerOptions, RedisMessageId,
    RedisProducer, RedisStreamer,
};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;
use uuid::Uuid;

/// How many delayed entries one promotion pass may move.
///
/// The script runs on every `pop`, so a backlog drains across successive
/// polls rather than in one pass. Bounding it matters because Lua runs
/// single-threaded and atomically: an unbounded pass over a backlog - a
/// worker down for hours, a burst of long delays all coming due at once -
/// blocks *every* Redis client, not just this queue, for as long as the
/// script takes to `XADD` the lot.
const PROMOTE_DUE_BATCH: usize = 128;

/// Lua script that atomically promotes due delayed entries.
///
/// `KEYS[1]` is the `<stream>:delayed` sorted set; `KEYS[2]` is the stream
/// itself. `ARGV[1]` is the cutoff score (current unix seconds) and
/// `ARGV[2]` caps how many entries move in one pass. The script returns the
/// number promoted; a full batch means more remain due and the next `pop`
/// will take another bite.
///
/// The `XADD` field name is `msg` to match sea-streamer-redis's default
/// payload field, so promoted entries decode identically to ones the producer
/// pushed directly.
const PROMOTE_DUE_SCRIPT: &str = r#"
local entries = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])
for _, entry in ipairs(entries) do
    redis.call('XADD', KEYS[2], '*', 'msg', entry)
    redis.call('ZREM', KEYS[1], entry)
end
return #entries
"#;

/// Value stored in the pending map: the original envelope plus the
/// `SharedMessage` needed to call `RedisConsumer::ack`.
type PendingEntry = (Envelope, sea_streamer::SharedMessage);

/// Redis-backed queue driver.
///
/// Construct via [`RedisQueueDriver::connect`]. The driver is `Send + Sync`
/// and can be wrapped in an `Arc` for sharing across tasks.
pub struct RedisQueueDriver {
    producer: RedisProducer,
    consumer: RedisConsumer,
    stream_key: StreamKey,
    /// `<stream>:delayed` - the sorted set holding envelopes whose
    /// `available_at` is still in the future. Promoted into the stream by
    /// every `pop` via `PROMOTE_DUE_SCRIPT`.
    delayed_key: String,
    /// Consumer-group name. Captured at construction so the introspection
    /// methods (`reserved_size`, `pending_size`) can scope XPENDING queries
    /// to the same group the consumer reads from.
    group_name: String,
    /// Direct Redis connection used for ZADD on `push`/`nack` and EVAL on
    /// `pop`. Sea-streamer's `RedisProducer` is intentionally bypassed for
    /// these operations because it speaks only XADD; the
    /// `ConnectionManager` is cheap to clone (internally a multiplexed
    /// connection plus an Arc-shared task) and is what the `redis` crate
    /// recommends for high-throughput async use.
    conn: ConnectionManager,
    /// Map from `ReservationToken` UUID → `(Envelope, SharedMessage)`.
    /// The `SharedMessage` is required by `RedisConsumer::ack`.
    pending: Mutex<HashMap<Uuid, PendingEntry>>,
}

impl RedisQueueDriver {
    /// Connect to Redis and initialize the producer + consumer.
    ///
    /// # Arguments
    ///
    /// * `url` - Redis URL, e.g. `"redis://127.0.0.1:6379"`.
    /// * `stream` - Redis stream key name.
    /// * `group` - Consumer group name (created with `MKSTREAM` if absent).
    /// * `consumer_id` - Unique consumer ID within the group.
    /// * `visibility_timeout` - How long a message can remain unacknowledged
    ///   before another consumer may re-claim it (`XAUTOCLAIM` idle threshold).
    pub async fn connect(
        url: &str,
        stream: &str,
        group: &str,
        consumer_id: &str,
        visibility_timeout: Duration,
    ) -> Result<Self, FrameworkError> {
        let uri = StreamerUri::from_str(url)
            .map_err(|e| FrameworkError::internal(format!("redis URI parse error: {e}")))?;

        let streamer = RedisStreamer::connect(uri, Default::default())
            .await
            .map_err(|e| FrameworkError::internal(format!("redis connect error: {e}")))?;

        let stream_key = StreamKey::new(stream)
            .map_err(|e| FrameworkError::internal(format!("redis stream key error: {e}")))?;

        // Producer - not anchored; we call send_to explicitly with the stream key.
        let producer: RedisProducer = streamer
            .create_generic_producer(Default::default())
            .await
            .map_err(|e| FrameworkError::internal(format!("redis producer error: {e}")))?;

        // Consumer - LoadBalanced for consumer-group semantics, manual ack.
        let mut opts = RedisConsumerOptions::new(ConsumerMode::LoadBalanced);
        opts.set_consumer_group(ConsumerGroup::new(group))
            .map_err(|e| FrameworkError::internal(format!("redis set group error: {e}")))?;
        opts.set_consumer_id(ConsumerId::new(consumer_id));
        opts.set_auto_commit(AutoCommit::Disabled);
        opts.set_auto_claim_idle(visibility_timeout);
        // How often the consumer *looks* for reclaimable entries, which is
        // a separate clock from how long an entry must be idle before it
        // qualifies. sea-streamer defaults the check to 30s and this
        // driver used to leave it there, so a worker configured with
        // `--visibility-timeout 5` still waited up to 35s for a lost job
        // to come back. The flag did not mean what an operator would read
        // it to mean.
        //
        // Tracking the configured timeout fixes that. Clamped at both
        // ends: a floor so a one-second timeout does not turn into an
        // XAUTOCLAIM storm, and a ceiling at sea-streamer's own default so
        // a longer timeout can only make reclaim faster than it was
        // before, never slower.
        opts.set_auto_claim_interval(Some(
            visibility_timeout
                .max(Duration::from_secs(1))
                .min(Duration::from_secs(30)),
        ));
        // Allow consumer to create the group/stream if it doesn't exist yet.
        opts.set_mkstream(true);
        // Create the consumer group at position 0 (beginning of stream) so
        // messages pushed before the first `pop()` call are not missed.
        // The default (Latest / `$`) would skip any messages already in the
        // stream when the group is first initialized on the initial `next()`.
        opts.set_auto_stream_reset(AutoStreamReset::Earliest);

        let consumer: RedisConsumer = streamer
            .create_consumer(std::slice::from_ref(&stream_key), opts)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis consumer error: {e}")))?;

        // Open a second Redis connection (via the `redis` crate, alongside
        // sea-streamer's pool) for ZSET operations and the promotion Lua
        // script. Same URL; the driver speaks two protocol layers at the
        // same endpoint.
        let client = redis::Client::open(url)
            .map_err(|e| FrameworkError::internal(format!("redis client open: {e}")))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis ConnectionManager: {e}")))?;

        let delayed_key = format!("{}:delayed", stream);

        Ok(Self {
            producer,
            consumer,
            stream_key,
            delayed_key,
            group_name: group.to_string(),
            conn,
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// Run the promotion Lua script to flush all due delayed entries onto the
    /// stream. Called from `pop` on every entry; cheap on an empty ZSET.
    async fn promote_due(&self) -> Result<(), FrameworkError> {
        let now = Utc::now().timestamp();
        let stream_name = self.stream_key.name();
        let script = redis::Script::new(PROMOTE_DUE_SCRIPT);
        let mut conn = self.conn.clone();
        script
            .key(&self.delayed_key)
            .key(stream_name)
            .arg(now)
            .arg(PROMOTE_DUE_BATCH)
            .invoke_async::<i64>(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis promote_due EVAL: {e}")))?;
        Ok(())
    }

    /// ZADD an envelope into the delayed ZSET with score = `available_at` seconds.
    async fn zadd_delayed(&self, env: &Envelope) -> Result<(), FrameworkError> {
        let json = env
            .to_json()
            .map_err(|e| FrameworkError::internal(format!("envelope encode error: {e}")))?;
        let score = env.available_at.timestamp();
        let mut conn = self.conn.clone();
        let _added: i64 = conn
            .zadd(&self.delayed_key, json, score)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis ZADD delayed: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl QueueDriver for RedisQueueDriver {
    /// Serialize the envelope to JSON and publish it.
    ///
    /// Envelopes whose `available_at` is in the future go to the
    /// `<stream>:delayed` ZSET and only enter the stream when a later `pop`
    /// runs the promotion script. Immediate envelopes go straight to the
    /// stream via the sea-streamer producer.
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        if env.available_at > Utc::now() {
            return self.zadd_delayed(&env).await;
        }

        let json = env
            .to_json()
            .map_err(|e| FrameworkError::internal(format!("envelope encode error: {e}")))?;

        // send_to returns a SendFuture; awaiting it delivers the receipt.
        let fut = self
            .producer
            .send_to(&self.stream_key, json.as_str())
            .map_err(|e| FrameworkError::internal(format!("redis send error: {e}")))?;

        fut.await
            .map_err(|e| FrameworkError::internal(format!("redis send receipt error: {e}")))?;

        Ok(())
    }

    /// Poll for the next message. Returns `None` if no message arrives within
    /// `visibility_timeout`. Internally polls in short (100 ms) probe windows
    /// so the caller's deadline is respected without holding the consumer
    /// locked across the full wait.
    ///
    /// Note: `visibility_timeout` controls how long *this call* waits for a
    /// message. The XAUTOCLAIM idle window (how long an unacked message stays
    /// in the PEL before reclaim) is set at construction time and is unrelated.
    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        // Promote any due delayed entries onto the stream before we poll. The
        // script is atomic; concurrent consumers won't double-promote the
        // same envelope.
        self.promote_due().await?;

        // Poll in short probe windows so we return promptly when the queue is
        // empty AND honour the caller's deadline when a message is slow to arrive
        // (e.g. right after a push on a fresh stream/consumer-group).
        let probe = Duration::from_millis(100);
        let deadline = tokio::time::Instant::now() + visibility_timeout;

        let msg = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let wait = remaining.min(probe);
            match tokio::time::timeout(wait, self.consumer.next()).await {
                // This probe timed out - loop and check deadline.
                Err(_elapsed) => continue,
                // Consumer returned an error.
                Ok(Err(e)) => {
                    return Err(FrameworkError::internal(format!(
                        "redis consumer next error: {e}"
                    )));
                }
                Ok(Ok(msg)) => break msg,
            }
        };

        // Parse the envelope from the message payload.
        // Bind the Payload to a local so its borrow lives long enough.
        let payload = msg.message();
        let payload_bytes = payload.as_bytes();
        let payload_str = std::str::from_utf8(payload_bytes)
            .map_err(|e| FrameworkError::internal(format!("redis message not valid UTF-8: {e}")))?;

        let mut envelope = Envelope::from_json(payload_str)
            .map_err(|e| FrameworkError::internal(format!("envelope decode error: {e}")))?;

        // Charge this delivery's redeliveries to the attempt count.
        //
        // A message reaches a second delivery only through XAUTOCLAIM,
        // which fires when the consumer holding it went idle past the
        // visibility window - i.e. the worker died without acking, nacking
        // or releasing. The stream entry is immutable, so the `attempts`
        // it carries is whatever was true at publication; Redis's own
        // delivery counter is the only record that the job was handed out
        // again.
        //
        // Without this, a job that kills its worker is immortal on the
        // Redis driver exactly as it was on the database one: reclaimed
        // unchanged, never able to exhaust `max_tries`, never
        // dead-lettered. The two drivers have to agree, because swapping
        // the backend must not change whether a poison job can be stopped.
        //
        // `nack` composes correctly on top: it re-publishes with
        // `attempts + 1` from the value the worker was handed, which is
        // now the adjusted one.
        let (id_ts, id_seq) = msg.message_id();
        let redeliveries = self
            .redelivery_count(&format!("{id_ts}-{id_seq}"))
            .await
            .saturating_sub(1);
        envelope.attempts = envelope.attempts.saturating_add(redeliveries);

        let token = ReservationToken(envelope.id);

        // Store the shared message so we can ack it later.
        // Call the `Message` trait's `to_owned` explicitly (not `ToOwned`).
        let shared = sea_streamer::Message::to_owned(&msg);
        {
            let mut g = lock::lock(&self.pending, "redis queue pending map")?;
            g.insert(token.0, (envelope.clone(), shared));
        }

        Ok(Some(Reservation { envelope, token }))
    }

    /// Acknowledge a previously popped message, removing it from the PEL.
    ///
    /// Idempotent: unknown / already-acked tokens are silently ignored.
    ///
    /// At-least-once: the XACK is queued by sea-streamer and flushed on the
    /// next consumer interaction. A crash between `ack().await?` and the
    /// next flush re-delivers the message.
    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        let entry = {
            let mut g = lock::lock(&self.pending, "redis queue pending map")?;
            g.remove(&token.0)
        };

        if let Some((_envelope, shared_msg)) = entry {
            self.consumer
                .ack(&shared_msg)
                .map_err(|e| FrameworkError::internal(format!("redis ack error: {e}")))?;

            // Flush the ack to Redis immediately so it doesn't linger.
            // `commit` requires `&mut self` which we don't have here because
            // the trait requires `&self`. With `AutoCommit::Disabled` the ack
            // is queued internally and will be committed when the consumer's
            // internal flush fires or when the next `next()` call triggers it.
            // This is acceptable: the message is out of the consumer's in-flight
            // set from our perspective the moment `ack` is called.
        }
        // Token not found → already acked or never seen → idempotent no-op.

        Ok(())
    }

    /// Return a message to the queue with incremented `attempts` and an
    /// optional delay before it becomes visible again.
    ///
    /// Implementation:
    /// 1. Retrieve and remove the `(Envelope, SharedMessage)` from the pending map.
    /// 2. Bump `envelope.attempts += 1`.
    /// 3. Set `envelope.available_at = now + requeue_delay`.
    /// 4. Re-publish the modified envelope. A zero `requeue_delay` re-publishes
    ///    to the stream via `XADD`; a non-zero delay lands the envelope in
    ///    `<stream>:delayed` and the next `pop` promotes it once due.
    /// 5. Acknowledge the original message via `XACK` (removes it from the PEL).
    ///
    /// At-least-once: the re-publish and ack are non-atomic. A crash between
    /// re-publish success and ack success causes one extra delivery with the
    /// pre-nack attempts counter.
    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, requeue_delay, true, "nack").await
    }

    /// Put the message back after `delay` without consuming an attempt.
    ///
    /// Identical to [`nack`](Self::nack) except for step 2: the pending-map
    /// copy carries the pre-run attempt count (the worker bumps only its own
    /// local envelope), so re-publishing it unchanged is exactly the "retry
    /// without burning an attempt" the release contract asks for.
    async fn release(
        &self,
        token: &ReservationToken,
        _env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, delay, false, "release").await
    }

    /// Total envelopes the driver currently holds across the live stream
    /// plus the delayed ZSET.
    ///
    /// `XLEN` counts every entry that's been XADD'd minus those XDEL'd or
    /// XTRIM'd. Acknowledged entries remain in the stream until trimmed, so
    /// this is an upper bound on "live work" rather than a strict count of
    /// undelivered jobs - adequate for the same dashboarding role Laravel's
    /// `Queue::size()` plays. For "ready to pop" backlog use
    /// `pending_size()` (subtracts the PEL); for explicit reserved counts
    /// use `reserved_size()`.
    async fn size(&self) -> Result<u64, FrameworkError> {
        let stream_len = self.xlen_stream().await?;
        let delayed = self.zcard_delayed().await?;
        Ok(stream_len.saturating_add(delayed))
    }

    /// Envelopes on the stream that no consumer has claimed yet -
    /// approximated as `XLEN(stream) - XPENDING(group)`.
    ///
    /// This is the closest analogue to "available for the next pop" on
    /// Redis Streams. It can read high when a previous run left acked
    /// entries on the stream awaiting `XTRIM`/`XDEL`; treat it as an upper
    /// bound on backlog rather than a strict ready-count.
    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        let stream_len = self.xlen_stream().await?;
        let reserved = self.reserved_size().await.unwrap_or(0);
        Ok(stream_len.saturating_sub(reserved))
    }

    /// Envelopes parked on the `<stream>:delayed` ZSET because their
    /// `available_at` is still in the future.
    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        self.zcard_delayed().await
    }

    /// Envelopes currently held in the consumer group's Pending Entries
    /// List - i.e. delivered to some consumer but not yet `XACK`'d.
    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        self.xpending_count().await
    }

    /// Envelopes that have never been delivered to any consumer in this
    /// group - scanned via `XRANGE (<last-delivered-id> +` in batches of
    /// `PROMOTE_DUE_BATCH`, starting just past the group's delivery
    /// cursor (`XINFO GROUPS`).
    ///
    /// # Why the cursor, not a whole-stream scan skipping this process's map
    ///
    /// `ack` only `XACK`s an entry - this driver never `XDEL`/`XTRIM`s the
    /// stream - so a whole-stream `XRANGE` that excluded only this
    /// process's in-memory `pending` map would report every acked job as
    /// pending forever, plus every job a *different* consumer in the group
    /// is currently holding. The group's `last-delivered-id` is the
    /// correct boundary instead: everything at or below it has been
    /// handed to *some* consumer at least once (acked, still reserved, or
    /// lost and awaiting `XAUTOCLAIM`), and everything above it has not.
    /// A released or nacked job is re-`XADD`ed under a fresh id above the
    /// cursor, so it reappears here exactly once its retry goes live -
    /// nothing is permanently hidden by this scheme.
    ///
    /// Cheaper than a whole-stream walk too: the scan starts at the cursor
    /// instead of the beginning of the stream, so cost tracks the backlog
    /// of never-delivered entries rather than the stream's full history.
    ///
    /// # Snapshot, not a lock-step guarantee
    ///
    /// Same "upper bound" register [`pending_size`](Self::pending_size)
    /// documents: the cursor is read once (`XINFO GROUPS`), then the scan
    /// runs; a concurrent `pop` that advances the cursor after the read
    /// can claim an entry this call already decided was unclaimed. Treat
    /// the result as a snapshot at the moment the cursor was read, not a
    /// guarantee that every listed job is still unclaimed by the time the
    /// caller sees it.
    ///
    /// # No group yet
    ///
    /// `XINFO GROUPS` errors when the stream key does not exist at all
    /// (nothing pushed yet); an existing stream with no group created for
    /// it yet returns an empty group list. Both fold to "no cursor, scan
    /// everything from the start" (`-`), matching `xpending_count`'s
    /// established "unknown group reads as empty" convention.
    ///
    /// Only those two. Every other `XINFO GROUPS` failure - a dropped
    /// connection, an auth error, a wrong-type key - propagates, because
    /// folding it into "no cursor" turns an outage into a silent full-stream
    /// scan that reports every acked job in the stream's history as pending.
    ///
    /// # Read-ahead
    ///
    /// Redis `XREADGROUP` claims a batch of entries at once, so the cursor can
    /// sit well past the entry a worker is actually running: entries already
    /// read into a consumer's buffer are below the cursor and do not appear
    /// here, even though no handler has touched them yet. The listing is the
    /// never-delivered backlog, not "everything not yet started".
    async fn pending_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        let filter = queue_filter(queue);
        let mut conn = self.conn.clone();

        let groups: redis::streams::StreamInfoGroupsReply =
            match conn.xinfo_groups(self.stream_key.name()).await {
                Ok(groups) => groups,
                // "Nothing has happened here yet" reads as no cursor, the same way
                // `xpending_count` folds "no group" into "0 reserved". Anything
                // else is an error the caller has to see.
                Err(e) if is_missing_stream_or_group(&e) => Default::default(),
                Err(e) => {
                    return Err(FrameworkError::internal(format!("redis XINFO GROUPS: {e}")));
                }
            };
        let mut start = groups
            .groups
            .iter()
            .find(|g| g.name == self.group_name)
            .map(|g| format!("({}", g.last_delivered_id))
            .unwrap_or_else(|| "-".to_string());

        let mut out = Vec::new();

        loop {
            let reply: redis::streams::StreamRangeReply = conn
                .xrange_count(
                    self.stream_key.name(),
                    start.as_str(),
                    "+",
                    PROMOTE_DUE_BATCH,
                )
                .await
                .map_err(|e| FrameworkError::internal(format!("redis XRANGE: {e}")))?;

            let batch_len = reply.ids.len();
            if batch_len == 0 {
                break;
            }

            for entry in &reply.ids {
                let payload: Option<String> = entry
                    .map
                    .get("msg")
                    .and_then(|v| redis::FromRedisValue::from_redis_value(v.clone()).ok());
                let Some(payload) = payload else {
                    tracing::warn!(
                        entry_id = %entry.id,
                        "redis pending_jobs: stream entry missing a `msg` field; skipping"
                    );
                    continue;
                };
                let env = match Envelope::from_json(&payload) {
                    Ok(env) => env,
                    Err(e) => {
                        tracing::warn!(
                            entry_id = %entry.id,
                            error = %e,
                            "redis pending_jobs: unparseable stream entry; skipping"
                        );
                        continue;
                    }
                };
                if !queue_matches(env.queue.as_deref(), &filter) {
                    continue;
                }
                out.push(InspectedJob::from_envelope(&env));
            }

            if batch_len < PROMOTE_DUE_BATCH {
                break;
            }
            // Exclusive-start form: "(" + the last id seen, so the next page
            // picks up immediately after it instead of re-reading it.
            let Some(last) = reply.ids.last() else {
                break;
            };
            start = format!("({}", last.id);
        }

        Ok(out)
    }

    /// Envelopes parked on the `<stream>:delayed` ZSET because their
    /// `available_at` is still in the future. `ZRANGE` returns members
    /// without their scores - fine here, since a listing carries no
    /// ordering contract the way promotion's due-order processing does.
    async fn delayed_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        let filter = queue_filter(queue);
        let mut conn = self.conn.clone();
        let members: Vec<String> = conn
            .zrange(&self.delayed_key, 0, -1)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis ZRANGE delayed: {e}")))?;
        Ok(members
            .iter()
            .filter_map(|json| match Envelope::from_json(json) {
                Ok(env) => Some(env),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "redis delayed_jobs: unparseable ZSET member; skipping"
                    );
                    None
                }
            })
            .filter(|env| queue_matches(env.queue.as_deref(), &filter))
            .map(|env| InspectedJob::from_envelope(&env))
            .collect())
    }

    /// Envelopes this consumer has popped but not yet acked, nacked, or
    /// released - this process's slice of the consumer group's Pending
    /// Entries List.
    ///
    /// **Per-consumer, not per-group**: another process's in-flight
    /// reservations are not visible here, only through Redis's own
    /// `XPENDING` - see the module doc's "Connection topology" note. Use
    /// [`reserved_size`](Self::reserved_size) for the group-wide count.
    async fn reserved_jobs(
        &self,
        queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        let filter = queue_filter(queue);
        let g = lock::lock(&self.pending, "redis queue pending map")?;
        Ok(g.values()
            .map(|(env, _msg)| env)
            .filter(|env| queue_matches(env.queue.as_deref(), &filter))
            .map(InspectedJob::from_envelope)
            .collect())
    }

    /// Delete every envelope the driver tracks: the stream itself, the
    /// delayed ZSET, and the in-process pending-reservation map.
    ///
    /// Returns an approximate count of dropped envelopes (stream entries +
    /// delayed entries observed at the moment `XLEN`/`ZCARD` ran). The
    /// stream's consumer group is destroyed alongside the stream; it is
    /// re-created on the next `pop` because the consumer is configured with
    /// `mkstream` + Earliest reset.
    async fn clear(&self) -> Result<u64, FrameworkError> {
        let mut conn = self.conn.clone();
        let stream_len = self.xlen_stream().await?;
        let delayed = self.zcard_delayed().await?;

        let stream_name = self.stream_key.name();
        let _: i64 = redis::cmd("DEL")
            .arg(stream_name)
            .arg(&self.delayed_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis clear DEL: {e}")))?;

        // Reservation tokens issued before clear are no longer meaningful;
        // drop the in-process map so ack/nack on them are silent no-ops.
        if let Ok(mut g) = lock::lock(&self.pending, "redis queue pending map") {
            g.clear();
        }

        Ok(stream_len.saturating_add(delayed))
    }

    fn name(&self) -> &'static str {
        "redis-streams"
    }
}

impl RedisQueueDriver {
    /// Shared body of [`QueueDriver::nack`] and [`QueueDriver::release`],
    /// which differ only in whether the requeue consumes an attempt.
    async fn requeue(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
        consume_attempt: bool,
        op: &'static str,
    ) -> Result<(), FrameworkError> {
        let entry = {
            let mut g = lock::lock(&self.pending, "redis queue pending map")?;
            g.remove(&token.0)
        };

        let (mut envelope, shared_msg) = match entry {
            Some(e) => e,
            // Already acked / unknown token - silently succeed.
            None => return Ok(()),
        };

        if consume_attempt {
            envelope.attempts += 1;
        }

        // Advance availability by the requested delay.
        let available_at = Utc::now()
            + chrono::Duration::from_std(requeue_delay).unwrap_or(chrono::Duration::zero());
        envelope.available_at = available_at;

        if requeue_delay.is_zero() {
            // Immediate retry - straight to the stream.
            let json = envelope.to_json().map_err(|e| {
                FrameworkError::internal(format!("envelope encode error ({op}): {e}"))
            })?;
            let send_fut = self
                .producer
                .send_to(&self.stream_key, json.as_str())
                .map_err(|e| {
                    FrameworkError::internal(format!("redis {op} re-publish error: {e}"))
                })?;
            send_fut.await.map_err(|e| {
                FrameworkError::internal(format!("redis {op} re-publish receipt error: {e}"))
            })?;
        } else {
            // Deferred retry - park on the delayed ZSET; pop will promote.
            self.zadd_delayed(&envelope).await?;
        }

        // Ack the original message so it leaves the PEL.
        self.consumer
            .ack(&shared_msg)
            .map_err(|e| FrameworkError::internal(format!("redis {op} ack error: {e}")))?;

        Ok(())
    }

    /// `XLEN <stream>` - total entries currently held by the stream
    /// (including acknowledged-but-not-trimmed ones).
    async fn xlen_stream(&self) -> Result<u64, FrameworkError> {
        let mut conn = self.conn.clone();
        let n: i64 = redis::cmd("XLEN")
            .arg(self.stream_key.name())
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis XLEN: {e}")))?;
        Ok(n.max(0) as u64)
    }

    /// `ZCARD <stream>:delayed` - entries parked awaiting their
    /// `available_at` deadline.
    async fn zcard_delayed(&self) -> Result<u64, FrameworkError> {
        let mut conn = self.conn.clone();
        let n: i64 = redis::cmd("ZCARD")
            .arg(&self.delayed_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis ZCARD delayed: {e}")))?;
        Ok(n.max(0) as u64)
    }

    /// How many times Redis has delivered one stream entry.
    ///
    /// `XPENDING <key> <group> IDLE 0 <id> <id> 1` returns one row per
    /// entry in the form `[id, consumer, idle-ms, delivery-count]`. The
    /// count is 1 on a first delivery and rises with every XAUTOCLAIM.
    ///
    /// # Cost
    ///
    /// One extra command per `pop`, on the already-multiplexed connection.
    /// It is unconditional because sea-streamer merges XREADGROUP and
    /// XAUTOCLAIM into a single message stream and carries no redelivery
    /// flag through, so there is nothing cheaper to branch on. Paying it
    /// buys the only signal that distinguishes a job whose worker died
    /// from a job being delivered for the first time.
    ///
    /// Returns 1 - "treat as a first delivery" - for anything unexpected:
    /// a missing group, an entry already acked, a reply shape this does
    /// not recognise. Guessing high here would burn attempts off healthy
    /// jobs and dead-letter work that never failed, which is worse than
    /// the bug this exists to fix.
    async fn redelivery_count(&self, entry_id: &str) -> u32 {
        let mut conn = self.conn.clone();
        let resp: redis::Value = match redis::cmd("XPENDING")
            .arg(self.stream_key.name())
            .arg(&self.group_name)
            .arg("IDLE")
            .arg(0)
            .arg(entry_id)
            .arg(entry_id)
            .arg(1)
            .query_async(&mut conn)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    entry_id,
                    "XPENDING lookup failed; treating this as a first delivery"
                );
                return 1;
            }
        };

        // [[id, consumer, idle-ms, delivery-count]]
        let rows = match resp {
            redis::Value::Array(rows) | redis::Value::Set(rows) => rows,
            _ => return 1,
        };
        let Some(first) = rows.into_iter().next() else {
            return 1;
        };
        let cells = match first {
            redis::Value::Array(cells) | redis::Value::Set(cells) => cells,
            _ => return 1,
        };
        match cells.get(3) {
            Some(redis::Value::Int(n)) => u32::try_from(*n).unwrap_or(1).max(1),
            _ => 1,
        }
    }

    /// `XPENDING <stream> <group>` summary - first element is the total
    /// count of entries in the group's Pending Entries List (delivered but
    /// not yet acked). Returns 0 if the group does not exist (cleared
    /// stream, never-popped driver instance).
    async fn xpending_count(&self) -> Result<u64, FrameworkError> {
        let mut conn = self.conn.clone();
        // XPENDING summary form returns
        //   [count, smallest-id, largest-id, [[consumer, count], ...]]
        // or all-nil when the group is empty. We only need the first cell.
        let resp: redis::Value = redis::cmd("XPENDING")
            .arg(self.stream_key.name())
            .arg(&self.group_name)
            .query_async(&mut conn)
            .await
            // The group may not exist yet (no `pop` has run on a fresh stream),
            // which surfaces as a Redis error. Treat that as "0 reserved".
            .unwrap_or(redis::Value::Nil);
        let count = match resp {
            redis::Value::Array(parts) | redis::Value::Set(parts) => match parts.first() {
                Some(redis::Value::Int(n)) => (*n).max(0) as u64,
                _ => 0,
            },
            redis::Value::Int(n) => n.max(0) as u64,
            _ => 0,
        };
        Ok(count)
    }
}

/// True for the two Redis errors that mean "this stream or group has not been
/// created yet", and false for every other failure.
///
/// The distinction matters because the caller's fallback for these is a
/// full-stream scan from `-`. That is right when there is genuinely nothing to
/// scan past, and wrong for a dropped connection or an auth failure, where it
/// would turn an outage into a listing of every acked entry in the stream's
/// history.
///
/// What the `redis` crate hands back (1.2):
///
/// - Missing stream key: the server replies `ERR no such key`, which the crate
///   models as `ErrorKind::Server(ServerErrorKind::ResponseError)` with
///   `detail()` carrying the text. `ERR` is the generic server code, so the
///   detail is the only thing that separates this from any other `ERR`.
/// - Missing group: the server replies `NOGROUP ...`. `NOGROUP` is not one of
///   the crate's known `ServerErrorKind`s, so it arrives as
///   `ErrorKind::Extension` with `code() == Some("NOGROUP")` - matched on the
///   code, which needs no string search.
fn is_missing_stream_or_group(e: &redis::RedisError) -> bool {
    if e.code() == Some("NOGROUP") {
        return true;
    }
    matches!(
        e.kind(),
        redis::ErrorKind::Server(redis::ServerErrorKind::ResponseError)
    ) && e
        .detail()
        .is_some_and(|d| d.to_ascii_lowercase().contains("no such key"))
}
