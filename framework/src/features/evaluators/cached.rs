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
    IdentityScopes, TeamField, UserIdField, capturing_identity_reads, observe_identity,
};
use crate::features::sync::FeatureSync;
use async_trait::async_trait;
use dashmap::DashMap;
use featureflag::{context::Context, evaluator::Evaluator};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// When the cache size reaches this threshold, a single insert sweeps
/// every entry whose age is `>= ttl` (i.e. would be re-fetched on its
/// next read anyway). This caps the map's growth: without it, a
/// high-cardinality or attacker-influenced `user_id`/`team` stream
/// would accumulate one never-revisited entry per distinct scope and
/// never reclaim them, since expired entries are only overwritten when
/// their exact key is re-read.
///
/// The sweep is amortised - it runs only on the miss/expiry insert
/// path once the map has grown past the threshold, mirroring the
/// brute-force dedup map's bounded eviction. Sized so a normally-scoped
/// workload (a bounded set of users/teams) never trips it, while a
/// pathological scope stream is held to roughly this many live entries
/// plus whatever was inserted since the last sweep.
const SWEEP_THRESHOLD: usize = 4096;

/// TTL-cached wrapper around any [`Evaluator`].
pub struct CachedEvaluator {
    inner: Arc<dyn Evaluator + Send + Sync>,
    ttl: Duration,
    /// Key format: `"{feature}::u={user_id?}::t={team?}"`. Empty
    /// segments encode "field absent in this context."
    ///
    /// Growth is bounded by an opportunistic sweep on insert (see
    /// [`SWEEP_THRESHOLD`]): once the map reaches the threshold, the
    /// next insert drops every entry older than `ttl`, so an unbounded
    /// stream of distinct scopes can't leak memory.
    cache: DashMap<String, CacheEntry>,
}

#[derive(Copy, Clone)]
struct CacheEntry {
    value: Option<bool>,
    inserted_at: Instant,
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
        let prefix = format!("{feature}::");
        self.cache.retain(|key, _| !key.starts_with(&prefix));
    }

    /// Drop every cached entry. Use sparingly - typically only on a
    /// bulk admin reload or in tests.
    pub fn invalidate_all(&self) {
        self.cache.clear();
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

    fn cache_key(feature: &str, context: &Context) -> String {
        let user = context
            .iter()
            .find_map(|c| c.extensions().get::<UserIdField>())
            .map(|field| field.as_str().to_string())
            .unwrap_or_default();
        let team = context
            .iter()
            .find_map(|c| c.extensions().get::<TeamField>())
            .map(|field| field.as_str().to_string())
            .unwrap_or_default();
        format!("{feature}::u={user}::t={team}")
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

        // Fast path: live entry.
        if let Some(found) = self.cache.get(&key)
            && found.inserted_at.elapsed() < self.ttl
        {
            let entry = *found;
            // Released before the replay below: nothing in `observe_identity`
            // touches this map today, and holding a shard guard across a
            // call into another module is how that stops being true.
            drop(found);
            // Fix round 6, Leak 4, narrowed by fix round 7: a cached answer
            // for a scoped flag is exactly as identity-dependent as a fresh
            // one, and this hit never reaches `self.inner`, so it replays
            // the axes the miss's own evaluation consulted. See
            // `crate::features::fields::observe_identity`'s own doc.
            observe_identity(entry.identity, context);
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
        let (value, identity) =
            capturing_identity_reads(|| self.inner.is_enabled(feature, context));
        let now = Instant::now();
        self.cache.insert(
            key,
            CacheEntry {
                value,
                inserted_at: now,
                identity,
            },
        );

        // Bounded-growth backstop: once the map crosses the threshold,
        // drop every entry that is already past its TTL (those are
        // dead weight - a read would re-fetch them anyway). This keeps
        // a high-cardinality scope stream from growing the cache
        // without bound. Amortised: it runs only on this insert path,
        // and only after the size trips the threshold.
        if self.cache.len() >= SWEEP_THRESHOLD {
            self.cache
                .retain(|_, entry| now.duration_since(entry.inserted_at) < self.ttl);
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
        fn is_enabled(&self, _feature: &str, context: &Context) -> Option<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            crate::features::fields::observe_identity(self.scopes, context);
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
    /// miss both go through `fields::observe_identity`, the one function that
    /// decides value-or-bare-read, so they cannot disagree about an absent
    /// field.
    #[tokio::test]
    async fn a_cache_hit_replays_a_bare_read_when_the_context_carries_no_field() {
        let inner = Arc::new(ScopedEvaluator::new(IdentityScopes {
            principal: true,
            tenant: true,
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
