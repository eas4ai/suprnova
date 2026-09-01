//! Redis-backed queue driver using Redis Streams consumer groups.
//!
//! # Design
//!
//! Messages are stored in a Redis Stream and consumed via consumer groups
//! (`XREADGROUP` / `XACK`). Each `pop` first makes one bounded `XAUTOCLAIM`
//! attempt, then asks `XREADGROUP` for one new message when no expired delivery
//! was reclaimed. The message stays in the PEL (pending-entry list) until a
//! terminal operation acknowledges it.
//!
//! ## Delivery semantics
//!
//! This driver provides **at-least-once delivery**. A reservation remains in
//! Redis's pending-entry list until an explicit terminal operation succeeds;
//! a worker that exits first leaves the entry available for reclaim.
//!
//! `ack`, `nack`, `release`, and `settle` compare the stream entry's consumer
//! owner and delivery count with the generation captured by `pop`. A single
//! Redis script then applies any successor publication and `XACK` atomically.
//! A delayed response may still make the caller retry, but the missing PEL
//! generation turns that retry into a no-op rather than a duplicate publish.
//! Job handlers still need to be idempotent because a worker can perform its
//! own external side effects and then exit before queue settlement.
//!
//! ## Attempt accounting across redeliveries
//!
//! A stream entry is immutable, so the `attempts` it carries is whatever
//! was true when it was published. That is not the same as how many times
//! the job has been handed to a worker: a worker that dies mid-handler
//! settles nothing, and XAUTOCLAIM redelivers the identical entry.
//!
//! Redis's own per-entry delivery counter is the only record of those
//! redeliveries, and the immutable stream payload does not carry it. The driver owns
//! XAUTOCLAIM and, after either claim or XREADGROUP delivery, atomically reads
//! the queue epoch, exact XPENDING generation, and current stream payload.
//! `pop` adds `delivery-count - 1` to that authoritative envelope before
//! handing it to the worker.
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
//! `visibility_timeout` is XAUTOCLAIM's idle threshold: how long an entry must
//! remain unacknowledged before it qualifies. Every `pop` makes one bounded,
//! self-targeted XAUTOCLAIM attempt before asking for new work. The driver
//! persists Redis's scan cursor across calls, including empty pages, so a busy
//! early page cannot starve later eligible entries. Reclaim latency after the
//! threshold therefore depends on the application's polling cadence; there is
//! no separate background claim interval.
//!
//! ## Visibility timeout
//!
//! The `visibility_timeout` argument to `connect` is stored once and passed
//! directly to driver-owned XAUTOCLAIM. Messages not acknowledged within that
//! window can be re-claimed by this consumer on a later `pop`; another driver
//! instance in the group can likewise claim them to its own consumer identity.
//! Values below one millisecond or with fractional-millisecond precision are
//! rejected because Redis represents the idle threshold in whole milliseconds.
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
//! a companion sorted set `<stream>:delayed` scored by `available_at` (unix
//! seconds). Each member carries a UUID prefix separated from its JSON by a
//! NUL byte, so Redis's unique-member rule cannot collapse equal envelopes.
//! On every `pop`, the driver runs a Lua script under `EVAL` that strips the
//! prefix (while accepting legacy raw-JSON members), atomically claims a bounded
//! batch of entries with `score <= now` (currently 128), `XADD`s them onto the
//! stream (field `msg`, matching sea-streamer-redis's payload encoding), and
//! `ZREM`s them. The script iterates
//! `ZREM` by member rather than using
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
//! Redis Streams has no native nack-with-delay. The driver freezes the retry
//! envelope once, stages it as a uniquely prefixed ZSET member (including a
//! zero-delay retry), and XACKs the original generation in the same Lua script.
//! A later `pop` promotes the staged retry onto the stream.
//!
//! Atomic settlement accepts at most 128 follow-ups. Redis runs Lua on its main
//! thread, so bounding both the forward `ZADD` loop and its possible rollback
//! prevents one queue operation from monopolizing the server.

//! ## Settlement receipts
//!
//! A terminal mutation writes one idempotency receipt so a dropped Redis
//! response can be reconciled without publishing successors twice. Its TTL is
//! the greater of one hour and four times the connection visibility timeout.
//! Consequently, unusually large visibility timeouts retain one small receipt
//! per completed reservation for proportionally longer; size Redis capacity and
//! timeout configuration together. The driver does not cap this duration,
//! because expiring a receipt while its reservation may still be legitimately
//! retried would weaken settlement outcome reporting. Once the TTL does expire,
//! retrying an already-applied mutation remains non-duplicating but is reported
//! as stale because Redis no longer retains proof of the prior result.
//!
//! ## Direct consumer reads
//!
//! New deliveries use direct `XREADGROUP ... >` commands rather than
//! sea-streamer's consumer. Sea-streamer 0.5.2 begins by replaying the current
//! consumer's PEL, which can immediately re-deliver an entry that this driver
//! just reclaimed with `XAUTOCLAIM`. Owning both commands keeps visibility and
//! delivery-generation transitions under one protocol.
//!
//! ## Connection topology
//!
//! `RedisQueueDriver` uses sea-streamer for producer `XADD`, a dedicated
//! `redis::aio::ConnectionManager` for blocking `XREADGROUP`, and another
//! manager for claims, inspection, ZSET operations, and Lua scripts. The read
//! connection is separate because a blocking stream command would otherwise
//! stall unrelated commands multiplexed on the same socket.
//!
//! The endpoint must be standalone Redis 6.2 or newer. Redis Cluster is rejected
//! because the settlement scripts atomically access stream, delayed-set, epoch,
//! and receipt keys. Startup reads `INFO server` and `INFO cluster`, so restricted
//! ACLs must permit those probes in addition to the stream, sorted-set, key/value,
//! and scripting commands used at runtime (`X*`, `Z*`, `GET`, `SET`, `DEL`,
//! `TYPE`, `EVAL`/`EVALSHA`, and `SCRIPT LOAD`).
//!
//! The configured stream name reserves an exclusive companion-key namespace:
//! `<stream>:delayed`, `<stream>:epoch`, and
//! `<stream>:settlement-receipt:<operation-uuid>`. Applications sharing the Redis
//! database must not use those keys for unrelated data; queue operations may read,
//! overwrite, or delete them as part of normal promotion, settlement, and clearing.

use crate::error::FrameworkError;
use crate::lock;
use crate::queue::driver::{
    QueueDriver, QueueFilterCapability, Reservation, ReservationToken, Settled,
};
use crate::queue::envelope::{Envelope, queue_filter, queue_matches};
use crate::queue::inspect::InspectedJob;
use async_trait::async_trait;
use chrono::Utc;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use redis::streams::{StreamAutoClaimOptions, StreamAutoClaimReply, StreamReadReply};
use sea_streamer::{Producer, StreamKey, Streamer, StreamerUri};
use sea_streamer_redis::{RedisProducer, RedisStreamer};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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

/// Maximum successor publications one atomic Redis settlement may stage.
const MAX_ATOMIC_FOLLOW_UPS: usize = 128;

fn validate_atomic_follow_up_count(count: usize) -> Result<(), FrameworkError> {
    if count > MAX_ATOMIC_FOLLOW_UPS {
        return Err(FrameworkError::internal(format!(
            "Redis atomic settlement accepts at most {MAX_ATOMIC_FOLLOW_UPS} follow-ups"
        )));
    }
    Ok(())
}

/// One Redis consumer probe per aggregate poll keeps an empty connection from
/// delaying later failover connections for a worker-sized visibility lease.
const POP_PROBE_BUDGET: Duration = Duration::from_millis(100);

fn pop_probe_budget(_requested_visibility: Duration) -> Duration {
    POP_PROBE_BUDGET
}

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
      local separator = string.find(entry, string.char(0), 1, true)
      local payload = entry
      if separator ~= nil then
          payload = string.sub(entry, separator + 1)
      end
      redis.call('XADD', KEYS[2], '*', 'msg', payload)
      redis.call('ZREM', KEYS[1], entry)
  end
  return #entries
"#;

/// Atomically verify one queue epoch and PEL delivery generation, record an
/// idempotency receipt, stage every successor, and acknowledge the exact entry.
/// Returns `1` for a new mutation, `2` for a matching prior receipt, and `0`
/// when the epoch or delivery generation is stale.
const FENCED_SETTLEMENT_SCRIPT: &str = r#"
local count = tonumber(ARGV[8])
if count == nil or count < 0 or count ~= math.floor(count) then
    return redis.error_reply('ERR suprnova fenced settlement invalid count')
end
local receipt_ttl = tonumber(ARGV[7])
if receipt_ttl == nil or receipt_ttl < 1 or receipt_ttl ~= math.floor(receipt_ttl) then
    return redis.error_reply('ERR suprnova fenced settlement invalid receipt TTL')
end
if #ARGV ~= 8 + (count * 2) then
    return redis.error_reply('ERR suprnova fenced settlement invalid arguments')
end

local scores = {}
local members = {}
for index = 1, count do
    local offset = 9 + ((index - 1) * 2)
    local score = tonumber(ARGV[offset])
    if score == nil then
        return redis.error_reply('ERR suprnova fenced settlement invalid score')
    end
    table.insert(scores, ARGV[offset])
    table.insert(members, ARGV[offset + 1])
end

local epoch = redis.pcall('GET', KEYS[3])
if type(epoch) == 'table' and epoch.err then
    return redis.error_reply(epoch.err)
end
if epoch == false then
    epoch = ''
end
if epoch ~= ARGV[5] then
    return 0
end

local receipt = redis.pcall('GET', KEYS[4])
if type(receipt) == 'table' and receipt.err then
    return redis.error_reply(receipt.err)
end
local matching_receipt = false
if receipt ~= false then
    if receipt ~= ARGV[6] then
        return redis.error_reply('ERR suprnova settlement receipt fingerprint mismatch')
    end
    matching_receipt = true
end

local pending = redis.pcall('XPENDING', KEYS[1], ARGV[1], ARGV[2], ARGV[2], 1)
if type(pending) == 'table' and pending.err then
    if string.find(pending.err, 'NOGROUP', 1, true) then
        if matching_receipt then
            return 2
        end
        return 0
    end
    return redis.error_reply(pending.err)
end
if type(pending) ~= 'table' then
    return 0
end
if #pending == 0 then
    if matching_receipt then
        return 2
    end
    return 0
end
if matching_receipt then
    return redis.error_reply('ERR suprnova settlement receipt conflicts with live PEL entry')
end
if #pending ~= 1 then
    return 0
end
local row = pending[1]
if type(row) ~= 'table' or #row < 4 then
    return 0
end
if row[1] ~= ARGV[2] or row[2] ~= ARGV[3] or tonumber(row[4]) ~= tonumber(ARGV[4]) then
    return 0
end

if count > 0 then
    local delayed_type = redis.call('TYPE', KEYS[2])
    if type(delayed_type) == 'table' then
        delayed_type = delayed_type.ok
    end
    if delayed_type ~= 'none' and delayed_type ~= 'zset' then
        return redis.error_reply('WRONGTYPE suprnova delayed queue key is not a sorted set')
    end
end

local ack_probe = redis.pcall('XACK', KEYS[1], ARGV[1], '0-0')
if type(ack_probe) == 'table' and ack_probe.err then
    return redis.error_reply(ack_probe.err)
end
if ack_probe ~= 0 then
    return redis.error_reply('ERR suprnova fenced settlement invalid XACK probe result')
end
local delete_probe = redis.pcall('DEL', KEYS[4])
if type(delete_probe) == 'table' and delete_probe.err then
    return redis.error_reply(delete_probe.err)
end
if delete_probe ~= 0 then
    return redis.error_reply('ERR suprnova fenced settlement receipt changed during script')
end
local stored = redis.pcall('SET', KEYS[4], ARGV[6], 'PX', receipt_ttl, 'NX')
if type(stored) == 'table' and stored.err then
    return redis.error_reply(stored.err)
end
if stored == false then
    return redis.error_reply('ERR suprnova fenced settlement receipt already exists')
end

local added_count = 0
for index = 1, count do
    local staged = redis.pcall('ZADD', KEYS[2], 'NX', scores[index], members[index])
    if (type(staged) == 'table' and staged.err) or staged ~= 1 then
        local rollback_error = nil
        for rollback_index = 1, added_count do
            local removed = redis.pcall('ZREM', KEYS[2], members[rollback_index])
            if type(removed) == 'table' and removed.err then
                rollback_error = removed.err
            elseif removed ~= 1 then
                rollback_error = 'staged member missing during rollback'
            end
        end
        local receipt_removed = redis.pcall('DEL', KEYS[4])
        if type(receipt_removed) == 'table' and receipt_removed.err then
            rollback_error = receipt_removed.err
        elseif receipt_removed ~= 1 then
            rollback_error = 'receipt missing during rollback'
        end
        if rollback_error ~= nil then
            return redis.error_reply('ERR suprnova fenced settlement rollback failed: ' .. rollback_error)
        end
        if type(staged) == 'table' and staged.err then
            return redis.error_reply(staged.err)
        end
        return redis.error_reply('ERR suprnova fenced settlement member already exists')
    end
    added_count = index
end

local acknowledged = redis.pcall('XACK', KEYS[1], ARGV[1], ARGV[2])
if (type(acknowledged) == 'table' and acknowledged.err) or acknowledged ~= 1 then
    local rollback_error = nil
    for index = 1, added_count do
        local removed = redis.pcall('ZREM', KEYS[2], members[index])
        if type(removed) == 'table' and removed.err then
            rollback_error = removed.err
        elseif removed ~= 1 then
            rollback_error = 'staged member missing during rollback'
        end
    end
    local receipt_removed = redis.pcall('DEL', KEYS[4])
    if type(receipt_removed) == 'table' and receipt_removed.err then
        rollback_error = receipt_removed.err
    elseif receipt_removed ~= 1 then
        rollback_error = 'receipt missing during rollback'
    end
    if rollback_error ~= nil then
        return redis.error_reply('ERR suprnova fenced settlement rollback failed: ' .. rollback_error)
    end
    if type(acknowledged) == 'table' and acknowledged.err then
        return redis.error_reply(acknowledged.err)
    end
    return redis.error_reply('ERR suprnova fenced settlement lost PEL entry')
end
return 1
"#;

const PENDING_SNAPSHOT_SCRIPT: &str = r#"
local epoch = redis.pcall('GET', KEYS[2])
if type(epoch) == 'table' and epoch.err then
    return redis.error_reply(epoch.err)
end
if epoch == false then
    epoch = ''
end
local pending = redis.pcall('XPENDING', KEYS[1], ARGV[1], ARGV[2], ARGV[2], 1)
if type(pending) == 'table' and pending.err then
    if string.find(pending.err, 'NOGROUP', 1, true) then
        return {epoch, {}, false}
    end
    return redis.error_reply(pending.err)
end
local entries = redis.pcall('XRANGE', KEYS[1], ARGV[2], ARGV[2], 'COUNT', 1)
if type(entries) == 'table' and entries.err then
    return redis.error_reply(entries.err)
end
local payload = false
if type(entries) == 'table' and #entries == 1 then
    local entry = entries[1]
    if type(entry) == 'table' and #entry >= 2 and entry[1] == ARGV[2] then
        local fields = entry[2]
        if type(fields) == 'table' then
            for index = 1, #fields, 2 do
                if fields[index] == 'msg' then
                    if payload ~= false then
                        return redis.error_reply('ERR suprnova stream entry has duplicate msg fields')
                    end
                    payload = fields[index + 1]
                end
            end
        end
    end
end
return {epoch, pending, payload}
"#;

const MUTATION_STATUS_SCRIPT: &str = r#"
local epoch = redis.pcall('GET', KEYS[2])
if type(epoch) == 'table' and epoch.err then
    return redis.error_reply(epoch.err)
end
if epoch == false then
    epoch = ''
end
if epoch ~= ARGV[5] then
    return 0
end
local receipt = redis.pcall('GET', KEYS[3])
if type(receipt) == 'table' and receipt.err then
    return redis.error_reply(receipt.err)
end
local matching_receipt = false
if receipt ~= false then
    if receipt ~= ARGV[6] then
        return redis.error_reply('ERR suprnova settlement receipt fingerprint mismatch')
    end
    matching_receipt = true
end
local pending = redis.pcall('XPENDING', KEYS[1], ARGV[1], ARGV[2], ARGV[2], 1)
if type(pending) == 'table' and pending.err then
    if string.find(pending.err, 'NOGROUP', 1, true) then
        if matching_receipt then
            return 2
        end
        return 0
    end
    return redis.error_reply(pending.err)
end
if type(pending) ~= 'table' then
    return 0
end
if #pending == 0 then
    if matching_receipt then
        return 2
    end
    return 0
end
if matching_receipt then
    return redis.error_reply('ERR suprnova settlement receipt conflicts with live PEL entry')
end
if #pending ~= 1 then
    return 0
end
local row = pending[1]
if type(row) ~= 'table' or #row < 4 then
    return 0
end
if row[1] == ARGV[2] and row[2] == ARGV[3] and tonumber(row[4]) == tonumber(ARGV[4]) then
    return 3
end
return 0
"#;

const CLEAR_SCRIPT: &str = r#"
local stream_len = redis.call('XLEN', KEYS[1])
local delayed_len = redis.call('ZCARD', KEYS[2])
redis.call('SET', KEYS[3], ARGV[1])
redis.call('DEL', KEYS[1], KEYS[2])
return {stream_len, delayed_len}
"#;

fn encode_delayed_member(payload: &str) -> String {
    format!("{}\0{payload}", Uuid::new_v4())
}

fn delayed_member_payload(member: &str) -> &str {
    // JSON escapes U+0000, so a literal NUL cannot occur in an envelope payload.
    // No separator means this member predates the uniqueness prefix.
    member
        .split_once('\0')
        .map_or(member, |(_, payload)| payload)
}

fn delayed_score(available_at: &chrono::DateTime<Utc>, prepared_at: &chrono::DateTime<Utc>) -> i64 {
    let round_up = available_at > prepared_at && available_at.timestamp_subsec_nanos() != 0;
    available_at.timestamp().saturating_add(i64::from(round_up))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DeliveryFence {
    epoch: String,
    entry_id: String,
    owner: String,
    deliveries: u32,
}

struct PendingEntry {
    envelope: Envelope,
    fence: DeliveryFence,
    lease_deadline: Instant,
    needs_reconciliation: AtomicBool,
    lifecycle: tokio::sync::Mutex<LifecycleState>,
}

impl PendingEntry {
    fn new(envelope: Envelope, fence: DeliveryFence, lease_deadline: Instant) -> Self {
        Self {
            envelope,
            fence,
            lease_deadline,
            needs_reconciliation: AtomicBool::new(false),
            lifecycle: tokio::sync::Mutex::new(LifecycleState::Reserved),
        }
    }
}

enum LifecycleState {
    Reserved,
    AckPending,
    RequeuePending(PreparedRequeue),
    SettlementPending(PreparedSettlement),
    Settled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequeueKind {
    Nack,
    Release,
}

impl RequeueKind {
    fn label(self) -> &'static str {
        match self {
            Self::Nack => "nack",
            Self::Release => "release",
        }
    }

    fn consumes_attempt(self) -> bool {
        matches!(self, Self::Nack)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreparedRequeue {
    kind: RequeueKind,
    requested_delay: Duration,
    payload: Arc<str>,
    target: RequeueTarget,
}

impl PreparedRequeue {
    fn new<Encode>(
        original: &Envelope,
        requested_delay: Duration,
        kind: RequeueKind,
        requested_at: chrono::DateTime<Utc>,
        encode: Encode,
    ) -> Result<Self, FrameworkError>
    where
        Encode: FnOnce(&Envelope) -> Result<String, FrameworkError>,
    {
        let delay = chrono::Duration::from_std(requested_delay).map_err(|_| {
            FrameworkError::internal(format!(
                "Redis requeue delay {requested_delay:?} cannot be represented"
            ))
        })?;
        let available_at = requested_at.checked_add_signed(delay).ok_or_else(|| {
            FrameworkError::internal(format!(
                "Redis requeue delay {requested_delay:?} exceeds the supported timestamp range"
            ))
        })?;
        let mut envelope = original.clone();
        if kind.consumes_attempt() {
            envelope.attempts = envelope.attempts.saturating_add(1);
        }
        envelope.available_at = available_at;
        let payload = Arc::<str>::from(encode(&envelope)?);
        let target = RequeueTarget::Delayed {
            score: delayed_score(&envelope.available_at, &requested_at),
            member: Arc::from(encode_delayed_member(payload.as_ref())),
        };
        Ok(Self {
            kind,
            requested_delay,
            payload,
            target,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RequeueTarget {
    Delayed { score: i64, member: Arc<str> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScheduledPublication {
    score: i64,
    payload: Arc<str>,
    member: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreparedSettlement {
    publications: Arc<[ScheduledPublication]>,
}

impl PreparedSettlement {
    fn new<Encode>(
        follow_ups: &[Envelope],
        prepared_at: chrono::DateTime<Utc>,
        mut encode: Encode,
    ) -> Result<Self, FrameworkError>
    where
        Encode: FnMut(&Envelope) -> Result<String, FrameworkError>,
    {
        let publications = follow_ups
            .iter()
            .map(|envelope| {
                let payload = Arc::<str>::from(encode(envelope)?);
                Ok(ScheduledPublication {
                    score: delayed_score(&envelope.available_at, &prepared_at),
                    member: Arc::from(encode_delayed_member(payload.as_ref())),
                    payload,
                })
            })
            .collect::<Result<Vec<_>, FrameworkError>>()?;
        Ok(Self {
            publications: publications.into(),
        })
    }

    fn matches(&self, follow_ups: &[Envelope]) -> Result<bool, FrameworkError> {
        if self.publications.len() != follow_ups.len() {
            return Ok(false);
        }
        for (publication, envelope) in self.publications.iter().zip(follow_ups) {
            let payload = envelope.to_json().map_err(|e| {
                FrameworkError::internal(format!("envelope encode error (settlement retry): {e}"))
            })?;
            if publication.payload.as_ref() != payload {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FenceOutcome {
    Stale,
    Applied,
    PreviouslyApplied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MutationStatus {
    Stale,
    PreviouslyApplied,
    Current,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MutationKind {
    Ack,
    Nack,
    Release,
    Settle,
}

impl MutationKind {
    fn fingerprint_tag(self) -> &'static [u8] {
        match self {
            Self::Ack => b"ack",
            Self::Nack => b"nack",
            Self::Release => b"release",
            Self::Settle => b"settle",
        }
    }
}

struct FencedMutation {
    operation_id: Uuid,
    kind: MutationKind,
    publications: Arc<[ScheduledPublication]>,
}

fn update_fingerprint_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn mutation_fingerprint(fence: &DeliveryFence, mutation: &FencedMutation) -> [u8; 32] {
    let mut hasher = Sha256::new();
    update_fingerprint_field(&mut hasher, b"suprnova-redis-settlement-v1");
    update_fingerprint_field(&mut hasher, mutation.operation_id.as_bytes());
    update_fingerprint_field(&mut hasher, mutation.kind.fingerprint_tag());
    update_fingerprint_field(&mut hasher, fence.epoch.as_bytes());
    update_fingerprint_field(&mut hasher, fence.entry_id.as_bytes());
    update_fingerprint_field(&mut hasher, fence.owner.as_bytes());
    update_fingerprint_field(&mut hasher, &fence.deliveries.to_be_bytes());
    update_fingerprint_field(
        &mut hasher,
        &(mutation.publications.len() as u64).to_be_bytes(),
    );
    for publication in mutation.publications.iter() {
        update_fingerprint_field(&mut hasher, &publication.score.to_be_bytes());
        update_fingerprint_field(&mut hasher, publication.member.as_bytes());
    }
    hasher.finalize().into()
}

fn mutation_for_lifecycle(
    token: &ReservationToken,
    lifecycle: &LifecycleState,
) -> Option<FencedMutation> {
    match lifecycle {
        LifecycleState::Reserved | LifecycleState::AckPending => Some(FencedMutation {
            operation_id: token.0,
            kind: MutationKind::Ack,
            publications: Arc::from([]),
        }),
        LifecycleState::RequeuePending(prepared) => {
            let RequeueTarget::Delayed { score, member } = prepared.target.clone();
            Some(FencedMutation {
                operation_id: token.0,
                kind: match prepared.kind {
                    RequeueKind::Nack => MutationKind::Nack,
                    RequeueKind::Release => MutationKind::Release,
                },
                publications: vec![ScheduledPublication {
                    score,
                    payload: prepared.payload.clone(),
                    member,
                }]
                .into(),
            })
        }
        LifecycleState::SettlementPending(prepared) => Some(FencedMutation {
            operation_id: token.0,
            kind: MutationKind::Settle,
            publications: prepared.publications.clone(),
        }),
        LifecycleState::Settled => None,
    }
}

enum ReconciliationProbe {
    Busy,
    Settled,
    Mutation(FencedMutation),
}

fn reconciliation_probe(
    token: &ReservationToken,
    entry: &Arc<PendingEntry>,
) -> ReconciliationProbe {
    let lifecycle = match entry.lifecycle.try_lock() {
        Ok(lifecycle) => lifecycle,
        Err(_) => return ReconciliationProbe::Busy,
    };
    mutation_for_lifecycle(token, &lifecycle)
        .map_or(ReconciliationProbe::Settled, ReconciliationProbe::Mutation)
}

fn decode_fence_outcome(value: i64) -> Result<FenceOutcome, FrameworkError> {
    match value {
        0 => Ok(FenceOutcome::Stale),
        1 => Ok(FenceOutcome::Applied),
        2 => Ok(FenceOutcome::PreviouslyApplied),
        other => Err(FrameworkError::internal(format!(
            "redis fenced settlement returned unexpected result {other}"
        ))),
    }
}

fn decode_mutation_status(value: i64) -> Result<MutationStatus, FrameworkError> {
    match value {
        0 => Ok(MutationStatus::Stale),
        2 => Ok(MutationStatus::PreviouslyApplied),
        3 => Ok(MutationStatus::Current),
        other => Err(FrameworkError::internal(format!(
            "redis fenced mutation status returned unexpected result {other}"
        ))),
    }
}

fn settlement_receipt_key(stream: &str, operation_id: Uuid) -> String {
    format!("{stream}:settlement-receipt:{operation_id}")
}

fn settlement_receipt_ttl_ms(visibility_timeout: Duration) -> u64 {
    const MIN_TTL_MS: u128 = 60 * 60 * 1_000;
    let ttl = visibility_timeout
        .as_millis()
        .saturating_mul(4)
        .max(MIN_TTL_MS);
    ttl.min(i64::MAX as u128) as u64
}

struct ClaimCursor {
    epoch: String,
    next_id: String,
}

impl Default for ClaimCursor {
    fn default() -> Self {
        Self {
            epoch: String::new(),
            next_id: "0-0".to_string(),
        }
    }
}

impl ClaimCursor {
    fn start_for_epoch(&mut self, epoch: &str) -> &str {
        if self.epoch != epoch {
            self.epoch.clear();
            self.epoch.push_str(epoch);
            self.next_id.clear();
            self.next_id.push_str("0-0");
        }
        &self.next_id
    }

    fn advance(&mut self, epoch: &str, next_id: &str) {
        let _ = self.start_for_epoch(epoch);
        self.next_id.clear();
        self.next_id.push_str(next_id);
    }
}

#[derive(Default)]
struct PendingRegistry {
    by_token: HashMap<Uuid, Arc<PendingEntry>>,
    current_by_entry_id: HashMap<String, Uuid>,
    reconciliation_queue: BTreeSet<(Instant, Uuid)>,
    reconciliation_deadlines: HashMap<Uuid, Instant>,
}

impl PendingRegistry {
    fn schedule_reconciliation(&mut self, token: Uuid, deadline: Instant) {
        self.unschedule_reconciliation(token);
        self.reconciliation_queue.insert((deadline, token));
        self.reconciliation_deadlines.insert(token, deadline);
    }

    fn unschedule_reconciliation(&mut self, token: Uuid) {
        if let Some(deadline) = self.reconciliation_deadlines.remove(&token) {
            self.reconciliation_queue.remove(&(deadline, token));
        }
    }

    fn take_due_reconciliation(&mut self, now: Instant) -> Option<Uuid> {
        let &(deadline, token) = self.reconciliation_queue.first()?;
        if deadline > now {
            return None;
        }
        self.reconciliation_queue.pop_first();
        self.reconciliation_deadlines.remove(&token);
        Some(token)
    }
}

type PendingMap = Mutex<PendingRegistry>;

fn register_pending_delivery(
    pending: &PendingMap,
    envelope: Envelope,
    fence: DeliveryFence,
    lease_deadline: Instant,
) -> Result<Option<Reservation>, FrameworkError> {
    let mut registry = lock::lock(pending, "redis queue pending registry")?;
    if let Some(current) = registry
        .current_by_entry_id
        .get(&fence.entry_id)
        .and_then(|token| registry.by_token.get(token))
        && current.fence.epoch == fence.epoch
        && current.fence.deliveries >= fence.deliveries
    {
        return Ok(None);
    }
    let token = loop {
        let candidate = Uuid::new_v4();
        if !registry.by_token.contains_key(&candidate) {
            break ReservationToken(candidate);
        }
    };
    let entry_id = fence.entry_id.clone();
    let entry = Arc::new(PendingEntry::new(envelope.clone(), fence, lease_deadline));

    if let Some(previous) = registry.current_by_entry_id.insert(entry_id, token.0) {
        registry.by_token.remove(&previous);
        registry.unschedule_reconciliation(previous);
    }
    registry.by_token.insert(token.0, entry);
    registry.schedule_reconciliation(token.0, lease_deadline);

    Ok(Some(Reservation { envelope, token }))
}

fn pending_entry(
    pending: &PendingMap,
    token: &ReservationToken,
) -> Result<Option<Arc<PendingEntry>>, FrameworkError> {
    Ok(lock::lock(pending, "redis queue pending registry")?
        .by_token
        .get(&token.0)
        .cloned())
}

fn pending_entry_is_current(
    pending: &PendingMap,
    token: &ReservationToken,
    entry: &Arc<PendingEntry>,
) -> Result<bool, FrameworkError> {
    Ok(lock::lock(pending, "redis queue pending registry")?
        .by_token
        .get(&token.0)
        .is_some_and(|current| Arc::ptr_eq(current, entry)))
}

fn forget_pending_entry(
    pending: &PendingMap,
    token: &ReservationToken,
    entry: &Arc<PendingEntry>,
) -> Result<(), FrameworkError> {
    let mut registry = lock::lock(pending, "redis queue pending registry")?;
    if registry
        .by_token
        .get(&token.0)
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        registry.by_token.remove(&token.0);
        registry.unschedule_reconciliation(token.0);
        if registry
            .current_by_entry_id
            .get(entry.fence.entry_id.as_str())
            == Some(&token.0)
        {
            registry
                .current_by_entry_id
                .remove(entry.fence.entry_id.as_str());
        }
    }
    Ok(())
}

fn mark_reconciliation_needed(
    pending: &PendingMap,
    token: &ReservationToken,
) -> Result<(), FrameworkError> {
    let mut registry = lock::lock(pending, "redis queue pending registry")?;
    let Some(entry) = registry.by_token.get(&token.0).cloned() else {
        return Ok(());
    };
    if !entry.needs_reconciliation.swap(true, Ordering::AcqRel) {
        registry.schedule_reconciliation(token.0, Instant::now());
    }
    Ok(())
}

fn take_reconciliation_candidate(
    pending: &PendingMap,
    now: Instant,
) -> Result<Option<(ReservationToken, Arc<PendingEntry>)>, FrameworkError> {
    let mut registry = lock::lock(pending, "redis queue pending registry")?;
    let Some(token) = registry.take_due_reconciliation(now) else {
        return Ok(None);
    };
    let Some(entry) = registry.by_token.get(&token).cloned() else {
        return Ok(None);
    };
    if entry.needs_reconciliation.load(Ordering::Acquire) || entry.lease_deadline <= now {
        return Ok(Some((ReservationToken(token), entry)));
    }
    registry.schedule_reconciliation(token, entry.lease_deadline);
    Ok(None)
}

fn reschedule_reconciliation_candidate(
    pending: &PendingMap,
    token: &ReservationToken,
    entry: &Arc<PendingEntry>,
) -> Result<(), FrameworkError> {
    let mut registry = lock::lock(pending, "redis queue pending registry")?;
    if registry
        .by_token
        .get(&token.0)
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        registry.schedule_reconciliation(token.0, Instant::now());
    }
    Ok(())
}

struct ReconciliationCandidate<'a> {
    pending: &'a PendingMap,
    token: ReservationToken,
    entry: Arc<PendingEntry>,
    armed: bool,
}

impl<'a> ReconciliationCandidate<'a> {
    fn new(pending: &'a PendingMap, token: ReservationToken, entry: Arc<PendingEntry>) -> Self {
        Self {
            pending,
            token,
            entry,
            armed: true,
        }
    }

    fn token(&self) -> &ReservationToken {
        &self.token
    }

    fn entry(&self) -> &Arc<PendingEntry> {
        &self.entry
    }

    fn forget(mut self) -> Result<(), FrameworkError> {
        let result = forget_pending_entry(self.pending, &self.token, &self.entry);
        if result.is_ok() {
            self.armed = false;
        }
        result
    }

    fn reschedule(mut self) -> Result<(), FrameworkError> {
        let result = reschedule_reconciliation_candidate(self.pending, &self.token, &self.entry);
        if result.is_ok() {
            self.armed = false;
        }
        result
    }
}

impl Drop for ReconciliationCandidate<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = reschedule_reconciliation_candidate(self.pending, &self.token, &self.entry);
        }
    }
}

fn finish_reconciliation_candidate(
    candidate: ReconciliationCandidate<'_>,
    status: Result<MutationStatus, FrameworkError>,
) -> Result<(), FrameworkError> {
    match status {
        Ok(MutationStatus::Stale | MutationStatus::PreviouslyApplied) => candidate.forget(),
        Ok(MutationStatus::Current) => candidate.reschedule(),
        Err(error) => {
            candidate.reschedule()?;
            Err(error)
        }
    }
}

async fn ack_pending_with<Apply, ApplyFuture>(
    pending: &PendingMap,
    token: &ReservationToken,
    apply: Apply,
) -> Result<(), FrameworkError>
where
    Apply: FnOnce(DeliveryFence) -> ApplyFuture + Send,
    ApplyFuture: Future<Output = Result<FenceOutcome, FrameworkError>> + Send,
{
    let Some(entry) = pending_entry(pending, token)? else {
        return Ok(());
    };
    let mut lifecycle = entry.lifecycle.lock().await;
    if !pending_entry_is_current(pending, token, &entry)? {
        return Ok(());
    }
    match &*lifecycle {
        LifecycleState::Reserved => {
            *lifecycle = LifecycleState::AckPending;
        }
        LifecycleState::AckPending => {}
        LifecycleState::RequeuePending(prepared) => {
            return Err(FrameworkError::internal(format!(
                "redis {} requeue is pending; retry {} before acknowledging",
                prepared.kind.label(),
                prepared.kind.label()
            )));
        }
        LifecycleState::SettlementPending(_) => {
            return Err(FrameworkError::internal(
                "redis atomic settlement is pending; retry settle before acknowledging",
            ));
        }
        LifecycleState::Settled => return forget_pending_entry(pending, token, &entry),
    }

    let _outcome = apply(entry.fence.clone()).await?;
    *lifecycle = LifecycleState::Settled;
    forget_pending_entry(pending, token, &entry)
}

fn ensure_requeue_intent(
    stored_kind: RequeueKind,
    stored_delay: Duration,
    requested_kind: RequeueKind,
    requested_delay: Duration,
) -> Result<(), FrameworkError> {
    if stored_kind == requested_kind && stored_delay == requested_delay {
        return Ok(());
    }
    Err(FrameworkError::internal(format!(
        "redis reservation is already being settled by {} with delay {:?}; retry that operation \
         instead of {} with delay {:?}",
        stored_kind.label(),
        stored_delay,
        requested_kind.label(),
        requested_delay
    )))
}

struct RequeueRequest {
    delay: Duration,
    kind: RequeueKind,
    requested_at: chrono::DateTime<Utc>,
}

async fn requeue_pending_with<Encode, Apply, ApplyFuture>(
    pending: &PendingMap,
    token: &ReservationToken,
    request: RequeueRequest,
    encode: Encode,
    apply: Apply,
) -> Result<(), FrameworkError>
where
    Encode: FnOnce(&Envelope) -> Result<String, FrameworkError> + Send,
    Apply: FnOnce(DeliveryFence, PreparedRequeue) -> ApplyFuture + Send,
    ApplyFuture: Future<Output = Result<FenceOutcome, FrameworkError>> + Send,
{
    let RequeueRequest {
        delay: requested_delay,
        kind: requested_kind,
        requested_at,
    } = request;
    let Some(entry) = pending_entry(pending, token)? else {
        return Ok(());
    };
    let mut lifecycle = entry.lifecycle.lock().await;
    if !pending_entry_is_current(pending, token, &entry)? {
        return Ok(());
    }

    let prepared = match &*lifecycle {
        LifecycleState::Reserved => {
            let prepared = PreparedRequeue::new(
                &entry.envelope,
                requested_delay,
                requested_kind,
                requested_at,
                encode,
            )?;
            *lifecycle = LifecycleState::RequeuePending(prepared.clone());
            prepared
        }
        LifecycleState::AckPending => {
            return Err(FrameworkError::internal(
                "redis acknowledgement is pending; retry ack instead of requeueing",
            ));
        }
        LifecycleState::RequeuePending(prepared) => {
            ensure_requeue_intent(
                prepared.kind,
                prepared.requested_delay,
                requested_kind,
                requested_delay,
            )?;
            prepared.clone()
        }
        LifecycleState::SettlementPending(_) => {
            return Err(FrameworkError::internal(
                "redis atomic settlement is pending; retry settle instead of requeueing",
            ));
        }
        LifecycleState::Settled => {
            drop(lifecycle);
            return forget_pending_entry(pending, token, &entry);
        }
    };

    let _outcome = apply(entry.fence.clone(), prepared).await?;
    *lifecycle = LifecycleState::Settled;
    forget_pending_entry(pending, token, &entry)
}

async fn settle_pending_with<Encode, Apply, ApplyFuture>(
    pending: &PendingMap,
    token: &ReservationToken,
    follow_ups: &[Envelope],
    encode: Encode,
    apply: Apply,
) -> Result<Settled, FrameworkError>
where
    Encode: FnMut(&Envelope) -> Result<String, FrameworkError> + Send,
    Apply: FnOnce(DeliveryFence, PreparedSettlement) -> ApplyFuture + Send,
    ApplyFuture: Future<Output = Result<FenceOutcome, FrameworkError>> + Send,
{
    let Some(entry) = pending_entry(pending, token)? else {
        return Ok(Settled::Stale);
    };
    let mut lifecycle = entry.lifecycle.lock().await;
    if !pending_entry_is_current(pending, token, &entry)? {
        return Ok(Settled::Stale);
    }
    validate_atomic_follow_up_count(follow_ups.len())?;

    let prepared = match &*lifecycle {
        LifecycleState::Reserved => {
            let prepared = PreparedSettlement::new(follow_ups, Utc::now(), encode)?;
            *lifecycle = LifecycleState::SettlementPending(prepared.clone());
            prepared
        }
        LifecycleState::AckPending => {
            return Err(FrameworkError::internal(
                "redis acknowledgement is pending; retry ack instead of settling",
            ));
        }
        LifecycleState::SettlementPending(prepared) => {
            if !prepared.matches(follow_ups)? {
                return Err(FrameworkError::internal(
                    "redis atomic settlement is already pending with different follow-ups",
                ));
            }
            prepared.clone()
        }
        LifecycleState::RequeuePending(prepared) => {
            return Err(FrameworkError::internal(format!(
                "redis {} requeue is pending; retry {} instead of settling",
                prepared.kind.label(),
                prepared.kind.label()
            )));
        }
        LifecycleState::Settled => {
            drop(lifecycle);
            forget_pending_entry(pending, token, &entry)?;
            return Ok(Settled::Stale);
        }
    };

    let outcome = apply(entry.fence.clone(), prepared).await?;
    *lifecycle = LifecycleState::Settled;
    forget_pending_entry(pending, token, &entry)?;
    Ok(match outcome {
        FenceOutcome::Applied | FenceOutcome::PreviouslyApplied => Settled::Atomically,
        FenceOutcome::Stale => Settled::Stale,
    })
}

struct PendingMetadata {
    fence: DeliveryFence,
    idle: Duration,
}

struct PendingLease {
    fence: DeliveryFence,
    deadline: Instant,
}

struct VerifiedPendingDelivery {
    fence: DeliveryFence,
    deadline: Instant,
    payload: String,
}

struct PendingSnapshot {
    epoch: String,
    metadata: Option<PendingMetadata>,
    payload: Option<String>,
}

struct PendingSnapshotRead {
    response: redis::Value,
    query_started: Instant,
    observed_at: Instant,
}

fn parse_pending_metadata(value: &redis::Value) -> Option<PendingMetadata> {
    let rows = match value {
        redis::Value::Array(rows) => rows,
        _ => return None,
    };
    let cells = match rows.first()? {
        redis::Value::Array(cells) => cells,
        _ => return None,
    };
    let entry_id = redis::from_redis_value_ref::<String>(cells.first()?).ok()?;
    let owner = redis::from_redis_value_ref::<String>(cells.get(1)?).ok()?;
    let idle_ms = redis::from_redis_value_ref::<u64>(cells.get(2)?).ok()?;
    let deliveries = redis::from_redis_value_ref::<u32>(cells.get(3)?).ok()?;
    if deliveries == 0 {
        return None;
    }
    Some(PendingMetadata {
        fence: DeliveryFence {
            epoch: String::new(),
            entry_id,
            owner,
            deliveries,
        },
        idle: Duration::from_millis(idle_ms),
    })
}

fn parse_pending_snapshot(value: &redis::Value) -> Option<PendingSnapshot> {
    let cells = match value {
        redis::Value::Array(cells) if cells.len() == 3 => cells,
        _ => return None,
    };
    let epoch = redis::from_redis_value_ref::<String>(cells.first()?).ok()?;
    let metadata = parse_pending_metadata(cells.get(1)?).map(|mut metadata| {
        metadata.fence.epoch = epoch.clone();
        metadata
    });
    let payload = redis::from_redis_value_ref::<String>(cells.get(2)?).ok();
    Some(PendingSnapshot {
        epoch,
        metadata,
        payload,
    })
}

struct RawDelivery {
    entry_id: String,
    payload: String,
}

fn raw_delivery_from_stream_id(
    mut entry: redis::streams::StreamId,
) -> Result<RawDelivery, FrameworkError> {
    let payload = entry.map.remove("msg").ok_or_else(|| {
        FrameworkError::internal(format!(
            "redis claimed stream entry {} has no msg field",
            entry.id
        ))
    })?;
    let payload = redis::from_redis_value::<String>(payload).map_err(|e| {
        FrameworkError::internal(format!(
            "redis claimed stream entry {} has invalid msg field: {e}",
            entry.id
        ))
    })?;
    Ok(RawDelivery {
        entry_id: entry.id,
        payload,
    })
}

fn current_pending_lease(
    metadata: &PendingMetadata,
    expected_entry_id: &str,
    consumer_id: &str,
    query_started: Instant,
    configured_timeout: Duration,
    now: Instant,
) -> Option<PendingLease> {
    if metadata.fence.entry_id != expected_entry_id || metadata.fence.owner != consumer_id {
        return None;
    }
    let remaining = configured_timeout.saturating_sub(metadata.idle);
    let deadline = query_started
        .checked_add(remaining)
        .unwrap_or(query_started);
    (deadline > now).then_some(PendingLease {
        fence: metadata.fence.clone(),
        deadline,
    })
}

fn verified_pending_snapshot(
    response: Option<&redis::Value>,
    expected_epoch: &str,
    expected_entry_id: &str,
    consumer_id: &str,
    query_started: Instant,
    configured_timeout: Duration,
    now: Instant,
) -> Option<VerifiedPendingDelivery> {
    let snapshot = parse_pending_snapshot(response?)?;
    if snapshot.epoch != expected_epoch {
        return None;
    }
    let lease = current_pending_lease(
        snapshot.metadata.as_ref()?,
        expected_entry_id,
        consumer_id,
        query_started,
        configured_timeout,
        now,
    )?;
    Some(VerifiedPendingDelivery {
        fence: lease.fence,
        deadline: lease.deadline,
        payload: snapshot.payload?,
    })
}

async fn verified_pending_delivery_with_revalidation<ReadSnapshot, SnapshotFuture>(
    expected_epoch: &str,
    expected_entry_id: &str,
    delivered_payload: &str,
    consumer_id: &str,
    configured_timeout: Duration,
    mut read_snapshot: ReadSnapshot,
) -> Result<Option<VerifiedPendingDelivery>, FrameworkError>
where
    ReadSnapshot: FnMut() -> SnapshotFuture,
    SnapshotFuture: Future<Output = Result<PendingSnapshotRead, FrameworkError>>,
{
    let first = read_snapshot().await?;
    let Some(first_snapshot) = parse_pending_snapshot(&first.response) else {
        return Ok(None);
    };
    if first_snapshot.epoch == expected_epoch {
        let verified = verified_pending_snapshot(
            Some(&first.response),
            expected_epoch,
            expected_entry_id,
            consumer_id,
            first.query_started,
            configured_timeout,
            first.observed_at,
        );
        return Ok(verified.filter(|delivery| delivery.payload == delivered_payload));
    }

    // A clear may have completed before Redis assigned this delivery. Re-read
    // once against the newly observed epoch and use only the payload returned
    // by that atomic epoch + XPENDING + XRANGE snapshot.
    let rebound_epoch = first_snapshot.epoch;
    let second = read_snapshot().await?;
    Ok(verified_pending_snapshot(
        Some(&second.response),
        &rebound_epoch,
        expected_entry_id,
        consumer_id,
        second.query_started,
        configured_timeout,
        second.observed_at,
    ))
}

/// Redis-backed queue driver.
///
/// Construct via [`RedisQueueDriver::connect`]. The driver is `Send + Sync`
/// and can be wrapped in an `Arc` for sharing across tasks.
pub struct RedisQueueDriver {
    producer: RedisProducer,
    stream_key: StreamKey,
    /// `<stream>:delayed` - the sorted set holding envelopes whose
    /// `available_at` is still in the future. Promoted into the stream by
    /// every `pop` via `PROMOTE_DUE_SCRIPT`.
    delayed_key: String,
    /// Durable generation marker changed by every successful `clear`.
    epoch_key: String,
    /// Consumer-group name. Captured at construction so the introspection
    /// methods (`reserved_size`, `pending_size`) can scope XPENDING queries
    /// to the same group the consumer reads from.
    group_name: String,
    /// Consumer identity used to fence a message that another consumer
    /// reclaims between `next()` and the authoritative XPENDING lookup.
    consumer_id: String,
    /// Connection-scoped XAUTOCLAIM idle threshold. Redis cannot vary this
    /// lease per `pop` call, so reservation aliases must use this value.
    visibility_timeout: Duration,
    /// Dedicated connection for bounded blocking `XREADGROUP` calls. Keeping
    /// it separate prevents one empty poll from delaying settlement commands.
    read_conn: ConnectionManager,
    /// Direct Redis connection used for delayed ZSET operations and Lua-backed
    /// promotion/settlement. Sea-streamer's consumer API is intentionally
    /// bypassed; its producer remains responsible for immediate `XADD`. The
    /// `ConnectionManager` is cheap to clone (internally a multiplexed
    /// connection plus an Arc-shared task) and is what the `redis` crate
    /// recommends for high-throughput async use.
    conn: ConnectionManager,
    /// Per-token lifecycle plus the current token for each stream entry.
    /// Entries survive command errors, but are removed after Redis reports
    /// either an applied transition or a stale delivery generation.
    pending: PendingMap,
    /// Coordinates local deliveries and settlements with destructive clears.
    operations: tokio::sync::RwLock<()>,
    /// Redis XAUTOCLAIM scan position, serialized across concurrent pops.
    claim_cursor: tokio::sync::Mutex<ClaimCursor>,
    /// Serializes delivery acquisition through local token registration.
    delivery_gate: tokio::sync::Mutex<()>,
}

fn redis_info_field<'a>(info: &'a str, key: &str) -> Option<&'a str> {
    info.lines().find_map(|line| {
        let (candidate, value) = line.trim_end_matches('\r').split_once(':')?;
        (candidate == key).then_some(value)
    })
}

fn validate_redis_server_info(server_info: &str, cluster_info: &str) -> Result<(), FrameworkError> {
    let version = redis_info_field(server_info, "redis_version")
        .ok_or_else(|| FrameworkError::internal("redis INFO server omitted redis_version"))?;
    let mut components = version.split('.');
    let major = components
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| FrameworkError::internal(format!("invalid Redis version `{version}`")))?;
    let minor = components
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| FrameworkError::internal(format!("invalid Redis version `{version}`")))?;
    if (major, minor) < (6, 2) {
        return Err(FrameworkError::internal(format!(
            "Redis 6.2 or newer is required for queue XAUTOCLAIM; server reports {version}"
        )));
    }

    match redis_info_field(cluster_info, "cluster_enabled") {
        Some("0") => Ok(()),
        Some("1") => Err(FrameworkError::internal(
            "Redis Cluster is not supported by the queue driver's standalone multi-key Lua connection",
        )),
        Some(value) => Err(FrameworkError::internal(format!(
            "invalid Redis cluster_enabled value `{value}`"
        ))),
        None => Err(FrameworkError::internal(
            "redis INFO cluster omitted cluster_enabled",
        )),
    }
}

fn validate_redis_visibility_timeout(visibility_timeout: Duration) -> Result<(), FrameworkError> {
    let milliseconds = visibility_timeout.as_millis();
    if milliseconds == 0 {
        return Err(FrameworkError::internal(
            "Redis queue visibility timeout must be at least 1 millisecond",
        ));
    }
    if milliseconds > (i64::MAX as u128) / 4
        || Instant::now().checked_add(visibility_timeout).is_none()
    {
        return Err(FrameworkError::internal(
            "Redis queue visibility timeout exceeds the supported Redis range",
        ));
    }
    if !visibility_timeout.subsec_nanos().is_multiple_of(1_000_000) {
        return Err(FrameworkError::internal(
            "Redis queue visibility timeout must use whole milliseconds",
        ));
    }
    Ok(())
}

fn new_delivery_read_command(
    stream: &str,
    group: &str,
    consumer_id: &str,
    block_for: Duration,
) -> redis::Cmd {
    let block_ms = block_for.as_millis().clamp(1, u64::MAX as u128) as u64;
    let mut command = redis::cmd("XREADGROUP");
    command
        .arg("GROUP")
        .arg(group)
        .arg(consumer_id)
        .arg("COUNT")
        .arg(1)
        .arg("BLOCK")
        .arg(block_ms)
        .arg("STREAMS")
        .arg(stream)
        .arg(">");
    command
}

impl RedisQueueDriver {
    /// Connect to Redis and initialize the producer, direct read connections,
    /// and consumer group.
    ///
    /// # Arguments
    ///
    /// * `url` - Redis URL, e.g. `"redis://127.0.0.1:6379"`.
    /// * `stream` - Redis stream key name.
    /// * `group` - Consumer group name (created with `MKSTREAM` if absent).
    /// * `consumer_id` - Unique consumer ID within the group.
    /// * `visibility_timeout` - How long a message can remain unacknowledged
    ///   before another consumer may re-claim it (`XAUTOCLAIM` idle threshold).
    ///   Must be at least one whole millisecond, Redis's time resolution.
    pub async fn connect(
        url: &str,
        stream: &str,
        group: &str,
        consumer_id: &str,
        visibility_timeout: Duration,
    ) -> Result<Self, FrameworkError> {
        validate_redis_visibility_timeout(visibility_timeout)?;

        let uri = StreamerUri::from_str(url)
            .map_err(|e| FrameworkError::internal(format!("redis URI parse error: {e}")))?;
        let stream_key = StreamKey::new(stream)
            .map_err(|e| FrameworkError::internal(format!("redis stream key error: {e}")))?;

        // Validate the direct-command backend before the producer creates any
        // queue state. One manager is reserved for blocking XREADGROUP; the
        // other remains available for claims, settlement, and inspection.
        let client = redis::Client::open(url)
            .map_err(|e| FrameworkError::internal(format!("redis client open: {e}")))?;
        let conn = ConnectionManager::new(client.clone())
            .await
            .map_err(|e| FrameworkError::internal(format!("redis command connection: {e}")))?;
        let read_conn = ConnectionManager::new(client)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis read connection: {e}")))?;
        let mut capability_conn = conn.clone();
        let server_info: String = redis::cmd("INFO")
            .arg("server")
            .query_async(&mut capability_conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis INFO server: {e}")))?;
        let cluster_info: String = redis::cmd("INFO")
            .arg("cluster")
            .query_async(&mut capability_conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis INFO cluster: {e}")))?;
        validate_redis_server_info(&server_info, &cluster_info)?;

        let streamer = RedisStreamer::connect(uri, Default::default())
            .await
            .map_err(|e| FrameworkError::internal(format!("redis connect error: {e}")))?;
        // The producer is not anchored; push names the stream explicitly.
        let producer: RedisProducer = streamer
            .create_generic_producer(Default::default())
            .await
            .map_err(|e| FrameworkError::internal(format!("redis producer error: {e}")))?;

        let delayed_key = format!("{}:delayed", stream);
        let epoch_key = format!("{}:epoch", stream);

        let driver = Self {
            producer,
            stream_key,
            delayed_key,
            epoch_key,
            group_name: group.to_string(),
            consumer_id: consumer_id.to_string(),
            visibility_timeout,
            read_conn,
            conn,
            pending: Mutex::new(PendingRegistry::default()),
            operations: tokio::sync::RwLock::new(()),
            claim_cursor: tokio::sync::Mutex::new(ClaimCursor::default()),
            delivery_gate: tokio::sync::Mutex::new(()),
        };
        driver.ensure_consumer_group().await?;
        Ok(driver)
    }

    /// Run the promotion Lua script to flush all due delayed entries onto the
    /// stream. Called from `pop` on every entry; cheap on an empty ZSET.
    async fn promote_due(&self) -> Result<(), FrameworkError> {
        let now = Utc::now().timestamp();
        let stream_name = self.stream_key.name();
        let script = redis::Script::new(PROMOTE_DUE_SCRIPT);
        // Never retried: the script XADDs promoted entries onto the stream, so
        // a second execution after a dropped connection would deliver the same
        // delayed job twice.
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
    /// The member prefix keeps repeated pushes of byte-identical envelopes distinct.
    async fn zadd_delayed(&self, env: &Envelope) -> Result<(), FrameworkError> {
        let json = env
            .to_json()
            .map_err(|e| FrameworkError::internal(format!("envelope encode error: {e}")))?;
        let member = encode_delayed_member(&json);
        let prepared_at = Utc::now();
        let score = delayed_score(&env.available_at, &prepared_at);
        let mut conn = self.conn.clone();
        let _added: i64 = conn
            .zadd(&self.delayed_key, member, score)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis ZADD delayed: {e}")))?;
        Ok(())
    }

    async fn queue_epoch(&self) -> Result<String, FrameworkError> {
        let epoch: Option<String> = crate::redis_retry::retry_read("queue epoch", || {
            let mut conn = self.conn.clone();
            let epoch_key = self.epoch_key.clone();
            async move { conn.get(epoch_key).await }
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("redis GET queue epoch: {e}")))?;
        Ok(epoch.unwrap_or_default())
    }

    async fn ensure_consumer_group(&self) -> Result<(), FrameworkError> {
        let mut conn = self.conn.clone();
        let result: redis::RedisResult<()> = conn
            .xgroup_create_mkstream(self.stream_key.name(), &self.group_name, "0")
            .await;
        match result {
            Ok(()) => Ok(()),
            Err(error) if is_busy_group(&error) => Ok(()),
            Err(error) => Err(FrameworkError::internal(format!(
                "redis XGROUP CREATE MKSTREAM: {error}"
            ))),
        }
    }

    async fn read_new_delivery(
        &self,
        block_for: Duration,
    ) -> Result<Option<RawDelivery>, FrameworkError> {
        let mut conn = self.read_conn.clone();
        let result: redis::RedisResult<StreamReadReply> = new_delivery_read_command(
            self.stream_key.name(),
            &self.group_name,
            &self.consumer_id,
            block_for,
        )
        .query_async(&mut conn)
        .await;
        let reply = match result {
            Ok(reply) => reply,
            Err(error) if is_missing_stream_or_group(&error) => {
                self.ensure_consumer_group().await?;
                return Ok(None);
            }
            Err(error) => {
                return Err(FrameworkError::internal(format!(
                    "redis XREADGROUP: {error}"
                )));
            }
        };

        let mut entries = reply.keys.into_iter().flat_map(|stream| stream.ids);
        let Some(entry) = entries.next() else {
            return Ok(None);
        };
        if entries.next().is_some() {
            return Err(FrameworkError::internal(
                "redis XREADGROUP returned more than the requested single delivery",
            ));
        }
        raw_delivery_from_stream_id(entry).map(Some)
    }

    async fn claim_expired(&self, epoch: &str) -> Result<Option<RawDelivery>, FrameworkError> {
        let mut cursor = self.claim_cursor.lock().await;
        let start = cursor.start_for_epoch(epoch).to_string();
        let min_idle_ms = self.visibility_timeout.as_millis().min(u64::MAX as u128) as u64;
        let mut conn = self.conn.clone();
        let result: redis::RedisResult<StreamAutoClaimReply> = conn
            .xautoclaim_options(
                self.stream_key.name(),
                &self.group_name,
                &self.consumer_id,
                min_idle_ms,
                &start,
                StreamAutoClaimOptions::default().count(1),
            )
            .await;
        let reply = match result {
            Ok(reply) => reply,
            Err(error) if is_missing_stream_or_group(&error) => {
                cursor.advance(epoch, "0-0");
                drop(cursor);
                self.ensure_consumer_group().await?;
                return Ok(None);
            }
            Err(error) => {
                return Err(FrameworkError::internal(format!(
                    "redis XAUTOCLAIM: {error}"
                )));
            }
        };
        cursor.advance(epoch, &reply.next_stream_id);
        drop(cursor);

        reply
            .claimed
            .into_iter()
            .next()
            .map(raw_delivery_from_stream_id)
            .transpose()
    }

    async fn reconcile_one_pending_entry(&self) -> Result<(), FrameworkError> {
        let Some((token, entry)) = take_reconciliation_candidate(&self.pending, Instant::now())?
        else {
            return Ok(());
        };
        let candidate = ReconciliationCandidate::new(&self.pending, token, entry);
        let mutation = match reconciliation_probe(candidate.token(), candidate.entry()) {
            ReconciliationProbe::Busy => {
                return candidate.reschedule();
            }
            ReconciliationProbe::Settled => {
                return candidate.forget();
            }
            ReconciliationProbe::Mutation(mutation) => mutation,
        };

        let status = self
            .mutation_status(&candidate.entry().fence, &mutation)
            .await;
        finish_reconciliation_candidate(candidate, status)
    }

    async fn apply_fenced_mutation(
        &self,
        fence: DeliveryFence,
        mutation: FencedMutation,
        operation: &'static str,
    ) -> Result<FenceOutcome, FrameworkError> {
        validate_atomic_follow_up_count(mutation.publications.len())?;
        let fingerprint = mutation_fingerprint(&fence, &mutation);
        let receipt_key = settlement_receipt_key(self.stream_key.name(), mutation.operation_id);
        let script = redis::Script::new(FENCED_SETTLEMENT_SCRIPT);
        let mut invocation = script.prepare_invoke();
        invocation
            .key(self.stream_key.name())
            .key(&self.delayed_key)
            .key(&self.epoch_key)
            .key(&receipt_key)
            .arg(&self.group_name)
            .arg(&fence.entry_id)
            .arg(&fence.owner)
            .arg(fence.deliveries)
            .arg(&fence.epoch)
            .arg(fingerprint.as_slice())
            .arg(settlement_receipt_ttl_ms(self.visibility_timeout))
            .arg(mutation.publications.len());

        for publication in mutation.publications.iter() {
            invocation
                .arg(publication.score)
                .arg(publication.member.as_ref());
        }

        let mut conn = self.conn.clone();
        match invocation.invoke_async::<i64>(&mut conn).await {
            Ok(result) => decode_fence_outcome(result),
            Err(error) => {
                let original = FrameworkError::internal(format!(
                    "redis {operation} fenced settlement EVAL: {error}"
                ));
                match self.mutation_status(&fence, &mutation).await {
                    Ok(MutationStatus::PreviouslyApplied) => Ok(FenceOutcome::PreviouslyApplied),
                    Ok(MutationStatus::Stale) => Ok(FenceOutcome::Stale),
                    Ok(MutationStatus::Current) | Err(_) => {
                        if let Err(error) = mark_reconciliation_needed(
                            &self.pending,
                            &ReservationToken(mutation.operation_id),
                        ) {
                            tracing::debug!(
                                error = %error,
                                "redis settlement reconciliation could not be scheduled"
                            );
                        }
                        Err(original)
                    }
                }
            }
        }
    }

    async fn mutation_status(
        &self,
        fence: &DeliveryFence,
        mutation: &FencedMutation,
    ) -> Result<MutationStatus, FrameworkError> {
        let fingerprint = mutation_fingerprint(fence, mutation);
        let receipt_key = settlement_receipt_key(self.stream_key.name(), mutation.operation_id);
        let script = redis::Script::new(MUTATION_STATUS_SCRIPT);
        let mut conn = self.conn.clone();
        let result = script
            .key(self.stream_key.name())
            .key(&self.epoch_key)
            .key(receipt_key)
            .arg(&self.group_name)
            .arg(&fence.entry_id)
            .arg(&fence.owner)
            .arg(fence.deliveries)
            .arg(&fence.epoch)
            .arg(fingerprint.as_slice())
            .invoke_async::<i64>(&mut conn)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "redis fenced settlement reconciliation EVAL: {e}"
                ))
            })?;
        decode_mutation_status(result)
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

    /// Poll for the next message. Returns `None` after one bounded 100 ms
    /// probe when no message is ready, allowing an aggregate driver to move
    /// promptly to its next connection.
    ///
    /// `visibility_timeout` is the requested lease from the generic trait. The
    /// connection-scoped XAUTOCLAIM timeout remains authoritative for Redis;
    /// neither value extends this call's fixed probe budget.
    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        // Acquire before the operations read lock so queued pop calls do not
        // delay a clear that is already waiting for exclusive access.
        let _delivery_guard = self.delivery_gate.lock().await;
        let _operation_guard = self.operations.read().await;
        self.promote_due().await?;
        if let Err(error) = self.reconcile_one_pending_entry().await {
            tracing::debug!(
                error = %error,
                "redis pending-entry reconciliation deferred"
            );
        }

        let epoch = self.queue_epoch().await?;
        let delivery = if let Some(delivery) = self.claim_expired(&epoch).await? {
            delivery
        } else {
            let Some(delivery) = self
                .read_new_delivery(pop_probe_budget(visibility_timeout))
                .await?
            else {
                return Ok(None);
            };
            delivery
        };

        let Some(pending_delivery) = self
            .pending_lease(&delivery.entry_id, &delivery.payload, &epoch)
            .await?
        else {
            return Ok(None);
        };
        let mut envelope = Envelope::from_json(&pending_delivery.payload)
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
        let redeliveries = pending_delivery.fence.deliveries.saturating_sub(1);
        envelope.attempts = envelope.attempts.saturating_add(redeliveries);

        register_pending_delivery(
            &self.pending,
            envelope,
            pending_delivery.fence,
            pending_delivery.deadline,
        )
    }

    /// Acknowledge a previously popped message, removing it from the PEL.
    ///
    /// Idempotent: unknown / already-acked tokens are silently ignored.
    ///
    /// The acknowledgement is applied only while Redis still reports the
    /// exact consumer and delivery generation captured by [`pop`](Self::pop).
    /// A reclaimed or already-settled delivery is an idempotent no-op.
    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        let _operation_guard = self.operations.read().await;
        ack_pending_with(&self.pending, token, |fence| {
            self.apply_fenced_mutation(
                fence,
                FencedMutation {
                    operation_id: token.0,
                    kind: MutationKind::Ack,
                    publications: Arc::from([]),
                },
                "ack",
            )
        })
        .await
    }

    /// Atomically publish chain follow-ups and acknowledge this reservation.
    ///
    /// Follow-ups are staged in the delayed ZSET, including ones already due;
    /// the next `pop` promotes them before reading the stream. Keeping every
    /// publication in one Lua mutation lets the script fence the whole
    /// transition against the exact PEL delivery generation.
    async fn settle(
        &self,
        token: &ReservationToken,
        follow_ups: &[Envelope],
    ) -> Result<Settled, FrameworkError> {
        let _operation_guard = self.operations.read().await;
        settle_pending_with(
            &self.pending,
            token,
            follow_ups,
            |envelope| {
                envelope.to_json().map_err(|e| {
                    FrameworkError::internal(format!("envelope encode error (settle): {e}"))
                })
            },
            |fence, prepared| {
                self.apply_fenced_mutation(
                    fence,
                    FencedMutation {
                        operation_id: token.0,
                        kind: MutationKind::Settle,
                        publications: prepared.publications,
                    },
                    "settle",
                )
            },
        )
        .await
    }

    /// Return a message to the queue with incremented `attempts` and an
    /// optional delay before it becomes visible again.
    ///
    /// Implementation:
    /// 1. Lock the retained reservation lifecycle for this token.
    /// 2. Prepare the successor once, bumping attempts for `nack` only and
    ///    freezing its payload and availability timestamp across retries.
    /// 3. Under one Redis Lua script, verify the exact PEL generation,
    ///    re-publish the modified envelope, and acknowledge the original.
    /// 4. Remove the local entry after Redis reports either `Applied` or
    ///    `Stale`; command errors retain the frozen intent for retry.
    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, requeue_delay, RequeueKind::Nack).await
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
        self.requeue(token, delay, RequeueKind::Release).await
    }

    fn queue_filter_capability(&self) -> QueueFilterCapability {
        QueueFilterCapability::Unsupported
    }

    fn reservation_deadline(
        &self,
        token: &ReservationToken,
        _fallback_deadline: Instant,
    ) -> Instant {
        lock::lock(&self.pending, "redis queue pending map")
            .ok()
            .and_then(|pending| {
                pending
                    .by_token
                    .get(&token.0)
                    .map(|entry| entry.lease_deadline)
            })
            .unwrap_or_else(Instant::now)
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
        pending_size_from_counts(stream_len, self.reserved_size().await)
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
    /// The same snapshot caveat that [`pending_size`](Self::pending_size)
    /// documents applies: the cursor is read once (`XINFO GROUPS`), then the scan
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
    /// New-delivery reads use `XREADGROUP COUNT 1 ... >` directly and do not
    /// maintain a background read-ahead buffer. A concurrent consumer can still
    /// advance the group cursor after this snapshot begins.
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
            .filter_map(
                |member| match Envelope::from_json(delayed_member_payload(member)) {
                    Ok(env) => Some(env),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "redis delayed_jobs: unparseable ZSET member; skipping"
                        );
                        None
                    }
                },
            )
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
        Ok(g.by_token
            .values()
            .map(|entry| &entry.envelope)
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
    /// re-created on the next `pop` before the driver asks for new work.
    async fn clear(&self) -> Result<u64, FrameworkError> {
        let _operation_guard = self.operations.write().await;
        let mut conn = self.conn.clone();
        let next_epoch = Uuid::new_v4().to_string();
        let (stream_len, delayed): (u64, u64) = redis::Script::new(CLEAR_SCRIPT)
            .key(self.stream_key.name())
            .key(&self.delayed_key)
            .key(&self.epoch_key)
            .arg(&next_epoch)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("redis clear EVAL: {e}")))?;

        {
            let mut registry = lock::lock(&self.pending, "redis queue pending map")?;
            registry.by_token.clear();
            registry.current_by_entry_id.clear();
            registry.reconciliation_queue.clear();
            registry.reconciliation_deadlines.clear();
        }
        *self.claim_cursor.lock().await = ClaimCursor::default();

        Ok(stream_len.saturating_add(delayed))
    }

    fn name(&self) -> &'static str {
        "redis-streams"
    }
}

impl RedisQueueDriver {
    /// Shared retryable body of [`QueueDriver::nack`] and
    /// [`QueueDriver::release`], which differ only in whether preparation
    /// consumes an attempt.
    async fn requeue(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
        kind: RequeueKind,
    ) -> Result<(), FrameworkError> {
        let _operation_guard = self.operations.read().await;
        requeue_pending_with(
            &self.pending,
            token,
            RequeueRequest {
                delay: requeue_delay,
                kind,
                requested_at: Utc::now(),
            },
            |envelope| {
                envelope.to_json().map_err(|e| {
                    FrameworkError::internal(format!(
                        "envelope encode error ({}): {e}",
                        kind.label()
                    ))
                })
            },
            |fence, prepared| {
                let operation = prepared.kind.label();
                let RequeueTarget::Delayed { score, member } = prepared.target;
                let mutation = FencedMutation {
                    operation_id: token.0,
                    kind: match prepared.kind {
                        RequeueKind::Nack => MutationKind::Nack,
                        RequeueKind::Release => MutationKind::Release,
                    },
                    publications: vec![ScheduledPublication {
                        score,
                        payload: prepared.payload,
                        member,
                    }]
                    .into(),
                };
                self.apply_fenced_mutation(fence, mutation, operation)
            },
        )
        .await
    }

    /// `XLEN <stream>` - total entries currently held by the stream
    /// (including acknowledged-but-not-trimmed ones).
    async fn xlen_stream(&self) -> Result<u64, FrameworkError> {
        // XLEN is a pure read.
        let n: i64 = crate::redis_retry::retry_read("queue XLEN", || {
            let mut conn = self.conn.clone();
            let stream = self.stream_key.name().to_string();
            async move { redis::cmd("XLEN").arg(&stream).query_async(&mut conn).await }
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("redis XLEN: {e}")))?;
        Ok(n.max(0) as u64)
    }

    /// `ZCARD <stream>:delayed` - entries parked awaiting their
    /// `available_at` deadline.
    async fn zcard_delayed(&self) -> Result<u64, FrameworkError> {
        // ZCARD is a pure read.
        let n: i64 = crate::redis_retry::retry_read("queue ZCARD", || {
            let mut conn = self.conn.clone();
            let delayed_key = self.delayed_key.clone();
            async move {
                redis::cmd("ZCARD")
                    .arg(&delayed_key)
                    .query_async(&mut conn)
                    .await
            }
        })
        .await
        .map_err(|e| FrameworkError::internal(format!("redis ZCARD delayed: {e}")))?;
        Ok(n.max(0) as u64)
    }

    /// Read the authoritative owner, idle time, and delivery count for one
    /// stream entry, then derive its remaining connection-scoped lease.
    ///
    /// `XPENDING <key> <group> IDLE 0 <id> <id> 1` returns one row per
    /// entry in the form `[id, consumer, idle-ms, delivery-count]`. The
    /// count is 1 on a first delivery and rises with every XAUTOCLAIM. The
    /// deadline is anchored at the instant immediately before this query, not
    /// response time, so a slow lookup cannot extend the reservation.
    ///
    /// # Cost
    ///
    /// One atomic snapshot command per delivered entry, on the already-
    /// multiplexed connection. If a concurrent `clear` changes the epoch, one
    /// additional snapshot binds the recreated entry's current payload and PEL
    /// generation. Paying this cost supplies the only authoritative signal that
    /// distinguishes a first delivery from a worker-loss redelivery and prevents
    /// old buffered bytes from being attached to a recreated stream ID.
    ///
    /// A command failure, missing/malformed row, changed owner, or elapsed
    /// deadline returns `None`; the caller must not run work it cannot prove
    /// this consumer still owns. In particular, a failed lookup cannot replace
    /// Redis's measured idle time with a fresh local timeout and thereby extend
    /// a reservation whose lease may already be partly consumed.
    async fn read_pending_snapshot(
        &self,
        entry_id: &str,
    ) -> Result<PendingSnapshotRead, FrameworkError> {
        let query_started = Instant::now();
        let response: redis::Value =
            crate::redis_retry::retry_read("queue delivery snapshot", || {
                let mut conn = self.conn.clone();
                let stream = self.stream_key.name().to_string();
                let epoch_key = self.epoch_key.clone();
                let group = self.group_name.clone();
                let entry_id = entry_id.to_string();
                async move {
                    redis::Script::new(PENDING_SNAPSHOT_SCRIPT)
                        .key(stream)
                        .key(epoch_key)
                        .arg(group)
                        .arg(entry_id)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "redis queue delivery snapshot failed for {entry_id}: {e}"
                ))
            })?;

        Ok(PendingSnapshotRead {
            response,
            query_started,
            observed_at: Instant::now(),
        })
    }

    async fn pending_lease(
        &self,
        entry_id: &str,
        delivered_payload: &str,
        expected_epoch: &str,
    ) -> Result<Option<VerifiedPendingDelivery>, FrameworkError> {
        verified_pending_delivery_with_revalidation(
            expected_epoch,
            entry_id,
            delivered_payload,
            &self.consumer_id,
            self.visibility_timeout,
            || self.read_pending_snapshot(entry_id),
        )
        .await
    }

    /// `XPENDING <stream> <group>` summary - first element is the total
    /// count of entries in the group's Pending Entries List (delivered but
    /// not yet acked). Returns 0 if the group does not exist (cleared
    /// stream, never-popped driver instance).
    async fn xpending_count(&self) -> Result<u64, FrameworkError> {
        // XPENDING summary form returns
        //   [count, smallest-id, largest-id, [[consumer, count], ...]]
        // or all-nil when the group is empty. We only need the first cell.
        // The summary is a pure read, so it is retried on a transient failure.
        let response = crate::redis_retry::retry_read("queue XPENDING summary", || {
            let mut conn = self.conn.clone();
            let stream = self.stream_key.name().to_string();
            let group = self.group_name.clone();
            async move {
                redis::cmd("XPENDING")
                    .arg(&stream)
                    .arg(&group)
                    .query_async(&mut conn)
                    .await
            }
        })
        .await;
        xpending_summary_count(response)
    }
}

fn pending_size_from_counts(
    stream_len: u64,
    reserved: Result<u64, FrameworkError>,
) -> Result<u64, FrameworkError> {
    Ok(stream_len.saturating_sub(reserved?))
}

fn xpending_summary_count(
    response: redis::RedisResult<redis::Value>,
) -> Result<u64, FrameworkError> {
    let response = match response {
        Ok(response) => response,
        Err(error) if is_missing_stream_or_group(&error) => return Ok(0),
        Err(error) => {
            return Err(FrameworkError::internal(format!(
                "redis XPENDING summary: {error}"
            )));
        }
    };
    let parts = match response {
        redis::Value::Array(parts) => parts,
        _ => {
            return Err(FrameworkError::internal(
                "redis XPENDING summary returned an invalid response",
            ));
        }
    };
    let count = match parts.first() {
        Some(redis::Value::Int(count)) => *count,
        _ => {
            return Err(FrameworkError::internal(
                "redis XPENDING summary returned an invalid count",
            ));
        }
    };
    u64::try_from(count)
        .map_err(|_| FrameworkError::internal("redis XPENDING summary returned a negative count"))
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

fn is_busy_group(error: &redis::RedisError) -> bool {
    error.code() == Some("BUSYGROUP")
        || error
            .detail()
            .is_some_and(|detail| detail.starts_with("BUSYGROUP"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn lifecycle_envelope() -> Envelope {
        Envelope {
            schema_version: crate::queue::CURRENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            job_name: "redis-lifecycle-probe".into(),
            queue: None,
            payload: serde_json::json!({}),
            dispatched_at: Utc::now(),
            available_at: Utc::now(),
            attempts: 0,
            max_tries: 3,
            backoff: crate::queue::BackoffSchedule::default(),
            timeout_secs: None,
            fail_on_timeout: false,
            idempotency_key: None,
            unique_lock_owner: None,
            debounce_id: None,
            debounce_owner: None,
            batch_id: None,
            chain_remaining: Vec::new(),
        }
    }

    fn pending_fixture() -> (PendingMap, ReservationToken) {
        let pending = Mutex::new(PendingRegistry::default());
        let envelope = lifecycle_envelope();
        let reservation = register_pending_delivery(
            &pending,
            envelope,
            DeliveryFence {
                epoch: "epoch-a".to_string(),
                entry_id: "1-0".to_string(),
                owner: "worker-a".to_string(),
                deliveries: 1,
            },
            Instant::now() + Duration::from_secs(30),
        )
        .expect("test delivery registration")
        .expect("test delivery should be registered");
        (pending, reservation.token)
    }

    #[test]
    fn redis_introspection_only_folds_missing_groups_to_zero() {
        let missing_group = redis::make_extension_error(
            "NOGROUP".to_string(),
            Some("consumer group does not exist".to_string()),
        );
        assert_eq!(
            xpending_summary_count(Err(missing_group)).expect("missing group is empty"),
            0
        );
        let missing_stream = redis::RedisError::from((
            redis::ErrorKind::Server(redis::ServerErrorKind::ResponseError),
            "response error",
            "no such key".to_string(),
        ));
        assert_eq!(
            xpending_summary_count(Err(missing_stream)).expect("missing stream is empty"),
            0
        );

        let populated = redis::Value::Array(vec![redis::Value::Int(3)]);
        assert_eq!(
            xpending_summary_count(Ok(populated)).expect("valid XPENDING summary"),
            3
        );

        let auth = redis::RedisError::from((
            redis::ErrorKind::AuthenticationFailed,
            "authentication failed",
        ));
        assert!(xpending_summary_count(Err(auth)).is_err());

        let io = redis::RedisError::from(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "peer reset during XPENDING",
        ));
        let io_error = xpending_summary_count(Err(io)).expect_err("I/O error must propagate");
        assert!(io_error.to_string().contains("peer reset during XPENDING"));

        let wrong_type = redis::make_extension_error(
            "WRONGTYPE".to_string(),
            Some("key has the wrong value type".to_string()),
        );
        assert!(xpending_summary_count(Err(wrong_type)).is_err());
        let generic_response = redis::RedisError::from((
            redis::ErrorKind::Server(redis::ServerErrorKind::ResponseError),
            "response error",
            "syntax error".to_string(),
        ));
        assert!(xpending_summary_count(Err(generic_response)).is_err());

        assert!(xpending_summary_count(Ok(redis::Value::Nil)).is_err());
        assert!(xpending_summary_count(Ok(redis::Value::Array(Vec::new()))).is_err());
        assert!(xpending_summary_count(Ok(redis::Value::Array(vec![redis::Value::Nil]))).is_err());
        assert!(
            xpending_summary_count(Ok(redis::Value::Array(vec![redis::Value::Int(-1)]))).is_err()
        );
        assert!(xpending_summary_count(Ok(redis::Value::Int(3))).is_err());
        assert!(xpending_summary_count(Ok(redis::Value::Set(vec![redis::Value::Int(3)]))).is_err());
    }

    #[test]
    fn redis_introspection_propagates_reserved_count_errors() {
        let error = pending_size_from_counts(
            7,
            Err(FrameworkError::internal("reserved count unavailable")),
        )
        .expect_err("reserved count failure must remain visible");

        assert!(error.to_string().contains("reserved count unavailable"));
    }

    #[test]
    fn same_envelope_deliveries_receive_distinct_reservation_tokens() {
        let pending = Mutex::new(PendingRegistry::default());
        let envelope = lifecycle_envelope();
        let envelope_id = envelope.id;

        let first = register_pending_delivery(
            &pending,
            envelope.clone(),
            DeliveryFence {
                epoch: "epoch-a".to_string(),
                entry_id: "1-0".to_string(),
                owner: "worker-a".to_string(),
                deliveries: 1,
            },
            Instant::now() + Duration::from_secs(30),
        )
        .expect("first delivery registration")
        .expect("first delivery should be registered");
        let second = register_pending_delivery(
            &pending,
            envelope,
            DeliveryFence {
                epoch: "epoch-a".to_string(),
                entry_id: "1-0".to_string(),
                owner: "worker-b".to_string(),
                deliveries: 2,
            },
            Instant::now() + Duration::from_secs(30),
        )
        .expect("reclaimed delivery registration")
        .expect("reclaimed delivery should be registered");

        assert_eq!(first.envelope.id, envelope_id);
        assert_eq!(second.envelope.id, envelope_id);
        assert_ne!(first.token, second.token);

        let registry = lock::lock(&pending, "test pending registry").expect("pending registry");
        assert!(!registry.by_token.contains_key(&first.token.0));
        assert!(registry.by_token.contains_key(&second.token.0));
        assert_eq!(
            registry.current_by_entry_id.get("1-0"),
            Some(&second.token.0)
        );
    }

    #[test]
    fn same_or_older_delivery_generation_cannot_replace_the_current_token() {
        let pending = Mutex::new(PendingRegistry::default());
        let envelope = lifecycle_envelope();
        let current_fence = DeliveryFence {
            epoch: "epoch-a".to_string(),
            entry_id: "1-0".to_string(),
            owner: "worker-a".to_string(),
            deliveries: 2,
        };
        let current = register_pending_delivery(
            &pending,
            envelope.clone(),
            current_fence.clone(),
            Instant::now() + Duration::from_secs(30),
        )
        .expect("current delivery registration")
        .expect("current generation is new");

        let duplicate = register_pending_delivery(
            &pending,
            envelope.clone(),
            current_fence,
            Instant::now() + Duration::from_secs(30),
        )
        .expect("duplicate delivery registration");
        assert!(duplicate.is_none());

        let older = register_pending_delivery(
            &pending,
            envelope,
            DeliveryFence {
                epoch: "epoch-a".to_string(),
                entry_id: "1-0".to_string(),
                owner: "worker-a".to_string(),
                deliveries: 1,
            },
            Instant::now() + Duration::from_secs(30),
        )
        .expect("older delivery registration");
        assert!(older.is_none());

        let registry = lock::lock(&pending, "test pending registry").expect("pending registry");
        assert_eq!(registry.by_token.len(), 1);
        assert!(registry.by_token.contains_key(&current.token.0));
    }

    #[test]
    fn delayed_members_preserve_duplicate_payloads_and_legacy_entries() {
        let payload = lifecycle_envelope().to_json().expect("test envelope JSON");
        let first = encode_delayed_member(&payload);
        let second = encode_delayed_member(&payload);

        assert_ne!(first, second, "each ZSET member needs a unique identity");
        assert_eq!(delayed_member_payload(&first), payload);
        assert_eq!(delayed_member_payload(&second), payload);
        assert_eq!(
            delayed_member_payload(&payload),
            payload,
            "members written before the prefix existed must remain readable"
        );
    }

    #[test]
    fn delayed_scores_round_fractional_deadlines_up_to_the_next_second() {
        let prepared_at = chrono::DateTime::<Utc>::from_timestamp(1_700_000_000, 500_000_000)
            .expect("valid preparation timestamp");
        let already_due = chrono::DateTime::<Utc>::from_timestamp(1_700_000_000, 400_000_000)
            .expect("valid due timestamp");
        let future = chrono::DateTime::<Utc>::from_timestamp(1_700_000_000, 600_000_000)
            .expect("valid future timestamp");
        let exact_future = chrono::DateTime::<Utc>::from_timestamp(1_700_000_001, 0)
            .expect("valid exact timestamp");

        assert_eq!(delayed_score(&already_due, &prepared_at), 1_700_000_000);
        assert_eq!(delayed_score(&future, &prepared_at), 1_700_000_001);
        assert_eq!(delayed_score(&exact_future, &prepared_at), 1_700_000_001);

        let mut follow_up = lifecycle_envelope();
        follow_up.available_at = future;
        let prepared =
            PreparedSettlement::new(std::slice::from_ref(&follow_up), prepared_at, |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            })
            .expect("prepare a fractional settlement");
        assert_eq!(prepared.publications[0].score, 1_700_000_001);
        assert!(prepared.matches(&[follow_up]).expect("match frozen intent"));
    }

    #[test]
    fn zero_delay_requeue_uses_a_frozen_unique_delayed_member() {
        let envelope = lifecycle_envelope();
        let prepared = PreparedRequeue::new(
            &envelope,
            Duration::ZERO,
            RequeueKind::Release,
            Utc::now(),
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
        )
        .expect("zero-delay requeue should prepare");

        let RequeueTarget::Delayed { member, .. } = prepared.target;
        assert_eq!(delayed_member_payload(&member), prepared.payload.as_ref());
    }

    #[test]
    fn requeue_rejects_an_unrepresentable_delay_before_encoding() {
        use std::cell::Cell;

        let envelope = lifecycle_envelope();
        let encoded = Cell::new(false);
        let error = PreparedRequeue::new(
            &envelope,
            Duration::MAX,
            RequeueKind::Release,
            Utc::now(),
            |_| {
                encoded.set(true);
                Ok("unexpected payload".to_string())
            },
        )
        .expect_err("an oversized delay must not become an immediate retry");

        assert!(error.to_string().contains("Redis requeue delay"));
        assert!(!encoded.get(), "invalid input must fail before encoding");
    }

    #[test]
    fn nack_attempt_accounting_saturates_at_the_envelope_limit() {
        let mut envelope = lifecycle_envelope();
        envelope.attempts = u32::MAX;
        let prepared = PreparedRequeue::new(
            &envelope,
            Duration::ZERO,
            RequeueKind::Nack,
            Utc::now(),
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
        )
        .expect("attempt accounting must not overflow");
        let requeued = Envelope::from_json(&prepared.payload).expect("prepared envelope");

        assert_eq!(requeued.attempts, u32::MAX);
    }

    #[test]
    fn release_preserves_attempts_while_nack_increments_them() {
        let mut envelope = lifecycle_envelope();
        envelope.attempts = 4;
        let encode = |envelope: &Envelope| {
            envelope
                .to_json()
                .map_err(|error| FrameworkError::internal(format!("test encode: {error}")))
        };

        let released = PreparedRequeue::new(
            &envelope,
            Duration::ZERO,
            RequeueKind::Release,
            Utc::now(),
            encode,
        )
        .expect("prepare release");
        let nacked = PreparedRequeue::new(
            &envelope,
            Duration::ZERO,
            RequeueKind::Nack,
            Utc::now(),
            encode,
        )
        .expect("prepare nack");

        let released = Envelope::from_json(&released.payload).expect("released envelope");
        let nacked = Envelope::from_json(&nacked.payload).expect("nacked envelope");
        assert_eq!(released.attempts, 4);
        assert_eq!(nacked.attempts, 5);
    }

    #[tokio::test]
    async fn stale_backend_generation_cannot_ack_the_delivery() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let effects = AtomicUsize::new(0);
        let authoritative = DeliveryFence {
            epoch: "epoch-a".to_string(),
            entry_id: "1-0".to_string(),
            owner: "worker-a".to_string(),
            deliveries: 2,
        };

        ack_pending_with(&pending, &token, |presented| {
            let outcome = if presented == authoritative {
                effects.fetch_add(1, Ordering::SeqCst);
                FenceOutcome::Applied
            } else {
                FenceOutcome::Stale
            };
            std::future::ready(Ok(outcome))
        })
        .await
        .expect("stale acknowledgement should be a successful no-op");

        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert!(
            !lock::lock(&pending, "test pending registry")
                .expect("pending registry")
                .by_token
                .contains_key(&token.0)
        );
    }

    #[tokio::test]
    async fn prior_queue_epoch_cannot_ack_an_identical_redis_generation() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let effects = AtomicUsize::new(0);
        let authoritative = DeliveryFence {
            epoch: "epoch-b".to_string(),
            entry_id: "1-0".to_string(),
            owner: "worker-a".to_string(),
            deliveries: 1,
        };

        ack_pending_with(&pending, &token, |presented| {
            let outcome = if presented == authoritative {
                effects.fetch_add(1, Ordering::SeqCst);
                FenceOutcome::Applied
            } else {
                FenceOutcome::Stale
            };
            std::future::ready(Ok(outcome))
        })
        .await
        .expect("a pre-clear acknowledgement should be a successful no-op");

        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_backend_generation_cannot_requeue_the_delivery() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let effects = AtomicUsize::new(0);
        let authoritative = DeliveryFence {
            epoch: "epoch-a".to_string(),
            entry_id: "1-0".to_string(),
            owner: "worker-a".to_string(),
            deliveries: 2,
        };

        requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay: Duration::ZERO,
                kind: RequeueKind::Nack,
                requested_at: Utc::now(),
            },
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |presented, _prepared| {
                let outcome = if presented == authoritative {
                    effects.fetch_add(1, Ordering::SeqCst);
                    FenceOutcome::Applied
                } else {
                    FenceOutcome::Stale
                };
                std::future::ready(Ok(outcome))
            },
        )
        .await
        .expect("stale requeue should be a successful no-op");

        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_backend_generation_cannot_publish_follow_ups() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let follow_up = lifecycle_envelope();
        let effects = AtomicUsize::new(0);
        let authoritative = DeliveryFence {
            epoch: "epoch-a".to_string(),
            entry_id: "1-0".to_string(),
            owner: "worker-a".to_string(),
            deliveries: 2,
        };

        let outcome = settle_pending_with(
            &pending,
            &token,
            std::slice::from_ref(&follow_up),
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |presented, prepared| {
                assert_eq!(prepared.publications.len(), 1);
                let outcome = if presented == authoritative {
                    effects.fetch_add(1, Ordering::SeqCst);
                    FenceOutcome::Applied
                } else {
                    FenceOutcome::Stale
                };
                std::future::ready(Ok(outcome))
            },
        )
        .await
        .expect("stale settlement should be reported without mutation");

        assert_eq!(outcome, crate::queue::driver::Settled::Stale);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn settlement_retry_rejects_different_follow_ups() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let first_follow_up = lifecycle_envelope();
        let second_follow_up = lifecycle_envelope();
        let effects = AtomicUsize::new(0);

        let first = settle_pending_with(
            &pending,
            &token,
            std::slice::from_ref(&first_follow_up),
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, _| std::future::ready(Err(FrameworkError::internal("injected settlement failure"))),
        )
        .await;
        assert!(first.is_err());

        let conflicting = settle_pending_with(
            &pending,
            &token,
            std::slice::from_ref(&second_follow_up),
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, _| {
                effects.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(FenceOutcome::Applied))
            },
        )
        .await;

        assert!(conflicting.is_err());
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert!(
            lock::lock(&pending, "test pending registry")
                .expect("pending registry")
                .by_token
                .contains_key(&token.0),
            "a conflicting retry must leave the original intent retryable"
        );
    }

    #[tokio::test]
    async fn settlement_retry_after_ambiguous_response_does_not_publish_twice() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let follow_up = lifecycle_envelope();
        let authoritative = AtomicBool::new(true);
        let effects = AtomicUsize::new(0);

        let first = settle_pending_with(
            &pending,
            &token,
            std::slice::from_ref(&follow_up),
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, _| {
                let result = if authoritative.swap(false, Ordering::SeqCst) {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Err(FrameworkError::internal("injected lost response"))
                } else {
                    Ok(FenceOutcome::PreviouslyApplied)
                };
                std::future::ready(result)
            },
        )
        .await;
        assert!(first.is_err());

        let retried = settle_pending_with(
            &pending,
            &token,
            std::slice::from_ref(&follow_up),
            |_| {
                Err(FrameworkError::internal(
                    "prepared follow-up was re-encoded",
                ))
            },
            |_, _| {
                let result = if authoritative.swap(false, Ordering::SeqCst) {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Ok(FenceOutcome::Applied)
                } else {
                    Ok(FenceOutcome::PreviouslyApplied)
                };
                std::future::ready(result)
            },
        )
        .await
        .expect("retry should observe the already-applied settlement");

        assert_eq!(retried, Settled::Atomically);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn settlement_without_follow_ups_still_fences_and_acks() {
        let (pending, token) = pending_fixture();
        let applied = settle_pending_with(
            &pending,
            &token,
            &[],
            |_| {
                Err(FrameworkError::internal(
                    "empty settlement encoded a follow-up",
                ))
            },
            |_, prepared| {
                assert!(prepared.publications.is_empty());
                std::future::ready(Ok(FenceOutcome::Applied))
            },
        )
        .await
        .expect("empty settlement should still acknowledge");
        assert_eq!(applied, Settled::Atomically);

        let repeated = settle_pending_with(
            &pending,
            &token,
            &[],
            |_| {
                Err(FrameworkError::internal(
                    "empty settlement encoded a follow-up",
                ))
            },
            |_, _| async {
                Err(FrameworkError::internal(
                    "completed settlement unexpectedly reached the backend",
                ))
            },
        )
        .await
        .expect("repeated settlement should be stale");
        assert_eq!(repeated, Settled::Stale);
    }

    #[tokio::test]
    async fn settlement_bounds_the_atomic_follow_up_batch() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let too_many = vec![lifecycle_envelope(); MAX_ATOMIC_FOLLOW_UPS + 1];
        let effects = AtomicUsize::new(0);
        let error = settle_pending_with(
            &pending,
            &token,
            &too_many,
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, _| {
                effects.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(FenceOutcome::Applied))
            },
        )
        .await
        .expect_err("an unbounded Lua settlement would block the Redis server");
        assert!(error.to_string().contains("at most 128 follow-ups"));
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let (pending, token) = pending_fixture();
        let maximum = vec![lifecycle_envelope(); MAX_ATOMIC_FOLLOW_UPS];
        let settled = settle_pending_with(
            &pending,
            &token,
            &maximum,
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, prepared| {
                assert_eq!(prepared.publications.len(), MAX_ATOMIC_FOLLOW_UPS);
                std::future::ready(Ok(FenceOutcome::Applied))
            },
        )
        .await
        .expect("the documented boundary must remain atomic");
        assert_eq!(settled, Settled::Atomically);
    }

    #[test]
    fn fenced_mutation_backend_enforces_the_atomic_follow_up_limit() {
        validate_atomic_follow_up_count(MAX_ATOMIC_FOLLOW_UPS)
            .expect("the documented boundary is accepted");
        let error = validate_atomic_follow_up_count(MAX_ATOMIC_FOLLOW_UPS + 1)
            .expect_err("backend must reject a caller that bypasses settlement preparation");

        assert!(error.to_string().contains("at most 128 follow-ups"));
    }

    #[test]
    fn fenced_settlement_result_rejects_unknown_values() {
        assert_eq!(
            decode_fence_outcome(0).expect("stale result"),
            FenceOutcome::Stale
        );
        assert_eq!(
            decode_fence_outcome(1).expect("applied result"),
            FenceOutcome::Applied
        );
        assert_eq!(
            decode_fence_outcome(2).expect("previously applied result"),
            FenceOutcome::PreviouslyApplied
        );
        assert!(decode_fence_outcome(-1).is_err());
        assert!(decode_fence_outcome(3).is_err());
    }

    #[test]
    fn fenced_mutation_status_distinguishes_current_stale_and_receipted() {
        assert_eq!(
            decode_mutation_status(0).expect("stale status"),
            MutationStatus::Stale
        );
        assert_eq!(
            decode_mutation_status(2).expect("receipt status"),
            MutationStatus::PreviouslyApplied
        );
        assert_eq!(
            decode_mutation_status(3).expect("current status"),
            MutationStatus::Current
        );
        assert!(decode_mutation_status(1).is_err());
        assert!(decode_mutation_status(4).is_err());
    }

    #[test]
    fn settlement_scripts_validate_epoch_before_receipt_lookup() {
        for (name, script) in [
            ("settlement", FENCED_SETTLEMENT_SCRIPT),
            ("status", MUTATION_STATUS_SCRIPT),
        ] {
            let epoch = script
                .find("local epoch = redis.pcall('GET'")
                .expect("script must read the queue epoch");
            let receipt = script
                .find("local receipt = redis.pcall('GET'")
                .expect("script must read the settlement receipt");
            assert!(
                epoch < receipt,
                "{name} script must reject a prior queue epoch before honoring its receipt"
            );
        }
    }

    #[test]
    fn settlement_scripts_never_treat_a_receipt_plus_live_pel_as_committed() {
        for (name, script) in [
            ("settlement", FENCED_SETTLEMENT_SCRIPT),
            ("status", MUTATION_STATUS_SCRIPT),
        ] {
            let pending = script
                .find("local pending = redis.pcall('XPENDING'")
                .expect("script must inspect the exact pending entry");
            let committed = script
                .find("return 2")
                .expect("script must recognize a committed receipt");
            assert!(
                pending < committed,
                "{name} script must inspect the PEL before accepting a receipt"
            );
            assert!(
                script.contains("settlement receipt conflicts with live PEL entry"),
                "{name} script must fail closed on an incomplete receipt state"
            );
        }
    }

    #[test]
    fn fenced_settlement_preflight_never_removes_a_delayed_member() {
        assert!(
            !FENCED_SETTLEMENT_SCRIPT.contains("redis.pcall('ZREM', KEYS[2], ARGV[6])"),
            "a rollback-permission probe must not mutate the live delayed ZSET"
        );
    }

    #[test]
    fn fenced_settlement_avoids_variadic_lua_after_writing_its_receipt() {
        assert!(
            !FENCED_SETTLEMENT_SCRIPT.contains("unpack("),
            "Lua argument expansion can fail outside pcall after the receipt write"
        );
        assert!(
            FENCED_SETTLEMENT_SCRIPT.contains("redis.pcall('ZADD', KEYS[2], 'NX'"),
            "each staged member must use fixed-arity ZADD NX ownership"
        );
    }

    #[tokio::test]
    async fn failed_ack_keeps_pending_entry_for_retry() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let attempts = AtomicUsize::new(0);

        let first = ack_pending_with(&pending, &token, |_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err(FrameworkError::internal("injected ack failure")))
        })
        .await;
        assert!(first.is_err());
        assert!(
            lock::lock(&pending, "test pending map")
                .expect("pending map")
                .by_token
                .contains_key(&token.0)
        );

        ack_pending_with(&pending, &token, |_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(FenceOutcome::Applied))
        })
        .await
        .expect("retry should finish the retained acknowledgement");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert!(
            !lock::lock(&pending, "test pending map")
                .expect("pending map")
                .by_token
                .contains_key(&token.0)
        );

        ack_pending_with(&pending, &token, |_| async {
            Err(FrameworkError::internal(
                "completed token unexpectedly reached the backend",
            ))
        })
        .await
        .expect("a completed token should be a no-op");
    }

    #[tokio::test]
    async fn cancelled_ack_freezes_ack_intent_until_ack_is_retried() {
        let (pending, token) = pending_fixture();
        let cancelled = tokio::time::timeout(
            Duration::from_millis(10),
            ack_pending_with(&pending, &token, |_| {
                std::future::pending::<Result<FenceOutcome, FrameworkError>>()
            }),
        )
        .await;
        assert!(cancelled.is_err(), "test ACK must be cancelled in flight");

        let entry = pending_entry(&pending, &token)
            .expect("pending lookup")
            .expect("cancelled ACK retains its entry");
        assert!(matches!(
            *entry.lifecycle.lock().await,
            LifecycleState::AckPending
        ));

        let divergent = requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay: Duration::ZERO,
                kind: RequeueKind::Release,
                requested_at: Utc::now(),
            },
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, _| std::future::ready(Ok(FenceOutcome::Applied)),
        )
        .await
        .expect_err("a pending ACK must reject a divergent release");
        assert!(divergent.to_string().contains("acknowledgement is pending"));

        ack_pending_with(&pending, &token, |_| {
            std::future::ready(Ok(FenceOutcome::PreviouslyApplied))
        })
        .await
        .expect("ACK retry should reconcile the frozen intent");
        assert!(
            pending_entry(&pending, &token)
                .expect("pending lookup")
                .is_none()
        );
    }

    #[tokio::test]
    async fn requeue_retry_preserves_entry_after_encode_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let settlement_attempts = AtomicUsize::new(0);

        let first = requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay: Duration::from_secs(5),
                kind: RequeueKind::Nack,
                requested_at: Utc::now(),
            },
            |_| Err(FrameworkError::internal("injected encode failure")),
            |_, _| {
                settlement_attempts.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(FenceOutcome::Applied))
            },
        )
        .await;
        assert!(first.is_err());
        assert_eq!(settlement_attempts.load(Ordering::SeqCst), 0);
        assert!(
            lock::lock(&pending, "test pending map")
                .expect("pending map")
                .by_token
                .contains_key(&token.0)
        );

        requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay: Duration::from_secs(5),
                kind: RequeueKind::Nack,
                requested_at: Utc::now(),
            },
            |envelope| {
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, _| {
                settlement_attempts.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(FenceOutcome::Applied))
            },
        )
        .await
        .expect("retry should encode and atomically settle");
        assert_eq!(settlement_attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn requeue_retry_reuses_prepared_mutation_after_backend_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let encoded = AtomicUsize::new(0);
        let settlement_attempts = AtomicUsize::new(0);
        let payloads = Mutex::new(Vec::<String>::new());
        let requested_at = Utc::now();
        let delay = Duration::from_secs(7);

        let first = requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay,
                kind: RequeueKind::Nack,
                requested_at,
            },
            |envelope| {
                encoded.fetch_add(1, Ordering::SeqCst);
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, prepared| {
                settlement_attempts.fetch_add(1, Ordering::SeqCst);
                payloads
                    .lock()
                    .expect("payload log")
                    .push(prepared.payload.to_string());
                std::future::ready(Err(FrameworkError::internal("injected settlement failure")))
            },
        )
        .await;
        assert!(first.is_err());

        requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay,
                kind: RequeueKind::Nack,
                requested_at: requested_at + chrono::Duration::seconds(30),
            },
            |_| Err(FrameworkError::internal("prepared payload was re-encoded")),
            |_, prepared| {
                settlement_attempts.fetch_add(1, Ordering::SeqCst);
                payloads
                    .lock()
                    .expect("payload log")
                    .push(prepared.payload.to_string());
                std::future::ready(Ok(FenceOutcome::Applied))
            },
        )
        .await
        .expect("retry should reuse the prepared publication");

        let payloads = payloads.lock().expect("payload log");
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0], payloads[1]);
        let retried = Envelope::from_json(&payloads[0]).expect("prepared envelope");
        assert_eq!(retried.attempts, 1);
        assert_eq!(
            retried.available_at,
            requested_at + chrono::Duration::seconds(7)
        );
        assert_eq!(encoded.load(Ordering::SeqCst), 1);
        assert_eq!(settlement_attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn requeue_retry_after_ambiguous_response_does_not_publish_twice() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let (pending, token) = pending_fixture();
        let encoded = AtomicUsize::new(0);
        let effects = AtomicUsize::new(0);
        let settlement_attempts = AtomicUsize::new(0);
        let authoritative = AtomicBool::new(true);

        let first = requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay: Duration::ZERO,
                kind: RequeueKind::Release,
                requested_at: Utc::now(),
            },
            |envelope| {
                encoded.fetch_add(1, Ordering::SeqCst);
                envelope
                    .to_json()
                    .map_err(|e| FrameworkError::internal(format!("test encode: {e}")))
            },
            |_, _| {
                settlement_attempts.fetch_add(1, Ordering::SeqCst);
                let result = if authoritative.swap(false, Ordering::SeqCst) {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Err(FrameworkError::internal("injected lost response"))
                } else {
                    Ok(FenceOutcome::Stale)
                };
                std::future::ready(result)
            },
        )
        .await;
        assert!(first.is_err());

        requeue_pending_with(
            &pending,
            &token,
            RequeueRequest {
                delay: Duration::ZERO,
                kind: RequeueKind::Release,
                requested_at: Utc::now(),
            },
            |_| Err(FrameworkError::internal("prepared payload was re-encoded")),
            |_, _| {
                settlement_attempts.fetch_add(1, Ordering::SeqCst);
                let result = if authoritative.swap(false, Ordering::SeqCst) {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Ok(FenceOutcome::Applied)
                } else {
                    Ok(FenceOutcome::Stale)
                };
                std::future::ready(result)
            },
        )
        .await
        .expect("retry should observe the already-applied settlement as stale");

        assert_eq!(encoded.load(Ordering::SeqCst), 1);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(settlement_attempts.load(Ordering::SeqCst), 2);
        assert!(
            !lock::lock(&pending, "test pending map")
                .expect("pending map")
                .by_token
                .contains_key(&token.0)
        );
    }

    fn configured_redis_test_url() -> String {
        std::env::var("QUEUE_REDIS_TEST_URL")
            .or_else(|_| std::env::var("REDIS_URL"))
            .expect("QUEUE_REDIS_TEST_URL or REDIS_URL must name an isolated test Redis")
    }

    #[ignore = "requires an explicitly configured isolated Redis"]
    #[tokio::test]
    async fn fenced_settlement_receipt_replays_a_lost_success_without_duplicates() {
        let stream = format!("test-receipt-{}", Uuid::new_v4());
        let driver = RedisQueueDriver::connect(
            &configured_redis_test_url(),
            &stream,
            "receipt-group",
            "receipt-consumer",
            Duration::from_secs(60),
        )
        .await
        .expect("connect test driver");
        driver
            .push(lifecycle_envelope())
            .await
            .expect("push original");
        let reservation = driver
            .pop(Duration::from_secs(5))
            .await
            .expect("pop original")
            .expect("original reservation");
        let entry = pending_entry(&driver.pending, &reservation.token)
            .expect("pending lookup")
            .expect("pending entry");
        let follow_up = lifecycle_envelope();
        let payload = Arc::<str>::from(follow_up.to_json().expect("follow-up JSON"));
        let publication = ScheduledPublication {
            score: follow_up.available_at.timestamp(),
            member: Arc::from(encode_delayed_member(&payload)),
            payload,
        };
        let mutation = || FencedMutation {
            operation_id: reservation.token.0,
            kind: MutationKind::Settle,
            publications: vec![publication.clone()].into(),
        };

        let first = driver
            .apply_fenced_mutation(entry.fence.clone(), mutation(), "test settle")
            .await
            .expect("first settlement");
        assert_eq!(first, FenceOutcome::Applied);

        let replay = driver
            .apply_fenced_mutation(entry.fence.clone(), mutation(), "test settle replay")
            .await
            .expect("receipt-backed replay");
        assert_eq!(replay, FenceOutcome::PreviouslyApplied);
        assert_eq!(driver.delayed_size().await.expect("delayed size"), 1);
        assert_eq!(driver.reserved_size().await.expect("reserved size"), 0);
        driver.clear().await.expect("clear test queue");
    }

    #[ignore = "requires an explicitly configured isolated Redis"]
    #[tokio::test]
    async fn fenced_settlement_rejects_a_receipt_while_the_pel_entry_is_live() {
        let stream = format!("test-incomplete-receipt-{}", Uuid::new_v4());
        let driver = RedisQueueDriver::connect(
            &configured_redis_test_url(),
            &stream,
            "incomplete-receipt-group",
            "incomplete-receipt-consumer",
            Duration::from_secs(60),
        )
        .await
        .expect("connect test driver");
        driver
            .push(lifecycle_envelope())
            .await
            .expect("push original");
        let reservation = driver
            .pop(Duration::from_secs(5))
            .await
            .expect("pop original")
            .expect("original reservation");
        let entry = pending_entry(&driver.pending, &reservation.token)
            .expect("pending lookup")
            .expect("pending entry");
        let mutation = FencedMutation {
            operation_id: reservation.token.0,
            kind: MutationKind::Ack,
            publications: Arc::from([]),
        };
        let fingerprint = mutation_fingerprint(&entry.fence, &mutation);
        let receipt_key = settlement_receipt_key(&stream, reservation.token.0);
        let mut conn = driver.conn.clone();
        let _: () = redis::cmd("SET")
            .arg(&receipt_key)
            .arg(fingerprint.as_slice())
            .arg("PX")
            .arg(settlement_receipt_ttl_ms(driver.visibility_timeout))
            .query_async(&mut conn)
            .await
            .expect("seed an incomplete receipt");

        let status = driver
            .mutation_status(&entry.fence, &mutation)
            .await
            .expect_err("a receipt cannot prove commit while its PEL row is live");
        assert!(status.to_string().contains("conflicts with live PEL entry"));
        let apply = driver
            .apply_fenced_mutation(entry.fence.clone(), mutation, "test incomplete receipt")
            .await
            .expect_err("settlement must fail closed on an incomplete receipt");
        assert!(apply.to_string().contains("conflicts with live PEL entry"));
        assert_eq!(driver.reserved_size().await.expect("reserved size"), 1);

        driver.clear().await.expect("clear test queue");
    }

    #[ignore = "requires an explicitly configured isolated Redis"]
    #[tokio::test]
    async fn fenced_receipts_from_a_prior_queue_epoch_are_stale() {
        let stream = format!("test-receipt-epoch-{}", Uuid::new_v4());
        let driver = RedisQueueDriver::connect(
            &configured_redis_test_url(),
            &stream,
            "receipt-epoch-group",
            "receipt-epoch-consumer",
            Duration::from_secs(60),
        )
        .await
        .expect("connect test driver");
        driver
            .push(lifecycle_envelope())
            .await
            .expect("push original");
        let reservation = driver
            .pop(Duration::from_secs(5))
            .await
            .expect("pop original")
            .expect("original reservation");
        let entry = pending_entry(&driver.pending, &reservation.token)
            .expect("pending lookup")
            .expect("pending entry");
        let follow_up = lifecycle_envelope();
        let payload = Arc::<str>::from(follow_up.to_json().expect("follow-up JSON"));
        let publication = ScheduledPublication {
            score: follow_up.available_at.timestamp(),
            member: Arc::from(encode_delayed_member(&payload)),
            payload,
        };
        let mutation = || FencedMutation {
            operation_id: reservation.token.0,
            kind: MutationKind::Settle,
            publications: vec![publication.clone()].into(),
        };

        assert_eq!(
            driver
                .apply_fenced_mutation(entry.fence.clone(), mutation(), "test settle")
                .await
                .expect("first settlement"),
            FenceOutcome::Applied
        );
        driver.clear().await.expect("clear test queue");

        assert_eq!(
            driver
                .mutation_status(&entry.fence, &mutation())
                .await
                .expect("prior-epoch status"),
            MutationStatus::Stale
        );
        assert_eq!(
            driver
                .apply_fenced_mutation(entry.fence.clone(), mutation(), "test prior-epoch replay",)
                .await
                .expect("prior-epoch replay"),
            FenceOutcome::Stale
        );
        assert_eq!(driver.delayed_size().await.expect("delayed size"), 0);
        assert_eq!(driver.reserved_size().await.expect("reserved size"), 0);
    }

    #[ignore = "requires an explicitly configured isolated Redis"]
    #[tokio::test]
    async fn epoch_revalidation_reloads_the_recreated_stream_payload() {
        let stream = format!("test-rebind-{}", Uuid::new_v4());
        let group = "rebind-group";
        let consumer = "rebind-consumer";
        let driver = RedisQueueDriver::connect(
            &configured_redis_test_url(),
            &stream,
            group,
            consumer,
            Duration::from_secs(60),
        )
        .await
        .expect("connect test driver");
        let old_epoch = driver.queue_epoch().await.expect("old epoch");
        let old_payload = lifecycle_envelope().to_json().expect("old JSON");
        let mut conn = driver.conn.clone();
        let old_id: String = redis::cmd("XADD")
            .arg(&stream)
            .arg("1-0")
            .arg("msg")
            .arg(&old_payload)
            .query_async(&mut conn)
            .await
            .expect("old stream entry");
        let _: () = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&stream)
            .arg(group)
            .arg("0")
            .query_async(&mut conn)
            .await
            .expect("old consumer group");
        let _: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group)
            .arg(consumer)
            .arg("COUNT")
            .arg(1)
            .arg("STREAMS")
            .arg(&stream)
            .arg(">")
            .query_async(&mut conn)
            .await
            .expect("old delivery");
        assert_eq!(old_id, "1-0");

        driver.clear().await.expect("rotate queue epoch");
        let new_payload = lifecycle_envelope().to_json().expect("new JSON");
        let new_id: String = redis::cmd("XADD")
            .arg(&stream)
            .arg("1-0")
            .arg("msg")
            .arg(&new_payload)
            .query_async(&mut conn)
            .await
            .expect("recreated stream entry");
        let _: () = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&stream)
            .arg(group)
            .arg("0")
            .query_async(&mut conn)
            .await
            .expect("recreated consumer group");
        let _: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group)
            .arg(consumer)
            .arg("COUNT")
            .arg(1)
            .arg("STREAMS")
            .arg(&stream)
            .arg(">")
            .query_async(&mut conn)
            .await
            .expect("recreated delivery");
        assert_eq!(new_id, "1-0");

        let rebound = driver
            .pending_lease("1-0", &old_payload, &old_epoch)
            .await
            .expect("revalidate changed epoch")
            .expect("recreated entry remains current");
        assert_ne!(rebound.fence.epoch, old_epoch);
        assert_eq!(rebound.payload, new_payload);
        driver.clear().await.expect("clear test queue");
    }

    #[ignore = "requires an explicitly configured isolated Redis"]
    #[tokio::test]
    async fn fenced_settlement_rolls_back_staged_members_when_xack_does_not_apply() {
        let stream = format!("test-rollback-{}", Uuid::new_v4());
        let driver = RedisQueueDriver::connect(
            &configured_redis_test_url(),
            &stream,
            "rollback-group",
            "rollback-consumer",
            Duration::from_secs(60),
        )
        .await
        .expect("connect test driver");
        driver
            .push(lifecycle_envelope())
            .await
            .expect("push original");
        let reservation = driver
            .pop(Duration::from_secs(5))
            .await
            .expect("pop original")
            .expect("original reservation");
        let entry = pending_entry(&driver.pending, &reservation.token)
            .expect("pending lookup")
            .expect("pending entry");
        let payload = Arc::<str>::from(lifecycle_envelope().to_json().expect("follow-up JSON"));
        let publication = ScheduledPublication {
            score: Utc::now().timestamp(),
            member: Arc::from(encode_delayed_member(&payload)),
            payload,
        };
        let mutation = FencedMutation {
            operation_id: reservation.token.0,
            kind: MutationKind::Settle,
            publications: vec![publication.clone()].into(),
        };
        let fingerprint = mutation_fingerprint(&entry.fence, &mutation);
        let receipt_key = settlement_receipt_key(&stream, mutation.operation_id);
        let forced_xack_failure = FENCED_SETTLEMENT_SCRIPT.replacen(
            "local acknowledged = redis.pcall('XACK', KEYS[1], ARGV[1], ARGV[2])",
            "local acknowledged = 0",
            1,
        );
        let mut conn = driver.conn.clone();
        let result: redis::RedisResult<i64> = redis::Script::new(&forced_xack_failure)
            .key(&stream)
            .key(&driver.delayed_key)
            .key(&driver.epoch_key)
            .key(&receipt_key)
            .arg(&driver.group_name)
            .arg(&entry.fence.entry_id)
            .arg(&entry.fence.owner)
            .arg(entry.fence.deliveries)
            .arg(&entry.fence.epoch)
            .arg(fingerprint.as_slice())
            .arg(settlement_receipt_ttl_ms(driver.visibility_timeout))
            .arg(1)
            .arg(publication.score)
            .arg(publication.member.as_ref())
            .invoke_async(&mut conn)
            .await;

        assert!(result.is_err(), "forced XACK miss must fail the script");
        assert_eq!(driver.delayed_size().await.expect("delayed size"), 0);
        assert_eq!(driver.reserved_size().await.expect("reserved size"), 1);
        let receipt_exists: bool = conn.exists(&receipt_key).await.expect("receipt lookup");
        assert!(!receipt_exists, "failed settlement must remove its receipt");
        driver
            .ack(&reservation.token)
            .await
            .expect("ack original after rollback");
        driver.clear().await.expect("clear test queue");
    }

    #[ignore = "requires an explicitly configured isolated Redis"]
    #[tokio::test]
    async fn fenced_settlement_preserves_a_delayed_member_equal_to_its_fingerprint() {
        let stream = format!("test-probe-collision-{}", Uuid::new_v4());
        let driver = RedisQueueDriver::connect(
            &configured_redis_test_url(),
            &stream,
            "probe-collision-group",
            "probe-collision-consumer",
            Duration::from_secs(60),
        )
        .await
        .expect("connect test driver");
        driver
            .push(lifecycle_envelope())
            .await
            .expect("push original");
        let reservation = driver
            .pop(Duration::from_secs(5))
            .await
            .expect("pop original")
            .expect("original reservation");
        let entry = pending_entry(&driver.pending, &reservation.token)
            .expect("pending lookup")
            .expect("pending entry");
        let payload = Arc::<str>::from(lifecycle_envelope().to_json().expect("follow-up JSON"));
        let mutation = FencedMutation {
            operation_id: reservation.token.0,
            kind: MutationKind::Settle,
            publications: vec![ScheduledPublication {
                score: Utc::now().timestamp(),
                member: Arc::from(encode_delayed_member(&payload)),
                payload,
            }]
            .into(),
        };
        let fingerprint = mutation_fingerprint(&entry.fence, &mutation);
        let mut conn = driver.conn.clone();
        let _: i64 = conn
            .zadd(&driver.delayed_key, fingerprint.as_slice(), 0)
            .await
            .expect("seed colliding delayed member");

        assert_eq!(
            driver
                .apply_fenced_mutation(entry.fence.clone(), mutation, "test settle")
                .await
                .expect("settlement must not mutate its preflight target"),
            FenceOutcome::Applied
        );
        let preserved: Option<i64> = conn
            .zscore(&driver.delayed_key, fingerprint.as_slice())
            .await
            .expect("lookup colliding delayed member");
        assert_eq!(preserved, Some(0));
        driver.clear().await.expect("clear test queue");
    }

    #[ignore = "requires an explicitly configured isolated Redis"]
    #[tokio::test]
    async fn fenced_settlement_backend_rejects_oversized_batch_before_receipt() {
        let stream = format!("test-large-settlement-{}", Uuid::new_v4());
        let driver = RedisQueueDriver::connect(
            &configured_redis_test_url(),
            &stream,
            "large-settlement-group",
            "large-settlement-consumer",
            Duration::from_secs(60),
        )
        .await
        .expect("connect test driver");
        driver
            .push(lifecycle_envelope())
            .await
            .expect("push original");
        let reservation = driver
            .pop(Duration::from_secs(5))
            .await
            .expect("pop original")
            .expect("original reservation");
        let entry = pending_entry(&driver.pending, &reservation.token)
            .expect("pending lookup")
            .expect("pending entry");
        let payload = Arc::<str>::from(lifecycle_envelope().to_json().expect("follow-up JSON"));
        let publications = (0..=MAX_ATOMIC_FOLLOW_UPS)
            .map(|_| ScheduledPublication {
                score: Utc::now().timestamp(),
                member: Arc::from(encode_delayed_member(&payload)),
                payload: Arc::clone(&payload),
            })
            .collect::<Vec<_>>();
        let mutation = FencedMutation {
            operation_id: reservation.token.0,
            kind: MutationKind::Settle,
            publications: publications.into(),
        };
        let receipt_key = settlement_receipt_key(&stream, mutation.operation_id);

        let error = driver
            .apply_fenced_mutation(entry.fence.clone(), mutation, "test large settle")
            .await
            .expect_err("backend boundary must reject an oversized mutation");
        assert!(error.to_string().contains("at most 128 follow-ups"));
        assert_eq!(driver.delayed_size().await.expect("delayed size"), 0);
        assert_eq!(driver.reserved_size().await.expect("reserved size"), 1);
        let mut conn = driver.conn.clone();
        let receipt_exists: bool = conn.exists(receipt_key).await.expect("receipt lookup");
        assert!(!receipt_exists);
        driver.clear().await.expect("clear test queue");
    }

    fn pending_reply(entry_id: &str, owner: &str, idle_ms: i64, deliveries: i64) -> redis::Value {
        redis::Value::Array(vec![redis::Value::Array(vec![
            redis::Value::BulkString(entry_id.as_bytes().to_vec()),
            redis::Value::BulkString(owner.as_bytes().to_vec()),
            redis::Value::Int(idle_ms),
            redis::Value::Int(deliveries),
        ])])
    }

    fn pending_snapshot_reply(
        epoch: &str,
        entry_id: &str,
        owner: &str,
        idle_ms: i64,
        deliveries: i64,
    ) -> redis::Value {
        redis::Value::Array(vec![
            redis::Value::BulkString(epoch.as_bytes().to_vec()),
            pending_reply(entry_id, owner, idle_ms, deliveries),
            redis::Value::BulkString(b"payload".to_vec()),
        ])
    }

    fn pending_delivery_snapshot_reply(
        epoch: &str,
        entry_id: &str,
        owner: &str,
        idle_ms: i64,
        deliveries: i64,
        payload: &str,
    ) -> redis::Value {
        redis::Value::Array(vec![
            redis::Value::BulkString(epoch.as_bytes().to_vec()),
            pending_reply(entry_id, owner, idle_ms, deliveries),
            redis::Value::BulkString(payload.as_bytes().to_vec()),
        ])
    }

    #[test]
    fn pending_snapshot_binds_the_queue_epoch_to_the_pel_generation() {
        let response = pending_snapshot_reply("epoch-a", "1-0", "worker-a", 250, 3);
        let snapshot = parse_pending_snapshot(&response).expect("valid delivery snapshot");
        let metadata = snapshot.metadata.expect("valid epoch plus XPENDING row");

        assert_eq!(snapshot.epoch, "epoch-a");
        assert_eq!(snapshot.payload.as_deref(), Some("payload"));
        assert_eq!(metadata.fence.epoch, "epoch-a");
        assert_eq!(metadata.fence.entry_id, "1-0");
        assert_eq!(metadata.fence.owner, "worker-a");
        assert_eq!(metadata.fence.deliveries, 3);
        assert!(parse_pending_snapshot(&redis::Value::Array(vec![])).is_none());
    }

    #[test]
    fn pending_snapshot_rejects_a_delivery_crossing_an_epoch_change() {
        let response = pending_snapshot_reply("epoch-b", "1-0", "worker-a", 10, 1);
        let query_started = Instant::now();

        assert!(
            verified_pending_snapshot(
                Some(&response),
                "epoch-a",
                "1-0",
                "worker-a",
                query_started,
                Duration::from_secs(5),
                query_started,
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn epoch_change_revalidates_once_and_uses_current_stream_payload() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let now = Instant::now();
        let responses = Arc::new(Mutex::new(VecDeque::from([
            pending_delivery_snapshot_reply("epoch-b", "1-0", "worker-a", 10, 1, "new payload"),
            pending_delivery_snapshot_reply("epoch-b", "1-0", "worker-a", 20, 1, "new payload"),
        ])));
        let reads = Arc::new(AtomicUsize::new(0));

        let verified = verified_pending_delivery_with_revalidation(
            "epoch-a",
            "1-0",
            "old payload",
            "worker-a",
            Duration::from_secs(5),
            {
                let responses = Arc::clone(&responses);
                let reads = Arc::clone(&reads);
                move || {
                    reads.fetch_add(1, Ordering::SeqCst);
                    let response = responses
                        .lock()
                        .expect("snapshot responses")
                        .pop_front()
                        .expect("bounded snapshot read");
                    std::future::ready(Ok(PendingSnapshotRead {
                        response,
                        query_started: now,
                        observed_at: now,
                    }))
                }
            },
        )
        .await
        .expect("revalidation succeeds")
        .expect("recreated entry is current");

        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(verified.fence.epoch, "epoch-b");
        assert_eq!(verified.payload, "new payload");
    }

    #[test]
    fn claimed_stream_entry_requires_the_envelope_payload_field() {
        let mut entry = redis::streams::StreamId {
            id: "1-0".to_string(),
            ..Default::default()
        };
        entry.map.insert(
            "msg".to_string(),
            redis::Value::BulkString(b"payload".to_vec()),
        );

        let delivery = raw_delivery_from_stream_id(entry).expect("valid claimed stream entry");
        assert_eq!(delivery.entry_id, "1-0");
        assert_eq!(delivery.payload, "payload");

        assert!(
            raw_delivery_from_stream_id(redis::streams::StreamId {
                id: "2-0".to_string(),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn settlement_receipt_fingerprint_binds_the_exact_delivery_and_intent() {
        let operation_id = Uuid::new_v4();
        let mutation = FencedMutation {
            operation_id,
            kind: MutationKind::Settle,
            publications: vec![ScheduledPublication {
                score: 42,
                payload: Arc::from("payload"),
                member: Arc::from("member-a"),
            }]
            .into(),
        };
        let fence = DeliveryFence {
            epoch: "epoch-a".to_string(),
            entry_id: "1-0".to_string(),
            owner: "worker-a".to_string(),
            deliveries: 1,
        };

        let fingerprint = mutation_fingerprint(&fence, &mutation);
        assert_eq!(fingerprint, mutation_fingerprint(&fence, &mutation));

        let mut changed_epoch = fence.clone();
        changed_epoch.epoch = "epoch-b".to_string();
        assert_ne!(fingerprint, mutation_fingerprint(&changed_epoch, &mutation));

        let changed_member = FencedMutation {
            operation_id,
            kind: MutationKind::Settle,
            publications: vec![ScheduledPublication {
                score: 42,
                payload: Arc::from("payload"),
                member: Arc::from("member-b"),
            }]
            .into(),
        };
        assert_ne!(fingerprint, mutation_fingerprint(&fence, &changed_member));
    }

    #[test]
    fn settlement_receipts_are_namespaced_and_outlive_multiple_visibility_windows() {
        let operation_id =
            Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("fixed operation id");
        assert_eq!(
            settlement_receipt_key("queue:{mail}", operation_id),
            "queue:{mail}:settlement-receipt:01234567-89ab-cdef-0123-456789abcdef"
        );
        assert_eq!(
            settlement_receipt_ttl_ms(Duration::from_secs(30)),
            3_600_000
        );
        assert_eq!(
            settlement_receipt_ttl_ms(Duration::from_secs(2 * 60 * 60)),
            8 * 60 * 60 * 1_000
        );
    }

    #[test]
    fn claim_cursor_advances_on_empty_pages_and_resets_only_for_a_new_epoch() {
        let mut cursor = ClaimCursor::default();

        assert_eq!(cursor.start_for_epoch("epoch-a"), "0-0");
        cursor.advance("epoch-a", "9-0");
        assert_eq!(cursor.start_for_epoch("epoch-a"), "9-0");
        cursor.advance("epoch-a", "0-0");
        assert_eq!(cursor.start_for_epoch("epoch-a"), "0-0");
        cursor.advance("epoch-a", "17-0");
        assert_eq!(cursor.start_for_epoch("epoch-b"), "0-0");
    }

    #[test]
    fn new_delivery_reads_never_replay_the_consumers_pending_entries() {
        let command = new_delivery_read_command(
            "test-stream",
            "test-group",
            "test-consumer",
            Duration::from_millis(100),
        );
        let packed = String::from_utf8(command.get_packed_command()).expect("RESP command bytes");

        assert!(packed.contains("XREADGROUP"));
        assert!(packed.contains("$1\r\n>\r\n"));
        assert!(
            !packed.contains("$3\r\n0-0\r\n"),
            "reading consumer history would redeliver a fresh XAUTOCLAIM result"
        );
    }

    #[test]
    fn redis_server_capabilities_require_xautoclaim_and_standalone_keyspace() {
        assert!(
            validate_redis_server_info("redis_version:6.2.0\r\n", "cluster_enabled:0\r\n").is_ok()
        );
        assert!(
            validate_redis_server_info("redis_version:7.4.1\r\n", "cluster_enabled:0\r\n").is_ok()
        );

        let old = validate_redis_server_info(
            "# Server\r\nredis_version:6.0.20\r\n",
            "# Cluster\r\ncluster_enabled:0\r\n",
        )
        .expect_err("Redis before 6.2 lacks XAUTOCLAIM");
        assert!(old.to_string().contains("Redis 6.2 or newer"));

        let cluster =
            validate_redis_server_info("redis_version:7.4.1\r\n", "cluster_enabled:1\r\n")
                .expect_err("multi-key scripts cannot run through a standalone client on Cluster");
        assert!(cluster.to_string().contains("Redis Cluster"));

        assert!(validate_redis_server_info("", "cluster_enabled:0\r\n").is_err());
        assert!(validate_redis_server_info("redis_version:7.4.1\r\n", "").is_err());
    }

    #[test]
    fn redis_visibility_timeout_must_survive_millisecond_conversion() {
        let zero = validate_redis_visibility_timeout(Duration::ZERO)
            .expect_err("zero cannot be represented as an XAUTOCLAIM idle threshold");
        assert!(zero.to_string().contains("at least 1 millisecond"));

        let sub_millisecond = validate_redis_visibility_timeout(Duration::from_nanos(999_999))
            .expect_err("sub-millisecond durations truncate to zero in Redis");
        assert!(
            sub_millisecond
                .to_string()
                .contains("at least 1 millisecond")
        );

        assert!(validate_redis_visibility_timeout(Duration::from_millis(1)).is_ok());

        let fractional = validate_redis_visibility_timeout(Duration::from_micros(1_500))
            .expect_err("Redis and local leases must use the same whole-millisecond value");
        assert!(fractional.to_string().contains("whole milliseconds"));

        let oversized = validate_redis_visibility_timeout(Duration::MAX)
            .expect_err("the lease and receipt deadlines must remain representable");
        assert!(oversized.to_string().contains("supported Redis range"));
    }

    #[test]
    fn reconciliation_queue_is_deduplicated_and_round_robin_fair() {
        let pending = Mutex::new(PendingRegistry::default());
        let now = Instant::now();
        let mut tokens = Vec::new();
        for index in 1..=3 {
            let reservation = register_pending_delivery(
                &pending,
                lifecycle_envelope(),
                DeliveryFence {
                    epoch: "epoch-a".to_string(),
                    entry_id: format!("{index}-0"),
                    owner: "worker-a".to_string(),
                    deliveries: 1,
                },
                now + Duration::from_secs(30),
            )
            .expect("test delivery registration")
            .expect("test delivery should register");
            mark_reconciliation_needed(&pending, &reservation.token)
                .expect("candidate should be queued");
            mark_reconciliation_needed(&pending, &reservation.token)
                .expect("duplicate scheduling should be harmless");
            tokens.push(reservation.token);
        }

        let first = take_reconciliation_candidate(&pending, Instant::now())
            .expect("registry lookup")
            .expect("first candidate");
        assert_eq!(first.0, tokens[0]);
        reschedule_reconciliation_candidate(&pending, &first.0, &first.1)
            .expect("first candidate should rotate");

        let second = take_reconciliation_candidate(&pending, Instant::now())
            .expect("registry lookup")
            .expect("second candidate");
        assert_eq!(second.0, tokens[1]);
        reschedule_reconciliation_candidate(&pending, &second.0, &second.1)
            .expect("second candidate should rotate");

        let third = take_reconciliation_candidate(&pending, Instant::now())
            .expect("registry lookup")
            .expect("third candidate");
        assert_eq!(third.0, tokens[2]);
        assert!(
            lock::lock(&pending, "test pending registry")
                .expect("pending registry")
                .by_token
                .contains_key(&third.0.0),
            "selection and elapsed time alone must never delete a reservation"
        );
    }

    #[test]
    fn completed_reservations_leave_no_reconciliation_tombstones() {
        let pending = Mutex::new(PendingRegistry::default());
        let now = Instant::now();
        for index in 0..64 {
            let reservation = register_pending_delivery(
                &pending,
                lifecycle_envelope(),
                DeliveryFence {
                    epoch: "epoch-a".to_string(),
                    entry_id: format!("{index}-0"),
                    owner: "worker-a".to_string(),
                    deliveries: 1,
                },
                now + Duration::from_secs(30),
            )
            .expect("register delivery")
            .expect("new delivery");
            let entry = pending_entry(&pending, &reservation.token)
                .expect("pending lookup")
                .expect("pending entry");
            forget_pending_entry(&pending, &reservation.token, &entry)
                .expect("forget completed delivery");
        }

        let registry = lock::lock(&pending, "test pending registry").expect("pending registry");
        assert!(registry.reconciliation_queue.is_empty());
        assert!(registry.reconciliation_deadlines.is_empty());
    }

    #[test]
    fn cancelled_reconciliation_requeues_its_in_flight_candidate() {
        let (pending, token) = pending_fixture();
        mark_reconciliation_needed(&pending, &token).expect("schedule reconciliation");
        let (selected_token, entry) = take_reconciliation_candidate(&pending, Instant::now())
            .expect("candidate lookup")
            .expect("scheduled candidate");

        {
            let _in_flight = ReconciliationCandidate::new(&pending, selected_token, entry);
        }

        let retried = take_reconciliation_candidate(&pending, Instant::now())
            .expect("candidate lookup after cancellation")
            .expect("dropped candidate must be requeued");
        assert_eq!(retried.0, token);
    }

    #[test]
    fn reconciliation_reschedules_and_reports_status_errors() {
        let (pending, token) = pending_fixture();
        mark_reconciliation_needed(&pending, &token).expect("schedule reconciliation");
        let (selected_token, entry) = take_reconciliation_candidate(&pending, Instant::now())
            .expect("candidate lookup")
            .expect("scheduled candidate");
        let candidate = ReconciliationCandidate::new(&pending, selected_token, entry);

        let error = finish_reconciliation_candidate(
            candidate,
            Err(FrameworkError::internal("injected status failure")),
        )
        .expect_err("status failures must reach pop's logging path");
        assert!(error.to_string().contains("injected status failure"));

        let retried = take_reconciliation_candidate(&pending, Instant::now())
            .expect("candidate lookup after error")
            .expect("failed status probe must remain scheduled");
        assert_eq!(retried.0, token);
    }

    #[tokio::test]
    async fn reconciliation_skips_a_busy_lifecycle_without_waiting() {
        let (pending, token) = pending_fixture();
        let entry = pending_entry(&pending, &token)
            .expect("pending lookup")
            .expect("pending entry");
        let _lifecycle = entry.lifecycle.lock().await;

        let probe = tokio::time::timeout(Duration::from_millis(10), async {
            reconciliation_probe(&token, &entry)
        })
        .await
        .expect("reconciliation probe must not await the lifecycle mutex");
        assert!(matches!(probe, ReconciliationProbe::Busy));
    }

    #[test]
    fn pop_probe_budget_is_fixed_and_independent_of_the_requested_lease() {
        assert_eq!(pop_probe_budget(Duration::ZERO), Duration::from_millis(100));
        assert_eq!(
            pop_probe_budget(Duration::from_secs(60)),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn pending_metadata_anchors_the_remaining_lease_at_query_start() {
        let response = pending_snapshot_reply("epoch-a", "1-0", "worker-a", 250, 3);
        let snapshot = parse_pending_snapshot(&response).expect("valid delivery snapshot");
        let metadata = snapshot.metadata.expect("valid epoch plus XPENDING row");
        assert_eq!(metadata.fence.entry_id, "1-0");
        assert_eq!(metadata.fence.owner, "worker-a");
        assert_eq!(metadata.idle, Duration::from_millis(250));
        assert_eq!(metadata.fence.deliveries, 3);

        let query_started = std::time::Instant::now();
        let decision = verified_pending_snapshot(
            Some(&response),
            "epoch-a",
            "1-0",
            "worker-a",
            query_started,
            Duration::from_secs(5),
            query_started + Duration::from_millis(100),
        )
        .expect("the same consumer still has a live lease");
        assert_eq!(
            decision.deadline,
            query_started + Duration::from_millis(4_750)
        );
        assert_eq!(decision.fence, metadata.fence);

        assert!(
            verified_pending_snapshot(
                None,
                "epoch-a",
                "1-0",
                "worker-a",
                query_started,
                Duration::from_secs(5),
                query_started + Duration::from_millis(100),
            )
            .is_none(),
            "a command error cannot fabricate authoritative ownership metadata"
        );
    }

    #[test]
    fn pending_metadata_rejects_missing_malformed_foreign_and_elapsed_entries() {
        assert!(parse_pending_metadata(&redis::Value::Array(vec![])).is_none());
        assert!(
            parse_pending_metadata(&redis::Value::Array(vec![redis::Value::Array(vec![
                redis::Value::BulkString(b"1-0".to_vec()),
                redis::Value::BulkString(b"worker-a".to_vec()),
                redis::Value::Int(10),
            ])]))
            .is_none()
        );

        let query_started = std::time::Instant::now();
        let foreign = parse_pending_metadata(&pending_reply("1-0", "worker-b", 10, 1))
            .expect("well-shaped foreign row");
        assert!(
            current_pending_lease(
                &foreign,
                "1-0",
                "worker-a",
                query_started,
                Duration::from_secs(5),
                query_started,
            )
            .is_none()
        );

        let elapsed = parse_pending_metadata(&pending_reply("1-0", "worker-a", 4_000, 1))
            .expect("well-shaped elapsed row");
        assert!(
            current_pending_lease(
                &elapsed,
                "1-0",
                "worker-a",
                query_started,
                Duration::from_secs(5),
                query_started + Duration::from_secs(2),
            )
            .is_none()
        );
        assert!(
            verified_pending_snapshot(
                None,
                "epoch-a",
                "1-0",
                "worker-a",
                query_started,
                Duration::from_secs(5),
                query_started + Duration::from_secs(5),
            )
            .is_none()
        );
    }
}
