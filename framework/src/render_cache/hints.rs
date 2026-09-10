//! Credible generation hints: the Tier 2 pub/sub channel that lets a node
//! shorten a validation lease it already holds, and the lease table that
//! decides which leases a hint reaches.
//!
//! # What a hint is, and what it is not
//!
//! A hint names dependency digests that just advanced somewhere. Its only
//! power is to make this node revalidate *earlier* than its own lease would
//! have made it revalidate anyway: it can shorten a lease, and it can do
//! nothing else. It cannot extend a lease, create one, prove an entry
//! current, bypass the generation ledger read a hit still makes, or touch
//! the authority epoch. The engine enforces the one rule that matters -
//! [`ValidationLease::hint_invalidate`] takes the minimum of the held
//! expiry and the instant offered - and nothing here reaches around it.
//!
//! That is what makes the channel safe to leave unauthenticated, and spec
//! 18 says so directly: a deployment receiving forged, duplicated,
//! reordered, or stale hints serves exactly what the same deployment serves
//! with hints disabled, because the worst a hostile publisher can buy is one
//! extra ledger read that returns the truth. A signature would cost key
//! material on a channel that gains nothing from it, so there is none, and
//! none should be added.
//!
//! Losing the channel entirely, or never configuring it, leaves behavior
//! identical to a build without it: the same entries served, the same
//! rebuilds admitted, and only the moment of revalidation different.
//!
//! # Clock behavior and skew
//!
//! A hint carries digests and nothing else - deliberately no instant. The
//! publisher's clock is not comparable with this node's, and a lease is
//! measured on this node's own [`Clock`]. So the applier shortens a matched
//! lease to *its own* `now_ms`, which expires it (a lease is valid only
//! while `now_ms` is strictly before its expiry), and the next lookup for
//! that key rereads the authority. A node whose clock runs fast or slow
//! therefore cannot move a peer's lease at all, in either direction, and no
//! skew assumption is needed between nodes for the hint path to be correct.
//! The staleness bound stays the lease's own `max_age_ms`, exactly as it is
//! with hints switched off.
//!
//! # The key namespace
//!
//! One channel, `<prefix>hints`, alongside the keys the other Tier 2
//! adapters write; see [`providers::redis`](super::providers::redis)'s own
//! module documentation for the whole table.
//!
//! # Bounds
//!
//! - One message carries at most [`MAX_HINT_DIGESTS`] digests. A message
//!   over that bound is dropped whole, never truncated.
//! - The publish queue and the inbound queue are both bounded, and a
//!   subscriber that fills its inbound queue is dropped and resubscribes
//!   rather than queued without limit.
//! - Neither publishing nor subscribing runs on a request's own task.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use suprnova_live::clock::Clock;
use suprnova_live::render_cache::coherence::ValidationLease;
use suprnova_live::render_cache::key::RenderKey;

use super::telemetry;
use crate::telemetry::Metrics;

/// The most digests one hint message may carry.
///
/// A message over this bound is dropped whole rather than truncated,
/// because a truncated hint is a silently wrong hint: the receiver would
/// revalidate the keys named in the part it kept and go on trusting leases
/// over the keys named in the part it discarded, with nothing anywhere
/// recording that it had done so. Dropping the message instead costs only
/// the acceleration - every lease still expires on its own schedule, and
/// the authority read that follows still returns the truth - and the drop
/// is recorded under `dropped_over_bound`.
///
/// The publisher never emits a message over the bound: an advance touching
/// more digests than this is split into several messages, each within it,
/// which is not truncation because no digest is discarded.
pub const MAX_HINT_DIGESTS: usize = 64;

/// The most bytes a well-formed message can occupy: the version tag, then
/// one comma and 64 hexadecimal characters per digest.
const MAX_HINT_BYTES: usize = VERSION_TAG.len() + (MAX_HINT_DIGESTS * 65);

/// The wire version tag every message begins with. A message that does not
/// begin with it is not one of ours and is dropped.
const VERSION_TAG: &str = "srh1";

/// How many hint messages may wait to be published before further ones are
/// dropped.
///
/// The queue exists so that no request ever waits on Redis to announce what
/// it just wrote. Overflow drops the message silently: the closed outcome
/// set spec 18 fixes has no publish-side value, adding a fifth would break
/// the closed set, and the consequence of a dropped announcement is exactly
/// the consequence of running with hints off - peers revalidate when their
/// own leases expire.
const MAX_PENDING_HINTS: usize = 256;

/// How many received messages may wait to be applied before this node drops
/// its own subscription.
///
/// Small on purpose. Applying a hint is an in-memory sweep, so a backlog
/// here means the applier is genuinely behind, and spec 18's answer to that
/// is to drop the subscription and resubscribe rather than to queue without
/// limit: a hint that arrives late is worth less than the memory it would
/// cost to hold, since the lease it would have shortened is closer to
/// expiring on its own with every millisecond that passes.
///
/// Public because the conformance test that proves the drop has to fill
/// this queue exactly - one message past what the applier and the queue can
/// hold between them - rather than flooding the channel and hoping.
pub const MAX_INBOUND_HINTS: usize = 64;

/// How long the subscriber waits before re-establishing a dropped
/// subscription, and the ceiling it doubles up to.
///
/// The same courtesy the shared Redis read retry extends for the same
/// reason: a Redis that just dropped us is usually restarting or
/// overloaded, and resubscribing the microsecond its socket closed helps
/// neither party. Doubling to the ceiling is what keeps a node whose hint
/// endpoint is simply gone from reconnecting thousands of times a minute
/// forever, which would be the one way this optional accelerator could cost
/// a deployment something real.
const RESUBSCRIBE_BACKOFF: Duration = Duration::from_millis(50);
/// The ceiling [`RESUBSCRIBE_BACKOFF`] doubles up to.
const MAX_RESUBSCRIBE_BACKOFF: Duration = Duration::from_secs(30);

/// The message named at least one digest a lease on this node observes, and
/// every such lease was shortened.
pub(crate) const APPLIED: &str = "applied";
/// The message named nothing this node holds a lease against - including a
/// message this node could not decode, which names nothing either way and
/// changes nothing.
pub(crate) const IGNORED_UNKNOWN_KEY: &str = "ignored_unknown_key";
/// The message carried more than [`MAX_HINT_DIGESTS`] digests and was
/// dropped whole.
pub(crate) const DROPPED_OVER_BOUND: &str = "dropped_over_bound";
/// This node's subscription ended and is being re-established.
pub(crate) const SUBSCRIBER_DROPPED: &str = "subscriber_dropped";

/// Counts one hint outcome under the closed
/// `suprnova.render_cache.hints` metric, and records it for a test
/// alongside - never instead of - the real increment, the way
/// `LookupOutcome::record` does.
fn count(outcome: &'static str) {
    Metrics::counter(telemetry::HINTS).inc_with(&[(telemetry::OUTCOME, outcome)]);
    #[cfg(any(test, feature = "testing"))]
    telemetry::record_hint_for_test(outcome);
}

/// Why a received message was not applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HintDecodeError {
    /// More than [`MAX_HINT_DIGESTS`] digests: dropped whole.
    OverBound,
    /// Not a message this build can read: wrong version tag, wrong digest
    /// width, or a character that is not lowercase hexadecimal.
    Malformed,
}

/// Renders `digests` as one message body.
///
/// Lowercase hexadecimal, the same shape the Redis key namespace already
/// uses for the identities it addresses, so a message is readable with
/// `redis-cli SUBSCRIBE` when an operator is working out whether the
/// channel carries anything at all.
///
/// # Panics
///
/// Never: `digests` is chunked to [`MAX_HINT_DIGESTS`] by the one caller,
/// and the formatting below cannot fail for a fixed-width byte array.
fn encode(digests: &[[u8; 32]]) -> String {
    let mut body = String::with_capacity(VERSION_TAG.len() + digests.len() * 65);
    body.push_str(VERSION_TAG);
    for digest in digests {
        body.push(',');
        body.push_str(&hex::encode(digest));
    }
    body
}

/// Reads one message body back, refusing anything outside the bound.
///
/// The digest count is decided before the byte length, and by counting
/// separators rather than by splitting, so an over-bound message is
/// reported as over-bound rather than as merely long, and so that deciding
/// it allocates nothing. Both checks run before any digest is parsed: this
/// is an unauthenticated channel, and the first thing a message has to earn
/// is the right to be looked at.
fn decode(body: &str) -> Result<Vec<[u8; 32]>, HintDecodeError> {
    if body.bytes().filter(|byte| *byte == b',').count() > MAX_HINT_DIGESTS {
        return Err(HintDecodeError::OverBound);
    }
    if body.len() > MAX_HINT_BYTES {
        return Err(HintDecodeError::Malformed);
    }
    let rest = body
        .strip_prefix(VERSION_TAG)
        .ok_or(HintDecodeError::Malformed)?;
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    let mut digests = Vec::new();
    for field in rest
        .strip_prefix(',')
        .ok_or(HintDecodeError::Malformed)?
        .split(',')
    {
        let mut digest = [0_u8; 32];
        hex::decode_to_slice(field, &mut digest).map_err(|_| HintDecodeError::Malformed)?;
        digests.push(digest);
    }
    Ok(digests)
}

/// One lease, and the dependency digests the entry it was granted for
/// observes.
struct LeaseEntry {
    lease: ValidationLease,
    /// Ascending, because
    /// [`GenerationSet::digests`](suprnova_live::render_cache::generation::GenerationSet::digests)
    /// reads them out of a `BTreeMap`'s keys - so membership is a binary
    /// search rather than a scan. Bounded by `MAX_OBSERVATIONS`, the same
    /// bound that already caps what one entry may observe at all.
    observed: Vec<[u8; 32]>,
}

impl LeaseEntry {
    /// Whether this entry observes any digest in `hinted`.
    fn observes_any(&self, hinted: &[[u8; 32]]) -> bool {
        hinted
            .iter()
            .any(|digest| self.observed.binary_search(digest).is_ok())
    }
}

/// The local validation leases for [`CoherenceMode::Lease`] routes, keyed by
/// the entry's lookup key, each carrying the dependency digests that entry
/// observes.
///
/// [`CoherenceMode::Lease`]: super::CoherenceMode::Lease
///
/// # Why the digests live here and not in a second index
///
/// A hint names digests; a lease is addressed by [`RenderKey`]. Something
/// has to route one to the other, and the only place both are in hand is
/// the moment a lease is granted, where the entry's header carries the
/// observations it was built from. Carrying them beside the lease keeps
/// this one structure, under one lock, bounded by one sweep: the
/// opportunistic cleanup that already evicts expired leases evicts their
/// digests with them, and there is no second map that could outlive it,
/// disagree with it, or need a bound of its own. A separate digest-to-keys
/// index would answer a hint in fewer steps and would have to be kept
/// consistent with this one at every mutation forever; this cannot drift,
/// because there is nothing to drift from.
///
/// The cost is that applying a hint walks the leases rather than indexing
/// them. That walk is bounded twice over - at most one entry per distinct
/// lease-mode key requested within the last `max_age_ms`, and at most
/// [`MAX_HINT_DIGESTS`] digests tested against each entry's sorted
/// observations - and, decisively, it never runs on a request's task: hints
/// are applied on the subscriber's own applier, so its cost can never
/// become a request's latency.
pub(crate) struct LeaseTable {
    entries: Mutex<BTreeMap<RenderKey, LeaseEntry>>,
}

impl LeaseTable {
    /// An empty table.
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Whether a still-valid lease is held for `key` at `now_ms`.
    pub(crate) fn valid_at(&self, key: &RenderKey, now_ms: u64) -> bool {
        self.lock()
            .get(key)
            .is_some_and(|entry| entry.lease.valid_at(now_ms))
    }

    /// Grants `key` a lease of `max_age_ms` from `now_ms`, recording the
    /// dependency digests the entry observes so a hint naming one of them
    /// can find it.
    ///
    /// Sweeps every expired lease first, which is what bounds this map to
    /// the distinct lease-mode keys requested within the last `max_age_ms`
    /// rather than to every key the process has ever seen.
    pub(crate) fn grant(
        &self,
        key: &RenderKey,
        now_ms: u64,
        max_age_ms: u64,
        observed: Vec<[u8; 32]>,
    ) {
        let mut entries = self.lock();
        entries.retain(|_, held| held.lease.valid_at(now_ms));
        entries.insert(
            key.clone(),
            LeaseEntry {
                lease: ValidationLease::grant(now_ms, max_age_ms),
                observed,
            },
        );
    }

    /// Shortens every lease that observes one of `hinted` to `now_ms`,
    /// returning how many were shortened.
    ///
    /// `now_ms` is *this* node's clock; see the module documentation for
    /// why a hint carries no instant of its own. Shortening goes through
    /// [`ValidationLease::hint_invalidate`], the engine's single
    /// never-widen rule, so this cannot extend a lease however wrong the
    /// message that prompted it was; and it cannot create one either, since
    /// it only ever visits leases already held.
    pub(crate) fn apply_hint(&self, hinted: &[[u8; 32]], now_ms: u64) -> usize {
        let mut entries = self.lock();
        let mut shortened = 0;
        for entry in entries.values_mut() {
            if entry.lease.valid_at(now_ms) && entry.observes_any(hinted) {
                entry.lease.hint_invalidate(now_ms);
                shortened += 1;
            }
        }
        // The leases just shortened are expired by construction, so this
        // one sweep both retires them and clears whatever else had elapsed.
        entries.retain(|_, held| held.lease.valid_at(now_ms));
        shortened
    }

    /// How many leases are held, for `RenderCache::lease_count_for_test`.
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<RenderKey, LeaseEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Decodes one received message and applies it, recording exactly one
/// closed outcome for it.
///
/// The single place a received message is judged, so the production path
/// and the test seam below cannot disagree about what a given payload does.
fn deliver(table: &LeaseTable, clock: &dyn Clock, body: &str) {
    match decode(body) {
        Err(HintDecodeError::OverBound) => count(DROPPED_OVER_BOUND),
        // A message this build cannot read names nothing this node holds a
        // lease against, which is what `ignored_unknown_key` says, and it
        // changes nothing - the same two facts that make an unrecognized
        // digest that outcome.
        Err(HintDecodeError::Malformed) => count(IGNORED_UNKNOWN_KEY),
        Ok(digests) => {
            // A clock that cannot be read is not a reason to guess an
            // instant: leaving every lease alone is the behaviour of a
            // build with hints switched off, which is always safe.
            let Ok(now) = clock.now() else {
                count(IGNORED_UNKNOWN_KEY);
                return;
            };
            if table.apply_hint(&digests, now.get()) > 0 {
                count(APPLIED);
            } else {
                count(IGNORED_UNKNOWN_KEY);
            }
        }
    }
}

/// The publisher, the subscriber, and the applier for one process, with the
/// tasks they run on.
///
/// Held by the RenderCache runtime, so dropping the runtime drops this, and
/// dropping this aborts every task it started. That matters most in this
/// crate's own test suite, where one process installs many runtimes: without
/// it, each install would leave a subscriber behind reconnecting forever to
/// whatever endpoint that test configured.
pub(crate) struct HintChannel {
    outbound: tokio::sync::mpsc::Sender<String>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for HintChannel {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl std::fmt::Debug for HintChannel {
    /// Prints nothing about the endpoint or the traffic - no URL (it can
    /// carry a password), no channel contents, no digest.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HintChannel")
            .field("tasks", &self.tasks.len())
            .finish()
    }
}

impl HintChannel {
    /// Starts the publisher, the subscriber, and the applier against
    /// `config`.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`](crate::FrameworkError) only when the URL
    /// or the driver configuration is unusable, which is a
    /// misconfiguration rather than an outage. An endpoint that is simply
    /// not there is not an error here at all: the tasks retry in the
    /// background and the node serves exactly as it would with hints
    /// switched off, which is the whole contract this channel is held to.
    pub(crate) fn start(
        config: &super::providers::RedisProviderConfig,
        table: Arc<LeaseTable>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, crate::FrameworkError> {
        let channel = super::providers::redis::hints_channel(&config.prefix);
        let provider = super::providers::redis::RedisProvider::open(
            config,
            super::providers::redis::RENDER_CACHE_REDIS_URL,
        )?;
        let client = redis::Client::open(config.url.as_str()).map_err(|error| {
            tracing::warn!(
                target: "suprnova::render_cache",
                kind = ?error.kind(),
                "render cache hint channel url is unusable",
            );
            crate::FrameworkError::internal(
                "RENDER_CACHE_REDIS_URL is not a usable Redis connection url for the \
                 generation hint channel. The value is not repeated here: it can carry a \
                 password.",
            )
        })?;

        let (outbound, outbound_rx) = tokio::sync::mpsc::channel(MAX_PENDING_HINTS);
        let (inbound, inbound_rx) = tokio::sync::mpsc::channel(MAX_INBOUND_HINTS);
        let tasks = vec![
            tokio::spawn(publish_loop(provider, channel.clone(), outbound_rx)),
            tokio::spawn(subscribe_loop(client, channel, inbound)),
            tokio::spawn(apply_loop(table, clock, inbound_rx)),
        ];
        Ok(Self { outbound, tasks })
    }

    /// Announces that `digests` advanced, without waiting for Redis.
    ///
    /// Never blocks and never fails: the message is handed to a bounded
    /// queue and dropped if that queue is full. Splitting an advance wider
    /// than [`MAX_HINT_DIGESTS`] into several messages is not truncation -
    /// every digest is announced, each in a message within the bound.
    pub(crate) fn publish(&self, digests: &[[u8; 32]]) {
        for chunk in digests.chunks(MAX_HINT_DIGESTS) {
            // `try_send`, never `send`: this runs on the task that just
            // wrote to the application database, and that task must not
            // wait on an accelerator's queue for any length of time at all.
            if self.outbound.try_send(encode(chunk)).is_err() {
                return;
            }
        }
    }
}

/// Drains the outbound queue onto the channel, one `PUBLISH` per message.
///
/// A failure here is logged through the tier providers' own collapse into
/// the single closed [`RenderCacheErrorKind::ProviderUnavailable`] kind and
/// then abandoned: the message is an acceleration nobody is waiting for,
/// and republishing it later would announce a generation that has since
/// been superseded.
///
/// [`RenderCacheErrorKind::ProviderUnavailable`]:
///     suprnova_live::render_cache::RenderCacheErrorKind::ProviderUnavailable
async fn publish_loop(
    provider: super::providers::redis::RedisProvider,
    channel: String,
    mut outbound: tokio::sync::mpsc::Receiver<String>,
) {
    while let Some(body) = outbound.recv().await {
        let mut conn = provider.connection();
        if let Err(error) = redis::cmd("PUBLISH")
            .arg(&channel)
            .arg(&body)
            .query_async::<i64>(&mut conn)
            .await
        {
            let _ = super::providers::redis::provider_error(&error);
        }
    }
}

/// Holds the subscription, re-establishing it whenever it ends.
///
/// Every end of a subscription is one `subscriber_dropped`, whether this
/// node dropped it for falling behind or the connection failed: both mean
/// the node is, for the moment, not hearing hints, which is the one fact an
/// operator needs from this counter. It is recorded once per drop and never
/// once per retry, so a hint endpoint that is simply absent contributes a
/// slow trickle rather than a meaningless rate.
async fn subscribe_loop(
    client: redis::Client,
    channel: String,
    inbound: tokio::sync::mpsc::Sender<String>,
) {
    let mut backoff = RESUBSCRIBE_BACKOFF;
    loop {
        match hold_subscription(&client, &channel, &inbound).await {
            // A subscription that carried traffic before it ended was a
            // working one, so the next attempt starts from the shortest
            // pause. Every other ending - a refusal, a socket that closed
            // before a single message, an endpoint that is simply not
            // there - doubles, which is what keeps an absent hint endpoint
            // from costing this node a reconnection every fifty
            // milliseconds for the life of the process.
            Ok(true) => backoff = RESUBSCRIBE_BACKOFF,
            Ok(false) => backoff = (backoff * 2).min(MAX_RESUBSCRIBE_BACKOFF),
            Err(error) => {
                let _ = super::providers::redis::provider_error(&error);
                backoff = (backoff * 2).min(MAX_RESUBSCRIBE_BACKOFF);
            }
        }
        count(SUBSCRIBER_DROPPED);
        // A courtesy pause before reconnecting, never a synchronization
        // device: nothing anywhere waits on this loop, and no test observes
        // it by waiting. See `RESUBSCRIBE_BACKOFF`.
        tokio::time::sleep(backoff).await;
    }
}

/// Subscribes and forwards messages until the subscription ends.
///
/// Returns `Ok(true)` when the subscription carried at least one message
/// before it ended - including the ending this node chooses itself, when
/// its bounded inbound queue is full, which is spec 18's answer to a
/// subscriber that falls behind: dropped and resubscribed, never queued
/// without limit.
async fn hold_subscription(
    client: &redis::Client,
    channel: &str,
    inbound: &tokio::sync::mpsc::Sender<String>,
) -> Result<bool, redis::RedisError> {
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(channel).await?;
    #[cfg(any(test, feature = "testing"))]
    seams::note_subscribed();
    let mut stream = pubsub.into_on_message();
    let mut delivered = false;
    while let Some(message) = futures::StreamExt::next(&mut stream).await {
        delivered = true;
        let Ok(body) = message.get_payload::<String>() else {
            // Not text this build can read. Counted as a delivered
            // message that named nothing, exactly as an undecodable body
            // is, so the two cannot be told apart by an attacker either.
            count(IGNORED_UNKNOWN_KEY);
            continue;
        };
        if inbound.try_send(body).is_err() {
            return Ok(true);
        }
    }
    Ok(delivered)
}

/// Applies received messages, one at a time, off every request's task.
async fn apply_loop(
    table: Arc<LeaseTable>,
    clock: Arc<dyn Clock>,
    mut inbound: tokio::sync::mpsc::Receiver<String>,
) {
    while let Some(body) = inbound.recv().await {
        #[cfg(any(test, feature = "testing"))]
        seams::hold_while_paused().await;
        deliver(&table, clock.as_ref(), &body);
    }
}

/// Delivers `body` as though it had arrived on the channel, running the
/// same decode, the same bound, the same application, and the same single
/// outcome the subscriber's own applier runs.
///
/// The seam every hint test that needs no live Redis drives. It is the
/// production path with the socket removed, not a second implementation of
/// it, so a test asserting what a payload does is asserting what a
/// subscriber would do with it.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn deliver_for_test(table: &LeaseTable, clock: &dyn Clock, body: &str) {
    deliver(table, clock, body);
}

/// Renders `digests` as a message body, for a test that needs a well-formed
/// one - or, by handing it more than the bound, a deliberately over-bound
/// one.
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub(crate) fn encode_for_test(digests: &[[u8; 32]]) -> String {
    encode(digests)
}

/// State barriers a test uses to observe the background subscriber without
/// waiting on a clock.
///
/// Compiled only under `cfg(test)` or the `testing` feature, the seam
/// `middleware::race_points` already uses for the same purpose. Nothing
/// here is consulted on a request's task.
#[cfg(any(test, feature = "testing"))]
pub(crate) mod seams {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use tokio::sync::Notify;

    static PAUSED: AtomicBool = AtomicBool::new(false);
    static SUBSCRIPTIONS: AtomicU64 = AtomicU64::new(0);

    fn resumed() -> &'static Notify {
        static RESUMED: std::sync::OnceLock<Notify> = std::sync::OnceLock::new();
        RESUMED.get_or_init(Notify::new)
    }

    fn subscribed_notify() -> &'static Notify {
        static SUBSCRIBED: std::sync::OnceLock<Notify> = std::sync::OnceLock::new();
        SUBSCRIBED.get_or_init(Notify::new)
    }

    /// Holds the applier before its next message, so a test can fill the
    /// bounded inbound queue and observe the drop that follows.
    pub fn pause_applier() {
        PAUSED.store(true, Ordering::SeqCst);
    }

    /// Releases the applier.
    pub fn resume_applier() {
        PAUSED.store(false, Ordering::SeqCst);
        resumed().notify_waiters();
    }

    /// Awaits while the applier is paused, registering before it reads the
    /// flag so a release between the two is never missed.
    pub(super) async fn hold_while_paused() {
        loop {
            let notified = resumed().notified();
            if !PAUSED.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    /// Records that a subscription was established.
    pub(super) fn note_subscribed() {
        SUBSCRIPTIONS.fetch_add(1, Ordering::SeqCst);
        subscribed_notify().notify_waiters();
    }

    /// How many subscriptions this process has established since it
    /// started.
    #[must_use]
    pub fn subscriptions() -> u64 {
        SUBSCRIPTIONS.load(Ordering::SeqCst)
    }

    /// Resolves once at least `at_least` subscriptions have been
    /// established, so a test can publish into a channel it knows is
    /// listening rather than into one it hopes is.
    pub async fn await_subscriptions(at_least: u64) {
        loop {
            let notified = subscribed_notify().notified();
            if SUBSCRIPTIONS.load(Ordering::SeqCst) >= at_least {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    /// The all-zero render key, built rather than spelled so the base64url
    /// width this parser requires cannot be miscounted by hand.
    fn fixture_key() -> RenderKey {
        RenderKey::from_base64url(&format!("rk1.{}", "A".repeat(43)))
            .expect("43 base64url characters decode to a 32-byte digest")
    }

    #[test]
    fn a_message_round_trips() {
        let digests = vec![digest(1), digest(2)];
        assert_eq!(decode(&encode(&digests)), Ok(digests));
    }

    #[test]
    fn an_empty_message_decodes_to_no_digests() {
        assert_eq!(decode(&encode(&[])), Ok(Vec::new()));
    }

    #[test]
    fn a_message_at_the_bound_is_read_and_one_past_it_is_dropped_whole() {
        let at_bound: Vec<[u8; 32]> = (0..MAX_HINT_DIGESTS)
            .map(|index| digest(u8::try_from(index % 251).expect("a byte")))
            .collect();
        assert_eq!(decode(&encode(&at_bound)).map(|read| read.len()), Ok(64));

        let over_bound: Vec<[u8; 32]> = (0..=MAX_HINT_DIGESTS)
            .map(|index| digest(u8::try_from(index % 251).expect("a byte")))
            .collect();
        assert_eq!(
            decode(&encode(&over_bound)),
            Err(HintDecodeError::OverBound),
            "a message over the bound is refused whole, never read in part"
        );
    }

    #[test]
    fn a_foreign_message_is_malformed_rather_than_read() {
        assert_eq!(decode("hello"), Err(HintDecodeError::Malformed));
        assert_eq!(decode("srh2,00"), Err(HintDecodeError::Malformed));
        assert_eq!(decode("srh1;00"), Err(HintDecodeError::Malformed));
        assert_eq!(decode("srh1,zz"), Err(HintDecodeError::Malformed));
        assert_eq!(
            decode(&format!("srh1,{}", "0".repeat(62))),
            Err(HintDecodeError::Malformed),
            "a digest of the wrong width is not a digest"
        );
    }

    #[test]
    fn a_hint_only_ever_shortens_a_lease() {
        let key = fixture_key();

        // A hint applied at an instant *past* the held expiry. The engine's
        // `hint_invalidate` takes the minimum, so the expiry stays where it
        // was: had it widened to the instant offered, the lease would still
        // read valid at 12_000, well past the 11_000 it was granted to.
        let table = LeaseTable::new();
        table.grant(&key, 1_000, 10_000, vec![digest(7)]);
        table.apply_hint(&[digest(7)], 50_000);
        assert!(
            !table.valid_at(&key, 12_000),
            "a hint offering a later instant must not extend an expiry"
        );

        // A hint naming nothing this table observes leaves every lease
        // exactly as it was - it neither shortens nor extends.
        let table = LeaseTable::new();
        table.grant(&key, 1_000, 10_000, vec![digest(7)]);
        assert_eq!(
            table.apply_hint(&[digest(9)], 5_000),
            0,
            "a digest this table observes nowhere shortens nothing"
        );
        assert!(
            table.valid_at(&key, 5_000),
            "and it leaves the lease it did not name standing"
        );

        // A hint naming an observed digest expires the lease at *this*
        // node's clock, so the next lookup rereads the authority.
        let table = LeaseTable::new();
        table.grant(&key, 1_000, 10_000, vec![digest(7)]);
        assert!(table.valid_at(&key, 2_000));
        assert_eq!(table.apply_hint(&[digest(7)], 2_000), 1);
        assert!(
            !table.valid_at(&key, 2_000),
            "an observed digest expires the lease at this node's own clock"
        );
    }

    #[test]
    fn a_hint_never_creates_a_lease() {
        let table = LeaseTable::new();
        assert_eq!(table.apply_hint(&[digest(7)], 2_000), 0);
        assert_eq!(table.len(), 0, "an empty table stays empty");
        assert!(!table.valid_at(&fixture_key(), 2_000));
    }
}
