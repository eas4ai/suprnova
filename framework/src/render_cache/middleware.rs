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
//! # Deliberately out of scope for this task
//!
//! - **Seed promotion deadlines.** `EntryHeader::seed_deadline_ms` is
//!   always `None` here. Reading a Live document's embedded public seed
//!   deadline is the Live-integration task this iteration's plan lists
//!   separately from the middleware; wiring it in here would reach past
//!   this task's subject.
//! - **Tenant-observed narrowing.** [`suprnova_live::render_cache::variance::ObservedContext::tenant`]
//!   is always `None`. Unlike a principal read, there is no collector hook
//!   that reports "rendering touched tenant-scoped state" - only whether a
//!   tenant was resolved for the request at all
//!   (`Request::live_tenant()`), which is a fact about the request, not an
//!   observation about what rendering did with it. Treating "a tenant
//!   exists" as "rendering used it" would force every route in any
//!   multi-tenant application to `PrivateCached`, regardless of whether it
//!   declared `Tenant` variance - `Tenant` variance stays available and
//!   correct at the key-derivation step, opted into per route.
//! - **Background rebuild's synthetic request.** The stale-service
//!   background rebuild reruns the original `Request` value rather than a
//!   freshly built one with cookies stripped: this codebase's `Request`
//!   has no public constructor from parts plus a hyper request the way
//!   this would need, and building one is a change to `http::request`,
//!   not to this middleware. If the background render happens to observe a
//!   principal from the carried cookie on a route that never declared
//!   `Principal` variance, [`key_omits_observed_privacy`] declines to
//!   store the result - narrowing the served class alone would not be
//!   enough, since narrowing never repartitions the key that was already
//!   derived before the render ran (see that function's own doc). So this
//!   is a possible wasted or misdirected rebuild, not a correctness or
//!   security defect - flagged here rather than silently matched to the
//!   ideal.

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
    CoherenceCheck, DependencyIdentity, GenerationLedger, GenerationSet, ObservationWindow,
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
    DimensionValue, ObservedContext, PrivateMaterial, VarianceDescriptor, VarianceDimension,
    classify,
};
use suprnova_live::render_cache::{FailurePolicy, RepresentationClass};

use crate::database::DB;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};
use crate::telemetry::metrics::Metrics;
use crate::{Auth, Lang};

use super::collector::{self, Collector};
use super::config::RenderCacheConfig;
use super::file_store::FileRenderStore;
use super::registry::RenderCachePolicyTable;
use super::telemetry as render_cache_telemetry;

/// Domain separator for the route identity digest this middleware derives
/// from a registered route pattern. Independent of Live's own internal
/// route-identity digest (a different digest, for a different purpose):
/// purpose separation keeps the two from ever being compared against each
/// other by accident, even though both ultimately hash a route pattern.
const ROUTE_IDENTITY_DOMAIN: &[u8] = b"suprnova/render-cache/route-identity/v1\0";

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
pub struct RenderCacheMiddleware {
    runtime: Arc<RenderCacheRuntime>,
}

impl RenderCacheMiddleware {
    /// Wraps an assembled runtime. `pub(crate)`: constructed only by
    /// [`super::RenderCache::install`].
    pub(crate) fn new(runtime: Arc<RenderCacheRuntime>) -> Self {
        Self { runtime }
    }
}

/// The assembled RenderCache runtime: stores, ledger, coordinator, keys,
/// policy table, configuration, and clock. One instance per installed
/// process; [`super::RenderCache::runtime`] hands out clones of the `Arc`.
pub struct RenderCacheRuntime {
    pub(crate) config: RenderCacheConfig,
    pub(crate) table: RenderCachePolicyTable,
    pub(crate) l0: MemoryRenderStore,
    pub(crate) l1: Option<FileRenderStore>,
    pub(crate) ledger: Arc<dyn GenerationLedger>,
    pub(crate) coordinator: Arc<dyn RebuildCoordinator>,
    pub(crate) keys: SnapshotKeyRing,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) limits: EntryLimits,
    /// Bounded local validation leases for [`CoherenceMode::Lease`] routes,
    /// keyed by the entry's lookup key. Bounded in practice, not in code: a
    /// key that stops being requested stops being touched here, so the map
    /// only ever holds as many entries as there are distinct lease-mode
    /// keys actually in active use.
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
        let Some(pattern) = request.route_pattern().map(str::to_owned) else {
            return next(request).await;
        };
        if !self.runtime.config.enabled || !matches!(request.method().as_str(), "GET" | "HEAD") {
            return next(request).await;
        }
        let Some(policy) = self.runtime.table.effective_policy(&pattern) else {
            return next(request).await;
        };
        match self.serve(request, next, &pattern, &policy).await {
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
        request: Request,
        next: Next,
        pattern: &str,
        policy: &RenderCachePolicy,
    ) -> Result<Response, ProviderFailure> {
        if !declared_query_ok(&request, policy) {
            LookupOutcome::Bypass.record();
            return Ok(next(request).await);
        }
        let epoch = match self.runtime.ledger.epoch().await {
            Ok(epoch) => epoch,
            Err(_) => return Err(ProviderFailure(request, next)),
        };
        let input = key_input(&self.runtime, &request, pattern, policy, epoch);
        let variance = input.variance.clone();
        let Ok(key) = RenderKey::derive(&input, &self.runtime.keys) else {
            LookupOutcome::Bypass.record();
            return Ok(next(request).await);
        };

        let hit = match lookup(&self.runtime, &key).await {
            Ok(hit) => hit,
            Err(()) => return Err(ProviderFailure(request, next)),
        };
        let Some((entry, stored, layer)) = hit else {
            LookupOutcome::Miss.record();
            return render_and_publish(&self.runtime, request, next, key, policy, epoch, variance)
                .await;
        };

        let coherence = match coherence(&self.runtime, &key, policy, entry.header()).await {
            Ok(coherence) => coherence,
            Err(()) => return Err(ProviderFailure(request, next)),
        };
        let now = self.runtime.now_ms();
        let state = freshness_state(
            policy,
            coherence,
            entry.header().class,
            stored.published_at_ms,
            now,
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
                self.spawn_background_rebuild(request, next, key, policy.clone(), epoch, variance);
                Ok(response)
            }
            FreshnessState::StaleOnError => {
                LookupOutcome::Miss.record();
                match render_and_publish(&self.runtime, request, next, key, policy, epoch, variance)
                    .await
                {
                    Ok(response) => Ok(response),
                    Err(ProviderFailure(request, _next)) => Ok(respond_hit(
                        &request,
                        policy,
                        &entry,
                        stored.published_at_ms,
                        now,
                        warning_header(FreshnessState::StaleOnError),
                    )),
                }
            }
            FreshnessState::Dead => {
                LookupOutcome::Miss.record();
                render_and_publish(&self.runtime, request, next, key, policy, epoch, variance).await
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
        request: Request,
        next: Next,
        key: RenderKey,
        policy: RenderCachePolicy,
        epoch: u64,
        variance: VarianceDescriptor,
    ) {
        let runtime = Arc::clone(&self.runtime);
        Metrics::counter(render_cache_telemetry::REBUILDS).inc();
        tokio::spawn(async move {
            let _ =
                render_and_publish(&runtime, request, next, key, &policy, epoch, variance).await;
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
fn variance_descriptor(
    runtime: &RenderCacheRuntime,
    request: &Request,
    policy: &RenderCachePolicy,
) -> VarianceDescriptor {
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
                // No producer yet for these (see collector.rs's module
                // doc); declaring one here would spend the bound for a
                // value that never changes.
                continue;
            }
        };
        let _ = variance.declare(dimension.clone(), value);
    }
    variance
}

/// Builds the lookup key input for `request` against `policy`. Callers
/// must have already confirmed [`declared_query_ok`].
fn key_input(
    runtime: &RenderCacheRuntime,
    request: &Request,
    pattern: &str,
    policy: &RenderCachePolicy,
    epoch: u64,
) -> RenderKeyInput {
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
    RenderKeyInput {
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
        variance: variance_descriptor(runtime, request, policy),
    }
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
        if leased {
            return Ok(Coherence::Coherent);
        }
        let result = authority_coherence(runtime, header).await?;
        if result == Coherence::Coherent {
            runtime
                .leases
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.clone(), ValidationLease::grant(now, max_age_ms));
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
) -> FreshnessState {
    if coherence == Coherence::Coherent {
        return evaluate_freshness(&policy.freshness(), class, published_at_ms, now_ms);
    }
    let age = now_ms.saturating_sub(published_at_ms);
    let effective_age = age.max(policy.freshness().fresh_ms());
    let synthetic_now = published_at_ms.saturating_add(effective_age);
    evaluate_freshness(&policy.freshness(), class, published_at_ms, synthetic_now)
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
    response = response.header(
        "Cache-Control",
        cache_control_value(header.class, policy.shared(), &policy.freshness(), None),
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
    key: RenderKey,
    policy: &RenderCachePolicy,
    epoch: u64,
    variance: VarianceDescriptor,
) -> Result<Response, ProviderFailure> {
    let now = runtime.now_ms();
    let admission = match runtime.coordinator.admit(&key, epoch, now).await {
        Ok(admission) => admission,
        Err(_) => return Err(ProviderFailure(request, next)),
    };
    match admission {
        RebuildAdmission::Lead(lease) => {
            let job = RenderJob {
                key,
                epoch,
                variance,
            };
            Ok(lead_render(runtime, request, next, *lease, policy, job).await)
        }
        RebuildAdmission::Wait(wait) => {
            wait.wait().await;
            match lookup(runtime, &key).await {
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
                        match coherence(runtime, &key, policy, entry.header()).await {
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
                            let epoch = match runtime.ledger.epoch().await {
                                Ok(epoch) => epoch,
                                Err(_) => return Err(ProviderFailure(request, next)),
                            };
                            Box::pin(render_and_publish(
                                runtime, request, next, key, policy, epoch, variance,
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
    let observed_context = ObservedContext {
        principal: if report.context.principal_read {
            Auth::id().map(|id| {
                PrivateMaterial::principal(&runtime.keys, &id, collector::permission_version())
            })
        } else {
            None
        },
        // See the module doc: no producer for "rendering touched tenant
        // state" exists yet, so tenant narrowing is not applied here.
        tenant: None,
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
    if key_omits_observed_privacy(&job, &observed_context, &report) {
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
    let Some(entry) = build_entry(
        &job,
        policy,
        classification.class,
        &observed,
        &response,
        now,
    ) else {
        LookupOutcome::Declined.record();
        let _ = runtime.coordinator.release(lease).await;
        return Ok(response);
    };
    store_entry(runtime, &lease, policy, &job, &entry, &observed, now).await;
    let _ = runtime.coordinator.release(lease).await;
    // The client that triggered this render sees the same headers a
    // subsequent hit would: an entry the middleware just proved safe to
    // store is, by construction, safe to serve with its validators too.
    conditional_response(
        &method,
        if_none_match.as_deref(),
        policy,
        &entry,
        now,
        now,
        None,
    )
}

/// Whether the render observed something private, or a locale, that the
/// lookup key's declared variance does not account for.
///
/// Classification narrows the served class and the `Cache-Control`/staleness
/// rules, but it cannot repartition the lookup key: the key was already
/// derived, before this render ran, from the route's *declared* variance
/// alone (see [`key_input`]). A route declared `PublicShared` with no
/// `Principal` variance, whose handler reads an identity out of
/// `auth::request_state` (bearer-token or remember-me authentication,
/// rather than a session read - a session read already forces
/// `Uncacheable` through `session_read`) would otherwise store its render
/// under one shared, principal-free key and serve it back to a different
/// signed-in visitor, or an anonymous one. The same shape applies to a
/// route that renders translated content without declaring `Locale`
/// variance: one language would be cached for everyone. Declining to store
/// here - not merely narrowing the class - is the fix; see ruling on this
/// task's fix round 1, item 1.
fn key_omits_observed_privacy(
    job: &RenderJob,
    observed_context: &ObservedContext,
    report: &super::collector::CollectorReport,
) -> bool {
    let declared = job.variance.dimensions();
    if observed_context.principal.is_some() && !declared.contains_key(&VarianceDimension::Principal)
    {
        return true;
    }
    if observed_context.tenant.is_some() && !declared.contains_key(&VarianceDimension::Tenant) {
        return true;
    }
    let locale_observed = report.storable().is_some_and(|identities| {
        identities
            .iter()
            .any(|identity| matches!(identity, DependencyIdentity::Locale))
    });
    locale_observed && !declared.contains_key(&VarianceDimension::Locale)
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
        CoherenceCheck::Coherent => Ok(()),
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
        seed_deadline_ms: None,
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
        let _ = l1.publish(&job.key, encoded, fence, now).await;
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
