//! One global middleware that serves proven Complete representations.
//!
//! See [`RenderCacheMiddleware::handle`] for the request flow. In short: a
//! request that matches a policy-bearing route and is a `GET` or `HEAD` is
//! looked up in the store; a coherent, fresh entry is served without
//! running the route handler at all; anything else falls through to a
//! coordinated render that, when the render's own dependency reads turn
//! out complete and eligible, publishes a new entry for the next request
//! to hit.
//!
//! # The honest boundary of what this guards against (fix rounds 5 and 6)
//!
//! `key_used_different_values_than_the_render_saw` declines to store a
//! render whose observed principal, tenant, or locale values differ from
//! the value the key was built from. It can only decline based on what the
//! collector actually recorded. Two categories of read still produce **no**
//! [`suprnova_live::render_cache::variance::ClassificationReason`] at all,
//! so nothing narrows the class and this guard has nothing to compare:
//!
//! - **Headers**, read through [`crate::http::Request::header`] and
//!   friends. Deliberately *not* instrumented: every request reads some
//!   header for some purpose, so recording every read would decline every
//!   response - a different way of shipping nothing, not a guard.
//! - **Configuration**, read through `Config::get::<T>()`. No producer
//!   exists (see `collector`'s own module doc): `Config::get` returns
//!   whole typed structs, so a read that touches secret configuration is
//!   indistinguishable at that seam from one that does not.
//!
//! Two things are narrower exceptions, not full coverage:
//!
//! - **Cookies**: [`crate::http::Request::cookies`] (and
//!   [`crate::http::Request::cookie`], which delegates to it) *are*
//!   instrumented, as a session read - cookies carry private material by
//!   nature (the type's own documentation example reads a session cookie),
//!   unlike a header read in general.
//! - **Feature flags, only through identity, only through the two
//!   evaluators this framework ships** (fix round 6, Leak 4; fix round 7,
//!   findings 1 and 2; fix round 8, finding 5): a middleware that resolves
//!   identity once, before the render, and stashes it where `is_enabled!`
//!   reads it *ambiently* during the render (the framework's own
//!   feature-flag middleware, whose documented purpose is exactly this)
//!   observes nothing through any instrumented accessor, so nothing
//!   narrowed. `DatabaseEvaluator` and `CachedEvaluator` (see
//!   `crate::features::fields::observe_identity`) now record the context's
//!   identity at the point `is_enabled!` actually reads it: the user id as a
//!   principal observation when the flag has any `user:`-scoped rule, and
//!   the team as a *tenant* observation when it has any `team:`-scoped rule.
//!   The condition is a property of the flag, not of which scope key matched
//!   this visitor, so a flag whose only override belongs to another user
//!   still records that axis for every reader of it - the reader's own id
//!   when the context carries one, and a bare read with no material when it
//!   does not. Both matter: a reader who carries no id reaches the same
//!   fall-through answer, so publishing their page under a key the
//!   override's owner also hits would bypass that override, and the bare
//!   read is what makes the empty-set path below decline for a route that
//!   declares no such dimension. A flag with only a global rule records
//!   nothing on either axis and stays cacheable for every visitor, signed in
//!   or not, which is correct: its answer does not depend on the reader.
//!   A custom `Evaluator` outside these two, an application-defined scope
//!   key that is neither `user:` nor `team:`, or a decision that varies on
//!   something other than the context's identity, is not covered - this
//!   observes *identity*, not the flag's own name or value.
//! - **The session's principal identifier is an identity read; every other
//!   session value is a session read.** `Auth::id()` resolves through
//!   request state first and falls back to the persisted session. That
//!   fallback goes through `crate::session::middleware`'s private
//!   `session_identity`, which records a principal read and, when there is
//!   an id, the principal value - never a session read. Only the two
//!   authentication identifiers reach it (the default guard's `user_id` and
//!   a named guard's own id), and a closed enum in that module is what
//!   enforces it rather than a convention. `session()`, `session_mut`, and a
//!   cookie read all still record a session read and still narrow straight
//!   to `Uncacheable`, because no key partitions by an arbitrary session
//!   value.
//!
//!   Three consequences follow, each with a test that holds it down. An
//!   anonymous visitor of a `PrivateCached` route declaring `Principal`
//!   caches under the `Anonymous` key: the render resolved no identity, so
//!   no material is observed, the key says `Anonymous`, and the empty-set
//!   path below finds them in agreement
//!   (`render_cache::middleware::an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`).
//!   A signed-in visitor of such a route is stored once per principal, and
//!   the entry observes the row the provider resolved them from, so a write
//!   to that row invalidates their page
//!   (`a_session_resolved_principal_is_stored_and_partitioned_per_principal`,
//!   `a_session_resolved_principal_render_observes_the_row_it_was_resolved_from`).
//!   And a principal read on a route that declares no `Principal` variance
//!   is declined by the value comparison below, so the reclassification
//!   opens no path to serving one visitor's page to another
//!   (`a_session_resolved_principal_is_declined_where_no_principal_variance_is_declared`).
//!
//!   What stays a boundary: this classifies the *identity read*, not the
//!   body. An anonymous render whose bytes derive from an input
//!   classification cannot see - a request header, `Config::get` - is
//!   storable as far as this is concerned, and declaring the matching
//!   variance is the route's own job. That is the same residual the
//!   header and configuration bullets below describe, and reclassifying
//!   the identity read neither widened nor narrowed it.
//! - **Authorization decisions are always treated as per-principal.**
//!   `Gate::allows` records that a decision was evaluated, never what the
//!   decision consulted, so `AuthorizationRead` requires the `Principal`
//!   dimension unconditionally. A route keyed only by `Tenant` whose gate is
//!   genuinely per-tenant therefore never caches, even though it is safe -
//!   proven functional, not a leak, by the sixth review. This is deliberate
//!   and fails closed: nothing here can tell a per-tenant gate from a
//!   per-user one, and treating every decision as per-user is the only safe
//!   default. The remedy is for such a route to declare `Principal`
//!   alongside `Tenant`, which partitions by both and does cache. Parked for
//!   a later iteration: having `Gate` record the identity it consulted would
//!   let the value comparison decide instead of the mapping.
//! - **Eloquent global scopes are not instrumented.** A registered
//!   [`crate::eloquent::scopes::GlobalScope`] runs inside every
//!   `Model::query` call, and its own registration doc invites the scope to
//!   read per-request state such as the current tenant id out of a
//!   thread-local, a `tokio::task_local!`, or an atomic, none of which this
//!   collector observes; a tenant-scoped global scope therefore partitions
//!   what the render reads without recording a `Tenant` observation, so the
//!   remedy is to declare `Tenant` variance on every route whose models
//!   carry one.
//!
//! A route handler that branches its output on a header or a config value -
//! without also declaring the corresponding variance - is outside what this
//! middleware can protect on its own.
//! [`crate::render_cache::collector::observe_undeclared`] exists for
//! exactly this: an application or a future adapter that knows it read
//! something undeclared can call it explicitly, and `classify` already
//! narrows to `Uncacheable` for it - but nothing calls it automatically for
//! header or config reads today. Do not read the guard's presence as
//! protection for these; it is not, and pretending otherwise is exactly the
//! shape that let four earlier rounds of this task's review each find a
//! different unguarded seam.
//!
//! # Seed promotion deadlines
//!
//! A Live document that mounted a public-seed island records the earliest
//! promotion deadline it embedded, and that deadline is stored with the
//! entry: the collector records it
//! ([`super::collector::observe_live_document_mount`], through
//! [`super::live::record_mount`]), `lead_render` reads it off the report
//! and hands it to `entry_header`, and every freshness decision afterwards
//! reads it back out of [`EntryHeader::seed_deadline_ms`]. An entry whose
//! seed deadline has passed is Dead however fresh its clock says it is, so
//! a cached document can never outlive the seed inside it.
//! `render_cache::live::a_public_seed_document_is_a_hit_until_its_seed_deadline`
//! is the end-to-end guard: the entry is a hit right up to the deadline and
//! renders again the moment it is past, which is only possible if the value
//! reached the stored header.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use sea_orm::{DbBackend, IsolationLevel};
use sha2::{Digest as _, Sha256};
use suprnova_live::clock::Clock;
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::identity::{BuildId, RouteIdentity};
use suprnova_live::render_cache::coherence::{
    FreshnessState, ValidationLease, evaluate_freshness, warning_header,
};
use suprnova_live::render_cache::entry::{
    CompleteEntry, DecodedEntry, EntryHeader, EntryLimits, REPLAYABLE_HEADERS, SafeHeaders,
    Validator, decode, encode, encode_composite,
};
use suprnova_live::render_cache::generation::{
    CoherenceCheck, GenerationLedger, GenerationSet, ObservationWindow,
};
use suprnova_live::render_cache::hot::{HotEntry, HotRequest, ResponseParts, respond, serve_hot};
use suprnova_live::render_cache::http::{
    ConditionalOutcome, cache_control_value, evaluate_conditional, vary_value,
};
use suprnova_live::render_cache::key::{RenderKey, RenderKeyInput};
use suprnova_live::render_cache::policy::{
    CoherenceMode, Eligibility, RenderCachePolicy, ResponseSignals,
};
use suprnova_live::render_cache::singleflight::{
    RebuildAdmission, RebuildCoordinator, RebuildLease,
};
use suprnova_live::render_cache::store::{
    MemoryRenderStore, PublishOutcome, RenderStore, StoredEntry,
};
use suprnova_live::render_cache::variance::{
    ClassificationOutcome, ClassificationReason, DimensionValue, ObservedContext, PrivateMaterial,
    VarianceDescriptor, VarianceDimension, classify,
};
use suprnova_live::render_cache::{
    FailurePolicy, RenderCacheError, RenderCacheErrorKind, RepresentationClass,
};

use crate::Auth;
use crate::database::DB;
use crate::http::{HttpResponse, Request, Response};
#[cfg(feature = "localization")]
use crate::localization::Lang;
use crate::middleware::{Middleware, Next};
use crate::telemetry::metrics::Metrics;

use super::L1Provider;
use super::collector::{self, Collector};
use super::config::RenderCacheConfig;
use super::live;
use super::registry::RenderCachePolicyTable;
use super::stitch;
use super::telemetry as render_cache_telemetry;

/// Domain separator for the route identity digest this middleware derives
/// from a registered route pattern. Independent of Live's own internal
/// route-identity digest (a different digest, for a different purpose):
/// purpose separation keeps the two from ever being compared against each
/// other by accident, even though both ultimately hash a route pattern.
const ROUTE_IDENTITY_DOMAIN: &[u8] = b"suprnova/render-cache/route-identity/v1\0";

/// Maximum re-admission depth for a singleflight waiter whose post-wait
/// entry comes back `StaleOnError` or `Dead`. See the recursive call site in
/// `render_and_publish` (fix round 3, item 5).
const MAX_WAIT_REBUILD_DEPTH: u32 = 8;

/// A provider (store, ledger, or coordinator) failed **before** the route
/// handler ran. Carries the untouched request and `next` back to the
/// caller so [`FailurePolicy`] can decide whether to pass the request
/// through uncached or refuse it - see [`RenderCacheMiddleware::handle`].
///
/// Never constructed once the route handler has actually run: a failure
/// after that point has a real response to fall back to, and this
/// middleware always serves it rather than manufacturing a closed
/// response for a caching problem the visible response has nothing to do
/// with.
struct ProviderFailure(Request, Next);

/// The RenderCache middleware: one global layer that serves proven
/// Complete representations. See the module documentation for the request
/// flow.
///
/// Holds no state of its own (fix round 3, item 5): an earlier version
/// captured `Arc<RenderCacheRuntime>` at construction, which meant a second
/// `RenderCache::install` in one process replaced the runtime
/// [`super::RenderCache::inspect`], [`super::RenderCache::advance_epoch`],
/// and friends read, while `register_global_middleware`'s per-type
/// idempotency (see `install`'s own doc) meant the *already registered*
/// middleware instance - still holding the first runtime - kept serving
/// every request. Inspection and epoch control would see one runtime while
/// requests were served from another. This type now reads
/// `super::RenderCache::runtime` fresh on every request instead, so there
/// is only ever one source of truth, and repeated `install` calls (this
/// crate's own test suite calls it once per test) behave correctly with no
/// special case.
pub struct RenderCacheMiddleware;

/// The authority epoch this node currently believes in, held between
/// requests instead of read from the ledger on each one.
///
/// The rule this type exists to enforce: **the epoch is read from authority
/// only on first use, or together with a generation reread; never on its
/// own, per request.** Before it, `serve` read
/// [`GenerationLedger::epoch`] for every GET and HEAD to a policy-covered
/// route - a cached route's hit could not cost less than one SQL statement,
/// which `00-overview.md:330` (a Complete L0 hit is served "with no
/// database/provider round trip") and spec 18's leases (lines 112-131:
/// leases exist "to avoid querying authority on every hot hit", and a hot
/// node holding one serves "without repeated authority reads") both forbid.
/// The epoch is now leased exactly like the generation set it is compared
/// against, and refreshed by the same reads: `authority_coherence`,
/// `fresh_reread_is_coherent`, and the waiter's re-admission in
/// `render_and_publish` each store the epoch the authority just reported.
///
/// A `Mutex<Option<u64>>` rather than atomics, deliberately. `None` is a
/// state an integer sentinel cannot express honestly (every `u64` is a
/// reachable epoch as far as this type is concerned), and a pair of atomics
/// carrying "present" and "value" separately would need an ordering
/// argument for a value read at most once per request. Under the lock there
/// is no tearing to argue about: the `u64` is only ever read or written
/// inside a critical section, and every one of those sections is a single
/// statement with no `.await` in it, so no guard is ever held across a
/// suspension point. Lock poisoning is absorbed with `into_inner`, matching
/// [`RenderCacheRuntime::leases`]: a panic elsewhere must not turn every
/// later request into an error.
///
/// Staleness is bounded, and by the same bound spec 18 already applies to a
/// lease-mode route: see [`coherence`]'s own comment for the three paths an
/// epoch advance takes to reach a request.
pub(super) struct EpochCache(Mutex<Option<u64>>);

impl EpochCache {
    /// An empty cache. The first request that needs the epoch reads the
    /// authority once and fills it.
    pub(super) const fn empty() -> Self {
        Self(Mutex::new(None))
    }

    /// The critical section all three accessors share. Every caller below
    /// dereferences the guard in the same statement it takes it, so the
    /// section is one statement long and no `.await` can appear inside it.
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<u64>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The leased epoch, or `None` before the first authority read and
    /// after an [`Self::invalidate`].
    pub(super) fn get(&self) -> Option<u64> {
        *self.slot()
    }

    /// Records the epoch an authority read just reported.
    pub(super) fn refresh(&self, epoch: u64) {
        *self.slot() = Some(epoch);
    }

    /// Drops the lease, so the next request that needs the epoch reads the
    /// authority. Called by [`super::RenderCache::advance_epoch`], which is
    /// what makes an emergency bump on this node reach the very next
    /// request, lease-mode routes included.
    pub(super) fn invalidate(&self) {
        *self.slot() = None;
    }
}

/// The assembled RenderCache runtime: stores, ledger, coordinator, keys,
/// policy table, configuration, and clock. One instance per installed
/// process; `super::RenderCache::runtime` hands out clones of the `Arc`.
pub struct RenderCacheRuntime {
    pub(crate) config: RenderCacheConfig,
    /// `config.build_id` parsed once, at install, into the bounded identity
    /// the lookup key feeds (final review, F6). Before this the key path
    /// parsed the configured text on every request and silently fell back to
    /// `default` when `APP_BUILD_ID` did not satisfy `BuildId`'s grammar, so
    /// every deploy with such a value shared one namespace with no signal;
    /// `RenderCache::install` now refuses such a value instead.
    pub(crate) build: BuildId,
    pub(crate) table: RenderCachePolicyTable,
    pub(crate) l0: MemoryRenderStore,
    /// The configured L1 provider, or `None` when L1 is disabled. Held as
    /// [`L1Provider`] rather than one concrete store so the profile decides
    /// which tier serves this process; every read and publication below
    /// goes through its `RenderStore` implementation.
    pub(crate) l1: Option<L1Provider>,
    pub(crate) ledger: Arc<dyn GenerationLedger>,
    /// The same authority [`ledger::SqlGenerationLedger`] wrapped in
    /// `ledger` above, kept as its concrete type because
    /// [`ledger::SqlGenerationLedger::advance_epoch`] is not part of the
    /// [`GenerationLedger`] trait (it is an emergency operator tool, not a
    /// per-request read or write) and so cannot be reached through the
    /// trait object. [`super::RenderCache::advance_epoch`] calls this field
    /// rather than constructing a fresh `SqlGenerationLedger` of its own -
    /// see fix round 2, item 7 - so a future ledger override reaches this
    /// operator too. `SqlGenerationLedger` is zero-sized and `Copy`, so
    /// keeping both this and `ledger` costs nothing.
    pub(crate) epoch_ledger: super::ledger::SqlGenerationLedger,
    pub(crate) coordinator: Arc<dyn RebuildCoordinator>,
    pub(crate) keys: SnapshotKeyRing,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) limits: EntryLimits,
    /// Local validation leases for [`CoherenceMode::Lease`] routes, keyed by
    /// the entry's lookup key.
    ///
    /// Bounded by opportunistic cleanup, not by a background sweep: every
    /// [`coherence`] call that inserts a fresh lease first evicts every
    /// entry whose lease has already expired (see the insert site), so the
    /// map holds at most one entry per distinct lease-mode key that has been
    /// requested within the last `max_age_ms` - not, as an earlier version
    /// of this comment claimed, an unbounded one held for the process
    /// lifetime (fix round 2, item 6). An entry whose underlying L0/L1 store
    /// entry was evicted separately is not proactively removed from here;
    /// it is inert (coherence is only ever consulted after a store hit) and
    /// is swept the same way once its lease's own timer expires.
    pub(crate) leases: Mutex<BTreeMap<RenderKey, ValidationLease>>,
    /// The leased authority epoch - see [`EpochCache`] for the rule it
    /// enforces and why a hit must not read the epoch on its own.
    pub(super) epoch_cache: EpochCache,
    /// Hot hits formed by [`hot_response`] on this runtime, for
    /// [`super::RenderCache::hot_serves_for_test`].
    ///
    /// A field rather than a process-global counter, so it needs no reset
    /// hook of its own: [`super::RenderCache::install`] builds a fresh
    /// runtime (and a fresh L0 with it), which zeroes this for the next test
    /// the way a new store zeroes its own contents.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) hot_serves: std::sync::atomic::AtomicU64,
    /// Background rebuilds this runtime *decided* to spawn, for
    /// [`super::RenderCache::background_rebuilds_for_test`].
    ///
    /// Counted in [`RenderCacheMiddleware::spawn_background_rebuild`],
    /// immediately before the `tokio::spawn` and therefore still on the
    /// request's own path, so it is already final when the dispatch that
    /// took the decision returns. That is what a test asserting a route
    /// *never* spawns one needs: waiting on the spawned task's own effects
    /// can only ever show that it has not finished yet, while this shows
    /// that it was never started. Zeroed by a fresh
    /// [`super::RenderCache::install`], like `hot_serves` beside it.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) background_rebuilds: std::sync::atomic::AtomicU64,
}

impl RenderCacheRuntime {
    /// Counts one hot hit served off this runtime.
    ///
    /// Compiled to nothing without the `testing` feature, so a production
    /// build pays neither the atomic nor the field. `Relaxed` is enough: the
    /// only reader is a test seam called from the same thread that dispatched
    /// the request it is asking about, after that request completed.
    fn count_hot_serve(&self) {
        #[cfg(any(test, feature = "testing"))]
        self.hot_serves
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Counts one background rebuild this runtime decided to spawn. Same
    /// shape and same reasoning as [`Self::count_hot_serve`]; see the
    /// `background_rebuilds` field for what makes the count useful.
    fn count_background_rebuild(&self) {
        #[cfg(any(test, feature = "testing"))]
        self.background_rebuilds
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Bounded, monotonic-enough wall clock reading in Unix milliseconds.
    /// A clock failure (closed provider) degrades to 0 rather than
    /// panicking or propagating - every caller of this treats age and
    /// freshness arithmetic as saturating, so a momentary 0 reads as "very
    /// old" rather than corrupting a comparison.
    pub(crate) fn now_ms(&self) -> u64 {
        self.clock
            .now()
            .map(suprnova_live::identity::UnixMillis::get)
            .unwrap_or_default()
    }
}

/// The permission version bound into `Principal` material, frozen at 0.
///
/// The engine's `PrivateMaterial::principal` keeps a version parameter so a
/// host may bind one; this host no longer does. Until the closing fix round
/// the framework bound a process-local `AtomicU64` here that
/// `RenderCache::bump_permission_version` incremented, which changed every
/// signed-in visitor's key on a bump but reset to 0 on restart while an L1
/// entry keyed under version 0 survived on disk (final review, F3). A bump
/// now advances a persisted generation on
/// [`collector::permission_version_identity`], which every render whose key
/// carries a resolved `Principal` observes (see [`run_render`]), so the
/// ledger rather than the key is what makes a pre-bump private entry a
/// miss, and it stays a miss across a restart because the generation lives
/// in the database.
const FROZEN_PERMISSION_VERSION: u64 = 0;

/// Coherence outcome of a stored entry against the current authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Coherence {
    Coherent,
    Moved,
}

/// Which layer answered a lookup, for telemetry and promotion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Layer {
    L0,
    L1,
}

/// What a lookup found: a Complete L0 publication prepared for hot service,
/// or a stored entry this request had to decode.
///
/// The two are the same publication - the same bytes, the same fence, the
/// same publication instant - and differ only in how much of the response
/// was already formed. A hot entry carries every header value a hit can
/// precompute, formed once when it was published (see
/// [`suprnova_live::render_cache::hot::HotEntry`]); a decoded one forms them
/// now. Everything the request flow reads off a hit - its header, its
/// publication instant, the layer that answered, and the Complete
/// representation behind it - is read through the accessors below, so both
/// shapes travel one path and cannot be judged by different rules.
pub(crate) enum FoundEntry {
    /// A Complete L0 hit, prepared at publication.
    Hot(Arc<HotEntry>),
    /// A stored entry decoded on this request. Boxed so a hot hit carries a
    /// pointer rather than a decoded entry's worth of stack: the decoding
    /// path has already allocated for every string in that header by the
    /// time this box is made, and the hot path is the one that must stay
    /// cheap to move.
    Decoded(Box<DecodedHit>),
}

/// A stored entry this request decoded, and where it came from.
pub(crate) struct DecodedHit {
    /// The decoded representation.
    pub(crate) entry: DecodedEntry,
    /// Its stored bytes and publication facts.
    pub(crate) stored: StoredEntry,
    /// The layer that answered.
    pub(crate) layer: Layer,
}

impl FoundEntry {
    /// The stored entry's header, for coherence and freshness.
    fn header(&self) -> &EntryHeader {
        match self {
            Self::Hot(hot) => hot.entry().header(),
            Self::Decoded(hit) => hit.entry.header(),
        }
    }

    /// The publication instant `Age` and freshness are measured from. A hot
    /// entry records the instant it was published under, which is the same
    /// instant its stored bytes carry.
    pub(crate) fn published_at_ms(&self) -> u64 {
        match self {
            Self::Hot(hot) => hot.published_at_ms(),
            Self::Decoded(hit) => hit.stored.published_at_ms,
        }
    }

    /// The layer that answered. A hot entry is an L0 slot by construction:
    /// only [`MemoryRenderStore`] holds one.
    const fn layer(&self) -> Layer {
        match self {
            Self::Hot(_) => Layer::L0,
            Self::Decoded(hit) => hit.layer,
        }
    }

    /// The Complete representation behind this hit, or `None` for a
    /// Composite entry, which is a finished answer to nobody: it has to be
    /// assembled for the request that asked for it.
    fn complete(&self) -> Option<&CompleteEntry> {
        match self {
            Self::Hot(hot) => Some(hot.entry()),
            Self::Decoded(hit) => match &hit.entry {
                DecodedEntry::Complete(entry) => Some(entry),
                DecodedEntry::Composite(_) => None,
            },
        }
    }
}

/// Closed lookup outcome, for telemetry's `outcome` attribute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LookupOutcome {
    L0Hit,
    L1Hit,
    Conditional,
    Stale,
    Miss,
    Bypass,
    Moved,
    Declined,
}

impl LookupOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::L0Hit => "l0",
            Self::L1Hit => "l1",
            Self::Conditional => "conditional",
            Self::Stale => "stale",
            Self::Miss => "miss",
            Self::Bypass => "bypass",
            Self::Moved => "moved",
            Self::Declined => "declined",
        }
    }

    pub(crate) fn record(self) {
        Metrics::counter(render_cache_telemetry::LOOKUPS)
            .inc_with(&[(render_cache_telemetry::OUTCOME, self.as_str())]);
        if matches!(
            self,
            Self::L0Hit | Self::L1Hit | Self::Conditional | Self::Stale
        ) {
            Metrics::counter(render_cache_telemetry::HITS)
                .inc_with(&[(render_cache_telemetry::OUTCOME, self.as_str())]);
        }
    }
}

#[async_trait]
impl Middleware for RenderCacheMiddleware {
    /// 1. `request.route_pattern()` -> effective policy, else pass through.
    /// 2. Method GET or HEAD, else pass through; disabled config -> pass
    ///    through.
    /// 3. Derive the lookup key from route, params, declared query, host,
    ///    media, encoding, build, epoch, and declared variance;
    ///    an undeclared query parameter bypasses.
    /// 4. Look up L0 then L1, decoding and evicting a defective entry.
    /// 5. A hit is checked for coherence and freshness, then served
    ///    (conditionally, or with the body for GET / headers only for
    ///    HEAD); a moved or expired entry falls through to a render.
    /// 6. A miss is admitted through the rebuild coordinator: the leader
    ///    renders, a waiter reuses the leader's publication or renders
    ///    without publishing, and an exhausted waiter list renders
    ///    without publishing too.
    /// 7. The leader's render runs under a request-scoped collector,
    ///    inside a read transaction when a database is configured, so the
    ///    generations it reads at close share one snapshot with the
    ///    data the render itself read.
    /// 8. After the render, the response is checked for eligibility and
    ///    classified from what the collector actually observed;
    ///    ineligible or uncacheable responses are served without storing.
    /// 9. A fresh reread outside the view catches any dependency that
    ///    moved during the render; a move discards the candidate.
    /// 10. A coherent candidate is encoded and published to L0 (and L1 when
    ///     the policy uses it) under a fence from the coordinator.
    /// 11. The served response carries ETag, Cache-Control, Vary, and Age
    ///     (and Warning when stale).
    /// 12. A provider failure before the handler ran is decided by the
    ///     route's [`FailurePolicy`]: pass through uncached, or refuse.
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Read fresh on every request rather than captured at construction
        // - see the type's own doc (fix round 3, item 5).
        let Some(runtime) = super::RenderCache::runtime() else {
            return next(request).await;
        };
        let Some(pattern) = request.route_pattern().map(str::to_owned) else {
            return next(request).await;
        };
        if !runtime.config.enabled || !matches!(request.method().as_str(), "GET" | "HEAD") {
            return next(request).await;
        }
        let Some(policy) = runtime.table.effective_policy(&pattern) else {
            return next(request).await;
        };
        match self.serve(&runtime, request, next, &pattern, &policy).await {
            Ok(response) => response,
            Err(ProviderFailure(request, next)) => match policy.failure() {
                FailurePolicy::Open => next(request).await,
                FailurePolicy::Closed => Ok(HttpResponse::text("").status(503)),
            },
        }
    }
}

impl RenderCacheMiddleware {
    async fn serve(
        &self,
        runtime: &Arc<RenderCacheRuntime>,
        request: Request,
        next: Next,
        pattern: &str,
        policy: &RenderCachePolicy,
    ) -> Result<Response, ProviderFailure> {
        if !declared_query_ok(&request, policy) {
            LookupOutcome::Bypass.record();
            return Ok(next(request).await);
        }
        // The leased epoch, filled by one authority read on the first
        // request of this runtime and renewed by every later read that
        // returns an epoch - never read here on its own. See [`EpochCache`].
        let epoch = match runtime.epoch_cache.get() {
            Some(epoch) => epoch,
            None => match runtime.ledger.epoch().await {
                Ok(epoch) => {
                    runtime.epoch_cache.refresh(epoch);
                    epoch
                }
                Err(_) => return Err(ProviderFailure(request, next)),
            },
        };
        // Test-only race seam (R72/R83): fires right after the epoch this
        // request's `RenderJob` will carry is captured, and before the
        // render it describes begins - so an epoch advance armed here is
        // baked into the job as already stale by the time that render's
        // own fresh reread checks it, proving such a render's candidate is
        // never published. Still true now the epoch is leased: an advance
        // through `RenderCache::advance_epoch` drops the lease and clears
        // L0, but this request already holds the value it captured, so its
        // job carries the old epoch into a render whose reread reads the
        // new one.
        #[cfg(any(test, feature = "testing"))]
        race_points::fire(&race_points::EPOCH_CAPTURED).await;
        let Ok(input) = key_input(runtime, &request, pattern, policy, epoch) else {
            // Fix round 5: a dimension's value could not be declared (see
            // `variance_descriptor`'s own doc) - bypass uncached rather
            // than publish a key that does not actually reflect what the
            // route declared.
            LookupOutcome::Bypass.record();
            return Ok(next(request).await);
        };
        let Ok(key) = RenderKey::derive(&input, &runtime.keys) else {
            LookupOutcome::Bypass.record();
            return Ok(next(request).await);
        };
        let mut job = RenderJob::new(input, key);

        let hit = match lookup(runtime, policy, job.key()).await {
            Ok(hit) => hit,
            Err(()) => return Err(ProviderFailure(request, next)),
        };
        let Some(found) = hit else {
            LookupOutcome::Miss.record();
            return render_and_publish(runtime, request, next, policy, job, 0).await;
        };

        let coherence = match coherence(runtime, job.key(), policy, found.header()).await {
            Ok(coherence) => coherence,
            Err(()) => return Err(ProviderFailure(request, next)),
        };
        let now = runtime.now_ms();
        let state = freshness_state(
            policy,
            coherence,
            found.header().class,
            found.published_at_ms(),
            now,
            found.header().seed_deadline_ms,
        );
        // A reread inside `coherence` may have renewed the leased epoch to a
        // value this request's key was not derived under - another node
        // advanced it, and this is where this node finds out. Every arm
        // below that rebuilds has to publish under the epoch it is judged
        // against, so each one re-derives the key from the same input first
        // (`RenderJob::restamp`); a `Moved` caused by the epoch is otherwise
        // handled exactly like a `Moved` caused by a generation.
        //
        // Per arm rather than once here, deliberately: the `Fresh` arm never
        // touches `job`, and it is the hottest path in this module. Restamping
        // before the match would cost it a second `epoch_cache` lock for no
        // effect at all - the one `serve` already took to derive the key is
        // the only one a fresh hit pays.
        match state {
            FreshnessState::Fresh => {
                (match found.layer() {
                    Layer::L0 => LookupOutcome::L0Hit,
                    Layer::L1 => LookupOutcome::L1Hit,
                })
                .record();
                // The header's presence is tested first, deliberately:
                // `evaluate_conditional` forms the entity tag to compare
                // against, and forming one allocates. A request that sent no
                // `If-None-Match` - the ordinary hit - must not pay for a
                // comparison whose answer is already known.
                if let Some(if_none_match) = request.header("if-none-match")
                    && let Some(complete) = found.complete()
                    && matches!(
                        evaluate_conditional(Some(if_none_match), complete.validator()),
                        ConditionalOutcome::NotModified
                    )
                {
                    LookupOutcome::Conditional.record();
                }
                Ok(deliver_hit(runtime, request, next, policy, found, now, None).await)
            }
            FreshnessState::StaleServable => {
                // The background rebuild below runs under `job`. Exercised by
                // `an_epoch_advanced_by_another_node_serves_a_stale_servable_entry_once_then_rebuilds`.
                if job.restamp(runtime).is_err() {
                    LookupOutcome::Bypass.record();
                    return Ok(next(request).await);
                }
                LookupOutcome::Stale.record();
                // Fix round 2, item 4: a route whose variance depends on
                // ambient (task-local) context does not get a background
                // rebuild - see the module doc's "Background rebuild's
                // ambient context" note for why. A stitched route is
                // excluded for the same reason and one more: its render is
                // the route's own guarded chain, and a `tokio::spawn`ed
                // task carries none of this request's auth task-locals, so
                // the shell it produced would be whatever the gate renders
                // for nobody. The stale entry is still served immediately
                // either way; only the background refresh is skipped.
                if is_stitched(policy) || variance_depends_on_ambient_context(policy) {
                    return Ok(deliver_hit(
                        runtime,
                        request,
                        next,
                        policy,
                        found,
                        now,
                        warning_header(state),
                    )
                    .await);
                }
                // Below this point the request is spent on the background
                // rebuild, so the served response has to be built from the
                // entry first - which only a Complete entry can be. A
                // Composite entry here means a store defect (`decode`
                // refuses a Composite entry under any other class, and this
                // route did not declare the stitched one), and so does a
                // Complete entry that cannot be formed into a valid
                // response; `hit_response` reports both as `None` and both
                // are treated as `deliver_hit` treats them: a miss.
                let Some(response) = hit_response(
                    runtime,
                    &found,
                    request.method(),
                    request.header("if-none-match"),
                    policy,
                    now,
                    warning_header(state),
                ) else {
                    LookupOutcome::Miss.record();
                    return Ok(next(request).await);
                };
                self.spawn_background_rebuild(
                    Arc::clone(runtime),
                    request,
                    next,
                    policy.clone(),
                    job,
                );
                Ok(Ok(response))
            }
            FreshnessState::StaleOnError => {
                // The foreground rebuild below runs under `job`; see the
                // note above the match for why this is per arm.
                if job.restamp(runtime).is_err() {
                    LookupOutcome::Bypass.record();
                    return Ok(next(request).await);
                }
                LookupOutcome::Miss.record();
                // Captured before `request` moves into the rebuild attempt,
                // so a fallback to the stale entry does not need the request
                // back - matching `lead_render`'s own capture.
                let method = request.method().clone();
                let if_none_match = request.header("if-none-match").map(str::to_owned);
                let outcome = render_and_publish(runtime, request, next, policy, job, 0).await;
                match stale_on_error_fallback(
                    runtime,
                    policy,
                    &found,
                    &outcome,
                    &method,
                    if_none_match.as_deref(),
                    now,
                ) {
                    Some(response) => {
                        LookupOutcome::Stale.record();
                        Ok(Ok(response))
                    }
                    None => outcome,
                }
            }
            FreshnessState::Dead => {
                // The rebuild below runs under `job`. Exercised by
                // `an_epoch_advanced_by_another_node_reaches_an_authority_mode_route_on_its_next_hit`
                // and by its lease-mode sibling, whose routes both declare a
                // zero stale window, so a `Moved` entry lands here.
                if job.restamp(runtime).is_err() {
                    LookupOutcome::Bypass.record();
                    return Ok(next(request).await);
                }
                LookupOutcome::Miss.record();
                render_and_publish(runtime, request, next, policy, job, 0).await
            }
        }
    }

    /// Spawns a bounded background rebuild for a stale-servable entry.
    /// Bounded by the coordinator's own lease/waiter limits, not by
    /// anything tracked here - a leaked task list is not a risk this
    /// spawns into, since exactly one rebuild per key can ever be
    /// admitted as leader at a time.
    fn spawn_background_rebuild(
        &self,
        runtime: Arc<RenderCacheRuntime>,
        request: Request,
        next: Next,
        policy: RenderCachePolicy,
        job: RenderJob,
    ) {
        Metrics::counter(render_cache_telemetry::REBUILDS).inc();
        runtime.count_background_rebuild();
        tokio::spawn(async move {
            let _ = render_and_publish(&runtime, request, next, &policy, job, 0).await;
        });
    }
}

/// Whether every query parameter present on `request` is declared by
/// `policy`. `false` means bypass: an undeclared query parameter is
/// request-specific information the lookup key does not account for, so
/// this request must not be answered from - or stored into - the shared
/// cache.
fn declared_query_ok(request: &Request, policy: &RenderCachePolicy) -> bool {
    let declared = policy.query().declared_names();
    request
        .query_params()
        .keys()
        .all(|name| declared.contains(name))
}

/// Whether `policy`'s declared variance depends on state that is task-local
/// rather than carried on the `Request` value itself - `Locale`
/// (`Lang::locale()`) or `Principal` (`Auth::id()`), both
/// `tokio::task_local!`-backed.
///
/// A `tokio::spawn`ed task does not inherit task-locals, so a background
/// rebuild for one of these routes would compute a *different* variance
/// than the key it is about to publish under: a locale-varying route's
/// background rebuild would render the default locale's content and publish
/// it under another locale's key, and a principal-varying route's would
/// render anonymously and publish under a specific principal's key. That is
/// a content-identity mismatch, not a wasted render. An earlier note called
/// it harmless on the grounds that
/// `key_used_different_values_than_the_render_saw` would decline the store;
/// round 1 of this task's review established that narrowing never
/// repartitions an already-derived key, so that justification does not
/// hold and this predicate is the actual guard (fix round 2, item 4).
///
/// `RenderCacheMiddleware::serve` therefore spawns no background rebuild at
/// all for such a route. It still serves its stale-servable entry
/// immediately - the "never blocks" guarantee is unaffected - it just does
/// not also refresh it in the background; the entry refreshes once it goes
/// Dead and the next request renders it in the foreground, where the
/// ambient context is the real request's own.
/// `render_cache::middleware::a_stale_principal_route_never_spawns_a_background_rebuild`
/// is the guard.
///
/// `Tenant` is deliberately excluded: `Request::live_tenant` reads a field
/// on the moved `Request`, not a task-local, so it is safe across the spawn.
/// A request id is also lost across the spawn - log correlation only, not a
/// cache-key concern, so it is not guarded here.
fn variance_depends_on_ambient_context(policy: &RenderCachePolicy) -> bool {
    policy.vary().contains(&VarianceDimension::Locale)
        || policy.vary().contains(&VarianceDimension::Principal)
}

/// A purpose-separated digest of a registered route pattern. See
/// [`ROUTE_IDENTITY_DOMAIN`].
fn route_identity(pattern: &str) -> RouteIdentity {
    let mut hasher = Sha256::new();
    hasher.update(ROUTE_IDENTITY_DOMAIN);
    hasher.update(pattern.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    RouteIdentity::from_bytes(&digest)
        .expect("sha-256 output is exactly 32 bytes, matching RouteIdentity's fixed length")
}

/// Builds the declared variance descriptor for a route's policy, reading
/// only what the policy actually declared - never more.
///
/// # Errors
///
/// Fails when a dimension's resolved value cannot be declared - most
/// reachably, `Host`'s value coming from `request.http_host()`, which is
/// attacker-controlled and can exceed the bound `declare` enforces on a
/// `Public` value. Fix round 5: the caller used to discard this with
/// `let _ = ...`, silently dropping the dimension from the key even though
/// the route declared it - a route that declares `Host` would then key
/// every request the same regardless of host, the same shape of silent
/// mis-key this task's review has repeatedly found. Propagated to the
/// caller instead, which bypasses uncached rather than publish a key that
/// does not actually reflect what it claims to.
///
/// Also fails for a declared `FeatureVersion`, `ConfigVersion`, or
/// `Application` dimension - fix round 6 moved this rejection here from the
/// engine's `RenderCachePolicy::validate` (see its own doc): this host has
/// no producer for any of the three, and "this host has no producer" is a
/// fact about the host, not about the host-neutral engine crate, which
/// should not have to learn about a host's capabilities to justify refusing
/// its own extension point. The same policies are rejected either way; only
/// where the rejection is noticed moves, from policy construction to the
/// first request against a route that declares one.
fn variance_descriptor(
    runtime: &RenderCacheRuntime,
    request: &Request,
    policy: &RenderCachePolicy,
) -> Result<VarianceDescriptor, RenderCacheError> {
    let mut variance = VarianceDescriptor::new();
    for dimension in policy.vary() {
        let value = match dimension {
            VarianceDimension::Locale => {
                #[cfg(feature = "localization")]
                let value = Lang::locale().as_str();
                // R96: without `localization` there is no negotiated
                // locale and nothing records locale material, so a route
                // declaring `Locale` variance resolves to one fixed public
                // value ("und", BCP 47 for "undetermined") rather than
                // failing to compile. This is honest, not merely
                // convenient: every request resolves the same value, so
                // such a route partitions nothing on this dimension -
                // there is genuinely no locale to vary on without the
                // feature.
                #[cfg(not(feature = "localization"))]
                let value = "und".to_owned();
                DimensionValue::Public(value)
            }
            VarianceDimension::Principal => match Auth::id() {
                Some(id) => DimensionValue::Private(PrivateMaterial::principal(
                    &runtime.keys,
                    &id,
                    FROZEN_PERMISSION_VERSION,
                )),
                None => DimensionValue::Anonymous,
            },
            VarianceDimension::Tenant => match request.live_tenant() {
                Some(id) => DimensionValue::Private(PrivateMaterial::tenant(&runtime.keys, id)),
                None => DimensionValue::Anonymous,
            },
            VarianceDimension::Encoding => {
                // Identity encoding only in this plan; recorded so a later
                // encoding layer cannot collide with an entry published
                // before it existed. See the module doc.
                DimensionValue::Public("identity".to_owned())
            }
            VarianceDimension::Host => match request.http_host() {
                Some(host) => DimensionValue::Public(host),
                None => DimensionValue::Anonymous,
            },
            VarianceDimension::Media => DimensionValue::Public("text/html".to_owned()),
            VarianceDimension::FeatureVersion
            | VarianceDimension::ConfigVersion
            | VarianceDimension::Application(_) => {
                // Fix round 6: this host has no producer for any of the
                // three - see this function's own doc for why the
                // rejection lives here now rather than in the engine's
                // `RenderCachePolicy::validate`.
                return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
            }
        };
        variance.declare(dimension.clone(), value)?;
    }
    Ok(variance)
}

/// Builds the lookup key input for `request` against `policy`. Callers
/// must have already confirmed [`declared_query_ok`].
///
/// # Errors
///
/// Propagates [`variance_descriptor`]'s error - see its own doc.
fn key_input(
    runtime: &RenderCacheRuntime,
    request: &Request,
    pattern: &str,
    policy: &RenderCachePolicy,
    epoch: u64,
) -> Result<RenderKeyInput, RenderCacheError> {
    let declared = policy.query().declared_names();
    let query: BTreeMap<String, String> = request
        .query_params()
        .into_iter()
        .filter(|(name, _)| declared.contains(name))
        .collect();
    let params: BTreeMap<String, String> = request.params().clone().into_iter().collect();
    let host = if policy.vary().contains(&VarianceDimension::Host) {
        request.http_host()
    } else {
        None
    };
    Ok(RenderKeyInput {
        route: route_identity(pattern),
        route_pattern: pattern.to_owned(),
        params,
        query,
        host,
        media: "text/html".to_owned(),
        encoding: None,
        build: runtime.build.clone(),
        epoch,
        variance: variance_descriptor(runtime, request, policy)?,
    })
}

/// Reads a key from L0, then L1 (promoting a decodable L1 hit to L0). A
/// defective entry (decode failure) is evicted from the layer it was found
/// in and treated as a miss on that layer.
///
/// The hot L0 slot is consulted first, for every route: a Complete
/// publication that carried a prepared entry answers with no decode, no
/// integrity hash, and no key allocation at all (see
/// [`suprnova_live::render_cache::hot::HotEntry`]). It is checked against
/// the same misplacement guard as a decoded entry, below, and a mismatch is
/// evicted and falls through to the decoding path rather than being served.
///
/// So is a misplaced one (final review, F7): an entry that decodes but whose
/// stored header names a different key than the one it was found under. The
/// store derives every path and map slot from the key alone and the entry's
/// own HMAC prevents forgery, so nothing request-derived can steer bytes to
/// the wrong slot; this comparison is defence in depth against a store
/// defect or a second writer sharing the key ring and directory, and it
/// costs one comparison per hit.
///
/// Both kinds are returned: a Composite entry is a hit like any other, and
/// what it takes to serve one is [`deliver_hit`]'s concern, not this
/// function's.
async fn lookup(
    runtime: &RenderCacheRuntime,
    policy: &RenderCachePolicy,
    key: &RenderKey,
) -> Result<Option<FoundEntry>, ()> {
    if let Some(hot) = runtime.l0.hot_get(key) {
        if hot.entry().header().key == *key {
            return Ok(Some(FoundEntry::Hot(hot)));
        }
        // Misplaced, exactly as below: evicted and never served. This drops
        // the stored bytes with the hot slot, since the two are one
        // publication in one map slot, so the decoding path below finds the
        // same nothing.
        let _ = runtime.l0.evict(key).await;
    }
    let l0_stored = runtime.l0.get(key).await.map_err(|_| ())?;
    if let Some(stored) = l0_stored {
        match decode(&stored.bytes, &runtime.keys, &runtime.limits) {
            Ok(entry) if entry.header().key == *key => {
                return Ok(Some(FoundEntry::Decoded(Box::new(DecodedHit {
                    entry,
                    stored,
                    layer: Layer::L0,
                }))));
            }
            // Defective (`Err`) or misplaced (`Ok` under another key): the
            // same treatment either way.
            Ok(_) | Err(_) => {
                let _ = runtime.l0.evict(key).await;
            }
        }
    }
    if let Some(l1) = &runtime.l1 {
        let l1_stored = l1.get(key).await.map_err(|_| ())?;
        if let Some(stored) = l1_stored {
            match decode(&stored.bytes, &runtime.keys, &runtime.limits) {
                Ok(entry) if entry.header().key == *key => {
                    promote_to_l0(runtime, policy, key, &entry, &stored).await;
                    return Ok(Some(FoundEntry::Decoded(Box::new(DecodedHit {
                        entry,
                        stored,
                        layer: Layer::L1,
                    }))));
                }
                // Defective or misplaced, as for L0 above; a misplaced L1
                // entry is never promoted.
                Ok(_) | Err(_) => {
                    let _ = l1.evict(key).await;
                }
            }
        }
    }
    Ok(None)
}

/// Promotes a decoded L1 hit into L0 under the fence and publication instant
/// it already carries.
///
/// A Complete entry is promoted hot, prepared from the frame that just
/// decoded - the same rule the lead publication follows (see
/// [`store_entry`]), so a promoted entry and a freshly published one are
/// indistinguishable to the next request. A Composite entry, or a Complete
/// one whose stored header values cannot be formed into HTTP headers, is
/// promoted as plain bytes and decoded again on the next hit.
///
/// Never fails the request: a promotion that does not land only means the
/// next request reads L1 again.
async fn promote_to_l0(
    runtime: &RenderCacheRuntime,
    policy: &RenderCachePolicy,
    key: &RenderKey,
    entry: &DecodedEntry,
    stored: &StoredEntry,
) {
    if let DecodedEntry::Complete(complete) = entry {
        match HotEntry::prepare(
            complete.clone(),
            policy.shared(),
            &policy.freshness(),
            stored.published_at_ms,
            stored.fence,
        ) {
            Ok(hot) => {
                runtime.l0.publish_hot(
                    key,
                    stored.bytes.clone(),
                    Arc::new(hot),
                    stored.fence,
                    stored.published_at_ms,
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    target: "suprnova::render_cache",
                    kind = %error,
                    "an L1 entry could not be prepared for hot service; \
                     promoting it as stored bytes instead",
                );
            }
        }
    }
    // L0 has no age-based expiry of its own (see
    // `MemoryRenderStore::publish`'s own doc); `u64::MAX` is the trait's
    // documented "never age-swept" value, never `0`, which is an ordinary,
    // honoured retention rather than a sentinel.
    let _ = runtime
        .l0
        .publish(
            key,
            stored.bytes.clone(),
            stored.fence,
            stored.published_at_ms,
            u64::MAX,
        )
        .await;
}

/// Checks a stored entry's coherence against the current authority.
/// [`CoherenceMode::Lease`] trusts a locally-granted, still-valid lease
/// instead of rereading; any other case rereads the authority and, under
/// `Lease`, grants a fresh lease on a coherent result.
async fn coherence(
    runtime: &RenderCacheRuntime,
    key: &RenderKey,
    policy: &RenderCachePolicy,
    header: &EntryHeader,
) -> Result<Coherence, ()> {
    if let CoherenceMode::Lease { max_age_ms } = policy.coherence() {
        let now = runtime.now_ms();
        let leased = runtime
            .leases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .is_some_and(|lease| lease.valid_at(now));
        // Fix round 2, item 6, restated for the leased epoch (task 5b). A
        // valid lease reports Coherent without consulting the authority at
        // all: not for the observed generations, and not for the epoch
        // either. That is what a lease is for. Spec 18 ("Local validation
        // leases and invalidation hints", lines 112-131) says leases exist
        // "to avoid querying authority on every hot hit" and that a hot node
        // holding one serves "without repeated authority reads", and
        // `00-overview.md:330` budgets a Complete L0 hit with no
        // database or provider round trip at all - which the previous
        // version of this code could not honour, because `key_input` read
        // `GenerationLedger::epoch` on every dispatch before `coherence`
        // ever ran, so every request to a cached route cost one statement
        // before it knew it was a hit.
        //
        // The epoch is therefore leased with the generations, and what a
        // valid lease trusts about it is bounded by the same `max_age_ms`
        // spec 18 already bounds staleness by. An epoch advance reaches a
        // request by one of three paths, none of them a per-request read:
        //
        // - Advanced on this node, through `RenderCache::advance_epoch`:
        //   that operator lever drops the runtime's leased epoch beside its
        //   L0 clear, so the very next request reads the authority once,
        //   derives its key under the new epoch, and misses - immediately,
        //   lease mode included. That is spec 18's "Global epoch bump
        //   provides a bounded emergency invalidation path" (line 253), and
        //   dropping the lease is what keeps the bound at one request
        //   rather than at `max_age_ms`. It is the guarantee
        //   `a_lease_mode_route_trusts_within_the_window_but_still_catches_an_epoch_bump`
        //   asserts, and it now discriminates.
        // - Advanced on another node, lease-mode route: this node finds out
        //   at the first reread after the lease expires, at most
        //   `max_age_ms` later. The reread below reports the new epoch and
        //   `CoherenceCheck::compare` reports `Moved` against the entry's
        //   own epoch.
        // - Advanced on another node, authority-mode route: the same
        //   comparison, at that route's very next hit, since it rereads on
        //   every hit anyway.
        //
        // What `serve` does with that `Moved` result is the route's own
        // freshness policy's business, not this function's, and it is not
        // always a foreground rebuild. `freshness_state` floors a moved
        // entry's effective age at `fresh_ms`, so a policy with a zero
        // stale-servable window lands on `Dead` and rebuilds in the
        // foreground; a policy with a non-zero one lands on `StaleServable`,
        // and the first request per key after the advance is served the
        // stored bytes once under a `Warning` header while a background
        // rebuild runs. Both re-derive the key under the refreshed epoch
        // before rebuilding (`RenderJob::restamp`), and both are bounded:
        // the stale window is the bound in the second case. See
        // `an_epoch_advanced_by_another_node_serves_a_stale_servable_entry_once_then_rebuilds`.
        //
        // The epoch half of `CoherenceCheck::compare` is consequently
        // reachable on a *found* entry now, which it was not while the key
        // was derived from a freshly read epoch on every dispatch: the
        // request and the entry can name the same key while the authority
        // has moved past both. That is the condition the fix round 2 review
        // asked about, and it is handled here rather than documented away.
        if leased {
            return Ok(Coherence::Coherent);
        }
        let result = authority_coherence(runtime, header).await?;
        if result == Coherence::Coherent {
            let mut leases = runtime.leases.lock().unwrap_or_else(|e| e.into_inner());
            // Fix round 2, item 6: opportunistic cleanup on every insert
            // bounds this map to distinct lease-mode keys requested within
            // the last `max_age_ms`, rather than every key ever seen for
            // the life of the process - see the field's own doc.
            leases.retain(|_, existing| existing.valid_at(now));
            leases.insert(key.clone(), ValidationLease::grant(now, max_age_ms));
        }
        return Ok(result);
    }
    authority_coherence(runtime, header).await
}

async fn authority_coherence(
    runtime: &RenderCacheRuntime,
    header: &EntryHeader,
) -> Result<Coherence, ()> {
    let digests = header.observed.digests();
    // One statement, not two: the generation set and the authority epoch are
    // read together (see
    // [`GenerationLedger::current_with_epoch`]), so a hit that has to consult
    // the authority costs one round trip rather than two.
    let (current, epoch) = runtime
        .ledger
        .current_with_epoch(&digests)
        .await
        .map_err(|_| ())?;
    // The one authority read a hit may make also renews the epoch lease, so
    // no request ever reads the epoch on its own (task 5b). Done before the
    // comparison, not after: this is the value the comparison judges by.
    runtime.epoch_cache.refresh(epoch);
    Ok(
        match CoherenceCheck::compare(&header.observed, &current, epoch, header.epoch) {
            CoherenceCheck::Coherent => Coherence::Coherent,
            CoherenceCheck::Moved(_) => Coherence::Moved,
        },
    )
}

/// Resolves an entry's servable state. A moved entry is never reported
/// `Fresh`: its data-level coherence has already failed, so at best it can
/// be served under the stale windows the same way a time-expired entry
/// can, never as if nothing happened. This is implemented by evaluating
/// freshness against an effective age that is never less than `fresh_ms`
/// when the entry is not coherent, so a moved entry that is still
/// time-fresh is evaluated exactly as if it had just gone stale.
fn freshness_state(
    policy: &RenderCachePolicy,
    coherence: Coherence,
    class: RepresentationClass,
    published_at_ms: u64,
    now_ms: u64,
    seed_deadline_ms: Option<u64>,
) -> FreshnessState {
    if coherence == Coherence::Coherent {
        return evaluate_freshness(
            &policy.freshness(),
            class,
            published_at_ms,
            now_ms,
            seed_deadline_ms,
        );
    }
    let age = now_ms.saturating_sub(published_at_ms);
    let effective_age = age.max(policy.freshness().fresh_ms());
    let synthetic_now = published_at_ms.saturating_add(effective_age);
    evaluate_freshness(
        &policy.freshness(),
        class,
        published_at_ms,
        synthetic_now,
        seed_deadline_ms,
    )
}

/// Whether `policy` declares the one class whose gate has to run again on
/// every hit, so a hit is never answered by this global middleware alone.
fn is_stitched(policy: &RenderCachePolicy) -> bool {
    policy.class() == RepresentationClass::PublicShellStitched
}

/// Answers a hit, or hands it to the route chain when the route is stitched.
///
/// A stitched route is never answered here. Its representation is a shared
/// *shell*, not a finished answer: the route's own authorization guard,
/// tenant middleware, and anything else it declared have to run on the hit
/// exactly as they run on a miss, and only then may the shell be served -
/// or, for a Composite entry, assembled from islands re-mounted for this
/// request. So the entry is attached to the request (see
/// [`Request::attach_prepared_hit`]) and the chain runs; the Live completion
/// middleware, the last middleware before the handler, serves it through
/// [`super::stitch::serve_prepared`]. A chain that refuses the request first
/// drops the hit unread, which is the point.
///
/// This is unconditional, and deliberately so: nothing here checks that a
/// consumer exists downstream. A route that declares the stitched class
/// without ending in the Live completion middleware simply drops every
/// prepared hit and renders, which makes the cache a permanent no-op there.
/// The alternative - answering such a hit here because nobody else will -
/// is the exact short circuit this class exists to forbid, and a stitched
/// shell is classified with its gate reads exempt, so serving one past an
/// authorization guard that never ran would hand a shared shell to a
/// visitor the guard would have refused. An ineffective cache is the safe
/// side of that trade.
///
/// The same fact constrains what a stitched route's chain may do to the
/// body. Route middleware that rewrites the response after the Live document
/// rendered it runs again on every hit, so its output would be stored on the
/// miss and applied a second time on the hit, leaving the entry's `ETag`
/// describing bytes no client received. Nothing here has to detect that: the
/// document records a SHA-256 of the body it rendered, and the composite
/// publisher declines any stitched document whose response body digest
/// differs from that recording, zero-island documents included. A route that
/// rewrites its body under this class is therefore never published and is
/// served uncached on every request.
///
/// Every other class keeps the behavior it has always had: the stored
/// representation is a finished answer and is served right here. A Composite
/// entry on such a route can only be a store defect - [`decode`] refuses a
/// Composite entry under any class but the stitched one - so it is counted
/// as a miss and the route renders.
async fn deliver_hit(
    runtime: &RenderCacheRuntime,
    mut request: Request,
    next: Next,
    policy: &RenderCachePolicy,
    found: FoundEntry,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Response {
    if !is_stitched(policy) {
        return match hit_response(
            runtime,
            &found,
            request.method(),
            request.header("if-none-match"),
            policy,
            now_ms,
            warning,
        ) {
            Some(response) => Ok(response),
            None => {
                LookupOutcome::Miss.record();
                next(request).await
            }
        };
    }
    request.attach_prepared_hit(Box::new(super::stitch::PreparedHit {
        entry: found,
        policy: policy.clone(),
        now_ms,
        warning,
    }));
    next(request).await
}

/// Forms the served response for a non-stitched hit, through the engine's
/// one response builder either way.
///
/// A hot entry replays values formed once at publication
/// ([`serve_hot`]); a decoded Complete entry forms them now
/// ([`complete_response`]). Both end in the same builder inside the engine,
/// so a hot hit and a decoded one cannot drift apart in status, header set,
/// body treatment, or the 304 decision.
///
/// `None` means there is nothing servable here and the caller falls through
/// to a render: a Composite entry on a route that never declared stitching
/// can only be a store defect ([`decode`] refuses a Composite entry under
/// any other class), and so is a Complete entry whose stored values cannot
/// be formed into a valid response.
fn hit_response(
    runtime: &RenderCacheRuntime,
    found: &FoundEntry,
    method: &hyper::Method,
    if_none_match: Option<&str>,
    policy: &RenderCachePolicy,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Option<HttpResponse> {
    match found {
        FoundEntry::Hot(hot) => Some(hot_response(
            runtime,
            hot,
            method,
            if_none_match,
            now_ms,
            warning,
        )),
        FoundEntry::Decoded(hit) => match &hit.entry {
            DecodedEntry::Complete(entry) => complete_response(
                method,
                if_none_match,
                policy,
                entry,
                hit.stored.published_at_ms,
                now_ms,
                warning,
            ),
            DecodedEntry::Composite(_) => None,
        },
    }
}

/// Forms a hot hit's response: the engine replays the values it formed when
/// the entry was published and hands back the stored body itself, shared
/// rather than copied, and this adopts that response into the framework's
/// own container without touching either.
pub(crate) fn hot_response(
    runtime: &RenderCacheRuntime,
    hot: &HotEntry,
    method: &hyper::Method,
    if_none_match: Option<&str>,
    now_ms: u64,
    warning: Option<&'static str>,
) -> HttpResponse {
    runtime.count_hot_serve();
    HttpResponse::from_engine_response(serve_hot(
        hot,
        HotRequest {
            method,
            if_none_match,
            now_ms,
        },
        warning,
    ))
}

/// Forms a decoded Complete entry's response through the engine's
/// [`respond`]: a 304 when `if_none_match` matches, the full representation
/// otherwise, body-free for `HEAD`, carrying `ETag`, `Cache-Control`,
/// `Vary`, `Age`, and `Warning` (when stale). The engine owns every one of
/// those decisions; nothing here re-forms a header of its own.
///
/// Takes the request's method and `If-None-Match` value rather than the
/// `Request` itself, because a stale-on-error fallback's request has
/// already been consumed by the failed rebuild by the time this runs - see
/// `serve`, which captures both before that attempt.
///
/// `None` when the stored entry cannot be formed into a valid response at
/// all. Every value a candidate publishes under was already bounded and
/// checked at publication (see `entry_header` and [`SafeHeaders`]), so this
/// is a store defect rather than a reachable input; each caller falls
/// through to a render rather than serving something malformed.
pub(crate) fn complete_response(
    method: &hyper::Method,
    if_none_match: Option<&str>,
    policy: &RenderCachePolicy,
    entry: &CompleteEntry,
    published_at_ms: u64,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Option<HttpResponse> {
    let header = entry.header();
    let freshness = policy.freshness();
    let formed = respond(
        ResponseParts {
            status: header.status,
            class: header.class,
            shared: policy.shared(),
            freshness: &freshness,
            headers: &header.headers,
            variance: &header.variance,
            validator: entry.validator(),
            body: entry.body(),
            published_at_ms,
            seed_deadline_ms: header.seed_deadline_ms,
            cache_control_override: None,
        },
        HotRequest {
            method,
            if_none_match,
            now_ms,
        },
        warning,
    );
    match formed {
        Ok(response) => Some(HttpResponse::from_engine_response(response)),
        Err(error) => {
            tracing::warn!(
                target: "suprnova::render_cache",
                kind = %error,
                "a stored entry could not be formed into a response; it was not served",
            );
            None
        }
    }
}

/// Admits a rebuild for `key` and either leads it (rendering and, if
/// eligible and still coherent, publishing), reuses a leader's completed
/// publication after waiting, or renders without publishing (`Wait`
/// exhausted, or `Bypass`).
/// One render's fixed identity: the lookup key, the epoch it was derived
/// and admitted under, and the declared variance it carries. Bundled so
/// `lead_render` and `publish` stay within a reasonable argument count
/// rather than threading each field through separately.
///
/// The key and the epoch are not two independent fields. `RenderKey::derive`
/// bakes the epoch into the key (tag 10 in
/// `suprnova_live::render_cache::key`), so a job whose `epoch` says one
/// thing and whose `key` was derived under another names a slot no later
/// request can find, publishes a header whose `epoch` disagrees with the key
/// it is stored at, and fences an L1 file under an epoch its own sweep will
/// judge by. That is why this type owns the [`RenderKeyInput`] the key came
/// from and exposes the epoch through it: refreshing the epoch and
/// re-deriving the key is one operation, [`Self::restamp`], and there is no
/// way to do half of it.
pub(crate) struct RenderJob {
    /// Always `RenderKey::derive(&input, keys)` for the current `input`.
    key: RenderKey,
    /// The input `key` was derived from, epoch included.
    input: RenderKeyInput,
}

impl RenderJob {
    /// Bundles an already-derived key with the input it was derived from.
    /// The caller has just called `RenderKey::derive(&input, keys)`, which
    /// is what makes the two agree at construction.
    fn new(input: RenderKeyInput, key: RenderKey) -> Self {
        Self { key, input }
    }

    /// The lookup key this render publishes under.
    fn key(&self) -> &RenderKey {
        &self.key
    }

    /// The authority epoch this render is judged against.
    fn epoch(&self) -> u64 {
        self.input.epoch
    }

    /// The declared variance the key carries.
    fn variance(&self) -> &VarianceDescriptor {
        &self.input.variance
    }

    /// Brings this job up to the runtime's leased epoch, re-deriving the key
    /// under it when it moved.
    ///
    /// A no-op in the ordinary case: the leased epoch is the one the key was
    /// derived under, because `serve` derived it from that same lease. It
    /// does work exactly when an authority read since then reported a
    /// different epoch - another node advanced it - and in that case the key
    /// must move with it, or this render would publish into the previous
    /// epoch's namespace.
    ///
    /// Proven by revert: with the `serve` call to this function disabled,
    /// both `an_epoch_advanced_by_another_node_reaches_a_lease_mode_route_when_its_lease_expires`
    /// and `..._reaches_an_authority_mode_route_on_its_next_hit` read
    /// `renders() == 3` at their final assertion instead of `2` - the
    /// rebuild published where nothing would look for it, so the next
    /// request rendered again.
    ///
    /// # Errors
    ///
    /// Propagates `RenderKey::derive`'s error. Deriving the same input again
    /// under a different `u64` cannot newly exceed any of that function's
    /// bounds (none of them involve the epoch), so this is a fail-closed
    /// path rather than a reachable one; callers treat it as a bypass.
    fn restamp(&mut self, runtime: &RenderCacheRuntime) -> Result<(), RenderCacheError> {
        let Some(epoch) = runtime.epoch_cache.get() else {
            // The lease was dropped underneath this request by
            // `RenderCache::advance_epoch`. There is no epoch to re-derive
            // under without an authority read this function does not make;
            // the render proceeds under the epoch it captured, and its own
            // fresh reread declines the candidate - the same outcome the
            // `EPOCH_CAPTURED` race seam proves.
            return Ok(());
        };
        if epoch == self.input.epoch {
            return Ok(());
        }
        self.input.epoch = epoch;
        self.key = RenderKey::derive(&self.input, &runtime.keys)?;
        Ok(())
    }
}

/// The stale response a failed rebuild falls back to, under `Warning`, or
/// `None` when the rebuild did not fail or this route and this entry have
/// nothing to fall back to. Serving it is what a `stale_on_error_ms` window
/// buys.
///
/// The one place that judgement lives, shared by the two paths a request can
/// reach a `StaleOnError` entry with a rebuild to make: the primary hit path
/// in [`RenderCacheMiddleware::serve`], and a waiter in
/// [`render_and_publish`] whose re-evaluation after the wait lands on such
/// an entry. Ruling R18: the waiter path used to rebuild with no fallback at
/// all, so a client that queued behind a leader and then re-evaluated onto a
/// stale-on-error entry received its own rebuild's failure while a request
/// that had arrived on that entry was served the stale bytes. One helper is
/// what keeps the two answers the same.
///
/// Stale-on-error exists for a foreground rebuild that fails, not only for a
/// provider failure before the handler ran: a handler that itself returns an
/// error or a 5xx status is an ordinary `Response` to
/// [`render_and_publish`], so that case is detected here too rather than
/// passed through (fix round 2, item 3).
///
/// Two exclusions:
///
/// - A **stitched** route has no stale-on-error fallback. Serving the stored
///   shell here would answer the request with a representation the route's
///   own chain never got to gate on this time round, which is precisely what
///   that class exists to prevent.
/// - A **Composite** entry on a route that did not declare stitching, and a
///   stored entry that cannot be formed into a valid response, leave nothing
///   to fall back to. [`hit_response`] reports both as `None`.
///
/// In both cases the caller returns the failed rebuild's own outcome, which
/// is what the client sees.
fn stale_on_error_fallback(
    runtime: &RenderCacheRuntime,
    policy: &RenderCachePolicy,
    found: &FoundEntry,
    outcome: &Result<Response, ProviderFailure>,
    method: &hyper::Method,
    if_none_match: Option<&str>,
    now: u64,
) -> Option<HttpResponse> {
    let rebuild_failed = match outcome {
        Ok(response) => {
            let status = match response {
                Ok(http) | Err(http) => http.status_code(),
            };
            status >= 500
        }
        Err(ProviderFailure(..)) => true,
    };
    if !rebuild_failed || is_stitched(policy) {
        return None;
    }
    hit_response(
        runtime,
        found,
        method,
        if_none_match,
        policy,
        now,
        warning_header(FreshnessState::StaleOnError),
    )
}

async fn render_and_publish(
    runtime: &Arc<RenderCacheRuntime>,
    request: Request,
    next: Next,
    policy: &RenderCachePolicy,
    mut job: RenderJob,
    depth: u32,
) -> Result<Response, ProviderFailure> {
    let now = runtime.now_ms();
    let admission = match runtime.coordinator.admit(job.key(), job.epoch(), now).await {
        Ok(admission) => admission,
        Err(_) => return Err(ProviderFailure(request, next)),
    };
    match admission {
        RebuildAdmission::Lead(lease) => {
            Ok(lead_render(runtime, request, next, *lease, policy, job).await)
        }
        RebuildAdmission::Wait(wait) => {
            wait.wait().await;
            match lookup(runtime, policy, job.key()).await {
                Ok(Some(found)) => {
                    // Fix round 1, item 4: the leader may have declined to
                    // publish (a moved dependency, an ineligible response,
                    // an overflowed report, an uncacheable classification)
                    // or may not have improved on what was already there.
                    // What `lookup` just found is therefore not proven
                    // fresh by virtue of having waited for it - it must
                    // pass the same coherence and freshness evaluation the
                    // primary hit path in `serve` applies to every hit,
                    // never served as if the wait itself were the proof.
                    let coherence_result =
                        match coherence(runtime, job.key(), policy, found.header()).await {
                            Ok(coherence) => coherence,
                            Err(()) => return Err(ProviderFailure(request, next)),
                        };
                    let now = runtime.now_ms();
                    let state = freshness_state(
                        policy,
                        coherence_result,
                        found.header().class,
                        found.published_at_ms(),
                        now,
                        found.header().seed_deadline_ms,
                    );
                    match state {
                        FreshnessState::Fresh => {
                            (match found.layer() {
                                Layer::L0 => LookupOutcome::L0Hit,
                                Layer::L1 => LookupOutcome::L1Hit,
                            })
                            .record();
                            Ok(deliver_hit(runtime, request, next, policy, found, now, None).await)
                        }
                        FreshnessState::StaleServable => {
                            LookupOutcome::Stale.record();
                            Ok(deliver_hit(
                                runtime,
                                request,
                                next,
                                policy,
                                found,
                                now,
                                warning_header(state),
                            )
                            .await)
                        }
                        // Neither state is safe to serve as a plain hit,
                        // so both rebuild. They part company afterwards:
                        // ruling R18, a `StaleOnError` entry falls back to
                        // its own stale bytes when that rebuild fails,
                        // through the same helper the primary hit path uses,
                        // so a waiter behind a failed leader is answered the
                        // way a request that had arrived a moment earlier
                        // would have been. A `Dead` entry has no window left
                        // to fall back into.
                        FreshnessState::StaleOnError | FreshnessState::Dead => {
                            LookupOutcome::Miss.record();
                            // Fix round 3, item 5: bounds a sustained herd
                            // against a route that never successfully
                            // publishes (every render sets a cookie, say) -
                            // without this, each leader cycle that fails to
                            // publish adds one nesting level to every waiter
                            // still recursing behind it. Past the bound,
                            // render without publishing rather than
                            // recursing again.
                            if depth >= MAX_WAIT_REBUILD_DEPTH {
                                return Ok(next(request).await);
                            }
                            // Re-admission under the current epoch. The
                            // `coherence` call a few lines above renewed the
                            // lease if it consulted the authority, so this
                            // reads the lease rather than the ledger; only a
                            // lease dropped underneath this request (an
                            // `advance_epoch` on this node) costs a read.
                            //
                            // Task 5b: this used to re-stamp `job.epoch`
                            // alone, leaving `job.key` derived under the
                            // epoch the request started with - a
                            // disagreement that published the rebuilt entry
                            // into the previous epoch's namespace, under a
                            // header whose `epoch` field named the new one.
                            // `restamp` re-derives the key with the epoch,
                            // which is the only way to change either.
                            if runtime.epoch_cache.get().is_none() {
                                match runtime.ledger.epoch().await {
                                    Ok(epoch) => runtime.epoch_cache.refresh(epoch),
                                    Err(_) => return Err(ProviderFailure(request, next)),
                                }
                            }
                            if job.restamp(runtime).is_err() {
                                return Ok(next(request).await);
                            }
                            // Captured before `request` moves into the
                            // rebuild, for the reason the primary arm's own
                            // capture gives - and only for the one state
                            // that can fall back, so a `Dead` re-admission
                            // pays neither the clone nor the header copy.
                            let fallback = (state == FreshnessState::StaleOnError).then(|| {
                                (
                                    request.method().clone(),
                                    request.header("if-none-match").map(str::to_owned),
                                )
                            });
                            let outcome = Box::pin(render_and_publish(
                                runtime,
                                request,
                                next,
                                policy,
                                job,
                                depth + 1,
                            ))
                            .await;
                            let Some((method, if_none_match)) = fallback else {
                                return outcome;
                            };
                            match stale_on_error_fallback(
                                runtime,
                                policy,
                                &found,
                                &outcome,
                                &method,
                                if_none_match.as_deref(),
                                now,
                            ) {
                                Some(response) => {
                                    LookupOutcome::Stale.record();
                                    Ok(Ok(response))
                                }
                                None => outcome,
                            }
                        }
                    }
                }
                _ => Ok(next(request).await),
            }
        }
        RebuildAdmission::Bypass => {
            LookupOutcome::Bypass.record();
            Ok(next(request).await)
        }
    }
}

/// Runs the render under the request-scoped collector - inside a read
/// transaction when a database is configured, so the render's own reads
/// and the generation reads at window-close share one snapshot - then
/// decides eligibility, classification, and (if still coherent against a
/// fresh reread) publication. Never fails outright: any provider issue
/// past this point is logged in effect by simply not caching, and the
/// render's own response is still served.
async fn lead_render(
    runtime: &Arc<RenderCacheRuntime>,
    request: Request,
    next: Next,
    lease: RebuildLease,
    policy: &RenderCachePolicy,
    job: RenderJob,
) -> Response {
    // Test-only race seam (R72/R83): fires before `run_render` opens the
    // render's consistent read view, so a write armed here has already
    // committed when that view opens and the render reads it - the one
    // arrival near a render that must publish rather than discard.
    #[cfg(any(test, feature = "testing"))]
    race_points::fire(&race_points::BEFORE_VIEW).await;
    let method = request.method().as_str().to_owned();
    let if_none_match = request.header("if-none-match").map(str::to_owned);
    let (response, report, observed) = run_render(
        runtime,
        request,
        next,
        job.epoch(),
        key_carries_a_resolved_principal(job.variance()),
        policy.class() == RepresentationClass::PublicShellStitched,
    )
    .await;
    // Test-only race seam (R72/R83): fires the instant the read view has
    // closed and the observed generation set is fixed, before anything
    // judges it. A write armed here is invisible to the render and visible
    // to the fresh reread below, which discards the candidate.
    #[cfg(any(test, feature = "testing"))]
    race_points::fire(&race_points::AFTER_VIEW_CLOSE).await;
    let Ok(response) = response else {
        let _ = runtime.coordinator.release(lease).await;
        return response;
    };
    let Some(observed) = observed else {
        // The report overflowed (ruling R55: an incomplete dependency set
        // is never storable), the in-transaction ledger read itself
        // failed, or this is a stitched route whose handler never began
        // and whose content bucket is therefore empty (see
        // [`render_under_collector`]); in every case there is nothing
        // safe to publish or to compare against later, so this candidate
        // is declined here rather than carrying a stand-in forward.
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    };

    let signals = response_signals(&response, &method);
    let eligibility = policy.eligibility(&signals);
    let Eligibility::Store(_) = eligibility else {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    };
    // Fix round 4, Leak B: classification is driven by what the collector
    // observed, never by re-reading an accessor - the previous version
    // re-read `Auth::id()` here, which is the *default guard's* slot
    // specifically, and had the observation vetoed whenever identity was
    // resolved through any other accessor. `classify` only ever tests
    // `.is_some()` on these fields, never the value itself, so which member
    // of the observed set stands in here is immaterial - the guard below is
    // what actually compares values, against every member of the set, not
    // just this one. Fix round 6: `principal_material`/`tenant_material`
    // are sets, not a single slot (see their own doc); any member serves
    // equally well here, falling back to a sentinel only when the reason
    // fired through a boolean-only read (`has_current_user`, say, when it
    // could not itself resolve an id) that recorded nothing concrete.
    const SENTINEL_OBSERVED_LABEL: &str = "observed";
    let observed_context = ObservedContext {
        principal: report.context.principal_read.then(|| {
            let id = report
                .context
                .principal_material
                .iter()
                .next()
                .map_or(SENTINEL_OBSERVED_LABEL, String::as_str);
            PrivateMaterial::principal(&runtime.keys, id, FROZEN_PERMISSION_VERSION)
        }),
        // Fix round 4: previously hard-coded `None` because nothing
        // produced this observation. `Request::live_tenant()` now records
        // one on every call (see its own doc).
        tenant: report.context.tenant_read.then(|| {
            let id = report
                .context
                .tenant_material
                .iter()
                .next()
                .map_or(SENTINEL_OBSERVED_LABEL, String::as_str);
            PrivateMaterial::tenant(&runtime.keys, id)
        }),
        session_read: report.context.session_read,
        authorization_read: report.context.authorization_read,
        secret_context_read: report.context.secret_context_read,
        undeclared_reads: report.undeclared.clone(),
    };
    let classification = classify(policy.class(), &observed_context);
    if classification.class == RepresentationClass::Uncacheable {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    }
    // The rendered Live document's own facts (if any) decline independently
    // of `classify`: an identity-bound island on a route that did not
    // declare stitching, a `NoStore` document intent, or a public-seed
    // island without a resolvable deadline. This can only decline, never
    // narrow or widen `classification.class` (see `document_declines`'s own
    // doc for why the document's cache intent does not feed classification
    // at all, and why the *declared* class is what it is passed).
    if live::document_declines(report.live_document.as_ref(), policy.class()) {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    }
    // The invariant `key_used_different_values_than_the_render_saw` relies
    // on without stating it: every requirement it checks is driven off
    // `classification.reasons`, so a `PrivateCached` class with an empty
    // reasons list has nothing there for it to check the resolved key
    // against, and the guard's loop simply never runs. See
    // `is_unreasoned_private_class`'s own doc for how this is reachable,
    // and for why a route that *declared* `PrivateCached` is excluded.
    //
    // R90: this check runs against a *copy*, never the real
    // `classification` - see `strip_classification_reasons_for_test`'s own
    // doc for why the test-only seam that copy exists for must never touch
    // the value passed to the value guard or to `entry_header` below.
    let classification_for_invariant = classification.clone();
    // Test-only, see `strip_classification_reasons_for_test`'s own doc: no
    // production code ever sets this flag, and `classify` never produces a
    // narrowed, reason-less class on its own, so this is a no-op on every
    // real request. Read from `report` (already extracted from the
    // collector by `run_render`, above), not the collector itself: the
    // scope that flag was set in has already closed by this point.
    #[cfg(any(test, feature = "testing"))]
    let classification_for_invariant = {
        let mut classification_for_invariant = classification_for_invariant;
        if report.strip_classification_reasons {
            classification_for_invariant.reasons.clear();
        }
        classification_for_invariant
    };
    if is_unreasoned_private_class(&classification_for_invariant, policy.class()) {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    }
    if key_used_different_values_than_the_render_saw(&job, &classification, &report, runtime) {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    }

    if let Err(()) = fresh_reread_is_coherent(runtime, &observed, job.epoch()).await {
        LookupOutcome::Moved.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    }

    let now = runtime.now_ms();
    let seed_deadline_ms = report
        .live_document
        .as_ref()
        .and_then(|facts| facts.seed_deadline_ms);
    let seed_remaining = report
        .live_document
        .as_ref()
        .and_then(|facts| live::seed_remaining_ms(facts, now));
    if seed_remaining == Some(0) {
        // The seed's own promotion deadline was reached between the render
        // starting and this point; publishing it now would store an entry
        // that is already dead on arrival.
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    }
    let Some(header) = entry_header(
        &job,
        policy,
        classification.class,
        &observed,
        &response,
        now,
        seed_deadline_ms,
    ) else {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    };
    // A stitched route that rendered a Live document publishes through the
    // composite publisher, which decides between a Composite entry and a
    // Complete shell only after every check that makes a shared shell safe
    // has passed, and declines the render outright when one has not (see
    // `stitch::build_composite_entry`). Everything else - every other
    // class, and a stitched route whose render mounted no Live document at
    // all and so recorded nothing to check - publishes the response's own
    // bytes as a Complete representation, exactly as before.
    let published = match (is_stitched(policy), report.live_document.as_ref()) {
        (true, Some(facts)) => stitch::build_composite_entry(header, response.body(), facts),
        _ => Some(DecodedEntry::Complete(CompleteEntry::new(
            header,
            Bytes::copy_from_slice(response.body()),
        ))),
    };
    let Some(entry) = published else {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    };
    store_entry(runtime, &lease, policy, &job, &entry, &observed, now).await;
    let _ = runtime.coordinator.release(lease).await;
    // The validator this client is given describes the bytes this client is
    // actually sent, which for a Composite publication is the leader's own
    // rendered document rather than anything reconstructed from the shell.
    let validator = match &entry {
        DecodedEntry::Complete(entry) => *entry.validator(),
        DecodedEntry::Composite(_) => Validator::strong_for(response.body()),
    };
    // The leader's own rendered document of a slotted stitched route holds
    // that leader's islands, mounted under authority derived for this one
    // request, exactly as every later assembly of the same shell does. The
    // directive follows what the bytes contain, not which code path produced
    // them, so both ask the one helper that decides it. A Complete entry,
    // and a Composite with no slots at all, keep the class's computed value.
    let cache_control_override = match &entry {
        DecodedEntry::Complete(_) => None,
        DecodedEntry::Composite(composite) => stitch::cache_control_override_for(composite),
    };
    // The client that triggered this render gets its own response back -
    // only the cache validators this middleware adds are attached, rather
    // than a response reconstructed from the stored entry. Reconstructing
    // is unavoidable for a later hit (the original response object no
    // longer exists by then), but here it is gratuitous: the entry's
    // headers are already filtered to the small replayable allowlist (see
    // `entry_header`), so reconstructing on the render itself would silently
    // drop any handler-set header outside that allowlist even on the very
    // request that produced it. See fix round 2, item 2.
    finish_fresh_render(
        response,
        if_none_match.as_deref(),
        policy,
        entry.header(),
        &validator,
        cache_control_override,
    )
}

/// The fresh-render counterpart of [`complete_response`]: serves the
/// handler's own response, untouched, with the cache validators (`ETag`,
/// `Cache-Control`, `Vary`, `Age`) attached - or a body-free 304 when the
/// request's `If-None-Match` already matches what was just rendered. Unlike
/// `complete_response`, this never reconstructs the body or the
/// non-validator headers from `entry`: for the render that produced `entry`,
/// the handler's own response is the authoritative one. `replace_header` is
/// used for each validator so a value the handler already set (a
/// `Cache-Control` of its own, say) is superseded rather than duplicated.
/// Age is always `0`: this response and the entry were published from the same
/// instant. Body suppression for `HEAD` is not this function's job - the
/// server strips the body for `HEAD` regardless (see fix round 2, item 7),
/// the same way it would for any handler's response with no cache in play.
///
/// `validator` is passed rather than read off the entry because a Composite
/// publication has no stored bytes to validate: its entry holds a shell with
/// holes, and this client is being sent the leader's own fully rendered
/// document. The caller therefore passes a strong validator over exactly
/// those bytes. A later hit on the same entry assembles a document from
/// islands re-rendered for *that* request, under a nonce minted for it, and
/// so legitimately carries a different validator - the two describe
/// different bytes, and each is strong for the bytes it was sent with.
///
/// `cache_control_override` is the one directive the class's computed value
/// must not be allowed to state, and it exists for the same reason the
/// validator is passed in: a Composite publication's bytes are the leader's
/// own fully rendered document, islands included. A slotted stitched route's
/// document holds one principal's islands under authority re-derived for one
/// request, so nothing may store it - which is what
/// [`stitch::cache_control_override_for`](super::stitch::cache_control_override_for)
/// decides, for this render and for every later assembly of the same entry
/// alike. `None` leaves the class's own value in place, which is what every
/// Complete publication and every zero-slot Composite gets.
fn finish_fresh_render(
    response: HttpResponse,
    if_none_match: Option<&str>,
    policy: &RenderCachePolicy,
    header: &EntryHeader,
    validator: &Validator,
    cache_control_override: Option<&'static str>,
) -> Response {
    let not_modified = matches!(
        evaluate_conditional(if_none_match, validator),
        ConditionalOutcome::NotModified
    );
    let mut out = if not_modified {
        HttpResponse::new().status(304)
    } else {
        response
    };
    out = out.replace_header("ETag", validator.etag());
    // Age is always 0 here (see this function's own doc), so the seed's
    // remaining lifetime at this instant is the deadline minus the very
    // publication time already stored in `header`.
    let seed_remaining = header
        .seed_deadline_ms
        .map(|deadline| deadline.saturating_sub(header.published_at_ms));
    out = out.replace_header(
        "Cache-Control",
        cache_control_override.map_or_else(
            || {
                cache_control_value(
                    header.class,
                    policy.shared(),
                    &policy.freshness(),
                    seed_remaining,
                )
            },
            ToOwned::to_owned,
        ),
    );
    if let Some(vary) = vary_value(&header.variance) {
        out = out.replace_header("Vary", vary);
    }
    out = out.replace_header("Age", "0");
    Ok(out)
}

/// Whether `classification` is a `PrivateCached` class that `classify`
/// genuinely narrowed to from a wider `declared` class (`declared`, which
/// is always `policy.class()`, is not itself `PrivateCached`) with no
/// recorded reason behind the narrowing.
/// `key_used_different_values_than_the_render_saw` drives every
/// requirement it checks off `classification.reasons`, so an empty list
/// gives its loop nothing to check the resolved key against and it returns
/// `false` unconditionally - not a bug in that guard, since its own
/// reasoning never anticipated this shape.
///
/// `declared == PrivateCached` is deliberately excluded (R89): `classify`
/// only ever narrows, so `class == PrivateCached` with empty reasons and
/// `declared == PrivateCached` means nothing narrowed at all - the route
/// simply declared `PrivateCached` up front and its render never happened
/// to read an identity. `RenderCachePolicy` already requires such a route
/// to declare `Principal` or `Tenant` variance to build at all, so the key
/// is already partitioned by the resolved principal before the render
/// begins; declining it would make a route Task 14 cached correctly
/// permanently uncacheable. An earlier version of this check omitted the
/// `declared` comparison and did exactly that (see this task's own fix
/// round 1 report, finding 9).
///
/// With that case excluded, `classify`'s own implementation cannot
/// currently reach the shape this checks for at all: every narrowing call
/// it makes pushes its reason unconditionally, so a `PrivateCached` result
/// with empty reasons only ever arises when `declared` was already
/// `PrivateCached` - which is exactly the case just excluded. This is
/// defense in depth against a future change to `classify` (or another
/// upstream classifier) that narrows without attaching a reason, not a
/// path any current input reaches; see the delivered test that reaches
/// this exact call site anyway, using a test-only seam, precisely because
/// nothing else does.
#[must_use]
fn is_unreasoned_private_class(
    classification: &ClassificationOutcome,
    declared: RepresentationClass,
) -> bool {
    classification.class == RepresentationClass::PrivateCached
        && classification.reasons.is_empty()
        && declared != RepresentationClass::PrivateCached
}

/// Whether the render's own observations diverge from the values the key
/// was already built from, or an observed locale the key does not account
/// for at all.
///
/// # Compare values, not properties (fix round 5); record every value, not
/// the last one (fix round 6)
///
/// Rounds 1, 3, and 4 each reconciled two independently computed things -
/// what the render read and what the key partitioned by - through a proxy:
/// "material is present" (round 1), "the dimension is declared" (round 3),
/// "the named dimension's value has type `Private`" (round 4). Each proxy
/// closed the previous gap and left the next one standing, because none of
/// them checked that the *value* the render saw is the *value* the key was
/// built from - only some property of it. Round 5 replaced the reconciliation
/// with a value comparison, but split the mechanism by dimension: it
/// re-derived `Locale` by calling `Lang::locale()` again after the render,
/// and recorded `Principal`/`Tenant` in a single last-write slot. Both
/// halves of that split were themselves proxies in disguise, and the
/// reviewer broke both, plus found a fourth leak the guard could never see
/// at all:
///
/// - **Re-derivation is unsafe for a scoped task-local**, because the scope
///   has already popped by the time this function runs. A handler that
///   renders inside [`crate::scope_locale`] - the framework's own
///   documented, supported API for a mid-render locale switch - has that
///   nested scope end the instant its future resolves, before this guard
///   ever gets to call `Lang::locale()` again; the re-read then saw the
///   *outer*, pre-switch locale, the same value the key was already built
///   from, and always agreed with it. A second reproduction needed no
///   nested-scope API at all: a per-route locale middleware installed after
///   [`super::RenderCache::install`] (the only position such a middleware
///   can occupy, since `install` appends) sets the task-local before the
///   handler runs and is, itself, scoped no wider than its own
///   `next(request)` call - gone by the time a post-render re-read outside
///   it would look.
/// - **A single last-write slot collapses several observed values and picks
///   one arbitrarily.** A handler that reads a named guard's identity to
///   build the body, then separately touches the default accessor for an
///   unrelated check, recorded only the second value under round 5's
///   `Option<String>` slot - the guard compared *that* against the key and
///   passed, even though the *body* was built from a different, unrecorded
///   value. Proven cross-identity, over real HTTP.
/// - **Leak 1, proven twice, is where round 5's value comparison for
///   `Principal`/`Tenant` itself held** (it is the reads-with-no-value-at-all
///   problem below that is new): because [`super::RenderCache::install`]
///   appends to the global middleware registry, this middleware derives the
///   key before any route middleware runs. A per-route impersonation
///   middleware - which the framework explicitly supports - sets the real
///   identity *after* key derivation, so the key partitions by the
///   impersonator's own identity while the render observes the
///   impersonation target's. A second reproduction needs no impersonation
///   at all: a non-default `SessionGuard`'s identity was, before round 5,
///   invisible to a guard built only on `Auth::id()`.
/// - **A fourth leak observes nothing at all.** A middleware that resolves
///   identity once, before the render, and stashes it where the render
///   reads it *ambiently* - the framework's own feature-flag middleware,
///   whose documented purpose is exactly this - never touches an
///   instrumented accessor during the render, so no reason fires and this
///   guard has nothing to compare. See the fixed read at
///   `crate::features::fields::observe_identity`, called from inside the
///   render this time, not the resolution outside it - and, since fix round
///   7, on the team axis as well as the user one, and only for a flag that
///   actually has a rule at that axis.
///
/// The fix, per the reviewer's diagnosis: **record the set of every value
/// observed for a dimension, and require every member to equal the key's
/// value. Re-derive nothing.** `Lang::locale()` was already an observation
/// point (it emits a `DependencyIdentity::Locale` dependency); it now also
/// records the concrete value at that same call, the same mechanism
/// identity already used, rather than being re-read afterward.
/// `CollectedContext::principal_material`/`tenant_material`/`locale_material`
/// are `BTreeSet<String>`, not a single slot - every accessor that resolves
/// a concrete value inserts into the set, and a dimension with two
/// different observed values in one render fails the comparison against
/// *both*, since the key can only ever equal one of them, which is correct:
/// a render that genuinely saw two different values for the same dimension
/// cannot be represented by any one key.
///
/// For every [`ClassificationOutcome::reasons`] entry, the required
/// dimension's *entire observed set* is compared against the key's material
/// for it - `PrincipalObserved` and `AuthorizationRead` (the decision is
/// per-user, whatever it inspects) require `Principal`; `TenantObserved`
/// requires `Tenant`. `SessionValueRead`, `SecretContextRead`, and
/// `UndeclaredContext` narrow to `Uncacheable` unconditionally inside
/// `classify` (`Uncacheable` is `RepresentationClass`'s maximum variant, so
/// `narrowest` always yields it), which the caller already declines before
/// reaching this guard - asserted here, in debug builds, rather than merely
/// relied upon.
///
/// # The empty-set path, and why both of its arms are safe
///
/// The set for a required dimension is empty when the render asked for that
/// dimension and no concrete value came back, so there is no value to
/// compare and the check falls back to asking what the *key* says. This is
/// the normal path, not a rare one: every anonymous visitor of every route
/// whose render touches `Auth::id()` reaches it, because an anonymous
/// resolution records the read with no material at all. (An earlier draft
/// of this doc called the path "now rare after fix round 6"; the sixth
/// review measured otherwise, and fix round 7 corrects both the claim and
/// the behaviour.)
///
/// Two key values continue, and both are agreement rather than absence:
///
/// - `Private(_)`: the key already partitions by a concrete identity that
///   this particular read did not name - the round 4 floor, unchanged.
/// - `Anonymous`: the render asked for an identity and found none, and the
///   key says none. Partitioning still holds, because a signed-in visitor
///   derives a `Private(_)` key and never reaches this entry.
///
/// An *undeclared* dimension still declines, because a route that checks
/// identity without declaring `Principal` would otherwise publish one
/// visitor's page under a key every other visitor hits.
///
/// Weakening this branch cannot admit a store that a value comparison would
/// have declined - the sixth review's argument, in one sentence: the branch
/// fires exactly when no value was observed for the dimension, which is
/// exactly when no stronger check is derivable from the report, so every
/// leak that passes through it is caused by the missing observation, never
/// by the fallback's weakness.
///
/// `Locale` is compared the same way, unconditionally (it is not a
/// `ClassificationReason` - locale is a content-variance concern, not a
/// `RepresentationClass` privacy concern - so it is checked outside the
/// reasons loop): every member of `locale_material` must equal the key's
/// declared `Locale` value, and an observed locale with no declared `Locale`
/// dimension at all declines outright, exactly as it did before round 5 (see
/// this task's fix round 1, item 1, for that check's original
/// introduction).
///
/// This closes all four leaks by construction: a comparison against a set
/// of actual values, never a property of one, and never a value re-read
/// after the state that produced it may already be gone.
///
/// # What this cannot see
///
/// See the module doc's "The honest boundary" section: a header or a
/// `Config::get` read produces no `ClassificationReason` at all, so there is
/// nothing here to compare against. This guard is not a substitute for a
/// route correctly declaring its own variance.
fn key_used_different_values_than_the_render_saw(
    job: &RenderJob,
    classification: &ClassificationOutcome,
    report: &super::collector::CollectorReport,
    runtime: &RenderCacheRuntime,
) -> bool {
    let declared = job.variance().dimensions();

    if !report.context.locale_material.is_empty() {
        match declared.get(&VarianceDimension::Locale) {
            Some(key_value) => {
                for observed_locale in &report.context.locale_material {
                    if &DimensionValue::Public(observed_locale.clone()) != key_value {
                        return true;
                    }
                }
            }
            // An observed locale with no declared `Locale` dimension at all:
            // the route would otherwise cache one language for everyone.
            None => return true,
        }
    }

    for reason in &classification.reasons {
        let (required, observed_ids) = match reason {
            ClassificationReason::PrincipalObserved | ClassificationReason::AuthorizationRead => (
                VarianceDimension::Principal,
                &report.context.principal_material,
            ),
            ClassificationReason::TenantObserved => {
                (VarianceDimension::Tenant, &report.context.tenant_material)
            }
            ClassificationReason::SessionValueRead
            | ClassificationReason::SecretContextRead
            | ClassificationReason::UndeclaredContext => {
                debug_assert_eq!(
                    classification.class,
                    RepresentationClass::Uncacheable,
                    "a session/secret/undeclared reason must force Uncacheable inside \
                     classify, which the caller already declines before this guard runs"
                );
                return true;
            }
        };
        if observed_ids.is_empty() {
            // Fix round 7, finding 3: `Anonymous` continues alongside
            // `Private(_)`. See this function's own doc for both arms.
            if !matches!(
                declared.get(&required),
                Some(DimensionValue::Private(_) | DimensionValue::Anonymous)
            ) {
                return true;
            }
            continue;
        }
        for observed_id in observed_ids {
            let expected = match required {
                VarianceDimension::Principal => {
                    DimensionValue::Private(PrivateMaterial::principal(
                        &runtime.keys,
                        observed_id,
                        FROZEN_PERMISSION_VERSION,
                    ))
                }
                VarianceDimension::Tenant => {
                    DimensionValue::Private(PrivateMaterial::tenant(&runtime.keys, observed_id))
                }
                _ => unreachable!("only Principal and Tenant reasons reach this match"),
            };
            if declared.get(&required) != Some(&expected) {
                return true;
            }
        }
    }

    false
}

/// Closes a collector report into the generations it observed, or `None`
/// when the report overflowed (see [`super::collector::CollectorReport::storable`])
/// or the ledger read itself failed.
///
/// Called from *inside* the render's own transaction (see [`run_render`])
/// so this reread and the render's own data reads share one snapshot, not
/// after the transaction has already committed, which would let a write
/// that happened during the render (and therefore before the transaction
/// committed) already be visible here, indistinguishable from one that
/// landed a week ago.
///
/// Ruling R55: obtains the observed list only through `storable()`, never
/// `observed` directly, and stores nothing when it returns `None`.
async fn close_window(
    report: &super::collector::CollectorReport,
    epoch: u64,
    ledger: &dyn GenerationLedger,
) -> Option<GenerationSet> {
    let identities = report.storable()?;
    let mut window = ObservationWindow::open(epoch);
    for identity in identities {
        // Final review, F10: an identity the window cannot hold is a
        // decline, never a silent omission. Unreachable today, because
        // `MAX_COLLECTED` is one below `MAX_OBSERVATIONS` so a storable
        // report always fits; a future bound change that broke that would
        // otherwise drop identities from the stored set silently, which is
        // the unsafe direction (an entry no write can invalidate).
        window.observe(identity.clone()).ok()?;
    }
    window.close(ledger).await.ok()
}

/// The isolation level the render transaction asks for on the active
/// backend (final review, F1 / ruling R117).
///
/// The render's own reads and the generation read at window close have to
/// share one snapshot, or a write that commits mid-render is visible to
/// the close read (and to the fresh reread) while the body was built from
/// the data before it, and a stale body is published under current
/// generations. PostgreSQL's default is `READ COMMITTED`, where every
/// statement sees the latest committed data, so it needs `REPEATABLE READ`
/// explicitly. InnoDB's default is already `REPEATABLE READ`, so the
/// explicit level is a no-op there and is passed for uniformity. SQLite's
/// WAL read transaction is a snapshot as of its first read already, and the
/// pinned SeaORM (2.0.2, `driver::sqlx_sqlite::set_transaction_config`)
/// does not error on a level but logs a `warn!` for every transaction that
/// carries one, so SQLite gets `None`: the backend default, with no log
/// noise, and the same snapshot. An unrecognised future backend (the enum
/// is `#[non_exhaustive]`) asks for `REPEATABLE READ`, the safe direction.
fn render_isolation_level(backend: DbBackend) -> Option<IsolationLevel> {
    match backend {
        DbBackend::Sqlite => None,
        _ => Some(IsolationLevel::RepeatableRead),
    }
}

/// Whether the key this render publishes under carries a resolved
/// `Principal` value, which is exactly the set of entries the permission
/// version used to partition (see [`FROZEN_PERMISSION_VERSION`]) and
/// therefore the set that must observe
/// [`collector::permission_version_identity`].
///
/// The key's variance, not the classification, is the precise criterion:
/// `PrivateCached` also arises from a tenant-only observation, where the
/// permission version never played a part, and a route that declares
/// `Principal` and caches one representation per signed-in visitor without
/// its render ever reading identity (ruling R89's shape) still keyed by the
/// version and still has to miss after a bump. An `Anonymous` value is
/// excluded for the same reason: the version was never bound into it.
fn key_carries_a_resolved_principal(variance: &VarianceDescriptor) -> bool {
    matches!(
        variance.dimensions().get(&VarianceDimension::Principal),
        Some(DimensionValue::Private(_))
    )
}

/// The body every [`run_render`] path shares: `next(request)` under a fresh
/// [`Collector::scope`], the report extracted while the scope is open, and
/// the window closed against `ledger` (see [`close_window`]). When
/// `observes_permission_generation` is set, the permission-version identity
/// is observed first, so it lands in the report like any other read and
/// counts toward the same bound.
///
/// `stitched` is whether this route publishes a stitched public shell,
/// the one class whose gate runs again on every hit and which therefore
/// classifies from content reads alone. Every other route folds the gate
/// bucket back into content here, before the window is closed, so both
/// the classification and the generation window cover exactly the
/// undivided set they covered before attribution existed.
///
/// A stitched route whose handler never began is declined outright: the
/// chain answered before the route handler ran (an authorization guard
/// that returns a page instead of calling the next layer, a tenant
/// refusal), so the content bucket is empty and classifying from it alone
/// would publish that gate response as the route's shared shell. The
/// decline reuses the same "no generation set" signal an overflowed
/// report already returns, so [`lead_render`] records the existing
/// `declined` outcome and no new telemetry label is introduced.
async fn render_under_collector(
    request: Request,
    next: Next,
    epoch: u64,
    ledger: &dyn GenerationLedger,
    observes_permission_generation: bool,
    stitched: bool,
) -> (
    Response,
    super::collector::CollectorReport,
    Option<GenerationSet>,
) {
    Collector::scope(async move {
        if observes_permission_generation {
            collector::observe(collector::permission_version_identity());
        }
        let response = next(request).await;
        let mut report = collector::current_report().unwrap_or_default();
        if !stitched {
            report.fold_gate_into_content();
        }
        let observed = if stitched && !report.handler_began {
            None
        } else {
            close_window(&report, epoch, ledger).await
        };
        (response, report, observed)
    })
    .await
}

/// Runs `next(request)` under [`Collector::scope`], inside a database
/// transaction when a database is configured, and closes the collector's
/// report into its observed generations (see [`close_window`]) while that
/// transaction is still open, so the render's own reads and the generation
/// reread share one snapshot. The transaction is opened through
/// `DB::transaction_with_isolation` at [`render_isolation_level`], because a
/// plain `DB::transaction` is `READ COMMITTED` on PostgreSQL and therefore
/// not a snapshot at all (final review, F1). The consequence for authors is
/// documented in the manual and in `render-cache.md`: on PostgreSQL a cached
/// route's handler that updates a row another transaction changed after the
/// render began sees a serialization failure; cached routes are read paths.
///
/// `request` is captured through a slot rather than moved directly into the
/// transaction closure, so that if the transaction itself cannot even open
/// (a provider failure, not a route failure), the still-untouched request
/// is recoverable for a plain, uncached render instead of being lost.
///
/// `observes_permission_generation` is [`key_carries_a_resolved_principal`]
/// for the job's variance; see that function for why the key, not the
/// classification, decides it. `stitched` is passed straight through to
/// [`render_under_collector`], which documents what it selects.
async fn run_render(
    runtime: &Arc<RenderCacheRuntime>,
    request: Request,
    next: Next,
    epoch: u64,
    observes_permission_generation: bool,
    stitched: bool,
) -> (
    Response,
    super::collector::CollectorReport,
    Option<GenerationSet>,
) {
    let backend = DB::connection()
        .ok()
        .map(|conn| conn.inner().get_database_backend());
    let Some(backend) = backend else {
        return render_under_collector(
            request,
            next,
            epoch,
            runtime.ledger.as_ref(),
            observes_permission_generation,
            stitched,
        )
        .await;
    };
    let slot: Arc<std::sync::Mutex<Option<Request>>> =
        Arc::new(std::sync::Mutex::new(Some(request)));
    let slot_for_closure = Arc::clone(&slot);
    let next_for_closure = next.clone();
    let ledger_for_closure = Arc::clone(&runtime.ledger);
    let result = DB::transaction_with_isolation(render_isolation_level(backend), move |_tx| {
        let slot = Arc::clone(&slot_for_closure);
        let next = next_for_closure.clone();
        let ledger = Arc::clone(&ledger_for_closure);
        Box::pin(async move {
            let request = slot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("the request is taken exactly once, by this closure, when it runs");
            Ok::<_, crate::FrameworkError>(
                render_under_collector(
                    request,
                    next,
                    epoch,
                    ledger.as_ref(),
                    observes_permission_generation,
                    stitched,
                )
                .await,
            )
        })
    })
    .await;
    match result {
        Ok(triple) => triple,
        Err(_) => {
            // The transaction could not even open, so the closure above
            // never ran and the request is still sitting in the slot.
            // Render without the shared read-view rather than losing the
            // request: correctness downstream is unaffected (the fresh
            // reread still catches a move), only the snapshot-consistency
            // optimization is lost.
            let request = slot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("a failed DB::transaction never invoked its closure");
            render_under_collector(
                request,
                next,
                epoch,
                runtime.ledger.as_ref(),
                observes_permission_generation,
                stitched,
            )
            .await
        }
    }
}

/// Rereads the observed dependencies and the epoch outside the render's
/// view; any move (a dependency's generation, or the epoch itself)
/// discards the candidate.
async fn fresh_reread_is_coherent(
    runtime: &RenderCacheRuntime,
    observed: &GenerationSet,
    epoch: u64,
) -> Result<(), ()> {
    let digests = observed.digests();
    // One statement, for the reason `authority_coherence` gives: the reread
    // and the epoch it is judged against are read together.
    let (current, fresh_epoch) = runtime
        .ledger
        .current_with_epoch(&digests)
        .await
        .map_err(|_| ())?;
    // Renews the epoch lease from the read that just returned it (task 5b),
    // before the race seam below: a hook armed there that itself advances
    // the epoch drops the lease, and re-filling it afterwards with the
    // pre-hook value would undo that.
    runtime.epoch_cache.refresh(fresh_epoch);
    // Test-only race seam (R72/R83): fires inside the reread, after the
    // values it will judge against have been read and before it judges
    // them, so a write armed here is on the far side of that read - the
    // comparison passes and the entry publishes already behind the ledger.
    #[cfg(any(test, feature = "testing"))]
    race_points::fire(&race_points::DURING_REREAD).await;
    match CoherenceCheck::compare(observed, &current, fresh_epoch, epoch) {
        CoherenceCheck::Coherent => {
            // Test-only race seam (R72/R83): fires after this reread has
            // already found the candidate coherent, so a write armed here
            // lands too late to be caught by *this* check but still lands
            // before the entry below is built and stored - proving that
            // such a write is instead caught at the *next* lookup, through
            // the stored `observed` set this reread already closed over.
            #[cfg(any(test, feature = "testing"))]
            race_points::fire(&race_points::AFTER_REREAD).await;
            Ok(())
        }
        CoherenceCheck::Moved(_) => Err(()),
    }
}

/// Builds the header a render's candidate entry would publish under, or
/// `None` when its headers cannot be safely replayed (an unsafe or
/// non-replayable header, or one exceeding a bound) - in which case the
/// candidate is declined the same as an ineligible or uncacheable one.
///
/// A replayable header whose *value* is not a valid HTTP header value is
/// dropped rather than declining the whole candidate, mirroring what
/// [`HttpResponse::into_hyper`](crate::http::HttpResponse) already does on
/// the way to the wire: the client never received that header either, so
/// storing the response without it stores exactly what was sent. Keeping it
/// would be worse than dropping the entry: `SafeHeaders` would accept the
/// value, the stored entry could then never be formed into a response, and
/// every later request would miss, render, republish and warn again on the
/// same value - which a request can influence.
///
/// Only the header: whether the stored representation is the response's own
/// bytes or a shell with holes cut in it is the caller's decision, and for a
/// stitched route the publisher's (see `stitch::build_composite_entry`).
fn entry_header(
    job: &RenderJob,
    policy: &RenderCachePolicy,
    class: RepresentationClass,
    observed: &GenerationSet,
    response: &HttpResponse,
    now: u64,
    seed_deadline_ms: Option<u64>,
) -> Option<EntryHeader> {
    let safe_pairs: Vec<(String, String)> = response
        .headers()
        .filter(|(name, _)| REPLAYABLE_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .filter(|(name, value)| {
            if http::HeaderValue::from_str(value).is_ok() {
                return true;
            }
            // The name is one of the eight in `REPLAYABLE_HEADERS`, so it is
            // closed and low-cardinality and safe to name; the value is the
            // request-influenced part and is never logged.
            tracing::warn!(
                target: "suprnova::render_cache",
                header = %name,
                "dropping a response header from the stored entry: its value is not a \
                 valid HTTP header value, so the wire drops it too",
            );
            false
        })
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let safe_headers = SafeHeaders::from_pairs(safe_pairs).ok()?;
    Some(EntryHeader {
        key: job.key().clone(),
        class,
        variance: job.variance().clone(),
        published_at_ms: now,
        fresh_ms: policy.freshness().fresh_ms(),
        stale_servable_ms: policy.freshness().stale_servable_ms(),
        stale_on_error_ms: policy.freshness().stale_on_error_ms(),
        observed: observed.clone(),
        epoch: job.epoch(),
        seed_deadline_ms,
        status: 200,
        headers: safe_headers,
        content_encoding: None,
    })
}

/// Encodes and publishes a built candidate to L0 (and L1 when the policy
/// uses it), under a fence minted by the coordinator for this lease. Never
/// fails the request: a publish failure (rejected, fenced, or a provider
/// error) just means the next request stays a miss - the response already
/// built from `entry` is served regardless.
async fn store_entry(
    runtime: &Arc<RenderCacheRuntime>,
    lease: &RebuildLease,
    policy: &RenderCachePolicy,
    job: &RenderJob,
    entry: &DecodedEntry,
    observed: &GenerationSet,
    now: u64,
) {
    let Ok(mut fence) = runtime.coordinator.publish_token(lease, now).await else {
        return;
    };
    fence.generation_digest = observed.digest();
    // Each kind is framed by its own codec: a Complete entry around its
    // final bytes, a Composite entry around its canonical header (which
    // carries the segment graph) and the shell those segments partition.
    let encoded = match entry {
        DecodedEntry::Complete(entry) => encode(entry, &runtime.keys),
        DecodedEntry::Composite(entry) => encode_composite(entry, &runtime.keys),
    };
    let Ok(encoded) = encoded else {
        return;
    };
    // Ruling R10: a Complete publication's hot entry is prepared from the
    // frame that is about to be stored, decoded back out of `encoded`, not
    // from the candidate that went into `encode`. Three things follow from
    // that and from nothing else: the hot body is a `Bytes` slice of the
    // stored frame, so L0 holds the body once rather than twice; the frame
    // is proven to round-trip before anything is published under it; and
    // this path and the L1 promotion path (see `promote_to_l0`) prepare
    // from the same input, so a promoted entry and a freshly published one
    // are the same thing.
    let hot = match entry {
        DecodedEntry::Complete(_) => {
            match decode(&encoded, &runtime.keys, &runtime.limits) {
                Ok(DecodedEntry::Complete(stored)) => match HotEntry::prepare(
                    stored,
                    policy.shared(),
                    &policy.freshness(),
                    now,
                    fence,
                ) {
                    Ok(hot) => Some(Arc::new(hot)),
                    // The entry is publishable, just not precomputable:
                    // publish it cold rather than dropping a publication
                    // over a header value the next hit could form for
                    // itself.
                    Err(error) => {
                        tracing::warn!(
                            target: "suprnova::render_cache",
                            kind = %error,
                            "a candidate could not be prepared for hot service; \
                             publishing it as stored bytes instead",
                        );
                        None
                    }
                },
                // This crate encoded the frame one line above, so a frame
                // that does not decode back into the kind it was encoded
                // from is a defect in this process, not a stored-entry
                // problem: publishing it would store bytes the next lookup
                // is guaranteed to evict.
                Ok(DecodedEntry::Composite(_)) => {
                    tracing::warn!(
                        target: "suprnova::render_cache",
                        "a freshly encoded Complete entry decoded back as a Composite one; \
                         nothing was published",
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(
                        target: "suprnova::render_cache",
                        kind = %error,
                        "a freshly encoded Complete entry did not decode back at all; \
                         nothing was published",
                    );
                    return;
                }
            }
        }
        DecodedEntry::Composite(_) => None,
    };
    // L0 has no age-based expiry of its own; an epoch advance clears it
    // outright instead (`RenderCache::advance_epoch`). `u64::MAX` is the
    // trait's documented "never age-swept" value.
    let outcome = match hot {
        Some(hot) => Ok(runtime
            .l0
            .publish_hot(job.key(), encoded.clone(), hot, fence, now)),
        None => {
            runtime
                .l0
                .publish(job.key(), encoded.clone(), fence, now, u64::MAX)
                .await
        }
    };
    if let Ok(PublishOutcome::Published) = outcome {
        Metrics::counter(render_cache_telemetry::PUBLICATIONS).inc();
    }
    if policy.layers().l1()
        && let Some(l1) = &runtime.l1
    {
        // The Dead edge (fix round 1, R93/F2; class-aware since fix round
        // 2, R99): the single source of truth `coherence::evaluate_freshness`
        // uses for its own Dead boundary, so `FileRenderStore::sweep` can
        // never disagree with it about when this entry is truly dead. This
        // is `fresh_ms + max(stale_servable_ms, stale_on_error_ms)` for
        // every class except `PrivateCached` (the two stale bands are both
        // measured from the end of the fresh interval, not cumulatively,
        // and `FreshnessPolicy::new` does not require `stale_on_error_ms >=
        // stale_servable_ms`); `PrivateCached` gets no stale grace period at
        // all and is `fresh_ms` alone - framing the class-blind value here
        // for a private entry left its L1 file retained for the full stale
        // window past the point it can never be served again (spec 16 line
        // 78: private entries have bounded retention and eviction
        // independent of public entries). This is the one call site with a
        // policy and a classified entry in scope to compute a real
        // retention from; every other `RenderStore::publish` caller
        // (including `file_store.rs`'s own tests) passes its own explicit
        // value.
        let retention_ms = policy.freshness().dead_after_ms(entry.header().class);
        let _ = l1
            .publish(job.key(), encoded, fence, now, retention_ms)
            .await;
    }
}

/// Builds the safety signals `RenderCachePolicy::eligibility` reads from a
/// concrete response.
fn response_signals(http: &HttpResponse, method: &str) -> ResponseSignals {
    let header_names: Vec<String> = http.headers().map(|(name, _)| name.to_owned()).collect();
    ResponseSignals {
        method: method.to_owned(),
        status: http.status_code(),
        streaming: http.is_streaming(),
        sets_cookie: header_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("set-cookie")),
        content_type: http
            .headers()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.to_owned()),
        header_names,
        private_observed: false,
    }
}

/// Test-only: wraps [`key_input`] with explicit route params and an
/// optional login instead of a `Request`.
#[doc(hidden)]
pub fn key_input_for_test(
    runtime: &RenderCacheRuntime,
    pattern: &str,
    params: &[(&str, &str)],
    login: Option<&str>,
    policy: &RenderCachePolicy,
) -> RenderKeyInput {
    let params: BTreeMap<String, String> = params
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let mut variance = VarianceDescriptor::new();
    for dimension in policy.vary() {
        if *dimension == VarianceDimension::Principal {
            let value = match login {
                Some(id) => DimensionValue::Private(PrivateMaterial::principal(
                    &runtime.keys,
                    id,
                    FROZEN_PERMISSION_VERSION,
                )),
                None => DimensionValue::Anonymous,
            };
            let _ = variance.declare(dimension.clone(), value);
        }
    }
    RenderKeyInput {
        route: route_identity(pattern),
        route_pattern: pattern.to_owned(),
        params,
        query: BTreeMap::new(),
        host: None,
        media: "text/html".to_owned(),
        encoding: None,
        build: runtime.build.clone(),
        // A fixed baseline matching the RenderCache migration's seeded
        // epoch: this test helper never advances the epoch, so every call
        // deriving a key for the same route and login always lands on the
        // same key regardless of when it runs in a test.
        epoch: 1,
        variance,
    }
}

/// Test-only race-injection seams for this module's own coherence checks.
/// Each hook fires from the exact point in the request flow its name
/// describes, awaited in place, so a test can land a write or an epoch
/// advance inside a window that is otherwise too narrow to hit
/// deterministically from outside.
///
/// Compiled only under `cfg(test)` or the `testing` feature (ruling R72):
/// an integration test under `framework/tests/` is a separate crate with
/// no `cfg(test)` of its own reaching this library, so it can only see
/// these hooks through the feature - which is on by default, so an
/// ordinary `cargo test` still exercises them, but a feature-matrix build
/// that turns default features off compiles the race suite's own test
/// file to nothing instead of failing against seams that do not exist.
#[doc(hidden)]
#[cfg(any(test, feature = "testing"))]
pub mod race_points {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A one-shot, boxed async closure a test arms and this module
    /// consumes exactly once, the next time its race point fires.
    pub type Hook = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

    /// One race point: an `armed` flag plus the hook itself behind a
    /// `Mutex`.
    ///
    /// `testing` is a default-on feature (see `Cargo.toml`'s `default`
    /// list), so `fire` runs on every GET/HEAD to a policy-covered route
    /// in an ordinary build, hits included - `EPOCH_CAPTURED` sits before
    /// the L0 lookup. Fix round 1, F6: `armed` is the fast path that keeps
    /// an unarmed race point to one relaxed load, rather than a mutex lock
    /// on every such request. Fix round 1, F8: `Mutex::new` has been
    /// `const` since Rust 1.63, so a race point needs no `OnceLock` layer
    /// to lazily initialize the way the previous version did.
    pub struct RacePoint {
        armed: AtomicBool,
        hook: Mutex<Option<Hook>>,
    }

    impl RacePoint {
        /// A disarmed race point, usable directly as a `static` initializer.
        const fn new() -> Self {
            Self {
                armed: AtomicBool::new(false),
                hook: Mutex::new(None),
            }
        }
    }

    /// Fires from `fresh_reread_is_coherent`, immediately after it
    /// finds the render still coherent and before its caller acts on that
    /// result - the exact window a write must land in to be "after the
    /// reread" and "before publication": late enough that it cannot itself
    /// be caught by this same reread, early enough that the entry this
    /// request publishes still carries the observations from before it.
    pub static AFTER_REREAD: RacePoint = RacePoint::new();

    /// Fires from `lead_render`, as its first statement - before
    /// `run_render` opens the consistent read view the render's own data
    /// reads and its window close share. A write armed here has therefore
    /// already committed when that view opens, so the render reads it, the
    /// window records the generation it produced, and the fresh reread
    /// agrees: the entry publishes and serves. This is the race window's
    /// lower boundary, and the one arrival a candidate must *not* be
    /// discarded for.
    pub static BEFORE_VIEW: RacePoint = RacePoint::new();

    /// Fires from `lead_render`, immediately after `run_render` returns -
    /// the read view has closed and the observed generation set is already
    /// fixed, and nothing has yet judged it. A write armed here is invisible
    /// to the render but fully visible to the fresh reread that follows, so
    /// the candidate is discarded and nothing is published at all. Fired
    /// before the eligibility, classification, and document checks that can
    /// each return early, so the arm is consumed on every path a render
    /// reaches, not only on the one that reaches the reread.
    pub static AFTER_VIEW_CLOSE: RacePoint = RacePoint::new();

    /// Fires from `fresh_reread_is_coherent`, between the ledger read and
    /// `CoherenceCheck::compare` - inside the reread itself, after it has
    /// read the generations it will judge against and before it judges
    /// them. A write armed here is on the far side of that read, so the
    /// comparison passes on values that are already behind and the entry
    /// publishes stale; the lookup-time coherence check on the next
    /// request is where it is caught. Paired
    /// with [`AFTER_REREAD`], which lands a write just outside the same
    /// window, this proves the comparison judges what the reread read
    /// rather than re-reading at comparison time.
    pub static DURING_REREAD: RacePoint = RacePoint::new();

    /// Fires from `RenderCacheMiddleware::serve`, immediately
    /// after the epoch a new `RenderJob` will carry is captured, and
    /// before the render that job describes begins - the exact window an
    /// epoch advance must land in to be baked into the job as stale by the
    /// time that render's own fresh reread checks it. Fires on whichever
    /// *request* reaches that point next, not necessarily the next
    /// *render*: `serve` captures the epoch before it knows whether the
    /// request will be a hit, a stale serve, or a render (fix round 1, F7).
    ///
    /// "Captured", not "read": since task 5b the epoch usually comes from
    /// the runtime's `EpochCache`, and only the first request of a runtime
    /// (or the first after an `advance_epoch`) reads the ledger here. The
    /// property is unchanged either way, because it never depended on where
    /// the value came from - only on the request already holding it when the
    /// hook fires. An `advance_epoch` armed here drops the lease and clears
    /// L0, so this request's job carries the pre-advance epoch into a fresh
    /// reread that reads the post-advance one, and that candidate is
    /// discarded rather than published.
    ///
    /// Scoped to what is actually true on both kinds of route. On an
    /// L0-only route (every route the race suite drives) the lookup that
    /// follows also misses, because the clear emptied the only tier there
    /// was. On an L1-backed route it need not: `advance_epoch` does not
    /// clear L1 (see
    /// [`crate::render_cache::RenderCache::advance_epoch`]), so the lookup
    /// can still find the pre-advance entry, `coherence` refreshes the
    /// epoch off its reread, `RenderJob::restamp` moves the rebuild to the
    /// post-advance epoch, and *that* candidate is published - correctly,
    /// under the new key. What the seam proves is the first sentence, the
    /// fate of the job that carried the stale epoch; it is not a claim
    /// that no entry is published at all.
    pub static EPOCH_CAPTURED: RacePoint = RacePoint::new();

    /// Arms `point` to run `hook` exactly once, the next time it fires.
    /// Replaces any hook already armed there.
    ///
    /// Fix round 2, N1: the hook write and the `armed` store are one
    /// critical section (both happen while `slot` is still locked), the
    /// same as [`disarm`] and `fire` below. Storing `armed` outside the
    /// lock (as an earlier version of this module did) let the two race
    /// points' state disagree under interleaving: `fire` could `take()` the
    /// hook and release the lock, an `arm` on another thread could then
    /// lock, write a new hook, and set `armed` true, and only *then* would
    /// `fire`'s own deferred `armed.store(false)` run - clobbering the new
    /// arm's `true` back to `false` while its hook sat in the `Mutex` as
    /// `Some(..)`. That hook would never fire; the symptom is a barrier
    /// that hangs forever waiting for a race that silently never happened.
    /// Keeping the flag and the hook inside one lock makes that
    /// interleaving impossible: whichever of `arm`/`disarm`/`fire` gets the
    /// lock next always sees (and leaves) a consistent pair.
    pub fn arm(point: &'static RacePoint, hook: Hook) {
        let mut slot = point.hook.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(hook);
        point.armed.store(true, Ordering::Relaxed);
    }

    /// Clears any hook armed at `point` without firing it, and lowers the
    /// flag `fire` checks. Fix round 1, F4: `AFTER_REREAD` only fires on
    /// a coherent reread, and `lead_render` has several decline paths that
    /// return before reaching it, so an arm a test made but that path never
    /// consumed would otherwise leak into whichever test runs next in the
    /// same process. Test-only cleanup; production code never calls this.
    /// Fix round 2, N1: flag and hook clear inside the same critical
    /// section - see [`arm`]'s own doc for why that matters.
    pub fn disarm(point: &'static RacePoint) {
        let mut slot = point.hook.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        point.armed.store(false, Ordering::Relaxed);
    }

    /// Fires `point` if a hook is armed there, consuming the arm; a no-op
    /// otherwise. The relaxed load of `armed` is the only cost an ordinary,
    /// unarmed request pays - it never reaches the mutex. Once that fast
    /// path decides to look further, the `take()` and the `armed` store
    /// that consume the arm run inside one critical section (fix round 2,
    /// N1; see [`arm`]'s own doc), so an `arm` racing this call can never
    /// land its hook in the gap between them.
    pub(crate) async fn fire(point: &'static RacePoint) {
        if !point.armed.load(Ordering::Relaxed) {
            return;
        }
        let hook = {
            let mut slot = point.hook.lock().unwrap_or_else(|e| e.into_inner());
            let hook = slot.take();
            point.armed.store(false, Ordering::Relaxed);
            hook
        };
        if let Some(hook) = hook {
            hook().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
    use suprnova_live::identity::{KeyId, UnixMillis};
    use suprnova_live::render_cache::singleflight::{
        LocalCoordinatorLimits, LocalRebuildCoordinator,
    };
    use suprnova_live::render_cache::store::{MemoryStoreLimits, PublicationFence};

    fn test_keys() -> SnapshotKeyRing {
        let active = KeyRecord::new(
            KeyId::parse("render-cache-middleware-test").expect("key id"),
            RootKey::new(vec![7; 32]).expect("root key"),
            UnixMillis::new(0),
            UnixMillis::new(u64::MAX / 2),
            UnixMillis::new(u64::MAX),
        )
        .expect("key record");
        SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
    }

    /// A runtime with an L0 store and no L1, enough for `lookup` alone: the
    /// ledger and coordinator are never consulted by it.
    fn lookup_only_runtime(keys: SnapshotKeyRing) -> RenderCacheRuntime {
        RenderCacheRuntime {
            config: RenderCacheConfig {
                enabled: true,
                profile: super::super::Profile::Embedded,
                l0: super::super::L0Limits {
                    max_entries: 8,
                    max_bytes: 1024 * 1024,
                },
                l1: super::super::L1Config::Disabled,
                coordinator: super::super::CoordinatorConfig::Local {
                    lease_ms: 30_000,
                    max_waiters: 128,
                },
                failure: FailurePolicy::Open,
                build_id: "test".to_owned(),
                clock_override: None,
                coordinator_override: None,
            },
            build: BuildId::parse("test").expect("build id"),
            table: RenderCachePolicyTable::default(),
            l0: MemoryRenderStore::new(MemoryStoreLimits {
                max_entries: 8,
                max_bytes: 1024 * 1024,
            }),
            l1: None,
            ledger: Arc::new(super::super::ledger::SqlGenerationLedger::new()),
            epoch_ledger: super::super::ledger::SqlGenerationLedger::new(),
            coordinator: Arc::new(LocalRebuildCoordinator::new(LocalCoordinatorLimits {
                lease_ms: 30_000,
                max_waiters: 8,
            })),
            keys,
            clock: Arc::new(suprnova_live::clock::SystemClock),
            limits: EntryLimits::default(),
            leases: Mutex::new(BTreeMap::new()),
            epoch_cache: EpochCache::empty(),
            hot_serves: std::sync::atomic::AtomicU64::new(0),
            #[cfg(any(test, feature = "testing"))]
            background_rebuilds: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The route policy `lookup` needs to prepare a promoted entry for hot
    /// service. These tests never reach that path (no L1 is configured), so
    /// the values are the plain public defaults.
    fn lookup_only_policy() -> RenderCachePolicy {
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .build()
            .expect("policy")
    }

    fn entry_for(key: &RenderKey) -> CompleteEntry {
        CompleteEntry::new(
            EntryHeader {
                key: key.clone(),
                class: RepresentationClass::PublicShared,
                variance: VarianceDescriptor::new(),
                published_at_ms: 1_000,
                fresh_ms: 60_000,
                stale_servable_ms: 0,
                stale_on_error_ms: 0,
                observed: GenerationSet::default(),
                epoch: 1,
                seed_deadline_ms: None,
                status: 200,
                headers: SafeHeaders::from_pairs([("content-type", "text/html; charset=utf-8")])
                    .expect("safe headers"),
                content_encoding: None,
            },
            Bytes::from_static(b"<!doctype html><html><body>stored</body></html>"),
        )
    }

    /// Final review, F7: a frame that decodes and verifies but whose header
    /// names a different key than the one it was found under is misplaced.
    /// `lookup` evicts it and reports a miss instead of serving another
    /// key's bytes. The control half proves the comparison is against the
    /// stored key, not a blanket refusal: the same bytes under their own key
    /// are a hit.
    ///
    /// Proven by revert: with the `entry.header().key == *key` guard removed
    /// from `lookup`'s L0 arm, the first assertion fails because the
    /// misplaced entry is returned as a hit.
    #[tokio::test]
    async fn a_decodable_entry_stored_under_another_key_is_evicted_and_treated_as_a_miss() {
        // Two rings from the same root derive the same keys, so the
        // encoder's ring and the runtime's ring agree; `SnapshotKeyRing` is
        // deliberately not `Clone`.
        let keys = test_keys();
        let runtime = lookup_only_runtime(test_keys());
        let own_key = RenderKey::for_test(&keys, "/own");
        let other_key = RenderKey::for_test(&keys, "/other");
        let encoded = encode(&entry_for(&other_key), &keys).expect("encode");
        let fence = PublicationFence {
            epoch: 1,
            generation_digest: [0; 32],
            token: 1,
        };

        // Misplaced: the entry for `/other` sits at `/own`'s slot.
        runtime
            .l0
            .publish(&own_key, encoded.clone(), fence, 1_000, u64::MAX)
            .await
            .expect("publish the misplaced entry");
        let policy = lookup_only_policy();
        let hit = lookup(&runtime, &policy, &own_key).await.expect("lookup");
        assert!(
            hit.is_none(),
            "an entry whose stored key is not the lookup key must be a miss, never served"
        );
        assert!(
            runtime.l0.get(&own_key).await.expect("get").is_none(),
            "the misplaced entry is evicted from the layer it was found in"
        );

        // Control: the same bytes under their own key are a hit.
        runtime
            .l0
            .publish(&other_key, encoded, fence, 1_000, u64::MAX)
            .await
            .expect("publish the well-placed entry");
        let hit = lookup(&runtime, &policy, &other_key).await.expect("lookup");
        assert!(
            hit.is_some_and(|found| found.header().key == other_key && found.layer() == Layer::L0),
            "control: a correctly placed entry is served from L0"
        );
    }

    #[test]
    fn a_reasonless_private_classification_narrowed_from_a_wider_declared_class_is_declined() {
        assert!(is_unreasoned_private_class(
            &ClassificationOutcome {
                class: RepresentationClass::PrivateCached,
                reasons: Vec::new(),
            },
            RepresentationClass::PublicShared,
        ));
    }

    #[test]
    fn a_declared_private_cached_route_with_no_narrowing_is_not_declined_by_this_check() {
        // R89: `classify` never narrows without a reason, so a
        // `PrivateCached` class with empty reasons here can only mean the
        // route declared `PrivateCached` up front - already required to
        // carry `Principal` or `Tenant` variance, and already correctly
        // cacheable per Task 14.
        assert!(!is_unreasoned_private_class(
            &ClassificationOutcome {
                class: RepresentationClass::PrivateCached,
                reasons: Vec::new(),
            },
            RepresentationClass::PrivateCached,
        ));
    }

    #[test]
    fn a_private_classification_with_a_reason_is_left_to_the_value_guard() {
        assert!(!is_unreasoned_private_class(
            &ClassificationOutcome {
                class: RepresentationClass::PrivateCached,
                reasons: vec![ClassificationReason::PrincipalObserved],
            },
            RepresentationClass::PublicShared,
        ));
    }

    #[test]
    fn a_non_private_class_is_never_declined_by_this_check() {
        assert!(!is_unreasoned_private_class(
            &ClassificationOutcome {
                class: RepresentationClass::PublicShared,
                reasons: Vec::new(),
            },
            RepresentationClass::PublicShared,
        ));
        assert!(!is_unreasoned_private_class(
            &ClassificationOutcome {
                class: RepresentationClass::Uncacheable,
                reasons: Vec::new(),
            },
            RepresentationClass::PublicShared,
        ));
    }
}
