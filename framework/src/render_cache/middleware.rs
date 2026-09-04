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
//! - **Anonymous identity resolution reads the session, which is
//!   `Uncacheable`.** `Auth::id()` resolves through request state first and
//!   falls back to `session()`, which records a session read; `classify`
//!   narrows any session read straight to `Uncacheable`. So an anonymous
//!   visitor of a route whose render calls `Auth::id()` never caches, even
//!   though its key correctly says `Anonymous` and the guard's empty-set
//!   path (below) would now accept it - the class decision happens first.
//!   A signed-in visitor resolves through request state and never reaches
//!   the fallback, so the same route does cache for them. Recording that
//!   fallback as an identity read rather than a session read would fix it
//!   and is measurably the only thing standing in the way, but it turns
//!   every session-authenticated render from `Uncacheable` into
//!   `PrivateCached`, which is a far larger widening than fix round 7 was
//!   scoped to make. Parked, deliberately, with the measurement recorded in
//!   `an_anonymous_render_that_resolves_identity_through_the_session_stays_uncacheable`.
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
//! # Deliberately out of scope for this task
//!
//! - **Seed promotion deadlines.** `EntryHeader::seed_deadline_ms` is
//!   always `None` here. Reading a Live document's embedded public seed
//!   deadline is the Live-integration task this iteration's plan lists
//!   separately from the middleware; wiring it in here would reach past
//!   this task's subject.
//! - **Background rebuild's ambient context (fix round 2, item 4).** A
//!   stale-servable background rebuild runs on a `tokio::spawn`ed task, and
//!   task-locals do not cross a spawn. `Lang`'s negotiated locale and
//!   `Auth`'s request-scoped identity are both task-local
//!   (`tokio::task_local!`-backed), so a background render for a route that
//!   declares `Locale` or `Principal` variance would compute a *different*
//!   variance than the one the key it is about to publish under was already
//!   derived from: a locale-varying route's background rebuild would render
//!   the default locale's content and publish it under another locale's
//!   key, and a principal-varying route's background rebuild would render
//!   anonymously and publish under a specific principal's key - a real
//!   content-identity mismatch, not merely a wasted render. (An earlier
//!   draft of this note called this "a possible wasted or misdirected
//!   rebuild, not a correctness or security defect," reasoning that
//!   `key_used_different_values_than_the_render_saw`'s narrowing would decline the store;
//!   round 1 of this task's review established that narrowing never
//!   repartitions an already-derived key, so that justification does not
//!   hold, and the gap is broader than the cookie-carried case it was
//!   framed around - the fix below is the actual guard.)
//!
//!   The fix: `RenderCacheMiddleware::serve` does not spawn a background
//!   rebuild at all for a route whose policy declares `Locale` or
//!   `Principal` variance (see `variance_depends_on_ambient_context`); such
//!   a route still serves its stale-servable entry immediately - the
//!   "never blocks" guarantee is unaffected - it just does not also try to
//!   refresh it in the background. The entry only refreshes once it goes
//!   Dead and the next request renders it in the foreground, where the
//!   ambient context is the real request's own. `Tenant` variance needs no
//!   such guard: `Request::live_tenant()` reads a field set on the `Request`
//!   value itself, not a task-local, so it survives the moved `Request`
//!   the spawn carries. A request id is also lost across the spawn - log
//!   correlation only, not a cache-key concern, so it is not guarded here.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Digest as _, Sha256};
use suprnova_live::clock::Clock;
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::identity::{BuildId, RouteIdentity};
use suprnova_live::render_cache::coherence::{
    FreshnessState, ValidationLease, age_seconds, evaluate_freshness, warning_header,
};
use suprnova_live::render_cache::entry::{
    CompleteEntry, EntryHeader, EntryLimits, REPLAYABLE_HEADERS, SafeHeaders, decode, encode,
};
use suprnova_live::render_cache::generation::{
    CoherenceCheck, GenerationLedger, GenerationSet, ObservationWindow,
};
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

use crate::database::DB;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};
use crate::telemetry::metrics::Metrics;
use crate::{Auth, Lang};

use super::collector::{self, Collector};
use super::config::RenderCacheConfig;
use super::file_store::FileRenderStore;
use super::live;
use super::registry::RenderCachePolicyTable;
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

/// The assembled RenderCache runtime: stores, ledger, coordinator, keys,
/// policy table, configuration, and clock. One instance per installed
/// process; `super::RenderCache::runtime` hands out clones of the `Arc`.
pub struct RenderCacheRuntime {
    pub(crate) config: RenderCacheConfig,
    pub(crate) table: RenderCachePolicyTable,
    pub(crate) l0: MemoryRenderStore,
    pub(crate) l1: Option<FileRenderStore>,
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
}

impl RenderCacheRuntime {
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

/// Coherence outcome of a stored entry against the current authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Coherence {
    Coherent,
    Moved,
}

/// Which layer answered a lookup, for telemetry and promotion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layer {
    L0,
    L1,
}

/// Closed lookup outcome, for telemetry's `outcome` attribute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LookupOutcome {
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

    fn record(self) {
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
        let epoch = match runtime.ledger.epoch().await {
            Ok(epoch) => epoch,
            Err(_) => return Err(ProviderFailure(request, next)),
        };
        // Test-only race seam (R72/R83): fires right after the epoch this
        // request's `RenderJob` will carry is captured, and before the
        // render it describes begins - so an epoch advance armed here is
        // baked into the job as already stale by the time that render's
        // own fresh reread checks it, proving such a render's candidate is
        // never published.
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
        let variance = input.variance.clone();
        let Ok(key) = RenderKey::derive(&input, &runtime.keys) else {
            LookupOutcome::Bypass.record();
            return Ok(next(request).await);
        };

        let hit = match lookup(runtime, &key).await {
            Ok(hit) => hit,
            Err(()) => return Err(ProviderFailure(request, next)),
        };
        let Some((entry, stored, layer)) = hit else {
            LookupOutcome::Miss.record();
            let job = RenderJob {
                key,
                epoch,
                variance,
            };
            return render_and_publish(runtime, request, next, policy, job, 0).await;
        };

        let coherence = match coherence(runtime, &key, policy, entry.header()).await {
            Ok(coherence) => coherence,
            Err(()) => return Err(ProviderFailure(request, next)),
        };
        let now = runtime.now_ms();
        let state = freshness_state(
            policy,
            coherence,
            entry.header().class,
            stored.published_at_ms,
            now,
            entry.header().seed_deadline_ms,
        );
        match state {
            FreshnessState::Fresh => {
                (match layer {
                    Layer::L0 => LookupOutcome::L0Hit,
                    Layer::L1 => LookupOutcome::L1Hit,
                })
                .record();
                if matches!(
                    evaluate_conditional(request.header("if-none-match"), entry.validator()),
                    ConditionalOutcome::NotModified
                ) {
                    LookupOutcome::Conditional.record();
                }
                Ok(respond_hit(
                    &request,
                    policy,
                    &entry,
                    stored.published_at_ms,
                    now,
                    None,
                ))
            }
            FreshnessState::StaleServable => {
                LookupOutcome::Stale.record();
                let response = respond_hit(
                    &request,
                    policy,
                    &entry,
                    stored.published_at_ms,
                    now,
                    warning_header(state),
                );
                // Fix round 2, item 4: a route whose variance depends on
                // ambient (task-local) context does not get a background
                // rebuild - see the module doc's "Background rebuild's
                // ambient context" note for why. The stale entry is still
                // served immediately either way; only the background
                // refresh is skipped.
                if !variance_depends_on_ambient_context(policy) {
                    self.spawn_background_rebuild(
                        Arc::clone(runtime),
                        request,
                        next,
                        policy.clone(),
                        RenderJob {
                            key,
                            epoch,
                            variance,
                        },
                    );
                }
                Ok(response)
            }
            FreshnessState::StaleOnError => {
                LookupOutcome::Miss.record();
                // Captured before `request` moves into the rebuild attempt,
                // so a fallback to the stale entry (below) does not need the
                // request back - matching `lead_render`'s own capture.
                let method = request.method().as_str().to_owned();
                let if_none_match = request.header("if-none-match").map(str::to_owned);
                let job = RenderJob {
                    key,
                    epoch,
                    variance,
                };
                let outcome = render_and_publish(runtime, request, next, policy, job, 0).await;
                // Stale-on-error exists for a foreground rebuild that fails,
                // not only for a provider failure before the handler ran: a
                // handler that itself returns an error or a 5xx status is an
                // ordinary `Response` to `render_and_publish`, so that case
                // must be detected here too rather than passed through. See
                // fix round 2, item 3.
                let rebuild_failed = match &outcome {
                    Ok(response) => {
                        let status = match response {
                            Ok(http) | Err(http) => http.status_code(),
                        };
                        status >= 500
                    }
                    Err(ProviderFailure(..)) => true,
                };
                if !rebuild_failed {
                    return outcome;
                }
                LookupOutcome::Stale.record();
                Ok(conditional_response(
                    &method,
                    if_none_match.as_deref(),
                    policy,
                    &entry,
                    stored.published_at_ms,
                    now,
                    warning_header(FreshnessState::StaleOnError),
                ))
            }
            FreshnessState::Dead => {
                LookupOutcome::Miss.record();
                let job = RenderJob {
                    key,
                    epoch,
                    variance,
                };
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
/// `tokio::task_local!`-backed. A `tokio::spawn`ed task does not inherit
/// task-locals, so a background rebuild for one of these routes would
/// compute a different variance than the key it is about to publish under.
/// See the module doc's "Background rebuild's ambient context" note (fix
/// round 2, item 4). `Tenant` is deliberately excluded: `Request::live_tenant`
/// reads a field on the moved `Request`, not a task-local, so it is safe
/// across the spawn.
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
            VarianceDimension::Locale => DimensionValue::Public(Lang::locale().as_str()),
            VarianceDimension::Principal => match Auth::id() {
                Some(id) => DimensionValue::Private(PrivateMaterial::principal(
                    &runtime.keys,
                    &id,
                    collector::permission_version(),
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
        build: BuildId::parse(&runtime.config.build_id)
            .unwrap_or_else(|_| BuildId::parse("default").expect("'default' is a valid build id")),
        epoch,
        variance: variance_descriptor(runtime, request, policy)?,
    })
}

/// Reads a key from L0, then L1 (promoting a decodable L1 hit to L0). A
/// defective entry (decode failure) is evicted from the layer it was found
/// in and treated as a miss on that layer.
async fn lookup(
    runtime: &RenderCacheRuntime,
    key: &RenderKey,
) -> Result<Option<(CompleteEntry, StoredEntry, Layer)>, ()> {
    let l0_stored = runtime.l0.get(key).await.map_err(|_| ())?;
    if let Some(stored) = l0_stored {
        match decode(&stored.bytes, &runtime.keys, &runtime.limits) {
            Ok(entry) => return Ok(Some((entry, stored, Layer::L0))),
            Err(_) => {
                let _ = runtime.l0.evict(key).await;
            }
        }
    }
    if let Some(l1) = &runtime.l1 {
        let l1_stored = l1.get(key).await.map_err(|_| ())?;
        if let Some(stored) = l1_stored {
            match decode(&stored.bytes, &runtime.keys, &runtime.limits) {
                Ok(entry) => {
                    let _ = runtime
                        .l0
                        .publish(
                            key,
                            stored.bytes.clone(),
                            stored.fence,
                            stored.published_at_ms,
                        )
                        .await;
                    return Ok(Some((entry, stored, Layer::L1)));
                }
                Err(_) => {
                    let _ = l1.evict(key).await;
                }
            }
        }
    }
    Ok(None)
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
        // Fix round 2, item 6: a valid lease reports Coherent without
        // itself consulting the epoch. The review's stated concern was that
        // this leaves an emergency `RenderCache::advance_epoch` unable to
        // reach a lease-mode route until the lease expires naturally, which
        // could be as long as `max_age_ms`. Investigated rather than
        // assumed: `RenderKey::derive` bakes the epoch into the lookup key
        // itself (`feed(10, &input.epoch.to_be_bytes())` in
        // `suprnova_live::render_cache::key`), and `key_input` always
        // derives that key from a freshly read epoch on every dispatch, in
        // `serve`, before `coherence` (or any coherence mode) ever runs. So
        // an epoch bump changes the lookup key for every route - lease mode
        // included - making the previously-published entry unreachable by
        // ordinary lookup on the very next request: a ordinary cache miss,
        // which renders immediately, not a "Moved" result this function
        // would need to detect. An explicit epoch comparison here would
        // therefore check a condition (`header.epoch` disagreeing with the
        // current epoch on a *found* entry) that cannot occur through this
        // host's own key derivation, so this documents the finding - the
        // narrower of the review's two offered fixes - rather than adding a
        // comparison with no reachable path to prove or exercise. This
        // reasoning is specific to this host: it would need re-establishing
        // before anyone changes `RenderKey::derive` to stop keying on epoch,
        // or introduces a lookup path that does not call `key_input` fresh
        // per request.
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
    let current = runtime.ledger.current(&digests).await.map_err(|_| ())?;
    let epoch = runtime.ledger.epoch().await.map_err(|_| ())?;
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

/// Builds the served response for a hit: a 304 when the request's
/// `If-None-Match` matches, the full representation otherwise (body-free
/// for `HEAD`). Carries `ETag`, `Cache-Control`, `Vary`, `Age`, and
/// `Warning` (when stale).
fn respond_hit(
    request: &Request,
    policy: &RenderCachePolicy,
    entry: &CompleteEntry,
    published_at_ms: u64,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Response {
    conditional_response(
        request.method().as_str(),
        request.header("if-none-match"),
        policy,
        entry,
        published_at_ms,
        now_ms,
        warning,
    )
}

/// The shared tail of [`respond_hit`] and a freshly published candidate:
/// builds the served response (a 304 when `if_none_match` matches, the
/// full representation otherwise, body-free for `HEAD`) with `ETag`,
/// `Cache-Control`, `Vary`, `Age`, and `Warning` (when stale). Takes the
/// request's method and `If-None-Match` value rather than the `Request`
/// itself, because a freshly rendered candidate's request has already been
/// consumed by the render by the time this needs to run - see
/// `lead_render`, which captures both before rendering.
fn conditional_response(
    method: &str,
    if_none_match: Option<&str>,
    policy: &RenderCachePolicy,
    entry: &CompleteEntry,
    published_at_ms: u64,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Response {
    let header = entry.header();
    let not_modified = matches!(
        evaluate_conditional(if_none_match, entry.validator()),
        ConditionalOutcome::NotModified
    );
    let is_head = method == "HEAD";
    let body = if not_modified || is_head {
        Bytes::new()
    } else {
        entry.body().clone()
    };
    let content_type = header
        .headers
        .iter()
        .find(|(name, _)| *name == "content-type")
        .map(|(_, value)| value.to_owned())
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let mut response = HttpResponse::bytes(body, content_type).status(if not_modified {
        304
    } else {
        header.status
    });
    for (name, value) in header.headers.iter() {
        if name == "content-type" || name == "cache-control" || name == "vary" {
            continue;
        }
        response = response.header(name.to_owned(), value.to_owned());
    }
    response = response.header("ETag", entry.validator().etag());
    let seed_remaining = header
        .seed_deadline_ms
        .map(|deadline| deadline.saturating_sub(now_ms));
    response = response.header(
        "Cache-Control",
        cache_control_value(
            header.class,
            policy.shared(),
            &policy.freshness(),
            seed_remaining,
        ),
    );
    if let Some(vary) = vary_value(&header.variance) {
        response = response.header("Vary", vary);
    }
    response = response.header("Age", age_seconds(published_at_ms, now_ms).to_string());
    if let Some(warning) = warning {
        response = response.header("Warning", warning);
    }
    Ok(response)
}

/// Admits a rebuild for `key` and either leads it (rendering and, if
/// eligible and still coherent, publishing), reuses a leader's completed
/// publication after waiting, or renders without publishing (`Wait`
/// exhausted, or `Bypass`).
/// One render's fixed identity: the lookup key, the epoch admission was
/// granted at, and the declared variance the key was derived from. Bundled
/// so `lead_render` and `publish` stay within a reasonable argument count
/// rather than threading each field through separately.
struct RenderJob {
    key: RenderKey,
    epoch: u64,
    variance: VarianceDescriptor,
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
    let admission = match runtime.coordinator.admit(&job.key, job.epoch, now).await {
        Ok(admission) => admission,
        Err(_) => return Err(ProviderFailure(request, next)),
    };
    match admission {
        RebuildAdmission::Lead(lease) => {
            Ok(lead_render(runtime, request, next, *lease, policy, job).await)
        }
        RebuildAdmission::Wait(wait) => {
            wait.wait().await;
            match lookup(runtime, &job.key).await {
                Ok(Some((entry, stored, layer))) => {
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
                        match coherence(runtime, &job.key, policy, entry.header()).await {
                            Ok(coherence) => coherence,
                            Err(()) => return Err(ProviderFailure(request, next)),
                        };
                    let now = runtime.now_ms();
                    let state = freshness_state(
                        policy,
                        coherence_result,
                        entry.header().class,
                        stored.published_at_ms,
                        now,
                        entry.header().seed_deadline_ms,
                    );
                    match state {
                        FreshnessState::Fresh => {
                            (match layer {
                                Layer::L0 => LookupOutcome::L0Hit,
                                Layer::L1 => LookupOutcome::L1Hit,
                            })
                            .record();
                            Ok(respond_hit(
                                &request,
                                policy,
                                &entry,
                                stored.published_at_ms,
                                now,
                                None,
                            ))
                        }
                        FreshnessState::StaleServable => {
                            LookupOutcome::Stale.record();
                            Ok(respond_hit(
                                &request,
                                policy,
                                &entry,
                                stored.published_at_ms,
                                now,
                                warning_header(state),
                            ))
                        }
                        // `StaleOnError`'s stale-only-on-provider-failure
                        // behavior is the primary hit path's own concern
                        // (and a separately known gap - out of scope for
                        // this fix); a waiter treats it the same as `Dead`:
                        // neither is safe to serve as a plain hit, so both
                        // fall through to a fresh admission attempt.
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
                            job.epoch = match runtime.ledger.epoch().await {
                                Ok(epoch) => epoch,
                                Err(_) => return Err(ProviderFailure(request, next)),
                            };
                            Box::pin(render_and_publish(
                                runtime,
                                request,
                                next,
                                policy,
                                job,
                                depth + 1,
                            ))
                            .await
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
    let method = request.method().as_str().to_owned();
    let if_none_match = request.header("if-none-match").map(str::to_owned);
    let (response, report, observed) = run_render(runtime, request, next, job.epoch).await;
    let Ok(response) = response else {
        let _ = runtime.coordinator.release(lease).await;
        return response;
    };
    let Some(observed) = observed else {
        // Either the report overflowed (ruling R55: an incomplete
        // dependency set is never storable) or the in-transaction ledger
        // read itself failed; either way there is nothing safe to compare
        // against later, so this candidate is declined here rather than
        // carrying a stand-in forward.
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
            PrivateMaterial::principal(&runtime.keys, id, collector::permission_version())
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
    // of `classify`: an identity-bound island, a `NoStore` document intent,
    // or a public-seed island without a resolvable deadline. This can only
    // decline, never narrow or widen `classification.class` (see
    // `document_declines`'s own doc for why the document's cache intent
    // does not feed classification at all).
    if live::document_declines(report.live_document.as_ref()) {
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
    // the value passed to the value guard or to `build_entry` below.
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

    if let Err(()) = fresh_reread_is_coherent(runtime, &observed, job.epoch).await {
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
    let Some(entry) = build_entry(
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
    store_entry(runtime, &lease, policy, &job, &entry, &observed, now).await;
    let _ = runtime.coordinator.release(lease).await;
    // The client that triggered this render gets its own response back -
    // only the cache validators this middleware adds are attached, rather
    // than a response reconstructed from the stored entry. Reconstructing
    // is unavoidable for a later hit (the original response object no
    // longer exists by then), but here it is gratuitous: the entry's
    // headers are already filtered to the small replayable allowlist (see
    // `build_entry`), so reconstructing on the render itself would silently
    // drop any handler-set header outside that allowlist even on the very
    // request that produced it. See fix round 2, item 2.
    finish_fresh_render(response, if_none_match.as_deref(), policy, &entry)
}

/// The fresh-render counterpart of [`conditional_response`]: serves the
/// handler's own response, untouched, with the cache validators (`ETag`,
/// `Cache-Control`, `Vary`, `Age`) attached - or a body-free 304 when the
/// request's `If-None-Match` already matches what was just rendered. Unlike
/// `conditional_response`, this never reconstructs the body or the
/// non-validator headers from `entry`: for the render that produced `entry`,
/// the handler's own response is the authoritative one. `replace_header` is
/// used for each validator so a value the handler already set (a
/// `Cache-Control` of its own, say) is superseded rather than duplicated.
/// Age is always `0`: this response and `entry` were published from the same
/// instant. Body suppression for `HEAD` is not this function's job - the
/// server strips the body for `HEAD` regardless (see fix round 2, item 7),
/// the same way it would for any handler's response with no cache in play.
fn finish_fresh_render(
    response: HttpResponse,
    if_none_match: Option<&str>,
    policy: &RenderCachePolicy,
    entry: &CompleteEntry,
) -> Response {
    let header = entry.header();
    let not_modified = matches!(
        evaluate_conditional(if_none_match, entry.validator()),
        ConditionalOutcome::NotModified
    );
    let mut out = if not_modified {
        HttpResponse::new().status(304)
    } else {
        response
    };
    out = out.replace_header("ETag", entry.validator().etag());
    // Age is always 0 here (see this function's own doc), so the seed's
    // remaining lifetime at this instant is the deadline minus the very
    // publication time already stored in `header`.
    let seed_remaining = header
        .seed_deadline_ms
        .map(|deadline| deadline.saturating_sub(header.published_at_ms));
    out = out.replace_header(
        "Cache-Control",
        cache_control_value(
            header.class,
            policy.shared(),
            &policy.freshness(),
            seed_remaining,
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
    let declared = job.variance.dimensions();

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
                        collector::permission_version(),
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
        let _ = window.observe(identity.clone());
    }
    window.close(ledger).await.ok()
}

/// Runs `next(request)` under [`Collector::scope`], inside `DB::transaction`
/// when a database is configured, and closes the collector's report into
/// its observed generations (see [`close_window`]) while that transaction
/// is still open, so the render's own reads and the generation reread
/// share one snapshot. `request` is captured through a slot rather than
/// moved directly into the transaction closure, so that if the transaction
/// itself cannot even open (a provider failure, not a route failure), the
/// still-untouched request is recoverable for a plain, uncached render
/// instead of being lost.
async fn run_render(
    runtime: &Arc<RenderCacheRuntime>,
    request: Request,
    next: Next,
    epoch: u64,
) -> (
    Response,
    super::collector::CollectorReport,
    Option<GenerationSet>,
) {
    if !DB::is_connected() {
        return Collector::scope(async move {
            let response = next(request).await;
            let report = collector::current_report().unwrap_or_default();
            let observed = close_window(&report, epoch, runtime.ledger.as_ref()).await;
            (response, report, observed)
        })
        .await;
    }
    let slot: Arc<std::sync::Mutex<Option<Request>>> =
        Arc::new(std::sync::Mutex::new(Some(request)));
    let slot_for_closure = Arc::clone(&slot);
    let next_for_closure = next.clone();
    let ledger_for_closure = Arc::clone(&runtime.ledger);
    let result = DB::transaction(move |_tx| {
        let slot = Arc::clone(&slot_for_closure);
        let next = next_for_closure.clone();
        let ledger = Arc::clone(&ledger_for_closure);
        Box::pin(Collector::scope(async move {
            let request = slot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("the request is taken exactly once, by this closure, when it runs");
            let response = next(request).await;
            let report = collector::current_report().unwrap_or_default();
            let observed = close_window(&report, epoch, ledger.as_ref()).await;
            Ok::<_, crate::FrameworkError>((response, report, observed))
        }))
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
            Collector::scope(async move {
                let response = next(request).await;
                let report = collector::current_report().unwrap_or_default();
                let observed = close_window(&report, epoch, runtime.ledger.as_ref()).await;
                (response, report, observed)
            })
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
    let current = runtime.ledger.current(&digests).await.map_err(|_| ())?;
    let fresh_epoch = runtime.ledger.epoch().await.map_err(|_| ())?;
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

/// Encodes and publishes a coherent candidate to L0 (and L1 when the
/// policy uses it), under a fence minted by the coordinator for this
/// lease. Never fails the request: a publish failure (rejected, fenced, or
/// a provider error) just means the next request stays a miss.
/// Builds the candidate entry a render's response would publish as, or
/// `None` when its headers cannot be safely replayed (an unsafe or
/// non-replayable header, or one exceeding a bound) - in which case the
/// candidate is declined the same as an ineligible or uncacheable one.
fn build_entry(
    job: &RenderJob,
    policy: &RenderCachePolicy,
    class: RepresentationClass,
    observed: &GenerationSet,
    response: &HttpResponse,
    now: u64,
    seed_deadline_ms: Option<u64>,
) -> Option<CompleteEntry> {
    let safe_pairs: Vec<(String, String)> = response
        .headers()
        .filter(|(name, _)| REPLAYABLE_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let safe_headers = SafeHeaders::from_pairs(safe_pairs).ok()?;
    let header = EntryHeader {
        key: job.key.clone(),
        class,
        variance: job.variance.clone(),
        published_at_ms: now,
        fresh_ms: policy.freshness().fresh_ms(),
        stale_servable_ms: policy.freshness().stale_servable_ms(),
        stale_on_error_ms: policy.freshness().stale_on_error_ms(),
        observed: observed.clone(),
        epoch: job.epoch,
        seed_deadline_ms,
        status: 200,
        headers: safe_headers,
        content_encoding: None,
    };
    let body = Bytes::copy_from_slice(response.body());
    Some(CompleteEntry::new(header, body))
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
    entry: &CompleteEntry,
    observed: &GenerationSet,
    now: u64,
) {
    let Ok(mut fence) = runtime.coordinator.publish_token(lease, now).await else {
        return;
    };
    fence.generation_digest = observed.digest();
    let Ok(encoded) = encode(entry, &runtime.keys) else {
        return;
    };
    if let Ok(PublishOutcome::Published) = runtime
        .l0
        .publish(&job.key, encoded.clone(), fence, now)
        .await
    {
        Metrics::counter(render_cache_telemetry::PUBLICATIONS).inc();
    }
    if policy.layers().l1()
        && let Some(l1) = &runtime.l1
    {
        // The total time from publication after which this entry is dead
        // by every freshness band (see
        // `suprnova_live::render_cache::coherence::evaluate_freshness`),
        // so `FileRenderStore::sweep` knows when the file is safe to
        // remove. This is the one call site with a policy in scope to
        // compute a real retention from; every other L1 caller (including
        // `file_store.rs`'s own tests) uses the generic
        // `RenderStore::publish`, which always frames zero.
        let retention_ms = entry
            .header()
            .fresh_ms
            .saturating_add(entry.header().stale_on_error_ms);
        let _ = l1
            .publish_with_retention(&job.key, encoded, fence, now, retention_ms)
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
                    collector::permission_version(),
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
        build: BuildId::parse(&runtime.config.build_id)
            .unwrap_or_else(|_| BuildId::parse("default").expect("'default' is a valid build id")),
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
    /// list), so [`fire`] runs on every GET/HEAD to a policy-covered route
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

    /// Fires from [`super::fresh_reread_is_coherent`], immediately after it
    /// finds the render still coherent and before its caller acts on that
    /// result - the exact window a write must land in to be "after the
    /// reread" and "before publication": late enough that it cannot itself
    /// be caught by this same reread, early enough that the entry this
    /// request publishes still carries the observations from before it.
    pub static AFTER_REREAD: RacePoint = RacePoint::new();

    /// Fires from [`super::RenderCacheMiddleware::serve`], immediately
    /// after the epoch a new [`super::RenderJob`] will carry is read, and
    /// before the render that job describes begins - the exact window an
    /// epoch advance must land in to be baked into the job as stale by the
    /// time that render's own fresh reread checks it. Fires on whichever
    /// *request* reaches that point next, not necessarily the next
    /// *render*: `serve` reads the epoch before it knows whether the
    /// request will be a hit, a stale serve, or a render (fix round 1, F7).
    pub static EPOCH_CAPTURED: RacePoint = RacePoint::new();

    /// Arms `point` to run `hook` exactly once, the next time it fires.
    /// Replaces any hook already armed there.
    ///
    /// Fix round 2, N1: the hook write and the `armed` store are one
    /// critical section (both happen while `slot` is still locked), the
    /// same as [`disarm`] and [`fire`] below. Storing `armed` outside the
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
    /// flag [`fire`] checks. Fix round 1, F4: `AFTER_REREAD` only fires on
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
