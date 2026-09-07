//! Redis-backed L1 [`RenderStore`]: one hash per key, shared by every node
//! pointed at the same Redis.
//!
//! # What the hash holds
//!
//! | Field | Meaning |
//! |---|---|
//! | `bytes` | the encoded entry, opaque here: its integrity is the codec's to check |
//! | `epoch`, `token`, `digest` | the [`PublicationFence`] the entry was published under |
//! | `published_at_ms` | the publisher's own publication instant, carried back in [`StoredEntry`] |
//!
//! The key is `<prefix>entry:` plus [`RenderKey::to_base64url`], never a
//! second hash of the key.
//!
//! # Fencing
//!
//! [`PublicationFence::supersedes`] decides every replacement, and it decides
//! it inside one Lua script: the stored `(epoch, token)` is read and compared
//! in the same atomic step that writes, so two nodes publishing the same
//! brand-new key cannot both find it absent and let the lower fence land
//! second. The script answers an explicit published or fenced status rather
//! than leaving the caller to infer one from a reply shape.
//!
//! # Retention is Redis's own
//!
//! `retention_ms` becomes the hash's key lifetime (`PEXPIRE`), so Redis
//! reclaims an entry's bytes by itself and this tier needs no sweep. That is
//! the difference from the SQL tier, whose rows a bounded sweep removes.
//! `u64::MAX` - the contract's "never age-swept" - stores the hash with no
//! lifetime at all.
//!
//! # Bounds
//!
//! `max_bytes` bounds one entry and is checked before any command runs.
//! Nothing bounds the keyspace: it is shared by every node, so no single
//! process holds an accurate picture of it, and growth is bounded by
//! retention rather than by eviction - the same rule the SQL tier follows.
//! [`RenderStore::inspect`] is bounded rather than exhaustive for the same
//! reason: it reports what a capped `SCAN` found, and says so when it stops
//! at the cap.

use async_trait::async_trait;
use bytes::Bytes;
use redis::Script;
use suprnova_live::render_cache::RenderCacheError;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::store::{
    PublicationFence, PublishOutcome, RenderStore, StoreInspection, StoredEntry,
};

use super::as_i64;
use super::redis::{
    RENDER_CACHE_REDIS_URL, RedisProvider, RedisProviderConfig, SharedScript, entry_key,
    entry_scan_pattern, provider_error, unreadable_status,
};
use crate::FrameworkError;

/// Entry hashes one [`RenderStore::inspect`] call looks at before it reports
/// what it has.
///
/// Inspection answers an operator's question about how large L1 has grown,
/// against a keyspace every node writes to, so it is bounded rather than
/// exhaustive: a store holding millions of entries answers in predictable
/// time and reports an approximate, bounded count instead of stalling on a
/// full keyspace walk. Approximate in both directions, not a floor: `SCAN`
/// guarantees only that every key present for the whole scan is returned at
/// least once, so a key returned by two rounds is counted twice, while the
/// caps below stop the scan before it has seen everything.
const INSPECT_KEY_CAP: usize = 10_000;

/// Entry hashes one `SCAN` round asks Redis for. A hint, not a guarantee -
/// it bounds the work of one round and the size of one reply.
const INSPECT_SCAN_BATCH: usize = 256;

/// `SCAN` rounds one inspection runs before it reports what it has.
///
/// The key cap alone does not bound the work: `MATCH` filters after the
/// server has scanned, so a keyspace full of another application's keys could
/// take thousands of rounds to yield ten thousand of this deployment's.
/// Either bound stopping the scan is the same capped inspection.
const INSPECT_SCAN_ROUNDS: usize = 1_000;

/// The largest retention this store hands Redis as a key lifetime.
///
/// Redis refuses a lifetime whose absolute deadline overflows its own signed
/// millisecond clock, and `u64::MAX` - the retention that means "never
/// age-swept" - is far past that. Ten years and beyond is therefore stored
/// with no lifetime at all, which is what a caller asking for a retention
/// that long is asking for.
const MAX_FINITE_RETENTION_MS: u64 = 10 * 365 * 24 * 60 * 60 * 1_000;

/// The retention argument that means "store this entry with no lifetime".
const NEVER_EXPIRES: i64 = -1;

/// Publishes one entry under its fence, or reports that a stored fence holds
/// the key.
///
/// `KEYS[1]` is the entry hash. `ARGV` is, in order: the encoded entry, the
/// fence's epoch, its token, its digest as hex, the publication instant, and
/// the retention in milliseconds - [`NEVER_EXPIRES`] for an entry Redis must
/// not age off on its own.
///
/// Returns `1` when the entry was stored and `0` when the stored fence
/// superseded it. The comparison is [`PublicationFence::supersedes`]: a
/// higher epoch, or an equal epoch with a higher token. A hash missing either
/// fence field is not a publication and is overwritten.
const PUBLISH_LUA: &str = r"
local held = redis.call('HMGET', KEYS[1], 'epoch', 'token')
local epoch = tonumber(ARGV[2])
local token = tonumber(ARGV[3])
if held[1] and held[2] then
    local held_epoch = tonumber(held[1])
    local held_token = tonumber(held[2])
    if held_epoch and held_token then
        if not (epoch > held_epoch or (epoch == held_epoch and token > held_token)) then
            return 0
        end
    end
end
redis.call('HSET', KEYS[1],
    'bytes', ARGV[1],
    'epoch', ARGV[2],
    'token', ARGV[3],
    'digest', ARGV[4],
    'published_at_ms', ARGV[5])
local retention = tonumber(ARGV[6])
if retention >= 0 then
    redis.call('PEXPIRE', KEYS[1], retention)
else
    redis.call('PERSIST', KEYS[1])
end
return 1
";

/// Counts one bounded `SCAN` round of this deployment's entry hashes.
///
/// `ARGV` is the cursor, the `MATCH` pattern, and the `COUNT` hint. Returns
/// the next cursor, how many entry hashes the round matched, how many of
/// those carry an entry, and how many bytes those entries occupy. Counting
/// inside the script is what keeps an inspection to one round trip per `SCAN`
/// round rather than one per key.
const INSPECT_LUA: &str = r"
local scanned = redis.call('SCAN', ARGV[1], 'MATCH', ARGV[2], 'COUNT', ARGV[3])
local names = scanned[2]
local entries = 0
local bytes = 0
for index = 1, #names do
    if redis.call('HEXISTS', names[index], 'bytes') == 1 then
        entries = entries + 1
        bytes = bytes + redis.call('HSTRLEN', names[index], 'bytes')
    end
end
return {scanned[1], #names, entries, bytes}
";

/// [`PUBLISH_LUA`], built once so every publication after the first ships a
/// hash rather than the whole body.
static PUBLISH: SharedScript = SharedScript::new(|| Script::new(PUBLISH_LUA));

/// [`INSPECT_LUA`], built once for the same reason.
static INSPECT: SharedScript = SharedScript::new(|| Script::new(INSPECT_LUA));

/// Redis-backed L1 store. See the module documentation for the hash layout,
/// the fence, and what retention means here.
///
/// `Debug` prints the bound and the namespace only: no endpoint, and no
/// stored entry.
#[derive(Debug)]
pub struct RedisRenderStore {
    provider: RedisProvider,
    max_bytes: u64,
}

impl RedisRenderStore {
    /// A store over `config`'s Redis and namespace, refusing any single entry
    /// larger than `max_bytes`.
    ///
    /// Building a store does not reach Redis; see `RedisProvider` for why,
    /// and for what does.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when the configured URL is not a usable
    /// Redis connection URL. The message names the setting and never repeats
    /// its value, which can carry a password.
    pub async fn connect(
        config: &RedisProviderConfig,
        max_bytes: u64,
    ) -> Result<Self, FrameworkError> {
        Ok(Self {
            provider: RedisProvider::connect(config, RENDER_CACHE_REDIS_URL).await?,
            max_bytes,
        })
    }
}

/// The retention argument [`PUBLISH_LUA`] reads: milliseconds, or
/// [`NEVER_EXPIRES`].
fn retention_argument(retention_ms: u64) -> i64 {
    if retention_ms >= MAX_FINITE_RETENTION_MS {
        NEVER_EXPIRES
    } else {
        as_i64(retention_ms)
    }
}

/// The text of one hash field, or `None` when the field is not UTF-8.
///
/// Every field this build writes but `bytes` is decimal or hex, which is
/// ASCII. A field holding anything else was written by something else, and
/// the decoders below have to be given the chance to say so: asking the
/// driver for a `String` instead would make it a decoding *error*, which
/// this store would have to report as a provider failure - an outage - for
/// a hash Redis is holding perfectly well.
fn field_text(field: Vec<u8>) -> Option<String> {
    String::from_utf8(field).ok()
}

/// A 64-character lowercase hex fence digest, or `None` when the field does
/// not hold one.
fn decode_digest(text: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(text).ok()?;
    bytes.try_into().ok()
}

/// One of the counters this store writes as decimal text, or `None` when the
/// field does not hold one.
///
/// Every value this build writes is a `u64` rendered by `as_i64`, so anything
/// else in the field was written by something else and describes no
/// publication this build can serve.
fn decode_number(text: &str) -> Option<u64> {
    text.parse().ok()
}

#[async_trait]
impl RenderStore for RedisRenderStore {
    async fn get(&self, key: &RenderKey) -> Result<Option<StoredEntry>, RenderCacheError> {
        let mut conn = self.provider.connection();
        // Every field comes back as bytes, the text ones included: a hash
        // holding something that is not UTF-8 in a field this build writes
        // as decimal or hex is another writer's key or a torn write, and
        // asking the driver for a `String` would make that a decoding error
        // - a provider failure, which the middleware's failure policy can
        // turn into a failed request. It is a miss, exactly as a missing
        // field is.
        let (bytes, epoch, token, digest, published_at_ms): (
            Option<Vec<u8>>,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
        ) = redis::cmd("HMGET")
            .arg(entry_key(self.provider.prefix(), key))
            .arg("bytes")
            .arg("epoch")
            .arg("token")
            .arg("digest")
            .arg("published_at_ms")
            .query_async(&mut conn)
            .await
            .map_err(|error| provider_error(&error))?;

        // A hash missing any field this build writes with the others is not
        // an entry: it is a torn write or another writer's key, and there is
        // nothing to serve. The same answer the file and SQL tiers give a
        // frame they cannot read.
        let (Some(bytes), Some(epoch), Some(token), Some(digest), Some(published_at_ms)) =
            (bytes, epoch, token, digest, published_at_ms)
        else {
            return Ok(None);
        };
        // Every one of the four is decoded here rather than by the driver,
        // so a value this build did not write is a miss rather than a
        // provider failure. A hash Redis is holding perfectly well is not an
        // outage, and reporting one would put the middleware's failure
        // policy - which can fail the request outright - on the wrong side
        // of a merely unreadable entry.
        let (Some(epoch), Some(token), Some(published_at_ms), Some(generation_digest)) = (
            field_text(epoch).as_deref().and_then(decode_number),
            field_text(token).as_deref().and_then(decode_number),
            field_text(published_at_ms)
                .as_deref()
                .and_then(decode_number),
            field_text(digest).as_deref().and_then(decode_digest),
        ) else {
            tracing::warn!(
                target: "suprnova::render_cache",
                "render cache Redis entry carries a publication this build cannot read and is treated as a miss",
            );
            return Ok(None);
        };
        Ok(Some(StoredEntry {
            bytes: Bytes::from(bytes),
            published_at_ms,
            fence: PublicationFence {
                epoch,
                generation_digest,
                token,
            },
        }))
    }

    async fn publish(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        retention_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        // Bounds before allocation: an entry over the bound is refused before
        // a command is built or its bytes reach the wire. A store bounded to
        // zero bytes holds nothing at all, matching every sibling store,
        // since `0 > 0` is false.
        if self.max_bytes == 0 || bytes.len() as u64 > self.max_bytes {
            return Ok(PublishOutcome::Rejected);
        }
        let mut conn = self.provider.connection();
        // Deliberately not retried on a transient failure. The script writes,
        // and a retry after a socket drop whose command Redis did execute
        // would republish an entry whose fence a peer may since have
        // superseded.
        let published: i64 = PUBLISH
            .key(entry_key(self.provider.prefix(), key))
            .arg(bytes.as_ref())
            .arg(as_i64(fence.epoch))
            .arg(as_i64(fence.token))
            .arg(hex::encode(fence.generation_digest))
            .arg(as_i64(now_ms))
            .arg(retention_argument(retention_ms))
            .invoke_async(&mut conn)
            .await
            .map_err(|error| provider_error(&error))?;
        match published {
            1 => Ok(PublishOutcome::Published),
            0 => Ok(PublishOutcome::Fenced),
            _ => Err(unreadable_status()),
        }
    }

    async fn evict(&self, key: &RenderKey) -> Result<(), RenderCacheError> {
        let mut conn = self.provider.connection();
        let _: i64 = redis::cmd("DEL")
            .arg(entry_key(self.provider.prefix(), key))
            .query_async(&mut conn)
            .await
            .map_err(|error| provider_error(&error))?;
        Ok(())
    }

    async fn inspect(&self) -> Result<StoreInspection, RenderCacheError> {
        let mut conn = self.provider.connection();
        let pattern = entry_scan_pattern(self.provider.prefix());
        let mut cursor = "0".to_owned();
        let mut seen = 0_usize;
        let mut inspection = StoreInspection {
            entries: 0,
            bytes: 0,
        };
        for _ in 0..INSPECT_SCAN_ROUNDS {
            let (next, matched, entries, bytes): (String, usize, usize, usize) = INSPECT
                .arg(&cursor)
                .arg(&pattern)
                .arg(INSPECT_SCAN_BATCH)
                .invoke_async(&mut conn)
                .await
                .map_err(|error| provider_error(&error))?;
            seen = seen.saturating_add(matched);
            inspection.entries = inspection.entries.saturating_add(entries);
            inspection.bytes = inspection.bytes.saturating_add(bytes);
            cursor = next;
            if cursor == "0" {
                return Ok(inspection);
            }
            if seen >= INSPECT_KEY_CAP {
                break;
            }
        }
        // Reported rather than silently partial: an operator reading these
        // numbers has to know they are approximate and bounded - the scan
        // stopped early, and `SCAN` may have returned one key more than once
        // along the way.
        tracing::warn!(
            target: "suprnova::render_cache",
            entries = inspection.entries,
            "render cache Redis inspection stopped at its scan cap; the counts are approximate",
        );
        Ok(inspection)
    }
}

#[cfg(test)]
mod tests {
    //! The publication script's argument layout, the inspection round's, and
    //! the retention rule.
    //!
    //! Behaviour is proven against a real Redis in
    //! `framework/tests/render_cache/tiers.rs`. What those tests cannot show
    //! is the shape of the scripts themselves - which argument carries what,
    //! and that the fence comparison and the write are one step - which is
    //! the part a later edit could silently invert.
    use super::*;

    #[test]
    fn the_publication_binds_every_argument_the_script_reads() {
        // Spelled out on both sides so an edit to either that forgets the
        // other fails here rather than storing an epoch as a token.
        assert!(
            PUBLISH_LUA.contains("local epoch = tonumber(ARGV[2])"),
            "{PUBLISH_LUA}"
        );
        assert!(
            PUBLISH_LUA.contains("local token = tonumber(ARGV[3])"),
            "{PUBLISH_LUA}"
        );
        assert!(PUBLISH_LUA.contains("'bytes', ARGV[1]"), "{PUBLISH_LUA}");
        assert!(PUBLISH_LUA.contains("'digest', ARGV[4]"), "{PUBLISH_LUA}");
        assert!(
            PUBLISH_LUA.contains("'published_at_ms', ARGV[5]"),
            "{PUBLISH_LUA}"
        );
        assert!(
            PUBLISH_LUA.contains("local retention = tonumber(ARGV[6])"),
            "{PUBLISH_LUA}"
        );
        assert_eq!(
            PUBLISH.get_hash().len(),
            40,
            "the built script is cached by its SHA-1"
        );
    }

    #[test]
    fn the_script_carries_the_supersedes_comparison_itself() {
        // A higher epoch, or an equal epoch with a higher token: exactly
        // `PublicationFence::supersedes`. Reading the fence first and writing
        // afterwards, in two round trips, would let two nodes publishing one
        // brand-new key both find it absent and the lower fence land second.
        assert!(
            PUBLISH_LUA
                .contains("epoch > held_epoch or (epoch == held_epoch and token > held_token)"),
            "{PUBLISH_LUA}"
        );
        assert!(
            PUBLISH_LUA.contains("return 0"),
            "a fenced publication is an explicit status: {PUBLISH_LUA}"
        );
        assert!(
            PUBLISH_LUA.contains("return 1"),
            "and so is a published one: {PUBLISH_LUA}"
        );
        assert!(
            !PUBLISH_LUA.contains("KEYS[2]"),
            "one key per publication: {PUBLISH_LUA}"
        );
    }

    #[test]
    fn no_caller_value_is_spliced_into_a_script_body() {
        // Every script here is a constant. A body built with `format!` from a
        // caller value would be a Lua injection, and a body rebuilt per call
        // would defeat the `EVALSHA` fast path as well.
        for body in [PUBLISH_LUA, INSPECT_LUA] {
            assert!(
                !body.contains("{}"),
                "a script is a constant, never a template: {body}"
            );
        }
    }

    #[test]
    fn the_inspection_round_scans_and_measures_in_one_script() {
        assert!(
            INSPECT_LUA.contains("redis.call('SCAN', ARGV[1], 'MATCH', ARGV[2], 'COUNT', ARGV[3])"),
            "{INSPECT_LUA}"
        );
        assert!(
            INSPECT_LUA.contains("HSTRLEN"),
            "bytes are measured where they are, never fetched to be counted: {INSPECT_LUA}"
        );
        assert!(
            INSPECT_LUA.contains("return {scanned[1], #names, entries, bytes}"),
            "the cursor, the keys matched, the entries, and the bytes: {INSPECT_LUA}"
        );
    }

    #[test]
    fn a_retention_beyond_ten_years_stores_the_entry_with_no_lifetime() {
        // `u64::MAX` is the contract's "never age-swept", and Redis refuses a
        // lifetime that large outright, so it becomes no lifetime at all.
        assert_eq!(retention_argument(u64::MAX), NEVER_EXPIRES);
        assert_eq!(retention_argument(MAX_FINITE_RETENTION_MS), NEVER_EXPIRES);
        // Everything shorter is honoured as given, including zero, which is
        // an ordinary value meaning "dead the instant it is published" and
        // which Redis reads as a key to remove.
        assert_eq!(retention_argument(0), 0);
        assert_eq!(retention_argument(60_000), 60_000);
        assert_eq!(
            retention_argument(MAX_FINITE_RETENTION_MS - 1),
            as_i64(MAX_FINITE_RETENTION_MS - 1)
        );
    }

    #[test]
    fn a_field_that_is_not_text_at_all_reads_back_as_nothing() {
        // The gate in front of both decoders. Without it the driver decodes
        // the field, and a hash carrying a stray byte in `epoch` becomes a
        // provider failure - which the middleware's closed failure policy
        // can turn into a failed request - rather than the miss it is.
        assert_eq!(field_text(b"1730".to_vec()).as_deref(), Some("1730"));
        assert_eq!(field_text(vec![0xff, 0xfe]), None);
        assert_eq!(
            field_text(vec![0xff]).as_deref().and_then(decode_number),
            None
        );
        assert_eq!(
            field_text(vec![0xff]).as_deref().and_then(decode_digest),
            None
        );
    }

    #[test]
    fn a_counter_field_this_build_did_not_write_reads_back_as_nothing() {
        assert_eq!(decode_number("0"), Some(0));
        assert_eq!(decode_number("18446744073709551615"), Some(u64::MAX));
        // Not a provider failure: a field holding something else is an entry
        // this build cannot serve, and a miss is what the caller does with it.
        for unreadable in ["", " 1", "-1", "1.5", "0x10", "many"] {
            assert_eq!(decode_number(unreadable), None, "{unreadable}");
        }
    }

    #[test]
    fn a_digest_that_is_not_thirty_two_bytes_of_hex_is_no_digest() {
        assert_eq!(decode_digest(&"00".repeat(32)), Some([0_u8; 32]));
        assert_eq!(decode_digest(""), None);
        assert_eq!(decode_digest("not hex"), None);
        assert_eq!(decode_digest(&"00".repeat(31)), None);
        assert_eq!(decode_digest(&"00".repeat(33)), None);
    }
}
