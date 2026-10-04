//! [`CachedEvaluator`] - TTL-bounded memoization in front of any
//! [`Evaluator`].
//!
//! Wraps an inner evaluator (typically [`DatabaseEvaluator`](super::database::DatabaseEvaluator))
//! with a process-local [`DashMap`] cache keyed by
//! `(feature, user_id, team)`. The cache's lookup path is fully
//! synchronous - matching featureflag's [`Evaluator::is_enabled`]
//! contract - so the hot path stays lock-free for concurrent
//! readers and never blocks on an async runtime.
//!
//! # When to use this
//!
//! [`DatabaseEvaluator`](super::database::DatabaseEvaluator) already snapshots flags into an in-memory
//! `HashMap` on construction and reload, so per-request DB queries
//! aren't a concern. `CachedEvaluator` exists to memoize the result
//! of the **scope-resolution walk** (build candidate keys, look each
//! up, fall back to global) when that walk's cost ever becomes
//! material - e.g. an evaluator chain whose links are not all
//! `DatabaseEvaluator`, or a custom evaluator whose `is_enabled`
//! computation is non-trivial.
//!
//! # Cross-replica coherence
//!
//! The cache is per-process. Flag changes on one replica are visible
//! to other replicas as soon as their inner evaluator reloads - there
//! is no cross-cluster cache-coherence protocol in v1. The cache TTL
//! therefore bounds the worst-case staleness across the cluster.
//! Callers who need millisecond propagation should either:
//!
//! * lower the TTL toward zero (and accept the cost of skipping the
//!   memoization), or
//! * call [`CachedEvaluator::invalidate`] from the admin-CRUD path
//!   that mutated the flag (Phase 13 Task 6 - admin handlers will
//!   wire this).
//!
//! # Why DashMap + manual TTL (not our Cache facade)
//!
//! The `Cache` facade is async by design - it has to be, to support
//! Redis as a backend. featureflag's `Evaluator::is_enabled` is sync.
//! Bridging the two via `block_on` inside `is_enabled` would tank
//! request throughput. The right reconciliation is two layers: a
//! sync per-process cache (this struct) for hot reads, and a
//! background invalidator that subscribes to a cross-process channel
//! and clears local entries - the invalidator is out of scope for
//! v1 since flag changes are operator-initiated, infrequent, and
//! already bounded by the TTL.

use crate::features::fields::{
    IdentityScopes, TeamField, UserIdField, capturing_identity_reads, observe_feature_read,
};
use crate::features::sync::FeatureSync;
use async_trait::async_trait;
use dashmap::DashMap;
use featureflag::{context::Context, evaluator::Evaluator};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The most entries the cache holds. The insert that reaches it frees room
/// down to half of it: first every entry whose age is `>= ttl` (it would be
/// re-fetched on its next read anyway), then, if that is not enough, the
/// oldest live entries.
///
/// Both halves matter. Dropping only expired entries could not bound a
/// high-cardinality or attacker-influenced `user_id`/`team` stream whose
/// entries are all still live, and a map stuck at the threshold was swept
/// in full on every later miss. Freeing down to half means each sweep pays
/// for many inserts, so the cost stays amortised. Evicting a live entry
/// costs only a later re-evaluation, never a wrong answer. Sized so a
/// normally-scoped workload (a bounded set of users/teams) never reaches it.
const SWEEP_THRESHOLD: usize = 4096;

/// TTL-cached wrapper around any [`Evaluator`].
pub struct CachedEvaluator {
    inner: Arc<dyn Evaluator + Send + Sync>,
    ttl: Duration,
    /// Keyed by the structured `(feature, user, team)` triple. A string
    /// key made by joining the three was ambiguous: identity values are
    /// unrestricted strings, so two different identities could join to the
    /// same key and share one cached decision.
    ///
    /// Growth is bounded by [`SWEEP_THRESHOLD`]: the insert that reaches
    /// it frees room down to half, expired entries first and the oldest
    /// live ones after, so an unbounded stream of distinct scopes can't
    /// leak memory.
    cache: DashMap<CacheKey, CacheEntry>,
    /// Set while one thread frees room, so concurrent misses do not all
    /// walk the map at once; they insert and move on.
    sweeping: AtomicBool,
    /// How many times each feature has been invalidated. A miss records the
    /// count it started under and its entry is served only while the count
    /// is unchanged, so an evaluation that overlapped an invalidation cannot
    /// leave its answer behind for later reads.
    invalidations: DashMap<String, u64>,
    /// How many times [`CachedEvaluator::invalidate_all`] has run; the same
    /// guard for every feature at once.
    invalidated_all: AtomicU64,
    /// How many times the full-map sweep ran, for the tests that bound it.
    #[cfg(test)]
    sweeps: std::sync::atomic::AtomicUsize,
}

/// The scope one cached answer belongs to.
///
/// An absent field and an empty one stay distinct (`None` versus
/// `Some("")`), because scope resolution treats them differently.
#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    feature: String,
    user: Option<String>,
    team: Option<String>,
}

/// The invalidation counts a miss started under: the feature's own, and
/// the [`CachedEvaluator::invalidate_all`] count.
#[derive(Copy, Clone, PartialEq, Eq)]
struct Generation {
    feature: u64,
    all: u64,
}

#[derive(Copy, Clone)]
struct CacheEntry {
    value: Option<bool>,
    inserted_at: Instant,
    /// The invalidation counts in force when the evaluation that produced
    /// this entry started.
    generation: Generation,
    /// Which identity axes the inner evaluation that produced this entry
    /// consulted, replayed on every hit that serves it (fix round 7,
    /// finding 2).
    ///
    /// A hit never reaches `inner`, so it can never learn the flag's scope
    /// for itself; it has to be told. Two bits rather than the observed
    /// values themselves because this entry's own cache key already
    /// contains the `(user, team)` it was stored under, so any context that
    /// can hit it carries the same identity the miss saw - the values are
    /// re-derivable from the context, the *scopes* are not.
    identity: IdentityScopes,
}

impl CachedEvaluator {
    /// Construct a new cached evaluator with the given TTL. A TTL of
    /// zero degenerates to "no caching" - every call falls through
    /// to `inner`. A very long TTL bounds the cross-replica staleness
    /// window; tune to taste.
    pub fn new(inner: Arc<dyn Evaluator + Send + Sync>, ttl: Duration) -> Self {
        Self {
            inner,
            ttl,
            cache: DashMap::new(),
            invalidations: DashMap::new(),
            invalidated_all: AtomicU64::new(0),
            sweeping: AtomicBool::new(false),
            #[cfg(test)]
            sweeps: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Reference to the underlying evaluator. Exposed for tests and
    /// for callers that need to dispatch a cache-bypassed lookup
    /// (e.g. admin tooling rendering "current vs cached" diffs).
    pub fn inner(&self) -> &Arc<dyn Evaluator + Send + Sync> {
        &self.inner
    }

    /// Drop every cached entry for a specific feature name. Intended
    /// for the admin-CRUD path: after [`DatabaseEvaluator::set_flag`](super::database::DatabaseEvaluator::set_flag)
    /// mutates a flag, callers invalidate the corresponding cached
    /// entries so the next `is_enabled` re-reads the snapshot.
    pub fn invalidate(&self, feature: &str) {
        // Count first, then drop. A miss that read the old count before
        // this line still inserts afterwards, but under that old count, so
        // no read serves it; a miss that reads the new count started after
        // the caller's flag change and evaluates the new state.
        *self.invalidations.entry(feature.to_string()).or_insert(0) += 1;
        self.cache.retain(|key, _| key.feature != feature);
    }

    /// Drop every cached entry. Use sparingly - typically only on a
    /// bulk admin reload or in tests.
    pub fn invalidate_all(&self) {
        // Same order and reason as `invalidate`.
        self.invalidated_all.fetch_add(1, Ordering::SeqCst);
        self.cache.clear();
    }

    /// Shrink the map to at most half of [`SWEEP_THRESHOLD`]: drop expired
    /// entries, then the oldest live ones if that was not enough.
    fn make_room(&self, now: Instant) {
        if self
            .sweeping
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Another miss is already freeing room.
            return;
        }
        #[cfg(test)]
        self.sweeps.fetch_add(1, Ordering::Relaxed);

        self.cache
            .retain(|_, entry| now.duration_since(entry.inserted_at) < self.ttl);
        let target = SWEEP_THRESHOLD / 2;
        let mut ages: Vec<Instant> = self.cache.iter().map(|entry| entry.inserted_at).collect();
        if ages.len() > target {
            // The newest `target` entries survive. Entries that share the
            // cut-off instant all go, which only frees a little more.
            let excess = ages.len() - target;
            let (_, cutoff, _) = ages.select_nth_unstable(excess - 1);
            let cutoff = *cutoff;
            self.cache.retain(|_, entry| entry.inserted_at > cutoff);
        }

        self.sweeping.store(false, Ordering::Release);
    }

    /// The invalidation counts in force for `feature` right now.
    fn generation(&self, feature: &str) -> Generation {
        Generation {
            all: self.invalidated_all.load(Ordering::SeqCst),
            feature: self.invalidations.get(feature).map_or(0, |count| *count),
        }
    }

    /// Number of entries currently held. Useful for tests + admin
    /// telemetry; not load-bearing.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Test convenience.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    fn cache_key(feature: &str, context: &Context) -> CacheKey {
        let user = context
            .iter()
            .find_map(|c| c.extensions().get::<UserIdField>())
            .map(|field| field.as_str().to_string());
        let team = context
            .iter()
            .find_map(|c| c.extensions().get::<TeamField>())
            .map(|field| field.as_str().to_string());
        CacheKey {
            feature: feature.to_string(),
            user,
            team,
        }
    }
}

#[async_trait]
impl FeatureSync for CachedEvaluator {
    /// Drops every cached entry for `feature` (all scopes). The
    /// `scope_key` argument is currently ignored - entries are keyed
    /// by `(feature, user, team)` and the user/team scope isn't
    /// derivable from the bare `scope_key` string, so we invalidate
    /// the whole feature prefix. For per-scope invalidation, an app
    /// would need a custom cache impl with a richer key.
    async fn on_flag_changed(&self, feature: &str, _scope_key: &str) {
        self.invalidate(feature);
    }

    /// Drops every cached entry for each changed feature. This is the half
    /// a reload needs that `on_flag_changed` cannot give it: a reload knows
    /// a set of names and no scope key at all.
    async fn on_snapshot_reloaded(&self, changed: &[String]) {
        for feature in changed {
            self.invalidate(feature);
        }
    }
}

impl Evaluator for CachedEvaluator {
    fn is_enabled(&self, feature: &str, context: &Context) -> Option<bool> {
        // TTL=0 short-circuits the cache entirely. Avoids the
        // insert+evict churn that would otherwise dominate when the
        // caller doesn't want caching. Nothing is stored, so nothing has to
        // be replayed later: `inner` does its own observing.
        if self.ttl.is_zero() {
            return self.inner.is_enabled(feature, context);
        }

        let key = Self::cache_key(feature, context);

        // Fast path: live entry, produced under the invalidation counts in
        // force now.
        if let Some(found) = self.cache.get(&key)
            && found.inserted_at.elapsed() < self.ttl
            && found.generation == self.generation(feature)
        {
            let entry = *found;
            // Released before the replay below: nothing in
            // `observe_feature_read` touches this map today, and holding a
            // shard guard across a call into another module is how that
            // stops being true.
            drop(found);
            // Fix round 6, Leak 4, narrowed by fix round 7: a cached answer
            // for a scoped flag is exactly as identity-dependent as a fresh
            // one, and this hit never reaches `self.inner`, so it replays
            // the axes the miss's own evaluation consulted. See
            // `crate::features::fields::observe_feature_read`'s own doc.
            observe_feature_read(feature, entry.identity, context);
            return entry.value;
        }

        // Miss or expired - consult inner and store the result. We
        // store None values too: "feature not configured" is itself
        // a stable answer worth caching to avoid re-walking the
        // scope chain on every request.
        //
        // The capture is what makes the replay above exact. It records what
        // this evaluation *asked* to observe, not what the render-cache
        // collector's sets gained across the call: an earlier accessor in
        // the same render may already hold the same value, and the miss may
        // happen with no collector active at all, and either would make a
        // difference-based record store nothing where the flag genuinely is
        // identity-dependent.
        //
        // The generation is read before `inner` runs, so an invalidation
        // that lands during the evaluation marks this answer stale.
        let generation = self.generation(feature);
        let (value, identity) =
            capturing_identity_reads(|| self.inner.is_enabled(feature, context));
        if generation != self.generation(feature) {
            // Invalidated while evaluating: this answer may predate the
            // change, so serve it to this caller only and cache nothing.
            return value;
        }
        let now = Instant::now();
        self.cache.insert(
            key,
            CacheEntry {
                value,
                inserted_at: now,
                generation,
                identity,
            },
        );

        // Bounded-growth backstop; see `SWEEP_THRESHOLD`.
        if self.cache.len() >= SWEEP_THRESHOLD {
            self.make_room(now);
        }

        value
    }

    fn on_new_context(
        &self,
        context: featureflag::context::ContextRef<'_>,
        fields: featureflag::fields::Fields<'_>,
    ) {
        // Pass through to the inner evaluator so the same field-to-
        // extension translation runs once per context creation. This
        // keeps the cached wrapper transparent: from the caller's
        // perspective, switching DatabaseEvaluator for
        // CachedEvaluator(DatabaseEvaluator) changes only the cache
        // behaviour, not the field-resolution behaviour.
        self.inner.on_new_context(context, fields);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_cache::collector::CollectedContext;
    use featureflag::context::Context;
    use featureflag::evaluator::with_default;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Build a context scoped to a distinct user id. The installed
    /// [`TranslatingEvaluator`] turns the `user_id` field into a
    /// `UserIdField` extension during `on_new_context`, so each id
    /// produces a distinct [`CachedEvaluator`] cache key.
    fn ctx_for_user(id: usize) -> Context {
        featureflag::context! { user_id = format!("user-{id}") }
    }

    /// Inner evaluator that counts how many times `is_enabled` was
    /// actually invoked. Lets tests assert cache hit/miss behaviour
    /// without relying on timing.
    struct CountingEvaluator {
        return_value: Option<bool>,
        calls: AtomicU32,
    }

    impl CountingEvaluator {
        fn new(return_value: Option<bool>) -> Self {
            Self {
                return_value,
                calls: AtomicU32::new(0),
            }
        }
        fn call_count(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Evaluator for CountingEvaluator {
        fn is_enabled(&self, _feature: &str, _context: &Context) -> Option<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.return_value
        }
    }

    /// Inner evaluator that observes a fixed set of identity axes, the way
    /// [`DatabaseEvaluator`](super::super::database::DatabaseEvaluator) does
    /// for a flag with rules at those scopes, and counts how many times it
    /// was actually reached.
    struct ScopedEvaluator {
        scopes: IdentityScopes,
        calls: AtomicU32,
    }

    impl ScopedEvaluator {
        fn new(scopes: IdentityScopes) -> Self {
            Self {
                scopes,
                calls: AtomicU32::new(0),
            }
        }
        fn call_count(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Evaluator for ScopedEvaluator {
        fn is_enabled(&self, feature: &str, context: &Context) -> Option<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            crate::features::fields::observe_feature_read(feature, self.scopes, context);
            Some(true)
        }

        fn on_new_context(
            &self,
            mut context: featureflag::context::ContextRef<'_>,
            fields: featureflag::fields::Fields<'_>,
        ) {
            if let Some(id) = fields.get("user_id").and_then(|v| v.as_str()) {
                context.extensions_mut().insert(UserIdField(id.to_string()));
            }
            if let Some(team) = fields.get("team").and_then(|v| v.as_str()) {
                context.extensions_mut().insert(TeamField(team.to_string()));
            }
        }
    }

    /// What one `is_enabled` call records into a fresh collector - the whole
    /// context, so a test can assert on the bare `*_read` flags as well as on
    /// the observed material.
    async fn observations_of(
        cached: &Arc<CachedEvaluator>,
        feature: &str,
        build_context: impl FnOnce() -> Context,
    ) -> CollectedContext {
        crate::render_cache::collector::Collector::scope(async {
            // These reads stand in for what a handler reads mid-render, so
            // attribute them to the content bucket the way the Live
            // completion middleware does on a real request.
            crate::render_cache::collector::begin_handler();
            with_default(cached.clone(), || {
                let ctx = build_context();
                cached.is_enabled(feature, &ctx);
            });
            crate::render_cache::collector::current_report()
                .expect("a collector is active")
                .context
        })
        .await
    }

    /// The identity-carrying context these tests replay through.
    fn alice_of_alpha() -> Context {
        featureflag::context! { user_id = "alice", team = "alpha" }
    }

    /// Fix round 6, Leak 4, as fix round 7 rebuilt it. A `CachedEvaluator`
    /// cache *hit* never reaches `self.inner`, so instrumenting only
    /// `DatabaseEvaluator::is_enabled` would miss every repeated flag check
    /// after the first - and a scoped flag's cached answer is exactly as
    /// identity-dependent as a fresh one.
    ///
    /// The miss and the hit run inside **separate collector scopes**, which
    /// is what makes this prove the hit path rather than the miss path: the
    /// second scope's report starts empty, `inner` is never reached (the
    /// call count proves it), and the values still have to be there.
    #[tokio::test]
    async fn a_cache_hit_replays_the_identity_axes_the_miss_consulted() {
        let inner = Arc::new(ScopedEvaluator::new(IdentityScopes {
            principal: true,
            tenant: true,
            known: false,
        }));
        let cached = Arc::new(CachedEvaluator::new(inner.clone(), Duration::from_secs(60)));

        let observed = observations_of(&cached, "flag", alice_of_alpha).await;
        let (principal, tenant) = (observed.principal_material, observed.tenant_material);
        assert_eq!(inner.call_count(), 1, "the first call is a miss");
        assert!(
            principal.contains("alice") && tenant.contains("alpha"),
            "the miss records what the inner evaluation consulted - got principal \
             {principal:?}, tenant {tenant:?}"
        );

        let observed = observations_of(&cached, "flag", alice_of_alpha).await;
        let (principal, tenant) = (observed.principal_material, observed.tenant_material);
        assert_eq!(
            inner.call_count(),
            1,
            "the second call must be served from the cache, never reaching inner"
        );
        assert!(
            principal.contains("alice"),
            "a cache hit must replay the principal axis the miss consulted, in a render \
             that never touched the miss - got {principal:?}"
        );
        assert!(
            tenant.contains("alpha"),
            "a cache hit must replay the tenant axis the miss consulted - got {tenant:?}"
        );
    }

    /// Fix round 8, finding 5, at the evaluator level. A hit whose captured
    /// axes say "principal consulted" and whose context carries no
    /// `UserIdField` must emit the same **bare read** the miss did. The miss
    /// and the hit run in separate collector scopes, so the second scope's
    /// report starts empty and `inner` is never reached (the call count
    /// proves it): the flag can only be set because the hit replayed it.
    ///
    /// This is the seam the round's brief asked to check: the replay and the
    /// miss both go through `fields::observe_feature_read`, the one function
    /// that decides value-or-bare-read, so they cannot disagree about an
    /// absent field.
    #[tokio::test]
    async fn a_cache_hit_replays_a_bare_read_when_the_context_carries_no_field() {
        let inner = Arc::new(ScopedEvaluator::new(IdentityScopes {
            principal: true,
            tenant: true,
            known: false,
        }));
        let cached = Arc::new(CachedEvaluator::new(inner.clone(), Duration::from_secs(60)));

        let observed = observations_of(&cached, "flag", Context::root).await;
        assert_eq!(inner.call_count(), 1, "the first call is a miss");
        assert!(
            observed.principal_read && observed.tenant_read,
            "the miss records both axes as read even with no field to name"
        );
        assert!(
            observed.principal_material.is_empty() && observed.tenant_material.is_empty(),
            "there is nothing to name - got principal {:?}, tenant {:?}",
            observed.principal_material,
            observed.tenant_material
        );

        let observed = observations_of(&cached, "flag", Context::root).await;
        assert_eq!(
            inner.call_count(),
            1,
            "the second call must be served from the cache, never reaching inner"
        );
        assert!(
            observed.principal_read,
            "a cache hit must replay the bare principal read the miss emitted, in a \
             render that never touched the miss"
        );
        assert!(observed.tenant_read, "and the bare tenant read with it");
        assert!(
            observed.principal_material.is_empty() && observed.tenant_material.is_empty(),
            "the replay names no value it does not have - got principal {:?}, tenant {:?}",
            observed.principal_material,
            observed.tenant_material
        );
    }

    /// The other direction, and the whole point of fix round 7's finding 2:
    /// a flag whose inner evaluation consulted no identity records nothing,
    /// on the miss or on any hit that follows it. Recording unconditionally
    /// is what made every page uncacheable for every signed-in visitor.
    #[tokio::test]
    async fn a_cache_hit_replays_nothing_when_the_miss_consulted_no_identity() {
        let inner = Arc::new(ScopedEvaluator::new(IdentityScopes::default()));
        let cached = Arc::new(CachedEvaluator::new(inner.clone(), Duration::from_secs(60)));

        let observed = observations_of(&cached, "flag", alice_of_alpha).await;
        assert!(
            observed.principal_material.is_empty() && observed.tenant_material.is_empty(),
            "got principal {:?}, tenant {:?}",
            observed.principal_material,
            observed.tenant_material
        );
        assert!(
            !observed.principal_read && !observed.tenant_read,
            "not even a bare read: the inner evaluation consulted no axis"
        );

        let observed = observations_of(&cached, "flag", alice_of_alpha).await;
        assert_eq!(inner.call_count(), 1, "the second call is a hit");
        assert!(
            observed.principal_material.is_empty() && observed.tenant_material.is_empty(),
            "a hit on a globally scoped flag must record nothing either - got principal \
             {:?}, tenant {:?}",
            observed.principal_material,
            observed.tenant_material
        );
        assert!(
            !observed.principal_read && !observed.tenant_read,
            "and no bare read on the hit either"
        );
    }

    #[test]
    fn cache_hits_on_second_call_with_same_context() {
        let inner = Arc::new(CountingEvaluator::new(Some(true)));
        let cached = CachedEvaluator::new(inner.clone(), Duration::from_secs(60));

        with_default(Arc::new(NoopEvaluator), || {
            let ctx = Context::root();
            assert_eq!(cached.is_enabled("flag", &ctx), Some(true));
            assert_eq!(cached.is_enabled("flag", &ctx), Some(true));
        });

        assert_eq!(
            inner.call_count(),
            1,
            "second call must come from the cache; inner saw {} calls",
            inner.call_count()
        );
    }

    #[test]
    fn ttl_expiry_falls_through_to_inner() {
        let inner = Arc::new(CountingEvaluator::new(Some(false)));
        let cached = CachedEvaluator::new(inner.clone(), Duration::from_millis(20));

        with_default(Arc::new(NoopEvaluator), || {
            let ctx = Context::root();
            assert_eq!(cached.is_enabled("flag", &ctx), Some(false));
            std::thread::sleep(Duration::from_millis(40));
            assert_eq!(cached.is_enabled("flag", &ctx), Some(false));
        });

        assert_eq!(
            inner.call_count(),
            2,
            "second call after TTL expiry must re-hit inner"
        );
    }

    #[test]
    fn ttl_zero_disables_cache() {
        let inner = Arc::new(CountingEvaluator::new(Some(true)));
        let cached = CachedEvaluator::new(inner.clone(), Duration::ZERO);

        with_default(Arc::new(NoopEvaluator), || {
            let ctx = Context::root();
            for _ in 0..5 {
                assert_eq!(cached.is_enabled("flag", &ctx), Some(true));
            }
        });

        assert_eq!(
            inner.call_count(),
            5,
            "TTL=0 must short-circuit caching and call inner every time"
        );
        assert!(
            cached.is_empty(),
            "TTL=0 must not populate the cache map either"
        );
    }

    #[test]
    fn none_is_cached_too() {
        let inner = Arc::new(CountingEvaluator::new(None));
        let cached = CachedEvaluator::new(inner.clone(), Duration::from_secs(60));

        with_default(Arc::new(NoopEvaluator), || {
            let ctx = Context::root();
            for _ in 0..3 {
                assert_eq!(cached.is_enabled("flag", &ctx), None);
            }
        });

        assert_eq!(
            inner.call_count(),
            1,
            "the None response must be cached the same as Some(_)"
        );
    }

    #[test]
    fn invalidate_clears_only_the_named_feature() {
        let inner = Arc::new(CountingEvaluator::new(Some(true)));
        let cached = CachedEvaluator::new(inner.clone(), Duration::from_secs(60));

        with_default(Arc::new(NoopEvaluator), || {
            let ctx = Context::root();
            assert_eq!(cached.is_enabled("flag-a", &ctx), Some(true));
            assert_eq!(cached.is_enabled("flag-b", &ctx), Some(true));
            assert_eq!(cached.len(), 2);

            cached.invalidate("flag-a");
            assert_eq!(cached.len(), 1, "only flag-a's entries should be gone");

            // flag-a re-fetches; flag-b stays cached.
            assert_eq!(cached.is_enabled("flag-a", &ctx), Some(true));
            assert_eq!(cached.is_enabled("flag-b", &ctx), Some(true));
        });

        assert_eq!(
            inner.call_count(),
            3,
            "expected calls: flag-a, flag-b (initial), flag-a (after invalidate)"
        );
    }

    #[test]
    fn expired_entries_are_swept_once_threshold_is_crossed() {
        // A moderate TTL: long enough that the fill loop below finishes
        // well inside it (so no entry expires mid-fill), short enough
        // that a single sleep ages every entry out afterwards.
        let inner = Arc::new(CountingEvaluator::new(Some(true)));
        let cached = CachedEvaluator::new(inner.clone(), Duration::from_millis(50));

        // The default evaluator translates `user_id` into a `UserIdField`
        // extension so each distinct id yields a distinct cache key.
        with_default(Arc::new(TranslatingEvaluator), || {
            // Fill to one short of the threshold with distinct scopes.
            // The sweep condition is `len() >= SWEEP_THRESHOLD`, so it
            // can never fire during this phase regardless of how long the
            // fill takes - the map grows one entry per distinct scope.
            for i in 0..SWEEP_THRESHOLD - 1 {
                let ctx = ctx_for_user(i);
                cached.is_enabled("flag", &ctx);
            }
            assert_eq!(
                cached.len(),
                SWEEP_THRESHOLD - 1,
                "below the threshold the map grows unbounded (one entry per distinct scope)"
            );

            // Age every entry past the TTL, then insert one more. That
            // insert takes the map to SWEEP_THRESHOLD, trips the sweep,
            // and drops every entry older than the TTL - all the
            // pre-existing ones - leaving only the entry just written.
            std::thread::sleep(Duration::from_millis(120));
            let ctx = ctx_for_user(SWEEP_THRESHOLD);
            cached.is_enabled("flag", &ctx);

            assert_eq!(
                cached.len(),
                1,
                "crossing the threshold must sweep every expired entry, keeping only the \
                 just-inserted one; got {} entries",
                cached.len()
            );
        });
    }

    /// Inner evaluator that answers `true` for exactly one user id, and
    /// translates both identity fields the way the database evaluator does.
    struct OneUser(&'static str);

    impl Evaluator for OneUser {
        fn is_enabled(&self, _feature: &str, context: &Context) -> Option<bool> {
            let user = context
                .iter()
                .find_map(|c| c.extensions().get::<UserIdField>())
                .map(|field| field.as_str().to_string());
            Some(user.as_deref() == Some(self.0))
        }

        fn on_new_context(
            &self,
            mut context: featureflag::context::ContextRef<'_>,
            fields: featureflag::fields::Fields<'_>,
        ) {
            if let Some(id) = fields.get("user_id").and_then(|v| v.as_str()) {
                context.extensions_mut().insert(UserIdField(id.to_string()));
            }
            if let Some(team) = fields.get("team").and_then(|v| v.as_str()) {
                context.extensions_mut().insert(TeamField(team.to_string()));
            }
        }
    }

    /// DRIVERS-013: two identities whose user and team concatenate to the
    /// same string get separate cache entries. The key used to be the string
    /// `{feature}::u={user}::t={team}`, so `(a::t=b, c)` and `(a, b::t=c)`
    /// shared one entry and the second read got the first one's answer.
    #[test]
    fn identities_that_concatenate_alike_never_share_an_entry() {
        let inner = Arc::new(OneUser("a::t=b"));
        let cached = Arc::new(CachedEvaluator::new(inner, Duration::from_secs(60)));

        with_default(cached.clone(), || {
            let first = featureflag::context! { user_id = "a::t=b", team = "c" };
            assert_eq!(cached.is_enabled("flag", &first), Some(true));
            let second = featureflag::context! { user_id = "a", team = "b::t=c" };
            assert_eq!(
                cached.is_enabled("flag", &second),
                Some(false),
                "user `a` in team `b::t=c` was served user `a::t=b`'s cached answer"
            );
        });
        assert_eq!(cached.len(), 2, "two identities, two entries");
    }

    /// DRIVERS-013, the empty-string variant: a context with an empty team
    /// and one with no team are different scopes and are cached apart.
    #[test]
    fn an_empty_identity_field_is_not_the_same_scope_as_an_absent_one() {
        let inner = Arc::new(CountingEvaluator::new(Some(true)));
        let cached = Arc::new(CachedEvaluator::new(inner.clone(), Duration::from_secs(60)));
        let translator = Arc::new(OneUser(""));

        with_default(translator, || {
            let with_empty_team = featureflag::context! { user_id = "u", team = "" };
            let without_team = featureflag::context! { user_id = "u" };
            cached.is_enabled("flag", &with_empty_team);
            cached.is_enabled("flag", &without_team);
        });
        assert_eq!(
            inner.call_count(),
            2,
            "each scope reaches the inner evaluator once"
        );
        assert_eq!(cached.len(), 2);
    }

    /// Inner evaluator whose answer can be switched, and which can be held
    /// in the middle of an evaluation after it has read the old answer.
    struct Switchable {
        enabled: std::sync::atomic::AtomicBool,
        entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
        release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    }

    impl Evaluator for Switchable {
        fn is_enabled(&self, _feature: &str, _context: &Context) -> Option<bool> {
            let answer = self.enabled.load(Ordering::SeqCst);
            let entered = self
                .entered
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            if let Some(entered) = entered {
                let release = self
                    .release
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                let _ = entered.send(());
                if let Some(release) = release {
                    let _ = release.recv();
                }
            }
            Some(answer)
        }
    }

    /// DRIVERS-014: an invalidation that lands while a miss is evaluating is
    /// not undone when that miss finishes. The miss used to insert its answer
    /// unconditionally, so a flag switched off by an admin stayed on in the
    /// cache until the TTL ran out.
    #[test]
    fn an_invalidation_during_a_miss_is_not_undone_by_the_miss() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let inner = Arc::new(Switchable {
            enabled: std::sync::atomic::AtomicBool::new(true),
            entered: std::sync::Mutex::new(Some(entered_tx)),
            release: std::sync::Mutex::new(Some(release_rx)),
        });
        let cached = Arc::new(CachedEvaluator::new(inner.clone(), Duration::from_secs(60)));

        let reader = {
            let cached = cached.clone();
            std::thread::spawn(move || {
                with_default(Arc::new(NoopEvaluator), || {
                    cached.is_enabled("flag", &Context::root())
                })
            })
        };
        entered_rx
            .recv()
            .expect("the miss reached the inner evaluator");
        // The admin path: change the flag, then invalidate, while the miss
        // still holds the old answer.
        inner.enabled.store(false, Ordering::SeqCst);
        cached.invalidate("flag");
        release_tx.send(()).expect("release the miss");
        assert_eq!(
            reader.join().expect("reader thread"),
            Some(true),
            "the overlapping read itself may return the old answer"
        );

        with_default(Arc::new(NoopEvaluator), || {
            assert_eq!(
                cached.is_enabled("flag", &Context::root()),
                Some(false),
                "a read after the invalidation returned must see the new answer"
            );
        });
    }

    /// DRIVERS-015: a stream of distinct live scopes cannot grow the map past
    /// its bound, and reaching the bound does not sweep the whole map on
    /// every insert. Both used to happen: the sweep removed only expired
    /// entries, so a full map of live ones kept growing and kept being
    /// swept once per miss.
    #[test]
    fn live_scopes_stay_bounded_and_the_sweep_stays_amortised() {
        let inner = Arc::new(CountingEvaluator::new(Some(true)));
        let cached = CachedEvaluator::new(inner, Duration::from_secs(60));
        let inserts = 3 * SWEEP_THRESHOLD;

        with_default(Arc::new(TranslatingEvaluator), || {
            for i in 0..inserts {
                cached.is_enabled("flag", &ctx_for_user(i));
            }
        });
        assert!(
            cached.len() <= SWEEP_THRESHOLD,
            "{} live entries held against a bound of {SWEEP_THRESHOLD}",
            cached.len()
        );
        let sweeps = cached.sweeps.load(Ordering::Relaxed);
        assert!(
            sweeps <= 2 * inserts / SWEEP_THRESHOLD + 1,
            "{sweeps} full-map sweeps for {inserts} inserts; each sweep must free room \
             for many inserts"
        );
    }

    #[test]
    fn invalidate_all_clears_everything() {
        let inner = Arc::new(CountingEvaluator::new(Some(true)));
        let cached = CachedEvaluator::new(inner.clone(), Duration::from_secs(60));

        with_default(Arc::new(NoopEvaluator), || {
            let ctx = Context::root();
            cached.is_enabled("flag-a", &ctx);
            cached.is_enabled("flag-b", &ctx);
            cached.invalidate_all();
            assert!(cached.is_empty());
        });
    }

    /// Stand-in evaluator for the `with_default(...)` scope-default -
    /// featureflag panics if a `Context::root()`-derived context
    /// is used while no global default is installed, so the tests
    /// thread a no-op default through their scope.
    struct NoopEvaluator;

    impl Evaluator for NoopEvaluator {
        fn is_enabled(&self, _feature: &str, _context: &Context) -> Option<bool> {
            None
        }
    }

    /// Default evaluator that translates the `user_id` context field
    /// into a [`UserIdField`] extension on `on_new_context`, the same
    /// way [`DatabaseEvaluator`](super::super::database::DatabaseEvaluator)
    /// does. Lets the sweep test create distinct cache keys per user
    /// without depending on a database-backed evaluator.
    struct TranslatingEvaluator;

    impl Evaluator for TranslatingEvaluator {
        fn is_enabled(&self, _feature: &str, _context: &Context) -> Option<bool> {
            None
        }

        fn on_new_context(
            &self,
            mut context: featureflag::context::ContextRef<'_>,
            fields: featureflag::fields::Fields<'_>,
        ) {
            if let Some(id) = fields.get("user_id").and_then(|v| v.as_str()) {
                context.extensions_mut().insert(UserIdField(id.to_string()));
            }
        }
    }
}
