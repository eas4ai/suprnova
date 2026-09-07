//! Shared plumbing for the Tier 2 Redis adapters: where they connect, the
//! key namespace they write under, the store clock every script reads, and
//! the collapse from a driver failure to the one provider kind each contract
//! exposes.
//!
//! # The key namespace
//!
//! Every key an adapter touches is `<prefix>` followed by a kind and the
//! address within it. Nothing is hashed a second time: a
//! [`RenderKey`] already *is* a fixed-length digest with a validating parser,
//! and an instance or promotion address is hex of identities the engine
//! validated on the way in.
//!
//! | Key | Type | Written by |
//! |---|---|---|
//! | `<prefix>entry:<render key>` | hash | [`RedisRenderStore`](super::RedisRenderStore) |
//! | `<prefix>lease:<render key>` | hash | [`RedisLeaseStore`](super::RedisLeaseStore) |
//! | `<prefix>token:<render key>` | counter | [`RedisLeaseStore`](super::RedisLeaseStore) |
//! | `<prefix>instance:<scope>:<instance>` | hash | [`RedisInstanceRecordStore`](super::RedisInstanceRecordStore) |
//! | `<prefix>promotion:<scope>:<idempotency>` | hash | [`RedisInstanceRecordStore`](super::RedisInstanceRecordStore) |
//! | `<prefix>instances` | sorted set | [`RedisInstanceRecordStore`](super::RedisInstanceRecordStore) |
//!
//! `<render key>` is [`RenderKey::to_base64url`] (`rk1.` plus 43 base64url
//! characters); `<scope>`, `<instance>`, and `<idempotency>` are lowercase
//! hex, 64 characters for the fixed scope fingerprint and 32 to 64 for the
//! 16- to 32-byte identities.
//!
//! # One clock, and it is never this node's
//!
//! Every cross-node expiry decision is `redis.call('TIME')` read *inside*
//! the script that guards the state it decides, exactly as the SQL adapters
//! inline the database's own clock into their statements. `STORE_NOW_LUA`
//! is that reader, and every script that decides an expiry begins with it. A
//! node whose clock runs fast can therefore neither extend a lease nor
//! declare a peer's record elapsed.
//!
//! # One script per decision
//!
//! Nothing here infers an outcome from the shape of a reply across two round
//! trips. A decision - acquired against held, stored against conflict
//! against missing, published against fenced - is made inside one Lua script
//! that returns an explicit status code, so the state it read and the state
//! it wrote are one atomic step. Scripts receive `KEYS` and `ARGV`; no
//! caller value is ever spliced into script text.
//!
//! # What these adapters are pointed at
//!
//! One Redis instance, running Redis 7 or newer: the scripts here touch keys
//! they do not declare in `KEYS` (the reclamation pass deletes the members
//! its own range read found), which Redis Cluster refuses, and they read the
//! store clock with `TIME` inside a script, which older servers refuse.

use std::fmt;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use redis::Script;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use suprnova_live::ledger::{InstanceRecordKey, LedgerError, LedgerErrorKind, PromotionRecordKey};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};

use crate::FrameworkError;

/// How long one attempt to reach Redis may take before it is abandoned.
///
/// These are the cache driver's numbers, and this is the same kind of
/// dependency: Redis accelerates a render cache, it never holds authority,
/// so a request must find out quickly that Redis is unreachable rather than
/// wait out the driver's default budget of six retries with an uncapped
/// exponential delay.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long one command may wait for its reply, for the same reason.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
/// How many times the manager rebuilds a dropped connection before a command
/// gives up, and the ceiling on the delay between those attempts.
const CONNECT_RETRIES: usize = 3;
/// The ceiling on the delay between reconnection attempts.
const MAX_RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// What a Redis endpoint prints as, everywhere one is printed at all.
///
/// One constant rather than one literal per `Debug` implementation, so the
/// redaction cannot be right in [`RedisProviderConfig`] and quietly absent in
/// the configuration types that carry the same URL
/// ([`L1Config`](crate::render_cache::L1Config),
/// [`CoordinatorConfig`](crate::render_cache::CoordinatorConfig), and Live's
/// own ledger driver).
pub(crate) const REDACTED_URL: &str = "redis://<redacted>";

/// The environment variable that names where the render cache tiers connect.
///
/// Named once here rather than spelled at each call site, so a refusal from
/// the L1 store, the lease store, or install's own `PING` cannot drift into
/// naming three different things.
pub(crate) const RENDER_CACHE_REDIS_URL: &str = "RENDER_CACHE_REDIS_URL";

/// The environment variable that names where the Live instance ledger
/// connects, for the same reason.
pub(crate) const LIVE_REDIS_URL: &str = "LIVE_REDIS_URL";

/// Where a Redis-backed tier provider connects, and under what namespace.
///
/// One configuration serves all three adapters, so a deployment points every
/// Tier 2 provider at one instance and one prefix.
#[derive(Clone, Eq, PartialEq)]
pub struct RedisProviderConfig {
    /// The Redis connection URL. It can carry a password, so it is never
    /// printed: see this type's [`fmt::Debug`] implementation.
    pub url: String,
    /// The literal string every key this deployment writes begins with, such
    /// as `suprnova_render:`. Two deployments sharing one Redis stay apart
    /// by choosing different prefixes.
    pub prefix: String,
}

impl fmt::Debug for RedisProviderConfig {
    /// Prints the namespace and a redacted endpoint, never the URL.
    ///
    /// A Redis URL routinely carries a password, and this type reaches logs
    /// and error paths through the configuration it belongs to.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisProviderConfig")
            .field("url", &REDACTED_URL)
            .field("prefix", &self.prefix)
            .finish()
    }
}

/// The connection and namespace the three Tier 2 adapters share.
///
/// # Connecting is not reaching
///
/// [`Self::connect`] builds a handle; it does not prove Redis is there. The
/// connection is established on the first command and rebuilt after a drop,
/// so an instance that is down at construction and up a moment later needs
/// no restart, and - the part that matters for the contract - an instance
/// that is down when a command runs reports the one provider failure the
/// caller's contract exposes rather than a construction error nobody is
/// positioned to handle. Refusing a profile whose Redis is unreachable at
/// boot is install's `PING`, an explicit liveness check, rather than a side
/// effect of building a handle.
pub(crate) struct RedisProvider {
    conn: ConnectionManager,
    prefix: String,
}

impl RedisProvider {
    /// Builds a handle on `config`, failing only when the URL or the driver
    /// configuration is unusable.
    ///
    /// `setting` is the environment variable an operator would change to fix
    /// the failure - `RENDER_CACHE_REDIS_URL` for the render cache tiers,
    /// `LIVE_REDIS_URL` for the Live instance ledger. Every message below
    /// names it, and none of them repeats the URL, which can carry a
    /// password.
    ///
    /// Asynchronous although it awaits nothing: the manager spawns its own
    /// background task as it is built, so this has to run on an executor, and
    /// the signature is what makes that a compile-time requirement rather
    /// than a panic in whichever caller forgot.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when the URL is not a Redis connection URL
    /// or the manager cannot be created.
    pub(crate) async fn connect(
        config: &RedisProviderConfig,
        setting: &'static str,
    ) -> Result<Self, FrameworkError> {
        Self::open(config, setting)
    }

    /// [`Self::connect`] without the `async` marker, for the one caller that
    /// has no `await` to give it.
    ///
    /// `LiveRuntime::bind` is synchronous and public, and it is where the
    /// Live instance ledger is built, so the Redis record store has to be
    /// constructible from a synchronous function. Nothing is awaited here in
    /// either form - the body is the same - but the connection manager
    /// spawns its own background task as it is built, so an executor still
    /// has to be present. That is checked explicitly and reported as a
    /// configuration failure, because the alternative is `tokio::spawn`'s
    /// panic in whichever caller happened to have no runtime.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when no asynchronous runtime is running on
    /// this thread, when the URL is not a Redis connection URL, or when the
    /// manager cannot be created. Every message names `setting` and none
    /// repeats the URL: it can carry a password.
    pub(crate) fn open(
        config: &RedisProviderConfig,
        setting: &'static str,
    ) -> Result<Self, FrameworkError> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(FrameworkError::internal(format!(
                "the Redis provider {setting} configures was built outside an asynchronous \
                 runtime, which its connection manager needs; build it from within the \
                 server's runtime."
            )));
        }
        let client = redis::Client::open(config.url.as_str()).map_err(|error| {
            tracing::warn!(
                target: "suprnova::render_cache",
                %error,
                setting,
                "redis provider url is unusable",
            );
            FrameworkError::internal(format!(
                "{setting} is not a usable Redis connection url. The value is not repeated \
                 here: it can carry a password."
            ))
        })?;
        let manager = ConnectionManagerConfig::new()
            .set_connection_timeout(Some(CONNECT_TIMEOUT))
            .set_response_timeout(Some(RESPONSE_TIMEOUT))
            .set_number_of_retries(CONNECT_RETRIES)
            .set_max_delay(MAX_RECONNECT_DELAY);
        let conn = ConnectionManager::new_lazy_with_config(client, manager).map_err(|error| {
            tracing::warn!(
                target: "suprnova::render_cache",
                %error,
                setting,
                "redis provider connection manager is unusable",
            );
            FrameworkError::internal(format!(
                "the Redis connection manager {setting} configures could not be created."
            ))
        })?;
        Ok(Self {
            conn,
            prefix: config.prefix.clone(),
        })
    }

    /// A connection to run one command on.
    ///
    /// Cloning is ownership, not routing: a clone shares the manager's
    /// internal state and reloads the live connection slot on every command,
    /// so it is the same connection the original would use.
    pub(crate) fn connection(&self) -> ConnectionManager {
        self.conn.clone()
    }

    /// The literal namespace every key this provider writes begins with.
    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }
}

impl fmt::Debug for RedisProvider {
    /// Prints the namespace and nothing else - no endpoint, no connection
    /// state, and no stored value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisProvider")
            .field("prefix", &self.prefix)
            .finish()
    }
}

/// Proves the Redis `setting` names is actually there, by asking it.
///
/// `RedisProvider::connect` (crate-private, so this is a plain code span
/// rather than a link) builds a handle and reaches nothing, which is what
/// makes a runtime outage a provider failure rather than a construction
/// error. A deployment pointed at an endpoint that is not there is a
/// different thing entirely - a misconfiguration - and it must fail once, at
/// boot, rather than on every request. This is the check that makes that
/// difference: both boot paths call it before building a Redis-backed
/// provider, exactly as they probe for the tier migration before building a
/// database-backed one.
///
/// `setting` is the environment variable an operator would change:
/// `RENDER_CACHE_REDIS_URL` from the render cache install,
/// `LIVE_REDIS_URL` from the Live runtime. The caller names it rather than
/// re-wording the failure afterwards, so there is one message per failure and
/// only one place that could stop naming a setting.
///
/// # Errors
///
/// Returns [`FrameworkError`] when the URL is unusable, the instance cannot
/// be reached, or `PING` does not answer. Every message names `setting` and
/// none repeats the URL, which routinely carries a password.
pub async fn ping(
    config: &RedisProviderConfig,
    setting: &'static str,
) -> Result<(), FrameworkError> {
    let provider = RedisProvider::connect(config, setting).await?;
    let mut conn = provider.connection();
    redis::cmd("PING")
        .query_async::<()>(&mut conn)
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "suprnova::render_cache",
                %error,
                setting,
                "redis provider did not answer PING at boot",
            );
            FrameworkError::internal(format!(
                "the Redis {setting} names did not answer PING. Fix {setting}, start the \
                 instance it names, or select a configuration that needs no Redis. The \
                 endpoint is not repeated here: it can carry a password."
            ))
        })
}

/// Milliseconds added to the store time every script computes, for tests
/// alone.
///
/// Per adapter instance rather than process-wide, for the reason the SQL
/// adapters give: several stores share one test binary, and a process-global
/// offset would let one test's expiry move another's. Production always
/// leaves it at zero.
#[derive(Debug, Default)]
pub(crate) struct StoreTimeOffset(AtomicU64);

impl StoreTimeOffset {
    /// Moves this adapter's view of the Redis clock forward by `offset_ms`.
    pub(crate) fn set(&self, offset_ms: u64) {
        self.0.store(offset_ms, Ordering::Relaxed);
    }

    /// The offset a script is handed, as the integer Lua reads it back as.
    pub(crate) fn get(&self) -> i64 {
        super::as_i64(self.0.load(Ordering::Relaxed))
    }
}

/// The store clock, as a Lua function every script in these adapters can
/// call.
///
/// `TIME` answers seconds and microseconds; milliseconds is the unit every
/// expiry in this crate is measured in, and the sum stays exact as a Lua
/// number (a double holds every integer below 2^53, and milliseconds since
/// 1970 is about 2^41). `offset_ms` is the test seam, zero in production.
pub(crate) const STORE_NOW_LUA: &str = r"
local function suprnova_store_now(offset_ms)
    local clock = redis.call('TIME')
    return (tonumber(clock[1]) * 1000) + math.floor(tonumber(clock[2]) / 1000) + offset_ms
end
";

/// A script that reads the store clock, built from [`STORE_NOW_LUA`] and
/// `body`.
///
/// Scripts are built once and reused, so the `EVALSHA` fast path is taken
/// after the first call rather than the whole body being shipped every time.
pub(crate) fn timed_script(body: &str) -> Script {
    Script::new(&format!("{STORE_NOW_LUA}{body}"))
}

/// The hash holding one published entry.
pub(crate) fn entry_key(prefix: &str, key: &RenderKey) -> String {
    format!("{prefix}entry:{}", key.to_base64url())
}

/// The hash holding one key's rebuild lease.
pub(crate) fn lease_key(prefix: &str, key: &RenderKey) -> String {
    format!("{prefix}lease:{}", key.to_base64url())
}

/// The counter holding one key's publication tokens.
///
/// A key of its own, and one that never expires: the counter has to outlive
/// every tenure, or a fenced-out leader's already-minted token could outrank
/// the publication that replaced it.
pub(crate) fn token_key(prefix: &str, key: &RenderKey) -> String {
    format!("{prefix}token:{}", key.to_base64url())
}

/// The hash holding one Live instance record.
pub(crate) fn instance_key(prefix: &str, key: &InstanceRecordKey) -> String {
    format!(
        "{prefix}instance:{}:{}",
        hex::encode(key.scope.as_bytes()),
        hex::encode(key.instance_id.as_bytes())
    )
}

/// The hash holding one reserved retry identity.
pub(crate) fn promotion_key(prefix: &str, key: &PromotionRecordKey) -> String {
    format!(
        "{prefix}promotion:{}:{}",
        hex::encode(key.scope.as_bytes()),
        hex::encode(key.idempotency_key.as_bytes())
    )
}

/// The sorted set indexing every instance record by its deadline.
///
/// One member per record, scored by `expires_at_ms`, so counting the live
/// records is a `ZCOUNT` from store time upward and reclaiming the elapsed
/// ones is a bounded range read - never a scan of the keyspace per mount.
pub(crate) fn instance_index_key(prefix: &str) -> String {
    format!("{prefix}instances")
}

/// The `MATCH` pattern that selects exactly this deployment's entry hashes.
///
/// The prefix is operator configuration and may contain the glob characters
/// `SCAN` reads, so it is escaped; the rest of the pattern is this module's
/// own constant plus base64url, which carries none.
pub(crate) fn entry_scan_pattern(prefix: &str) -> String {
    format!("{}entry:*", glob_escape(prefix))
}

/// `text` with every character Redis's glob matcher reads escaped, so it
/// matches itself and nothing else.
fn glob_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '*' | '?' | '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Collapses a driver failure into the one closed provider kind the
/// RenderCache contract exposes, logging its closed-set kind first.
///
/// The cause never reaches the caller: [`RenderCacheError`] carries a kind
/// and nothing else, exactly so a driver message - which can echo a command,
/// its arguments, or the endpoint - can never travel back into a response.
/// It does not reach the log either: `redis::ErrorKind` is a closed enum
/// whose variants carry no free text, so its `Debug` says which class of
/// failure this was and repeats no argument of the command that failed.
/// Mirrors [`provider_error`](super::provider_error), which does the same for
/// the SQL adapters.
pub(crate) fn provider_error(error: &redis::RedisError) -> RenderCacheError {
    tracing::warn!(
        target: "suprnova::render_cache",
        kind = ?error.kind(),
        "render cache tier provider failure",
    );
    RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable)
}

/// Collapses a driver failure into the one closed provider kind the ledger
/// contract exposes, logging its closed-set kind first.
///
/// The same collapse as [`provider_error`], for the contract on the other
/// side of the Live record store. [`LedgerError`] carries a kind and nothing
/// else, which is what keeps an encoded record out of a response even though
/// a driver message could repeat one; logging `redis::ErrorKind` rather than
/// that message keeps the record out of the log for the same reason.
pub(crate) fn ledger_error(error: &redis::RedisError) -> LedgerError {
    tracing::warn!(
        target: "suprnova::render_cache",
        kind = ?error.kind(),
        "live instance record store provider failure",
    );
    LedgerError::new(LedgerErrorKind::ProviderUnavailable)
}

/// A reply this build cannot read as the status its own script returns.
///
/// Reaching this means the script and the code that reads it disagree, which
/// is a broken build rather than a Redis failure - but it is still reported
/// as a provider failure, because there is nothing else the caller's closed
/// contract can say and answering "absent" would invent an answer.
pub(crate) fn unreadable_status() -> RenderCacheError {
    super::provider_error_kind("unreadable_script_status")
}

/// [`unreadable_status`] for the ledger side.
pub(crate) fn unreadable_ledger_status() -> LedgerError {
    tracing::warn!(
        target: "suprnova::render_cache",
        "live instance record store script returned a status this build does not define",
    );
    LedgerError::new(LedgerErrorKind::ProviderUnavailable)
}

/// One script, built once and shared by every operation that runs it.
pub(crate) type SharedScript = LazyLock<Script>;

#[cfg(test)]
mod tests {
    //! The key namespace, the store clock, and the glob escape.
    //!
    //! Behaviour is proven against a real Redis in
    //! `framework/tests/render_cache/tiers.rs`; what those tests cannot show
    //! is the exact shape of the names, which is the part a later build could
    //! change without any test noticing until two deployments collided or a
    //! key stopped being found.
    use super::*;
    use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
    use suprnova_live::identity::{
        IdempotencyKey, InstanceId, KeyId, ScopeFingerprint, UnixMillis,
    };

    fn ring() -> SnapshotKeyRing {
        let active = KeyRecord::new(
            KeyId::parse("render-cache-test").expect("key id"),
            RootKey::new(vec![9; 32]).expect("root key"),
            UnixMillis::new(0),
            UnixMillis::new(u64::MAX / 2),
            UnixMillis::new(u64::MAX),
        )
        .expect("key record");
        SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
    }

    fn scope() -> ScopeFingerprint {
        ScopeFingerprint::from_bytes(&[0x10_u8; 32]).expect("a scope fingerprint")
    }

    #[test]
    fn a_render_key_is_its_own_name_and_never_a_second_hash_of_one() {
        let key = RenderKey::for_test(&ring(), "/tier-two");
        let encoded = key.to_base64url();
        assert!(encoded.starts_with("rk1."), "{encoded}");
        assert_eq!(encoded.len(), 47, "rk1. plus 43 base64url characters");

        assert_eq!(entry_key("p:", &key), format!("p:entry:{encoded}"));
        assert_eq!(lease_key("p:", &key), format!("p:lease:{encoded}"));
        assert_eq!(token_key("p:", &key), format!("p:token:{encoded}"));
        // The name has to be the key itself: `from_base64url` is its exact
        // inverse, so an operator looking at Redis can tell which key a hash
        // belongs to, and no second digest can ever collide with a first.
        assert_eq!(
            RenderKey::from_base64url(&encoded).expect("the name parses back"),
            key
        );
    }

    #[test]
    fn a_record_name_is_hex_of_the_scope_and_the_identity_within_it() {
        let narrow = InstanceRecordKey {
            scope: scope(),
            instance_id: InstanceId::from_bytes(&[0x21_u8; 16]).expect("an instance identity"),
        };
        let wide = InstanceRecordKey {
            scope: scope(),
            instance_id: InstanceId::from_bytes(&[0x21_u8; 32]).expect("an instance identity"),
        };
        assert_eq!(
            instance_key("p:", &narrow),
            format!("p:instance:{}:{}", "10".repeat(32), "21".repeat(16))
        );
        assert_eq!(
            instance_key("p:", &wide),
            format!("p:instance:{}:{}", "10".repeat(32), "21".repeat(32))
        );
        // The narrow identity's name must never be a prefix collision of the
        // wide one's: they are distinct keys and distinct records.
        assert_ne!(instance_key("p:", &narrow), instance_key("p:", &wide));

        let promotion = PromotionRecordKey {
            scope: scope(),
            idempotency_key: IdempotencyKey::from_bytes(&[0x31_u8; 32]).expect("a retry identity"),
        };
        assert_eq!(
            promotion_key("p:", &promotion),
            format!("p:promotion:{}:{}", "10".repeat(32), "31".repeat(32))
        );
        assert_eq!(instance_index_key("p:"), "p:instances");
    }

    #[test]
    fn the_inspection_pattern_matches_this_deployment_and_nothing_else() {
        assert_eq!(
            entry_scan_pattern("suprnova_render:"),
            "suprnova_render:entry:*"
        );
        // A prefix an operator wrote with a glob character in it must match
        // itself, not every key that happens to fit the pattern.
        assert_eq!(entry_scan_pattern("odd*[p]:"), "odd\\*\\[p\\]:entry:*");
        assert_eq!(glob_escape("plain"), "plain");
    }

    #[test]
    fn every_script_reads_the_clock_from_redis_rather_than_from_this_node() {
        let script = timed_script("return suprnova_store_now(tonumber(ARGV[1]))");
        // `Script` keeps the body it hashes, and the prelude is what makes
        // `suprnova_store_now` exist: a script built any other way would read
        // a node's clock or fail to compile inside Redis.
        assert!(STORE_NOW_LUA.contains("redis.call('TIME')"));
        assert!(
            STORE_NOW_LUA.contains("+ offset_ms"),
            "the test seam is an argument, never a process-wide switch"
        );
        assert_eq!(
            script.get_hash().len(),
            40,
            "a script is cached by its SHA-1"
        );
    }

    #[test]
    fn a_driver_failure_is_logged_as_a_kind_and_never_as_its_message() {
        // What a failing script or command actually carries back: the
        // detail string repeats the arguments, which for these adapters are
        // render keys, hex identities, and encoded records.
        let error = redis::RedisError::from((
            redis::ErrorKind::Extension,
            "script failed",
            "EVALSHA rk1.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA \
             instance:0123456789abcdef"
                .to_owned(),
        ));
        // The message is the leak this guards against, so prove it is
        // really in the error before proving it is not in what is logged.
        let message = error.to_string();
        assert!(message.contains("rk1."), "{message}");
        assert!(message.contains("0123456789abcdef"), "{message}");

        // `kind = ?error.kind()` is the field both warn sites log.
        let logged = format!("{:?}", error.kind());
        assert_eq!(logged, "Extension");
        assert!(!logged.contains("rk1."), "{logged}");
        assert!(!logged.contains("0123456789abcdef"), "{logged}");
        assert!(!logged.contains("EVALSHA"), "{logged}");
        assert!(!logged.contains("instance:"), "{logged}");
    }

    #[test]
    fn the_configuration_prints_its_namespace_and_never_its_endpoint() {
        let config = RedisProviderConfig {
            url: "redis://someone:hunter2@10.0.0.1:6379/".to_owned(),
            prefix: "suprnova_render:".to_owned(),
        };
        let printed = format!("{config:?}");
        assert!(printed.contains("suprnova_render:"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("10.0.0.1"), "{printed}");
    }
}
