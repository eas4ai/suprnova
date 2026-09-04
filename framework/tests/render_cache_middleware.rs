//! Task 14: the RenderCache middleware end to end - hit, miss, bypass,
//! conditional, stale, and singleflight - dispatched through the real
//! middleware chain with a controllable clock and ledger.
//!
//! Every test in this file is `#[serial_test::serial]`: `RenderCache`'s
//! installed runtime and the process-wide global middleware registry are
//! both process-global state (see `RenderCache::install`'s own doc for
//! why), so two of these tests running concurrently in the same test
//! binary would install over each other. Plain `#[tokio::test]`
//! (current-thread) is deliberate too, matching `render_cache_orm.rs`'s own
//! choice for the same underlying reason: `TestContainer::fake()` writes a
//! thread-local, and a multi-thread runtime can migrate a future between
//! worker threads between polls, which would make that registration
//! invisible to whichever thread resumes the test. The brief's original
//! sketch of this file called for `flavor = "multi_thread"` on every test,
//! for `RenderCache::inspect_route_for_test`'s now-removed `block_in_place`
//! call (see ruling R53) - with that call gone, nothing here needs more
//! than one thread, and singleflight's leader/waiter interleaving is
//! exercised correctly by tokio's cooperative scheduler on a single
//! thread, driven by explicit `Notify`-based state barriers rather than
//! real OS parallelism.

use suprnova::render_cache::{RenderCache, RenderCacheMiddleware};
use suprnova::{StatusCode, async_trait};

mod render_cache_middleware_support;
use render_cache_middleware_support::{
    advance_posts, boot_with_render_cache, boot_with_render_cache_and_l1_for_test,
    boot_with_render_cache_preserving_global_middleware_for_test, clock, counting_route,
    dispatch_get, dispatch_head, ensure_round3_authz_gate,
};

#[tokio::test]
#[serial_test::serial]
async fn stale_on_error_serves_stale_when_the_foreground_rebuild_fails() {
    let harness = boot_with_render_cache().await;
    let first = dispatch_get(&harness, "/stale/1", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);

    // `/stale/{id}`'s policy is fresh 60_000, stale-servable 60_000,
    // stale-on-error 120_000 (see the harness). age 130_000 gives
    // past_fresh = 70_000, which lands inside [stale_servable_ms (60_000),
    // stale_on_error_ms (120_000)): the StaleOnError band, not the
    // StaleServable band the other stale test exercises.
    clock(&harness).advance_ms(130_000);
    counting_route::fail_next_render(&harness);
    let served = dispatch_get(&harness, "/stale/1", &[]).await;

    // Fix round 2, item 3, proven to discriminate: before the fix, this
    // branch only treated a `ProviderFailure` (a failure before the handler
    // ran) as a reason to fall back to the stale entry; a handler that ran
    // and returned a 500 - the failure mode stale-on-error is documented
    // and named for - passed straight through as an ordinary response.
    // Reverting the `serve` fix and re-running this test, the assertion
    // below failed with `served.status == 500`.
    assert_eq!(
        served.status,
        StatusCode::OK,
        "a handler-level failure inside the stale-on-error window must still \
         serve the stale entry, not the failure"
    );
    assert_eq!(
        served.header("warning"),
        Some("110 - \"Response is Stale\"")
    );
    assert_eq!(
        served.body, first.body,
        "the served body is the stale entry's, not a fresh render's"
    );
    assert_eq!(
        counting_route::renders(),
        2,
        "the foreground rebuild attempt still ran once and failed"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_second_request_is_an_l0_hit_that_runs_no_handler_and_carries_validators() {
    let harness = boot_with_render_cache().await;
    let first = dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);
    let etag = first.header("etag").expect("etag").to_owned();
    assert!(etag.starts_with("\"sha256-"));
    assert_eq!(first.header("cache-control"), Some("private, max-age=60"));

    let second = dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.body, first.body);
    assert_eq!(counting_route::renders(), 1, "a hit runs no handler");
    assert_eq!(second.header("age"), Some("0"));

    // Fix round 2, item 7: the empty-body checks that used to follow each of
    // these two assertions were vacuous, proven by the reviewer - they pass
    // even with the middleware's own body suppression removed, because the
    // server strips the body for `HEAD` and the protocol suppresses it for
    // `304` regardless of what this middleware puts in the response body.
    // Isolating which layer actually did the suppression is not observable
    // through a full HTTP dispatch, so they are dropped rather than left
    // looking like coverage; the status and header assertions here (and
    // `renders()` staying at 1 below) still prove the middleware treated
    // both as hits, which is what this test is actually about.
    let conditional = dispatch_get(&harness, "/cached/1", &[("if-none-match", &etag)]).await;
    assert_eq!(conditional.status, StatusCode::NOT_MODIFIED);

    let head = dispatch_head(&harness, "/cached/1").await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn a_moved_generation_misses_and_a_write_during_the_render_discards_the_candidate() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/cached/1", &[]).await;
    advance_posts(&harness).await;
    let after_write = dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(after_write.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 2, "a moved generation is a miss");

    counting_route::write_during_next_render(&harness);
    let raced = dispatch_get(&harness, "/cached/2", &[]).await;
    assert_eq!(raced.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 3);
    // `inspect_route_for_test` derives its key from the pattern alone with
    // empty params (see its own doc) - correct for a fixed path like
    // `/sets-cookie` below, but `/cached/{id}` has a dynamic segment a real
    // dispatch fills in, so checking *this* route's actual key needs the
    // explicit-params form with `id` supplied, matching what dispatching
    // `/cached/2` really derived.
    let key = RenderCache::key_for_route_for_test("/cached/{id}", &[("id", "2")], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_none(),
        "a candidate with moved observations is never published"
    );
    dispatch_get(&harness, "/cached/2", &[]).await;
    assert_eq!(counting_route::renders(), 4);
}

#[tokio::test]
#[serial_test::serial]
async fn stale_service_is_policy_driven_bounded_and_never_private() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/stale/1", &[]).await;
    clock(&harness).advance_ms(70_000);
    let stale = dispatch_get(&harness, "/stale/1", &[]).await;
    // "Does not block" is proven by the response itself: it is served with
    // the stale entry's Warning header rather than waiting for a fresh
    // render (a blocking implementation could never emit this header, since
    // the response it eventually returns would be freshly rendered - not
    // stale). Whether the background rebuild `tokio::spawn`s independently
    // of it has *also* finished by the time this assertion runs is a genuine
    // race, not a property of the middleware: the client's dispatch here
    // does a real TCP round trip that yields to the scheduler repeatedly,
    // while the spawned rebuild only awaits a local SQLite read, so it
    // reliably finishes first. `renders()` is therefore bounded, not
    // pinned to exactly one, to avoid pinning a scheduling accident.
    assert_eq!(stale.header("warning"), Some("110 - \"Response is Stale\""));
    assert!(
        (1..=2).contains(&counting_route::renders()),
        "the client's own render already ran, and the background rebuild may or may not \
         have completed yet, but nothing else could have rendered"
    );

    clock(&harness).advance_ms(200_000);
    let dead = dispatch_get(&harness, "/stale/1", &[]).await;
    assert!(dead.header("warning").is_none());
    assert!(counting_route::renders() >= 2);

    dispatch_get(&harness, "/private/1", &[("x-test-login", "user-7")]).await;
    clock(&harness).advance_ms(70_000);
    let before = counting_route::renders();
    dispatch_get(&harness, "/private/1", &[("x-test-login", "user-7")]).await;
    assert_eq!(
        counting_route::renders(),
        before + 1,
        "private output is never served stale"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn undeclared_query_and_ineligible_responses_bypass_without_poisoning() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/cached/1", &[]).await;
    let bypass = dispatch_get(&harness, "/cached/1?utm=abc", &[]).await;
    assert_eq!(bypass.status, StatusCode::OK);
    assert!(
        bypass.header("etag").is_none(),
        "an undeclared query bypasses the cache"
    );
    let hit = dispatch_get(&harness, "/cached/1", &[]).await;
    assert!(
        hit.header("age").is_some(),
        "the valid entry survived the bypass"
    );

    let cookie = dispatch_get(&harness, "/sets-cookie", &[]).await;
    assert!(cookie.header("etag").is_none());
    assert!(
        RenderCache::inspect_route_for_test("/sets-cookie")
            .await
            .is_none()
    );
}

// Plain `#[tokio::test]` (current-thread), not `flavor = "multi_thread"`:
// this test spawns two tasks that both need the thread-local
// `TestContainer::fake()` registration `boot_with_render_cache` made, and
// bare `tokio::spawn` does not carry a task-local container into a
// spawned task on a different worker thread. A current-thread runtime
// still gives tokio genuine task concurrency - the leader and the waiter
// interleave cooperatively at their own `.await` points, driven by the
// `Notify`-based state barriers below, not by real OS parallelism, which
// this test does not need.
#[tokio::test]
#[serial_test::serial]
async fn concurrent_misses_render_once_and_waiters_reuse_the_publication() {
    let harness = boot_with_render_cache().await;
    counting_route::hold_next_render(&harness);
    let a = tokio::spawn({
        let h = harness.clone();
        async move { dispatch_get(&h, "/cached/9", &[]).await }
    });
    // No render has happened in this test yet, so the first one to start
    // is render 1.
    counting_route::wait_until_rendering_count(&harness, 1).await;
    let b = tokio::spawn({
        let h = harness.clone();
        async move { dispatch_get(&h, "/cached/9", &[]).await }
    });
    counting_route::wait_until_waiting(&harness, 1).await;
    counting_route::release_render(&harness);
    let (a, b) = (a.await.expect("a"), b.await.expect("b"));
    assert_eq!(a.body, b.body);
    assert_eq!(
        counting_route::renders(),
        1,
        "one leader rendered; the waiter reused the publication"
    );
}

/// Ruling R55: a render whose collector report overflowed must never be
/// published. An overflowed report's dependency list is incomplete by
/// definition, and storing under an incomplete dependency set is exactly
/// how an entry outlives the write that should have invalidated it - the
/// missing dependency is the one write that would have caught it.
#[tokio::test]
#[serial_test::serial]
async fn an_overflowed_report_is_never_published() {
    let harness = boot_with_render_cache().await;
    let first = dispatch_get(&harness, "/overflow", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);
    assert!(
        first.header("etag").is_none(),
        "an overflowed render is served but never carries cache validators"
    );

    let key = RenderCache::key_for_route_for_test("/overflow", &[], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_none(),
        "an overflowed report must never be published"
    );

    // Confirms the decline is not a one-shot fluke of the first render:
    // every request to this route renders fresh, because nothing was ever
    // stored to hit against.
    dispatch_get(&harness, "/overflow", &[]).await;
    assert_eq!(counting_route::renders(), 2);
}

/// Fix round 1, item 1 (Critical): classification narrows the served class
/// and the `Cache-Control`/staleness rules, but it cannot repartition the
/// lookup key, which was already derived from the route's *declared*
/// variance before this render ran. `/leaky` declares `PublicShared` with
/// no `Principal` variance, and its handler reads an identity held in
/// `auth::request_state` (`Auth::id()`, the way bearer-token or
/// remember-me authentication would, not a session read - a session read
/// already forces `Uncacheable`). Without the fix, alice's render
/// publishes under the one shared, principal-free key and bob's request
/// for the same route is served alice's body back.
#[tokio::test]
#[serial_test::serial]
async fn a_route_with_no_declared_principal_variance_never_leaks_across_identities() {
    let harness = boot_with_render_cache().await;
    let alice = dispatch_get(&harness, "/leaky", &[("x-test-login", "alice")]).await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&alice.body).contains("alice"));

    let bob = dispatch_get(&harness, "/leaky", &[("x-test-login", "bob")]).await;
    assert_eq!(bob.status, StatusCode::OK);
    assert!(
        !String::from_utf8_lossy(&bob.body).contains("alice"),
        "a route with no declared Principal variance must never serve one identity's \
         rendered body to a different identity"
    );
    assert!(String::from_utf8_lossy(&bob.body).contains("bob"));

    // Neither render was ever safe to store: a bug that instead served bob
    // a cache hit (merely narrowing the served class to PrivateCached, not
    // repartitioning the key) would render only once, not twice.
    assert_eq!(counting_route::renders(), 2);
}

/// Fix round 1, item 2 (Critical): `RenderCache::install` must never clear
/// the process-wide global middleware registry - an application that
/// registered its own logging, session, CSRF, or auth middleware before
/// calling `install` must keep every one of them.
#[tokio::test]
#[serial_test::serial]
async fn install_does_not_clear_the_applications_own_global_middleware() {
    struct MarkerMiddleware;

    #[async_trait]
    impl suprnova::Middleware for MarkerMiddleware {
        async fn handle(
            &self,
            request: suprnova::Request,
            next: suprnova::Next,
        ) -> suprnova::Response {
            next(request).await
        }
    }

    suprnova::middleware::clear_global_middleware_for_test();
    suprnova::middleware::register_global_middleware(MarkerMiddleware);
    assert!(suprnova::middleware::has_global_middleware::<
        MarkerMiddleware,
    >());

    let _harness = boot_with_render_cache_preserving_global_middleware_for_test().await;

    assert!(
        suprnova::middleware::has_global_middleware::<MarkerMiddleware>(),
        "RenderCache::install must not clear an application's own already-registered \
         global middleware"
    );
    assert!(
        suprnova::middleware::has_global_middleware::<RenderCacheMiddleware>(),
        "install must still register its own middleware"
    );
}

/// Fix round 1, item 4 (Important, proven): a singleflight waiter must
/// re-evaluate coherence and freshness on what it finds after waiting, the
/// same as the primary hit path does - not serve it as an unconditional
/// hit just because it waited for someone else to try.
///
/// `/cached/{id}` has no stale window (fresh then dead), so a request
/// against a dead entry always reaches `render_and_publish`'s admission -
/// both the leader's and the waiter's. The leader is armed (via
/// `write_during_next_render`, the same mechanism the "moved" test uses)
/// to have its own candidate discarded as moved, so it never republishes:
/// the entry the waiter's post-wait lookup finds is still the original,
/// long-dead one. Without the fix, the waiter serves that dead entry
/// directly (`renders()` stops at 2 - the leader's one discarded attempt);
/// with the fix, the waiter's own freshness check sees it is dead and
/// renders a third, genuinely fresh time instead of trusting the wait.
#[tokio::test]
#[serial_test::serial]
async fn a_singleflight_waiter_never_serves_a_superseded_entry_as_fresh() {
    let harness = boot_with_render_cache().await;
    let original = dispatch_get(&harness, "/cached/9", &[]).await;
    assert_eq!(counting_route::renders(), 1);
    // Past `fresh_ms` (60_000) with no stale window declared for this
    // route: the entry is dead, so both dispatches below reach admission
    // in the foreground rather than taking the StaleServable hit path
    // (which would never call `admit` from a client-visible request at
    // all).
    clock(&harness).advance_ms(70_000);

    counting_route::hold_next_render(&harness);
    let leader = {
        let h = harness.clone();
        tokio::spawn(async move { dispatch_get(&h, "/cached/9", &[]).await })
    };
    // Fix round 3, item 4: the `original` dispatch above already rendered
    // once, so the leader's held render is the *second* one - waiting for
    // "any render" (count 1) here was already satisfied before
    // `hold_next_render` was even armed, which is exactly what made this
    // test hang intermittently. See `wait_until_rendering_count`'s own doc.
    counting_route::wait_until_rendering_count(&harness, 2).await;
    let waiter = {
        let h = harness.clone();
        tokio::spawn(async move { dispatch_get(&h, "/cached/9", &[]).await })
    };
    counting_route::wait_until_waiting(&harness, 1).await;
    counting_route::write_during_next_render(&harness);
    counting_route::release_render(&harness);

    let (leader_response, waiter_response) =
        (leader.await.expect("leader"), waiter.await.expect("waiter"));
    assert_eq!(leader_response.status, StatusCode::OK);
    assert_eq!(waiter_response.status, StatusCode::OK);
    assert_ne!(
        waiter_response.body, original.body,
        "the waiter served a superseded, long-dead entry with no coherence or freshness \
         check, indistinguishable from a fresh hit"
    );
    assert_eq!(
        counting_route::renders(),
        3,
        "the leader's discarded attempt (2) plus the waiter's own fresh re-render (3) - a \
         waiter that trusted the wait alone would leave this at 2"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_stale_principal_route_never_spawns_a_background_rebuild() {
    let harness = boot_with_render_cache().await;
    dispatch_get(
        &harness,
        "/stale-principal/1",
        &[("x-test-login", "user-7")],
    )
    .await;
    assert_eq!(counting_route::renders(), 1);

    // past_fresh = 70_000 - 60_000 = 10_000, inside the StaleServable band
    // [0, stale_servable_ms (60_000)).
    clock(&harness).advance_ms(70_000);
    let stale = dispatch_get(
        &harness,
        "/stale-principal/1",
        &[("x-test-login", "user-7")],
    )
    .await;
    assert_eq!(stale.header("warning"), Some("110 - \"Response is Stale\""));

    // Fix round 2, item 4, proven to discriminate: before the fix, this
    // route's declared `Principal` variance did not stop a background
    // rebuild from spawning. That rebuild runs with no task-local identity
    // at all (`Auth::id()` returns `None` inside the spawned task
    // regardless of who made the original request), so it renders
    // anonymously and would publish that anonymous render under this
    // specific principal's already-derived key -
    // `key_omits_observed_privacy` does not catch this shape, since it
    // only flags an *observed* identity the key does not declare, not a
    // *declared* dimension the render failed to observe. Reverting the
    // `serve` fix and re-running this test, `renders()` reliably reached 2
    // immediately after the stale dispatch above - the same "the
    // background rebuild only awaits a local SQLite read, so it reliably
    // finishes first" timing `stale_service_is_policy_driven_bounded_and_never_private`
    // above already relies on. With the fix, no task is ever spawned for a
    // `Principal`-varying route, so this is not a race: nothing could
    // increment `renders()` a second time no matter how long this waited.
    assert_eq!(
        counting_route::renders(),
        1,
        "a Principal-varying route must not spawn a background rebuild"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn l1_participates_with_promotion_and_dual_publish() {
    let harness = boot_with_render_cache_and_l1_for_test().await;

    // A miss populates both tiers.
    dispatch_get(&harness, "/l1-cached/1", &[]).await;
    assert_eq!(counting_route::renders(), 1);
    let key1 = RenderCache::key_for_route_for_test("/l1-cached/{id}", &[("id", "1")], None);
    assert!(
        RenderCache::inspect(&key1)
            .await
            .expect("inspect l0")
            .is_some(),
        "a fresh publish reaches L0"
    );
    assert!(
        RenderCache::inspect_l1_for_test("/l1-cached/{id}", &[("id", "1")], None)
            .await
            .is_some(),
        "a fresh publish reaches L1 too - a dual publish, not L0-only"
    );

    // A second key's publish evicts the first from L0 (this harness's L0 is
    // capped at a single entry) but not from L1 (sized generously).
    dispatch_get(&harness, "/l1-cached/2", &[]).await;
    assert_eq!(counting_route::renders(), 2);
    assert!(
        RenderCache::inspect(&key1)
            .await
            .expect("inspect l0")
            .is_none(),
        "L0's single-entry capacity evicted the first key"
    );
    assert!(
        RenderCache::inspect_l1_for_test("/l1-cached/{id}", &[("id", "1")], None)
            .await
            .is_some(),
        "L1 still holds the first key: an L0 eviction is not an L1 eviction"
    );

    // The next request for the evicted key is served from L1 - not
    // re-rendered - and promoted back into L0.
    let promoted = dispatch_get(&harness, "/l1-cached/1", &[]).await;
    assert_eq!(promoted.status, StatusCode::OK);
    assert!(promoted.header("etag").is_some());
    assert_eq!(counting_route::renders(), 2, "an L1 hit runs no handler");
    assert!(
        RenderCache::inspect(&key1)
            .await
            .expect("inspect l0")
            .is_some(),
        "an L1 hit is promoted back into L0"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_lease_mode_route_trusts_within_the_window_but_still_catches_an_epoch_bump() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/leased/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the first request renders and publishes"
    );

    // Grants the lease: this hit's `coherence` check finds no existing
    // lease yet, so it rereads the authority once and, finding it
    // coherent, grants one.
    dispatch_get(&harness, "/leased/1", &[]).await;
    assert_eq!(counting_route::renders(), 1);

    // Trusts the just-granted lease: this hit's `coherence` check skips the
    // authority reread entirely.
    let leased_hit = dispatch_get(&harness, "/leased/1", &[]).await;
    assert_eq!(counting_route::renders(), 1);
    assert!(leased_hit.header("etag").is_some());

    // Fix round 2, item 6: this asserts the observable guarantee the review
    // asked for - an emergency epoch advance reaches a lease-mode route
    // immediately, not after the lease's own `max_age_ms` expires - but it
    // is *not* a discriminating test for a `coherence`-level epoch check,
    // and this codebase does not have one. Investigated, not assumed:
    // `RenderKey::derive` bakes the epoch into the lookup key itself, and
    // `key_input` re-derives that key from a freshly read epoch on every
    // dispatch, before `coherence` (lease mode or not) ever runs. So the
    // epoch bump below changes the lookup key for `/leased/1`, and the
    // previously-published entry becomes unreachable by ordinary lookup on
    // the request below - an ordinary cache miss, which renders
    // immediately regardless of coherence mode. This test passes
    // identically whether or not `coherence`'s lease branch consults the
    // epoch (confirmed: it still passed with that consultation removed),
    // because the key mismatch already guarantees the outcome. See
    // `coherence`'s own comment for the fuller reasoning and its scope.
    RenderCache::advance_epoch().await.expect("advance epoch");
    let after_bump = dispatch_get(&harness, "/leased/1", &[]).await;
    assert_eq!(after_bump.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        2,
        "an emergency epoch advance must reach a lease-mode route immediately, not wait out \
         the lease"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_lease_grant_sweeps_every_expired_lease_first() {
    let harness = boot_with_render_cache().await;
    for id in 1..=3 {
        let path = format!("/leased/{id}");
        dispatch_get(&harness, &path, &[]).await; // miss: renders and publishes
        dispatch_get(&harness, &path, &[]).await; // hit: grants a lease
    }
    assert_eq!(RenderCache::lease_count_for_test(), 3);

    // Past `max_age_ms` (60_000) for every lease granted above.
    clock(&harness).advance_ms(61_000);

    // Fix round 2, item 6, proven to discriminate: before the fix, the
    // lease map was insert-only, so the three now-expired leases above
    // would stay in the map for the process lifetime - a lease-mode route
    // keyed by an ever-growing identifier (a principal, a query value)
    // grows it without bound. Reverting just the `leases.retain(...)` sweep
    // line and re-running this test, `lease_count_for_test()` read 4 below,
    // not 1: the three expired leases were never removed.
    dispatch_get(&harness, "/leased/4", &[]).await; // miss: renders and publishes
    dispatch_get(&harness, "/leased/4", &[]).await; // hit: grants a lease,
    // opportunistically sweeping every already-expired lease first.
    assert_eq!(
        RenderCache::lease_count_for_test(),
        1,
        "granting a new lease must sweep every already-expired one first"
    );
}

/// Fix round 3, item 1 (Critical, reviewer's first attack, deliverable).
/// Same shape as `a_route_with_no_declared_principal_variance_never_leaks_across_identities`,
/// but the handler reads identity through `suprnova::auth_user_id()` -
/// which reads `auth::request_state::current_user_id()` directly, bypassing
/// `Auth::id()`'s own explicit `observe_principal_read()` call entirely -
/// rather than `Auth::id()`. Proven failing against the pre-round-3 code by
/// temporarily reverting `request_state.rs`'s `read_state` instrumentation
/// and re-running: alice's identity leaked into bob's response.
#[tokio::test]
#[serial_test::serial]
async fn a_route_reading_identity_through_an_uninstrumented_accessor_never_leaks_across_identities()
{
    let harness = boot_with_render_cache().await;
    let alice = dispatch_get(
        &harness,
        "/leaky-via-request-state",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&alice.body).contains("alice"));

    let bob = dispatch_get(
        &harness,
        "/leaky-via-request-state",
        &[("x-test-login", "bob")],
    )
    .await;
    assert_eq!(bob.status, StatusCode::OK);
    assert!(
        !String::from_utf8_lossy(&bob.body).contains("alice"),
        "a route with no declared Principal variance must never serve one identity's \
         rendered body to a different identity, regardless of which accessor read it"
    );
    assert!(String::from_utf8_lossy(&bob.body).contains("bob"));
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store"
    );
}

/// Fix round 3, item 2 (Critical, reviewer's second attack, deliverable).
/// Drives the served body entirely from a `Gate::allows` decision - no
/// identity accessor is ever read - on a route declaring no variance
/// dimension at all. Proven failing against the pre-round-3
/// `key_omits_observed_privacy` (which only ever inspected
/// `ObservedContext.principal`/`.tenant`, never the classification outcome)
/// by temporarily reverting it and re-running: the admin-checked body was
/// published and served back to the non-admin request.
#[tokio::test]
#[serial_test::serial]
async fn a_body_driven_by_an_authorization_decision_never_leaks_across_roles() {
    ensure_round3_authz_gate();
    let harness = boot_with_render_cache().await;
    let admin = dispatch_get(&harness, "/authz-driven", &[("x-test-role", "admin")]).await;
    assert_eq!(admin.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&admin.body).contains("allowed=true"));

    let guest = dispatch_get(&harness, "/authz-driven", &[("x-test-role", "guest")]).await;
    assert_eq!(guest.status, StatusCode::OK);
    assert!(
        String::from_utf8_lossy(&guest.body).contains("allowed=false"),
        "a route with no declared variance must never serve an authorization-gated body \
         computed for one role to a request that would have gotten a different decision"
    );
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store: an authorization decision narrowed the \
         class to PrivateCached with nothing in the key to partition by, which must \
         decline to store, not merely narrow"
    );
}

/// Fix round 3, item 3 (Critical). The guard tested whether `Principal` was
/// *declared*, not whether the key's resolved value for it actually
/// partitions - `variance_descriptor` declares `Principal` as
/// `DimensionValue::Anonymous` when no identity is visible, which is
/// "declared" but not a partition. This is the test item 3 explicitly
/// calls out as missing: two distinct principals on a route that *does*
/// declare `Principal` must derive two distinct keys. (A test that
/// dispatches the same login twice, as an earlier version of this file's
/// singleflight coverage did, cannot distinguish a real per-principal
/// partition from a guard that always passes.)
#[tokio::test]
#[serial_test::serial]
async fn two_distinct_principals_derive_two_distinct_keys_on_a_declared_principal_route() {
    let _harness = boot_with_render_cache().await;
    let alice_key =
        RenderCache::key_for_route_for_test("/private/{id}", &[("id", "1")], Some("alice"));
    let bob_key = RenderCache::key_for_route_for_test("/private/{id}", &[("id", "1")], Some("bob"));
    assert_ne!(
        alice_key, bob_key,
        "two distinct principals must derive two distinct keys on a route that declares \
         Principal variance"
    );
}

/// Fix round 3, item 5 (second smaller item). Reproduces the production
/// shape the reviewer described: a second `RenderCache::install` in one
/// process, without clearing the already-registered global middleware
/// first (`boot_with_render_cache_preserving_global_middleware_for_test`,
/// same as the round 1 regression test above, does not clear it).
#[tokio::test]
#[serial_test::serial]
async fn a_second_install_in_one_process_is_served_by_the_runtime_it_installed() {
    let first = boot_with_render_cache().await;
    let dispatched_under_first = dispatch_get(&first, "/cached/1", &[]).await;
    assert_eq!(dispatched_under_first.status, StatusCode::OK);

    let second = boot_with_render_cache_preserving_global_middleware_for_test().await;
    let dispatched_under_second = dispatch_get(&second, "/cached/1", &[]).await;
    assert_eq!(dispatched_under_second.status, StatusCode::OK);

    // Fix round 3, item 5, proven to discriminate: before the fix,
    // `RenderCacheMiddleware` captured `Arc<RenderCacheRuntime>` at
    // construction, and `register_global_middleware`'s per-type
    // idempotency (see `install`'s own doc) meant the already-registered
    // (first) middleware instance - still holding the first runtime - kept
    // serving every request, including the dispatch above, made after the
    // second install. `RenderCache::inspect` and `key_for_route_for_test`
    // both read `RenderCache::runtime()`, the slot the second install just
    // replaced - the second runtime's own, freshly constructed
    // `MemoryRenderStore`, entirely separate from the first's. Reverting
    // the fix (restoring the captured-`Arc` field) and re-running this
    // test, the assertion below failed: the dispatch above actually
    // published into the *first* runtime's store, which the second
    // runtime's `inspect` can never see.
    let key = RenderCache::key_for_route_for_test("/cached/{id}", &[("id", "1")], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_some(),
        "a request dispatched after a second install must be served by, and publish \
         into, the runtime that install just replaced the slot with"
    );
}
