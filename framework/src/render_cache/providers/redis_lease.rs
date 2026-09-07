//! Redis-backed [`LeaseStore`]: one hash per key, so exactly one node at a
//! time leads the rebuild of a key across every process pointed at the same
//! Redis.
//!
//! # What the keys hold
//!
//! | Key | Field | Meaning |
//! |---|---|---|
//! | `<prefix>lease:<render key>` | `lease_id` | this key's own tenure counter: one on creation, one more on every takeover |
//! | | `epoch` | the authority epoch the current or last tenure was taken at, recorded for inspection and never a condition of acquisition |
//! | | `expires_at_ms` | store time when the current tenure ends; zero once released |
//! | `<prefix>token:<render key>` | | the last publication token minted for this key |
//!
//! `<render key>` is [`RenderKey::to_base64url`], the key itself and never a
//! second hash of it.
//!
//! # One clock, and it is never this node's
//!
//! Expiry is the `expires_at_ms` field compared against `redis.call('TIME')`
//! *inside* the script that guards the key, not a key lifetime Redis
//! enforces. A node whose clock runs fast can therefore neither extend its
//! own lease nor declare a peer's expired, which is the whole reason
//! leadership lives in Redis rather than in a process.
//!
//! # Neither key ever expires
//!
//! The token counter must outlive every tenure: were it to restart, a
//! fenced-out leader's already-minted token could outrank the publication
//! that replaced it, which is exactly the inversion the fence exists to
//! prevent. So it carries no lifetime, and releasing never deletes it.
//!
//! The lease hash carries none either, and for the same shape of reason. Its
//! `lease_id` is what tells a fenced-out holder apart from the holder that
//! replaced it, and it counts up from whatever the hash currently holds. Were
//! Redis to reclaim the hash at the tenure's expiry, the next acquisition
//! would start the count over at one, and a former leader still holding
//! tenure one would mint against a tenure that is not its own. The contract
//! permits a key lifetime as a safety net; this store declines it, exactly as
//! the SQL lease store declines to delete its row, and accepts one small hash
//! per key that has ever been leased - the same bound the token counter
//! already carries.
//!
//! # One script per decision
//!
//! Acquire, mint, and release each read the state, read the clock, and write,
//! inside one script that answers an explicit status. Nothing here infers an
//! outcome from the shape of a reply across two round trips, and nothing
//! polls or waits for another node's lease: a key a peer holds is a bypass.

use async_trait::async_trait;
use suprnova_live::render_cache::RenderCacheError;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::{LeaseAttempt, LeaseStore};

use super::redis::{
    RedisProvider, RedisProviderConfig, SharedScript, StoreTimeOffset, lease_key, provider_error,
    timed_script, token_key, unreadable_status,
};
use super::{as_i64, as_u64};
use crate::FrameworkError;

/// The status [`ACQUIRE_LUA`] returns when an unexpired lease holds the key.
const HELD: i64 = 0;
/// The status it returns when the caller now holds the lease.
const ACQUIRED: i64 = 1;
/// The value [`MINT_LUA`] returns when the caller no longer holds the lease.
/// `INCR` never returns it, so it cannot be confused with a token.
const NOT_HELD: i64 = -1;

/// Takes the lease when no unexpired one holds the key by store time.
///
/// `KEYS[1]` is the lease hash. `ARGV` is the authority epoch, the lease
/// lifetime in milliseconds, and the store-time test offset.
///
/// Returns `{0, 0, 0}` when a peer holds an unexpired lease, and
/// `{1, lease_id, expires_at_ms}` when the caller now holds it. The tenure
/// counter advances from whatever the hash carries, so a tenure id is never
/// reused and a fenced-out holder is always detectable; the token counter is
/// untouched, which is what stops a new tenure from reissuing a token an
/// older one already published under.
const ACQUIRE_LUA: &str = r"
local now = suprnova_store_now(tonumber(ARGV[3]))
local held = redis.call('HMGET', KEYS[1], 'lease_id', 'expires_at_ms')
local tenure = tonumber(held[1])
local expires_at = tonumber(held[2])
if tenure and expires_at and expires_at > now then
    return {0, 0, 0}
end
local next_tenure = (tenure or 0) + 1
local next_expiry = now + tonumber(ARGV[2])
redis.call('HSET', KEYS[1],
    'lease_id', next_tenure,
    'epoch', ARGV[1],
    'expires_at_ms', next_expiry)
return {1, next_tenure, next_expiry}
";

/// Mints the next publication token while the caller still holds the lease.
///
/// `KEYS[1]` is the lease hash and `KEYS[2]` the token counter. `ARGV` is the
/// caller's lease id and the store-time test offset.
///
/// Returns the minted token, or `-1` when the lease id does not hold the key
/// or its tenure has passed by store time. The counter is a key of its own so
/// that it survives every tenure and every release.
const MINT_LUA: &str = r"
local now = suprnova_store_now(tonumber(ARGV[2]))
local held = redis.call('HMGET', KEYS[1], 'lease_id', 'expires_at_ms')
local tenure = tonumber(held[1])
local expires_at = tonumber(held[2])
if not tenure or not expires_at then
    return -1
end
if tenure ~= tonumber(ARGV[1]) or expires_at <= now then
    return -1
end
return redis.call('INCR', KEYS[2])
";

/// Ends the caller's tenure by expiring it.
///
/// `KEYS[1]` is the lease hash and `ARGV[1]` the caller's lease id. Returns
/// `1` when this call ended the tenure and `0` when the lease id no longer
/// held the key, in which case nothing changed. Deliberately not a `DEL`:
/// see the module documentation for why the hash has to stay.
const RELEASE_LUA: &str = r"
local held = redis.call('HGET', KEYS[1], 'lease_id')
if held and tonumber(held) == tonumber(ARGV[1]) then
    redis.call('HSET', KEYS[1], 'expires_at_ms', 0)
    return 1
end
return 0
";

/// [`ACQUIRE_LUA`], built once so every acquisition after the first ships a
/// hash rather than the whole body.
static ACQUIRE: SharedScript = SharedScript::new(|| timed_script(ACQUIRE_LUA));

/// [`MINT_LUA`], built once for the same reason.
static MINT: SharedScript = SharedScript::new(|| timed_script(MINT_LUA));

/// [`RELEASE_LUA`], built once for the same reason. It decides nothing by
/// store time - a release is the holder's own - so it needs no clock.
static RELEASE: SharedScript = SharedScript::new(|| redis::Script::new(RELEASE_LUA));

/// Redis-backed rebuild lease store. See the module documentation for the key
/// layout, the clock, and why nothing here expires.
///
/// `Debug` prints the namespace and the test offset only - there is no lease
/// state in this type, which is the point: Redis holds it.
#[derive(Debug)]
pub struct RedisLeaseStore {
    provider: RedisProvider,
    time_offset_ms: StoreTimeOffset,
}

impl RedisLeaseStore {
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
        Ok(Self {
            provider: RedisProvider::connect(config).await?,
            time_offset_ms: StoreTimeOffset::default(),
        })
    }

    /// Moves this store's view of the Redis clock forward by `offset_ms`
    /// milliseconds, so a test can reach a lease expiry without waiting for
    /// one.
    ///
    /// Never called by production code, and deliberately not a process-wide
    /// switch: parallel tests in one binary would otherwise move each other's
    /// store clocks. Not part of the public contract: doc-hidden, the same
    /// shape as
    /// [`SqlLeaseStore::set_time_offset_for_test`](super::SqlLeaseStore::set_time_offset_for_test).
    #[doc(hidden)]
    pub fn set_time_offset_for_test(&self, offset_ms: u64) {
        self.time_offset_ms.set(offset_ms);
    }
}

#[async_trait]
impl LeaseStore for RedisLeaseStore {
    async fn try_acquire(
        &self,
        key: &RenderKey,
        epoch: u64,
        ttl_ms: u64,
    ) -> Result<LeaseAttempt, RenderCacheError> {
        let mut conn = self.provider.connection();
        // Deliberately not retried on a transient failure: the script writes,
        // and a retry after a socket drop whose command Redis did execute
        // would advance the tenure counter past the id this caller was told
        // it holds, fencing the caller out of its own lease.
        let (status, lease_id, expires_at_ms): (i64, u64, u64) = ACQUIRE
            .key(lease_key(self.provider.prefix(), key))
            .arg(as_i64(epoch))
            .arg(as_i64(ttl_ms))
            .arg(self.time_offset_ms.get())
            .invoke_async(&mut conn)
            .await
            .map_err(|error| provider_error(&error))?;
        match status {
            ACQUIRED => Ok(LeaseAttempt::Acquired {
                lease_id,
                expires_at_ms,
            }),
            HELD => Ok(LeaseAttempt::Held),
            _ => Err(unreadable_status()),
        }
    }

    async fn mint_token(
        &self,
        key: &RenderKey,
        lease_id: u64,
    ) -> Result<Option<u64>, RenderCacheError> {
        let mut conn = self.provider.connection();
        // Not retried, for the reason `try_acquire` is not: a second `INCR`
        // after a drop would spend a token nobody published under, and the
        // caller would be told the second one is its fence.
        let minted: i64 = MINT
            .key(lease_key(self.provider.prefix(), key))
            .key(token_key(self.provider.prefix(), key))
            .arg(as_i64(lease_id))
            .arg(self.time_offset_ms.get())
            .invoke_async(&mut conn)
            .await
            .map_err(|error| provider_error(&error))?;
        if minted == NOT_HELD {
            return Ok(None);
        }
        if minted < 0 {
            return Err(unreadable_status());
        }
        Ok(Some(as_u64(minted)))
    }

    async fn release(&self, key: &RenderKey, lease_id: u64) -> Result<(), RenderCacheError> {
        let mut conn = self.provider.connection();
        let _: i64 = RELEASE
            .key(lease_key(self.provider.prefix(), key))
            .arg(as_i64(lease_id))
            .invoke_async(&mut conn)
            .await
            .map_err(|error| provider_error(&error))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! The lease scripts' key and argument layout.
    //!
    //! Behaviour is proven against a real Redis in
    //! `framework/tests/render_cache/tiers.rs`. What those tests cannot show
    //! is that each decision is made inside one script against Redis's own
    //! clock, and that neither key is ever removed - the two properties this
    //! store's whole contract rests on.
    use super::*;

    const SCRIPTS: [&str; 3] = [ACQUIRE_LUA, MINT_LUA, RELEASE_LUA];

    #[test]
    fn every_expiry_is_decided_on_redis_own_clock_inside_the_script() {
        for body in [ACQUIRE_LUA, MINT_LUA] {
            assert!(
                body.contains("suprnova_store_now(tonumber(ARGV["),
                "an expiry decided on a node clock is not an expiry: {body}"
            );
            assert!(
                body.contains("expires_at"),
                "and it is decided against the stored deadline: {body}"
            );
        }
        // Release ends the caller's own tenure and decides no expiry, so it
        // needs no clock and takes no offset.
        assert!(!RELEASE_LUA.contains("suprnova_store_now"), "{RELEASE_LUA}");
    }

    #[test]
    fn acquisition_advances_the_tenure_and_never_the_token_counter() {
        assert!(
            ACQUIRE_LUA.contains("local next_tenure = (tenure or 0) + 1"),
            "a takeover advances the tenure from whatever the hash carries: {ACQUIRE_LUA}"
        );
        assert!(
            !ACQUIRE_LUA.contains("INCR"),
            "acquiring must not touch the token counter: {ACQUIRE_LUA}"
        );
        assert!(
            !ACQUIRE_LUA.contains("KEYS[2]"),
            "and must not even name it: {ACQUIRE_LUA}"
        );
        assert!(
            ACQUIRE_LUA.contains("return {0, 0, 0}") && ACQUIRE_LUA.contains("return {1,"),
            "held and acquired are explicit statuses: {ACQUIRE_LUA}"
        );
    }

    #[test]
    fn minting_is_guarded_by_the_tenure_and_by_store_time() {
        assert!(
            MINT_LUA.contains("tenure ~= tonumber(ARGV[1]) or expires_at <= now"),
            "a fenced-out holder mints nothing: {MINT_LUA}"
        );
        assert!(
            MINT_LUA.contains("redis.call('INCR', KEYS[2])"),
            "and the token comes from the counter that outlives every tenure: {MINT_LUA}"
        );
        assert!(
            MINT_LUA.contains("return -1"),
            "\"you no longer hold this\" is a value INCR can never return: {MINT_LUA}"
        );
    }

    #[test]
    fn release_expires_the_tenure_rather_than_deleting_anything() {
        assert!(
            RELEASE_LUA.contains("redis.call('HSET', KEYS[1], 'expires_at_ms', 0)"),
            "{RELEASE_LUA}"
        );
        assert!(
            RELEASE_LUA.contains("tonumber(held) == tonumber(ARGV[1])"),
            "a stale lease id releases nothing: {RELEASE_LUA}"
        );
    }

    #[test]
    fn nothing_this_store_writes_is_ever_removed_or_given_a_lifetime() {
        // The tenure counter and the token counter both have to outlive every
        // tenure. A `DEL` or a key lifetime on either would restart a counter,
        // and a restarted counter is how a fenced-out leader's token outranks
        // the publication that replaced it.
        for body in SCRIPTS {
            for forbidden in ["DEL", "EXPIRE", "PEXPIRE", "PEXPIREAT", "UNLINK"] {
                assert!(!body.contains(forbidden), "{forbidden} in {body}");
            }
        }
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
}
