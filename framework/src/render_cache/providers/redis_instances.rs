//! Redis-backed [`InstanceRecordStore`]: one hash per Live instance, one per
//! reserved retry identity, and one sorted set indexing the instances by
//! their deadlines.
//!
//! # What the keys hold
//!
//! | Key | Field | Meaning |
//! |---|---|---|
//! | `<prefix>instance:<scope>:<instance>` | `record` | the encoded record, opaque here: only the kernel's codec reads it |
//! | | `version` | the version a compare-and-store must carry to replace this state |
//! | | `expires_at_ms` | store-time deadline after which the record is gone |
//! | `<prefix>promotion:<scope>:<idempotency>` | | the same three fields; nothing ever replaces a reservation, so its version stays at one |
//! | `<prefix>instances` | | sorted set, one member per instance record, scored by `expires_at_ms` |
//!
//! `<scope>`, `<instance>`, and `<idempotency>` are lowercase hex of
//! identities the engine validated on the way in.
//!
//! Records are bytes and stay bytes. This adapter never decodes one, never
//! logs one, and never puts one in a message: the point of the port is that
//! authority is the kernel's and storage is the host's.
//!
//! # One clock
//!
//! `expires_at_ms` is the deadline the kernel computed on *its* clock and
//! handed over as an argument; every comparison against it is
//! `redis.call('TIME')` read inside the script that reads or guards the hash.
//! A record past that deadline answers exactly as one that was never written,
//! which is what the port promises. Each hash also carries `PEXPIREAT` at the
//! same deadline, so Redis reclaims its bytes on its own; a hash Redis has
//! already removed is simply absent, which is the same answer.
//!
//! # No host transaction, ever
//!
//! Unlike the SQL record store, this one cannot join the host's ambient
//! transaction: Redis has no transaction for it to join, and a `MULTI` would
//! not be one - it cannot span the host's database work, and it cannot roll
//! back. A claim taken here therefore stands whether or not the request that
//! took it commits, so a Redis-backed Live ledger claims the successor first
//! and the host's effects follow. The Live specification allows either
//! coupling; a deployment that needs a claim to disappear with a rolled-back
//! request chooses the database tier.
//!
//! # Bounds
//!
//! A record over
//! [`MAX_RECORD_BYTES`] is refused
//! before any command is built, and reclamation of elapsed records is bounded
//! to `RECLAIM_BATCH` members per creating operation - the same rule, and
//! the same number, the in-memory reference store applies, so a burst of
//! expiries that arrive together is paid for over the operations that follow
//! rather than by whichever one is unlucky. Counting the live records is a
//! `ZCOUNT` over the index rather than a scan of the keyspace, so a mount
//! costs one logarithmic range count however many instances exist.

use async_trait::async_trait;
use suprnova_live::identity::UnixMillis;
use suprnova_live::ledger::{
    CasOutcome, InstanceRecordKey, InstanceRecordStore, LedgerError, LedgerErrorKind,
    MAX_RECORD_BYTES, PromotionRecordKey, StoredRecord,
};

use super::redis::{
    LIVE_REDIS_URL, RedisProvider, RedisProviderConfig, SharedScript, StoreTimeOffset,
    instance_index_key, instance_key, ledger_error, promotion_key, timed_script,
    unreadable_ledger_status,
};
use super::{as_i64, as_u64};
use crate::FrameworkError;

/// Elapsed records one creating operation reclaims.
///
/// Bounded for the reason the in-memory reference store bounds it: a store
/// holding a million records that all elapse at once still answers in
/// predictable time, and the backlog drains over the operations that follow.
/// Reclamation is hygiene, never a correctness gate - every read and every
/// guard already refuses a record past its deadline whether or not a batch
/// has reached it.
pub(crate) const RECLAIM_BATCH: usize = 64;

/// The status a script returns for a key no unexpired record holds.
const MISSING: i64 = 0;
/// The status a compare-and-store returns when the record moved on.
const CONFLICT: i64 = 1;
/// The status a compare-and-store returns when it replaced the record.
const STORED: i64 = 2;
/// The status a read returns when it found a live record.
const PRESENT: i64 = 1;
/// The status a creation returns when a live record already held the key.
const NOT_CREATED: i64 = 0;
/// The status a creation returns when this call created the record.
const CREATED: i64 = 1;

/// Reads one record, treating an elapsed one as absent.
///
/// `KEYS[1]` is the record hash and `ARGV[1]` the store-time test offset.
/// Returns `{0, '', 0, 0}` when nothing live holds the key and
/// `{1, record, version, expires_at_ms}` when something does. One script
/// serves instance records and promotion reservations, which carry the same
/// three fields.
const LOAD_LUA: &str = r"
local now = suprnova_store_now(tonumber(ARGV[1]))
local held = redis.call('HMGET', KEYS[1], 'record', 'version', 'expires_at_ms')
if not held[1] or not held[2] or not held[3] then
    return {0, '', 0, 0}
end
local expires_at = tonumber(held[3])
if not expires_at or expires_at <= now then
    return {0, '', 0, 0}
end
return {1, held[1], tonumber(held[2]), expires_at}
";

/// Creates one instance record when no unexpired one holds the key, and
/// reclaims a bounded batch of elapsed records on the way.
///
/// `KEYS[1]` is the record hash and `KEYS[2]` the expiry index. `ARGV` is the
/// encoded record, its deadline, the store-time test offset, and the
/// reclamation batch size.
///
/// Returns `1` when this call created the record and `0` when a live record
/// already held the key. An elapsed record is not a holder and is replaced.
/// The index member is the record's own key, so reclaiming a member and
/// removing the hash it names are one step.
const INSERT_LUA: &str = r"
local now = suprnova_store_now(tonumber(ARGV[3]))
local due = redis.call('ZRANGEBYSCORE', KEYS[2], '-inf', string.format('%d', now),
    'LIMIT', 0, tonumber(ARGV[4]))
if #due > 0 then
    redis.call('ZREM', KEYS[2], unpack(due))
    redis.call('DEL', unpack(due))
end
local expires_at = tonumber(redis.call('HGET', KEYS[1], 'expires_at_ms'))
if expires_at and expires_at > now then
    return 0
end
redis.call('DEL', KEYS[1])
redis.call('HSET', KEYS[1], 'record', ARGV[1], 'version', 1, 'expires_at_ms', ARGV[2])
redis.call('PEXPIREAT', KEYS[1], ARGV[2])
redis.call('ZADD', KEYS[2], ARGV[2], KEYS[1])
return 1
";

/// Creates one promotion reservation under the rules [`INSERT_LUA`] creates a
/// record under.
///
/// `KEYS[1]` is the reservation hash. `ARGV` is the encoded reservation, its
/// deadline, and the store-time test offset. A reservation is only ever
/// looked up by its own key, so it joins no index and needs no reclamation
/// pass: `PEXPIREAT` is what removes it.
const INSERT_PROMOTION_LUA: &str = r"
local now = suprnova_store_now(tonumber(ARGV[3]))
local expires_at = tonumber(redis.call('HGET', KEYS[1], 'expires_at_ms'))
if expires_at and expires_at > now then
    return 0
end
redis.call('DEL', KEYS[1])
redis.call('HSET', KEYS[1], 'record', ARGV[1], 'version', 1, 'expires_at_ms', ARGV[2])
redis.call('PEXPIREAT', KEYS[1], ARGV[2])
return 1
";

/// Replaces one record only while it still carries the version the caller
/// read and is still there by store time.
///
/// `KEYS[1]` is the record hash and `KEYS[2]` the expiry index. `ARGV` is the
/// encoded record, the version the caller read, the new deadline, and the
/// store-time test offset.
///
/// Returns `{0, 0}` when no unexpired record holds the key, `{1, 0}` when one
/// does at another version, and `{2, version}` when this call replaced it.
/// The expiry guard is not redundant with the version guard: without it an
/// elapsed record would be replaced back into life at a fresh deadline, which
/// would turn "this record is gone" into "this record is current" across
/// every node reading it.
const COMPARE_AND_STORE_LUA: &str = r"
local now = suprnova_store_now(tonumber(ARGV[4]))
local held = redis.call('HMGET', KEYS[1], 'version', 'expires_at_ms')
local version = tonumber(held[1])
local expires_at = tonumber(held[2])
if not version or not expires_at or expires_at <= now then
    return {0, 0}
end
if version ~= tonumber(ARGV[2]) then
    return {1, 0}
end
local next_version = version + 1
redis.call('HSET', KEYS[1], 'record', ARGV[1], 'version', next_version, 'expires_at_ms', ARGV[3])
redis.call('PEXPIREAT', KEYS[1], ARGV[3])
redis.call('ZADD', KEYS[2], ARGV[3], KEYS[1])
return {2, next_version}
";

/// Removes one instance record and its index member together.
///
/// `KEYS[1]` is the record hash and `KEYS[2]` the expiry index. Removing a
/// record that is not there is not a failure. The two removals are one script
/// because a hash removed without its member would leave the index naming a
/// key that no longer exists.
const REMOVE_LUA: &str = r"
redis.call('DEL', KEYS[1])
redis.call('ZREM', KEYS[2], KEYS[1])
return 1
";

/// Counts the instance records that are still live by store time.
///
/// `KEYS[1]` is the expiry index and `ARGV[1]` the store-time test offset.
/// The exclusive lower bound is store time itself, so a member whose deadline
/// has passed is not counted whether or not a reclamation pass has reached
/// it - which is what keeps the configured capacity measured against records
/// that actually exist.
const COUNT_LUA: &str = r"
local now = suprnova_store_now(tonumber(ARGV[1]))
return redis.call('ZCOUNT', KEYS[1], string.format('(%d', now), '+inf')
";

/// [`LOAD_LUA`], built once so every read after the first ships a hash rather
/// than the whole body.
static LOAD: SharedScript = SharedScript::new(|| timed_script(LOAD_LUA));

/// [`INSERT_LUA`], built once for the same reason.
static INSERT: SharedScript = SharedScript::new(|| timed_script(INSERT_LUA));

/// [`INSERT_PROMOTION_LUA`], built once for the same reason.
static INSERT_PROMOTION: SharedScript = SharedScript::new(|| timed_script(INSERT_PROMOTION_LUA));

/// [`COMPARE_AND_STORE_LUA`], built once for the same reason.
static COMPARE_AND_STORE: SharedScript = SharedScript::new(|| timed_script(COMPARE_AND_STORE_LUA));

/// [`REMOVE_LUA`], built once for the same reason. It decides nothing by
/// store time, so it needs no clock.
static REMOVE: SharedScript = SharedScript::new(|| redis::Script::new(REMOVE_LUA));

/// [`COUNT_LUA`], built once for the same reason.
static COUNT: SharedScript = SharedScript::new(|| timed_script(COUNT_LUA));

/// Redis-backed Live instance and promotion record store. See the module
/// documentation for the key layout, the clock, and why no host transaction
/// reaches it.
///
/// `Debug` prints the namespace and the test offset only. There is no record
/// state in this type, and if there were, it would not be printed: a record
/// is instance authority, and this crate redacts that everywhere.
#[derive(Debug)]
pub struct RedisInstanceRecordStore {
    provider: RedisProvider,
    time_offset_ms: StoreTimeOffset,
}

impl RedisInstanceRecordStore {
    /// A store over `config`'s Redis and namespace.
    ///
    /// Building a store does not reach Redis; see `RedisProvider` for why,
    /// and for what does.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when the configured URL is not a usable
    /// Redis connection URL. The message names the setting and never repeats
    /// its value, which can carry a password.
    pub async fn connect(config: &RedisProviderConfig) -> Result<Self, FrameworkError> {
        Self::open(config, LIVE_REDIS_URL)
    }

    /// [`Self::connect`] without the `async` marker, naming the setting whose
    /// value produced `config`.
    ///
    /// The Live instance ledger is built by `LiveRuntime::bind`, which is
    /// synchronous, so this store has to be constructible without an `await`
    /// to give it. Nothing is awaited in either form; see `RedisProvider::open`
    /// (crate-private, so this is a plain code span rather than a link) for
    /// what still has to be true of the caller and how that is checked.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when no asynchronous runtime is running on
    /// this thread or the configured URL is not a usable Redis connection
    /// URL. Every message names `setting` and never repeats its value, which
    /// can carry a password.
    pub fn open(
        config: &RedisProviderConfig,
        setting: &'static str,
    ) -> Result<Self, FrameworkError> {
        Ok(Self {
            provider: RedisProvider::open(config, setting)?,
            time_offset_ms: StoreTimeOffset::default(),
        })
    }

    /// Moves this store's view of the Redis clock forward by `offset_ms`
    /// milliseconds, so a test can reach a record's deadline without waiting
    /// for one.
    ///
    /// Never called by production code, and deliberately not a process-wide
    /// switch: parallel tests in one binary would otherwise move each other's
    /// store clocks. Not part of the public contract: doc-hidden, the same
    /// shape as
    /// [`SqlInstanceRecordStore::set_time_offset_for_test`](super::SqlInstanceRecordStore::set_time_offset_for_test).
    #[doc(hidden)]
    pub fn set_time_offset_for_test(&self, offset_ms: u64) {
        self.time_offset_ms.set(offset_ms);
    }

    /// Reads one record from either kind of hash under the store-time guard.
    async fn load_from(&self, name: String) -> Result<Option<StoredRecord>, LedgerError> {
        let mut conn = self.provider.connection();
        let (status, bytes, version, expires_at): (i64, Vec<u8>, u64, u64) = LOAD
            .key(name)
            .arg(self.time_offset_ms.get())
            .invoke_async(&mut conn)
            .await
            .map_err(|error| ledger_error(&error))?;
        match status {
            MISSING => Ok(None),
            PRESENT => Ok(Some(StoredRecord {
                bytes,
                version,
                expires_at: UnixMillis::new(expires_at),
            })),
            _ => Err(unreadable_ledger_status()),
        }
    }
}

/// Refuses a record the codec's own bound would refuse, before any command is
/// built or its bytes reach the wire.
fn within_bounds(bytes: &[u8]) -> Result<(), LedgerError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(LedgerError::new(LedgerErrorKind::CapacityExceeded));
    }
    Ok(())
}

/// Reads a creation script's status as "this call created it".
fn created(status: i64) -> Result<bool, LedgerError> {
    match status {
        CREATED => Ok(true),
        NOT_CREATED => Ok(false),
        _ => Err(unreadable_ledger_status()),
    }
}

#[async_trait]
impl InstanceRecordStore for RedisInstanceRecordStore {
    async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError> {
        self.load_from(instance_key(self.provider.prefix(), key))
            .await
    }

    async fn insert_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        within_bounds(bytes)?;
        let mut conn = self.provider.connection();
        // Deliberately not retried on a transient failure: the script writes,
        // and a retry after a socket drop whose command Redis did execute
        // would answer "a peer holds it" for a record this caller created.
        let status: i64 = INSERT
            .key(instance_key(self.provider.prefix(), key))
            .key(instance_index_key(self.provider.prefix()))
            .arg(bytes)
            .arg(as_i64(expires_at.get()))
            .arg(self.time_offset_ms.get())
            .arg(i64::try_from(RECLAIM_BATCH).unwrap_or(i64::MAX))
            .invoke_async(&mut conn)
            .await
            .map_err(|error| ledger_error(&error))?;
        created(status)
    }

    async fn compare_and_store(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError> {
        within_bounds(bytes)?;
        let mut conn = self.provider.connection();
        let (status, version): (i64, u64) = COMPARE_AND_STORE
            .key(instance_key(self.provider.prefix(), key))
            .key(instance_index_key(self.provider.prefix()))
            .arg(bytes)
            .arg(as_i64(expected_version))
            .arg(as_i64(expires_at.get()))
            .arg(self.time_offset_ms.get())
            .invoke_async(&mut conn)
            .await
            .map_err(|error| ledger_error(&error))?;
        match status {
            MISSING => Ok(CasOutcome::Missing),
            CONFLICT => Ok(CasOutcome::Conflict),
            STORED => Ok(CasOutcome::Stored { version }),
            _ => Err(unreadable_ledger_status()),
        }
    }

    async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
        let mut conn = self.provider.connection();
        let _: i64 = REMOVE
            .key(instance_key(self.provider.prefix(), key))
            .key(instance_index_key(self.provider.prefix()))
            .invoke_async(&mut conn)
            .await
            .map_err(|error| ledger_error(&error))?;
        Ok(())
    }

    async fn load_promotion(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        self.load_from(promotion_key(self.provider.prefix(), key))
            .await
    }

    async fn insert_promotion_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        within_bounds(bytes)?;
        let mut conn = self.provider.connection();
        let status: i64 = INSERT_PROMOTION
            .key(promotion_key(self.provider.prefix(), key))
            .arg(bytes)
            .arg(as_i64(expires_at.get()))
            .arg(self.time_offset_ms.get())
            .invoke_async(&mut conn)
            .await
            .map_err(|error| ledger_error(&error))?;
        created(status)
    }

    async fn count_instances(&self) -> Result<usize, LedgerError> {
        let mut conn = self.provider.connection();
        let live: i64 = COUNT
            .key(instance_index_key(self.provider.prefix()))
            .arg(self.time_offset_ms.get())
            .invoke_async(&mut conn)
            .await
            .map_err(|error| ledger_error(&error))?;
        Ok(usize::try_from(as_u64(live)).unwrap_or(usize::MAX))
    }
}

#[cfg(test)]
mod tests {
    //! The record scripts' key and argument layout.
    //!
    //! Behaviour is proven against a real Redis in
    //! `framework/tests/render_cache/tiers.rs`, including the whole engine
    //! ledger conformance suite over this store. What those tests cannot show
    //! is that each decision is made inside one script against Redis's own
    //! clock, that reclamation is bounded to a batch, and that counting is a
    //! range count rather than a keyspace scan.
    use super::*;

    const SCRIPTS: [&str; 6] = [
        LOAD_LUA,
        INSERT_LUA,
        INSERT_PROMOTION_LUA,
        COMPARE_AND_STORE_LUA,
        REMOVE_LUA,
        COUNT_LUA,
    ];

    #[test]
    fn every_deadline_is_decided_on_redis_own_clock_inside_the_script() {
        for body in [
            LOAD_LUA,
            INSERT_LUA,
            INSERT_PROMOTION_LUA,
            COMPARE_AND_STORE_LUA,
            COUNT_LUA,
        ] {
            assert!(
                body.contains("suprnova_store_now(tonumber(ARGV["),
                "a deadline decided on a node clock is not a deadline: {body}"
            );
        }
        // Removing a record is the kernel's own decision and restores no
        // authority, so it must not need a clock the host may be unable to
        // read.
        assert!(!REMOVE_LUA.contains("suprnova_store_now"), "{REMOVE_LUA}");
    }

    #[test]
    fn a_read_treats_an_elapsed_record_as_one_that_was_never_written() {
        assert!(
            LOAD_LUA.contains("if not expires_at or expires_at <= now then"),
            "{LOAD_LUA}"
        );
        assert!(
            LOAD_LUA.contains("return {0, '', 0, 0}"),
            "absence is a fixed-width status, not a short reply: {LOAD_LUA}"
        );
        assert!(
            LOAD_LUA.contains("return {1, held[1], tonumber(held[2]), expires_at}"),
            "and a hit carries the record, its version, and its deadline: {LOAD_LUA}"
        );
    }

    #[test]
    fn creation_binds_every_argument_the_script_reads() {
        // Pins which argument the *script* reads for what, the way
        // `redis_store`'s publication test pins `PUBLISH_LUA`. It does not
        // check the order the call site binds them in - only a live Redis
        // sees both halves at once, and the `live_redis_*` tests are what
        // prove they agree. What this catches is an edit to the script that
        // moves a position without the reader moving with it.
        assert!(
            INSERT_LUA.contains("'record', ARGV[1], 'version', 1, 'expires_at_ms', ARGV[2]"),
            "ARGV[1] is the encoded record and ARGV[2] its deadline: {INSERT_LUA}"
        );
        assert!(
            INSERT_LUA.contains("redis.call('PEXPIREAT', KEYS[1], ARGV[2])"),
            "and the same deadline is the key's own lifetime: {INSERT_LUA}"
        );
        assert!(
            INSERT_LUA.contains("redis.call('ZADD', KEYS[2], ARGV[2], KEYS[1])"),
            "and its score in the expiry index: {INSERT_LUA}"
        );
        assert!(
            INSERT_LUA.contains("suprnova_store_now(tonumber(ARGV[3]))"),
            "ARGV[3] is the store-time test offset: {INSERT_LUA}"
        );
        assert!(
            INSERT_LUA.contains("'LIMIT', 0, tonumber(ARGV[4])"),
            "ARGV[4] is the reclamation batch: {INSERT_LUA}"
        );
        assert!(
            !INSERT_LUA.contains("ARGV[5]"),
            "the script reads four arguments and no fifth: {INSERT_LUA}"
        );
    }

    #[test]
    fn the_replacement_binds_every_argument_the_script_reads() {
        // Same reading as `creation_binds_every_argument_the_script_reads`
        // above: which argument the script reads for what, not the order the
        // call site binds them in.
        assert!(
            COMPARE_AND_STORE_LUA
                .contains("'record', ARGV[1], 'version', next_version, 'expires_at_ms', ARGV[3]"),
            "ARGV[1] is the encoded record and ARGV[3] the new deadline: \
             {COMPARE_AND_STORE_LUA}"
        );
        assert!(
            COMPARE_AND_STORE_LUA.contains("if version ~= tonumber(ARGV[2]) then"),
            "ARGV[2] is the version the caller read: {COMPARE_AND_STORE_LUA}"
        );
        assert!(
            COMPARE_AND_STORE_LUA.contains("redis.call('PEXPIREAT', KEYS[1], ARGV[3])"),
            "and the new deadline is the key's own lifetime: {COMPARE_AND_STORE_LUA}"
        );
        assert!(
            COMPARE_AND_STORE_LUA.contains("redis.call('ZADD', KEYS[2], ARGV[3], KEYS[1])"),
            "and its score in the expiry index: {COMPARE_AND_STORE_LUA}"
        );
        assert!(
            COMPARE_AND_STORE_LUA.contains("suprnova_store_now(tonumber(ARGV[4]))"),
            "ARGV[4] is the store-time test offset: {COMPARE_AND_STORE_LUA}"
        );
        assert!(
            !COMPARE_AND_STORE_LUA.contains("ARGV[5]"),
            "the script reads four arguments and no fifth: {COMPARE_AND_STORE_LUA}"
        );
    }

    #[test]
    fn creation_reclaims_a_bounded_batch_and_never_replaces_a_live_record() {
        assert!(
            INSERT_LUA.contains("'LIMIT', 0, tonumber(ARGV[4])"),
            "reclamation is bounded to the batch the caller asks for: {INSERT_LUA}"
        );
        assert_eq!(
            RECLAIM_BATCH, 64,
            "the same bound the in-memory reference store applies per operation"
        );
        assert!(
            INSERT_LUA.contains("if expires_at and expires_at > now then\n    return 0"),
            "a live record holds its key against every peer: {INSERT_LUA}"
        );
        assert!(
            INSERT_LUA.contains("redis.call('ZADD', KEYS[2], ARGV[2], KEYS[1])"),
            "and a created record joins the expiry index: {INSERT_LUA}"
        );
        // A reservation is only ever looked up by its own key, so it joins no
        // index; `PEXPIREAT` is the whole of its reclamation.
        assert!(
            !INSERT_PROMOTION_LUA.contains("ZADD"),
            "{INSERT_PROMOTION_LUA}"
        );
        assert!(
            !INSERT_PROMOTION_LUA.contains("KEYS[2]"),
            "{INSERT_PROMOTION_LUA}"
        );
        assert!(
            INSERT_PROMOTION_LUA.contains("redis.call('PEXPIREAT', KEYS[1], ARGV[2])"),
            "{INSERT_PROMOTION_LUA}"
        );
    }

    #[test]
    fn the_replacement_carries_the_version_it_read_and_advances_it_by_one() {
        assert!(
            COMPARE_AND_STORE_LUA
                .contains("if version ~= tonumber(ARGV[2]) then\n    return {1, 0}"),
            "a write derived from a stale read is a conflict: {COMPARE_AND_STORE_LUA}"
        );
        assert!(
            COMPARE_AND_STORE_LUA.contains("local next_version = version + 1"),
            "{COMPARE_AND_STORE_LUA}"
        );
        assert!(
            COMPARE_AND_STORE_LUA.contains("expires_at <= now then\n    return {0, 0}"),
            "and an elapsed record is missing rather than replaced back into life: \
             {COMPARE_AND_STORE_LUA}"
        );
        assert!(
            COMPARE_AND_STORE_LUA.contains("return {2, next_version}"),
            "{COMPARE_AND_STORE_LUA}"
        );
    }

    #[test]
    fn counting_is_a_range_count_over_the_index_and_excludes_elapsed_members() {
        assert!(
            COUNT_LUA.contains("redis.call('ZCOUNT', KEYS[1], string.format('(%d', now), '+inf')"),
            "counting must never scan the keyspace once per mount: {COUNT_LUA}"
        );
        assert!(
            !COUNT_LUA.contains("SCAN")
                && !COUNT_LUA.contains("KEYS ")
                && !COUNT_LUA.contains("ZRANGE"),
            "{COUNT_LUA}"
        );
    }

    #[test]
    fn removing_a_record_removes_its_index_member_in_the_same_step() {
        assert!(
            REMOVE_LUA.contains("redis.call('DEL', KEYS[1])"),
            "{REMOVE_LUA}"
        );
        assert!(
            REMOVE_LUA.contains("redis.call('ZREM', KEYS[2], KEYS[1])"),
            "an index naming a key that no longer exists over-counts capacity: {REMOVE_LUA}"
        );
    }

    #[test]
    fn no_caller_value_is_spliced_into_a_script_body() {
        for body in SCRIPTS {
            assert!(
                !body.contains("{}"),
                "a script is a constant, never a template: {body}"
            );
        }
    }

    #[test]
    fn a_record_over_the_codec_bound_is_refused_before_any_command() {
        assert!(within_bounds(&vec![0_u8; MAX_RECORD_BYTES]).is_ok());
        assert_eq!(
            within_bounds(&vec![0_u8; MAX_RECORD_BYTES + 1])
                .expect_err("a record over the bound is refused")
                .kind(),
            LedgerErrorKind::CapacityExceeded
        );
    }

    #[test]
    fn a_creation_status_this_build_does_not_define_is_a_provider_failure() {
        assert!(created(CREATED).expect("a creation"));
        assert!(!created(NOT_CREATED).expect("a key already held"));
        assert_eq!(
            created(7)
                .expect_err("a status this build does not define")
                .kind(),
            LedgerErrorKind::ProviderUnavailable,
            "answering absence for a reply nobody wrote would invent authority"
        );
        // A created record starts at version one, in the script itself: a
        // compare-and-store the kernel derives from its first read carries
        // that version, so the two have to agree.
        for body in [INSERT_LUA, INSERT_PROMOTION_LUA] {
            assert!(body.contains("'version', 1,"), "{body}");
        }
    }
}
