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
    create_user, dispatch_get, dispatch_head, ensure_per_tenant_authz_gate,
    ensure_round3_authz_gate, ensure_round4_per_user_authz_gate, rename_user,
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

// ---------------------------------------------------------------------
// Fix round 4: the guard reconciles what the render observed against what
// the key partitioned by through loose proxies, so each patch closed one
// proxy and left another. These tests are parameterised over
// `ClassificationReason` rather than pinned to two remembered leak shapes:
// for each reason, a route declaring the *wrong* dimension must decline to
// store (proven via render count and cross-identity body checks, the same
// technique the reviewer used), and a route declaring the *matching*
// dimension must serve a correctly partitioned cache. Round 3's two attack
// tests are kept above as the record of what actually happened; these are
// additional, not a replacement.
// ---------------------------------------------------------------------

/// Fix round 4, Leak A (Critical, proven) and `PrincipalObserved`'s half of
/// the parameterised rule. A route declaring `Tenant` variance whose
/// handler reads an identity narrows for a `PrincipalObserved` reason;
/// round 3's guard passed because `Tenant` happened to resolve private,
/// even though `Tenant` is not the dimension that reason names. Verified
/// failing (bob's render served alice's identity, render count staying at
/// 1) against the pre-fix guard by temporarily restoring the "some
/// dimension is private" check.
#[tokio::test]
#[serial_test::serial]
async fn principal_observed_requires_the_principal_dimension_to_partition() {
    let harness = boot_with_render_cache().await;

    let alice = dispatch_get(
        &harness,
        "/tenant-declared-reads-principal/1",
        &[("x-test-tenant", "acme"), ("x-test-login", "alice")],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&alice.body).contains("alice"));

    let bob = dispatch_get(
        &harness,
        "/tenant-declared-reads-principal/1",
        &[("x-test-tenant", "acme"), ("x-test-login", "bob")],
    )
    .await;
    assert!(
        !String::from_utf8_lossy(&bob.body).contains("alice"),
        "a route declaring Tenant, not Principal, must never serve one principal's \
         PrincipalObserved-narrowed body to a different principal in the same tenant"
    );
    assert!(String::from_utf8_lossy(&bob.body).contains("bob"));
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store: Tenant resolves private for this key, \
         but Tenant is not the dimension PrincipalObserved requires"
    );

    // The matching dimension: a route declaring Principal partitions
    // correctly - a repeat from the same principal is a hit, a different
    // principal is a genuine, correctly separated miss.
    let carol_first = dispatch_get(
        &harness,
        "/principal-declared-reads-principal/1",
        &[("x-test-login", "carol")],
    )
    .await;
    assert!(String::from_utf8_lossy(&carol_first.body).contains("carol"));
    let renders_after_carol = counting_route::renders();

    let carol_again = dispatch_get(
        &harness,
        "/principal-declared-reads-principal/1",
        &[("x-test-login", "carol")],
    )
    .await;
    assert_eq!(carol_again.body, carol_first.body);
    assert_eq!(
        counting_route::renders(),
        renders_after_carol,
        "a repeat request from the same, matching principal is a cache hit"
    );

    let dave = dispatch_get(
        &harness,
        "/principal-declared-reads-principal/1",
        &[("x-test-login", "dave")],
    )
    .await;
    assert!(String::from_utf8_lossy(&dave.body).contains("dave"));
    assert!(!String::from_utf8_lossy(&dave.body).contains("carol"));
    assert_eq!(
        counting_route::renders(),
        renders_after_carol + 1,
        "a different principal is a genuine miss, correctly partitioned"
    );
}

/// `TenantObserved`'s half of the parameterised rule: a route declaring
/// `Principal` variance whose handler reads the Live tenant narrows for a
/// `TenantObserved` reason, but `Principal` is not the dimension that
/// reason names. Verified failing (globex's render served acme's tenant
/// identity) against the pre-fix guard the same way as the principal test
/// above.
#[tokio::test]
#[serial_test::serial]
async fn tenant_observed_requires_the_tenant_dimension_to_partition() {
    let harness = boot_with_render_cache().await;

    let acme = dispatch_get(
        &harness,
        "/principal-declared-reads-tenant/1",
        &[("x-test-login", "shared-user"), ("x-test-tenant", "acme")],
    )
    .await;
    assert!(String::from_utf8_lossy(&acme.body).contains("acme"));

    let globex = dispatch_get(
        &harness,
        "/principal-declared-reads-tenant/1",
        &[("x-test-login", "shared-user"), ("x-test-tenant", "globex")],
    )
    .await;
    assert!(
        !String::from_utf8_lossy(&globex.body).contains("acme"),
        "a route declaring Principal, not Tenant, must never serve one tenant's \
         TenantObserved-narrowed body to a different tenant behind the same principal"
    );
    assert!(String::from_utf8_lossy(&globex.body).contains("globex"));
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store"
    );

    let acme2_first = dispatch_get(
        &harness,
        "/tenant-declared-reads-tenant/1",
        &[("x-test-tenant", "acme2")],
    )
    .await;
    let renders_after = counting_route::renders();

    let acme2_again = dispatch_get(
        &harness,
        "/tenant-declared-reads-tenant/1",
        &[("x-test-tenant", "acme2")],
    )
    .await;
    assert_eq!(acme2_again.body, acme2_first.body);
    assert_eq!(
        counting_route::renders(),
        renders_after,
        "a repeat request for the same, matching tenant is a cache hit"
    );

    let globex2 = dispatch_get(
        &harness,
        "/tenant-declared-reads-tenant/1",
        &[("x-test-tenant", "globex2")],
    )
    .await;
    assert!(String::from_utf8_lossy(&globex2.body).contains("globex2"));
    assert_eq!(
        counting_route::renders(),
        renders_after + 1,
        "a different tenant is a genuine miss, correctly partitioned"
    );
}

/// `AuthorizationRead`'s half of the parameterised rule: a route declaring
/// `Tenant` variance whose handler drives its body from a per-user
/// `Gate::allows` decision narrows for an `AuthorizationRead` reason, which
/// requires `Principal` (the decision is per-user), not `Tenant`.
#[tokio::test]
#[serial_test::serial]
async fn authorization_read_requires_the_principal_dimension_to_partition() {
    ensure_round4_per_user_authz_gate();
    let harness = boot_with_render_cache().await;

    let admin = dispatch_get(
        &harness,
        "/tenant-declared-reads-authz/1",
        &[("x-test-tenant", "acme"), ("x-test-login", "admin")],
    )
    .await;
    assert!(String::from_utf8_lossy(&admin.body).contains("allowed=true"));

    let guest = dispatch_get(
        &harness,
        "/tenant-declared-reads-authz/1",
        &[("x-test-tenant", "acme"), ("x-test-login", "guest")],
    )
    .await;
    assert!(
        String::from_utf8_lossy(&guest.body).contains("allowed=false"),
        "a route declaring Tenant, not Principal, must never serve an admin's per-user \
         authorization decision to a non-admin sharing the same tenant"
    );
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store"
    );

    let admin2_first = dispatch_get(
        &harness,
        "/principal-declared-reads-authz/1",
        &[("x-test-login", "admin")],
    )
    .await;
    assert!(String::from_utf8_lossy(&admin2_first.body).contains("allowed=true"));
    let renders_after = counting_route::renders();

    let admin2_again = dispatch_get(
        &harness,
        "/principal-declared-reads-authz/1",
        &[("x-test-login", "admin")],
    )
    .await;
    assert_eq!(admin2_again.body, admin2_first.body);
    assert_eq!(
        counting_route::renders(),
        renders_after,
        "a repeat request from the same admin is a cache hit"
    );

    let guest2 = dispatch_get(
        &harness,
        "/principal-declared-reads-authz/1",
        &[("x-test-login", "guest2")],
    )
    .await;
    assert!(String::from_utf8_lossy(&guest2.body).contains("allowed=false"));
    assert_eq!(
        counting_route::renders(),
        renders_after + 1,
        "a different principal is a genuine miss, correctly partitioned"
    );
}

/// Fix round 4, Leak B (Critical, proven). Same shape as round 3's
/// `a_route_reading_identity_through_an_uninstrumented_accessor...` test,
/// but the accessor is fully instrumented (via round 3's seam) and the
/// leak is instead that classification re-read `Auth::id()` specifically -
/// the default guard's own slot - to build the observed value, vetoing the
/// observation for any identity resolved through a different guard.
/// Verified failing (bob's render served alice's identity) against the
/// pre-fix `Auth::id()` re-read by temporarily restoring it.
#[tokio::test]
#[serial_test::serial]
async fn a_route_reading_identity_through_a_non_default_guard_never_leaks_across_identities() {
    let harness = boot_with_render_cache().await;
    let alice = dispatch_get(
        &harness,
        "/leaky-via-named-guard",
        &[("x-test-named-login", "alice")],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&alice.body).contains("alice"));

    let bob = dispatch_get(
        &harness,
        "/leaky-via-named-guard",
        &[("x-test-named-login", "bob")],
    )
    .await;
    assert!(
        !String::from_utf8_lossy(&bob.body).contains("alice"),
        "a route with no declared variance must never leak one named-guard identity's \
         body to another, even though Auth::id() (the default guard's own slot) never \
         reflects either identity"
    );
    assert!(String::from_utf8_lossy(&bob.body).contains("bob"));
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store"
    );
}

/// Fix round 4, Leak C (proven). `session()` records a session read;
/// `session_mut` - the idiomatic way to read and touch session state in one
/// call - did not, even though its closure can read whatever it also
/// mutates. Verified failing (the second dispatch was an unwarranted cache
/// hit, `renders()` staying at 1) against the pre-fix `session_mut` by
/// temporarily removing its `observe_session_read()` call.
#[tokio::test]
#[serial_test::serial]
async fn session_mut_reads_are_observed_and_force_uncacheable() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/session-mut-reading", &[]).await;
    dispatch_get(&harness, "/session-mut-reading", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "a session_mut read must be observed and force Uncacheable, the same as session()"
    );
}

// ---------------------------------------------------------------------
// Fix round 5: rounds 1, 3, and 4 each reconciled what the render observed
// against what the key partitioned by through a proxy - presence, then
// declared-dimension, then value-type - and each proxy closed the previous
// gap while leaving the next one standing. These three tests reproduce the
// round 5 review's three proven leaks directly: same, correct declaration,
// different value - the axis round 4's parameterised tests above did not
// attack (those vary the *declared* dimension while holding the read
// fixed). All three are proven failing against the pre-fix code below,
// per the instruction to reproduce before fixing.
// ---------------------------------------------------------------------

/// Fix round 5, Leak 1, first reproduction (Critical, proven over real
/// HTTP). Because `RenderCache::install` appends to the global middleware
/// registry, this middleware derives the key before any route middleware
/// runs. `ImpersonationMiddleware` is registered *after* `install` (see
/// the harness's own comment at that registration site), so it runs after
/// the key has already been derived from `LoginHeader`'s identity (the
/// impersonator's own) - the handler then reads `Auth::id()` and sees
/// whichever identity `ImpersonationMiddleware` set instead. Verified
/// failing (bob's target received alice's target's rendered body, render
/// count staying at 1) against the pre-fix guard by temporarily restoring
/// round 4's "the named dimension's value has type Private" check in place
/// of the value comparison.
#[tokio::test]
#[serial_test::serial]
async fn impersonation_after_key_derivation_never_serves_the_impersonators_page() {
    let harness = boot_with_render_cache().await;

    let alice_target = dispatch_get(
        &harness,
        "/principal-declared-reads-principal/1",
        &[
            ("x-test-login", "impersonator"),
            ("x-test-impersonate", "alice"),
        ],
    )
    .await;
    assert_eq!(alice_target.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&alice_target.body).contains("alice"));

    let bob_target = dispatch_get(
        &harness,
        "/principal-declared-reads-principal/1",
        &[
            ("x-test-login", "impersonator"),
            ("x-test-impersonate", "bob"),
        ],
    )
    .await;
    assert!(
        !String::from_utf8_lossy(&bob_target.body).contains("alice"),
        "impersonating one user must never serve the page rendered while impersonating a \
         different user, even though the key was derived from the same impersonator's own \
         identity both times"
    );
    assert!(String::from_utf8_lossy(&bob_target.body).contains("bob"));
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store: the key was fixed to the impersonator \
         before the impersonation middleware - which runs after key derivation - ever ran"
    );
}

/// Fix round 5, Leak 3 (Critical, proven). `Lang::set_locale`, mid-render,
/// is documented as supported; the key was already derived from
/// `Lang::locale()`'s pre-render value. Verified failing (the second
/// dispatch was an unwarranted cache hit under the old locale's key,
/// `renders()` staying at 1) against the pre-fix guard by temporarily
/// removing the locale re-derivation check.
#[tokio::test]
#[serial_test::serial]
async fn a_mid_render_locale_switch_never_publishes_under_the_old_locales_key() {
    let harness = boot_with_render_cache().await;
    let first = dispatch_get(&harness, "/locale-declared-switches-mid-render/1", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&first.body).contains("before=en"));
    assert!(String::from_utf8_lossy(&first.body).contains("after=fr"));

    dispatch_get(&harness, "/locale-declared-switches-mid-render/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "a render that switches locale mid-render must never be stored under the key \
         built from the locale it started with - a hit here would mean the switch was \
         silently ignored and the old locale's cached page kept being served"
    );
}

/// Fix round 5, Leak 2. Cookies carry private material by nature but
/// produce no `ClassificationReason` on their own; `Request::cookies` (and
/// `Request::cookie`, which delegates to it) now records a session read.
/// Verified failing (the second dispatch was an unwarranted cache hit,
/// `renders()` staying at 1) against the pre-fix `cookies()` by temporarily
/// removing its `observe_session_read()` call.
#[tokio::test]
#[serial_test::serial]
async fn cookie_reads_are_observed_and_force_uncacheable() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/cookie-reading", &[]).await;
    dispatch_get(&harness, "/cookie-reading", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "a cookie read must be observed and force Uncacheable, the same as a session read"
    );
}

// ---------------------------------------------------------------------
// Fix round 6: round 5 split the reconciliation mechanism - re-derive
// Locale, record Principal/Tenant in a single last-write slot - and both
// halves of that split were themselves proxies. Re-derivation cannot see a
// scope that has already popped (the first two tests below); a single slot
// cannot represent more than one observed value (the third). The fourth
// test is a different shape entirely: nothing is observed at all. All four
// are proven failing against the pre-fix code first, per the instruction.
// ---------------------------------------------------------------------

/// Fix round 6, Leak 1 (proven). A handler rendering inside `scope_locale` -
/// the framework's own documented, supported API - whose nested scope pops
/// before this handler itself returns, let alone before the guard runs.
/// Distinct from round 5's `locale_switching_handler` test: that handler
/// mutated the *same*, still-active outer scope via `Lang::set_locale` - no
/// nested scope ever popped, which is incidentally why round 5's
/// post-render re-read happened to catch it. This one pops, and round 5's
/// re-read could not have seen it.
///
/// Verified failing against the pre-fix guard by temporarily restoring a
/// post-render `Lang::locale()` re-read in place of the recorded
/// `locale_material` set: the second dispatch was an unwarranted cache hit
/// (`renders()` staying at 1).
#[tokio::test]
#[serial_test::serial]
async fn a_nested_scope_locale_render_never_publishes_under_the_outer_locales_key() {
    let harness = boot_with_render_cache().await;
    let first = dispatch_get(&harness, "/nested-scope-locale/1", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&first.body).contains("locale=fr"));

    dispatch_get(&harness, "/nested-scope-locale/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "a render inside a nested scope_locale must never be stored under the outer, \
         pre-switch locale's key - a hit here would mean the nested switch was silently \
         ignored"
    );
}

/// Fix round 6, Leak 1, second reproduction (proven): the same leak via a
/// different mechanism, named explicitly in the review - a locale
/// established by a middleware positioned *after* `RenderCache::install`
/// ("the only position a per-route locale middleware can occupy"), rather
/// than a nested scope inside the handler itself.
///
/// Verified failing against the pre-fix guard the same way as the
/// nested-scope test above.
#[tokio::test]
#[serial_test::serial]
async fn a_locale_set_by_a_middleware_after_install_never_publishes_under_the_earlier_locales_key()
{
    let harness = boot_with_render_cache().await;
    let first = dispatch_get(&harness, "/late-locale/1", &[("x-test-late-locale", "1")]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&first.body).contains("locale=fr"));

    dispatch_get(&harness, "/late-locale/1", &[("x-test-late-locale", "1")]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "a locale set after RenderCacheMiddleware in the chain must never be stored \
         under the key derived before that middleware ran"
    );
}

/// Fix round 6, Leak 3 (proven, cross-identity). A handler reads a named
/// guard's identity to build the body, then separately touches the default
/// accessor for an unrelated check - the shape round 5's single last-write
/// slot could not survive. Two dispatches share the *same* default-guard
/// login ("shared-default", so the key - always built from the default
/// identity alone, see `variance_descriptor`'s `Principal` arm - is
/// identical both times) but *different* named-guard identities.
///
/// Verified failing against the pre-fix single-slot guard by temporarily
/// restoring an `Option<String>` for `principal_material` (overwritten by
/// the later `Auth::id()` touch): bob's response contained alice's
/// identity and `renders()` stayed at 1.
#[tokio::test]
#[serial_test::serial]
async fn a_named_guard_identity_overwritten_by_a_later_default_touch_never_leaks_across_identities()
{
    let harness = boot_with_render_cache().await;

    let alice = dispatch_get(
        &harness,
        "/named-guard-then-default/1",
        &[
            ("x-test-login", "shared-default"),
            ("x-test-named-login", "alice"),
        ],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&alice.body).contains("alice"));

    let bob = dispatch_get(
        &harness,
        "/named-guard-then-default/1",
        &[
            ("x-test-login", "shared-default"),
            ("x-test-named-login", "bob"),
        ],
    )
    .await;
    assert!(
        !String::from_utf8_lossy(&bob.body).contains("alice"),
        "a shared default-guard identity must never let two different named-guard \
         identities collide on the same cached representation"
    );
    assert!(String::from_utf8_lossy(&bob.body).contains("bob"));
    assert_eq!(
        counting_route::renders(),
        2,
        "neither render was ever safe to store: the key was fixed to the shared default \
         identity while the render also observed a different, unrecorded-under-round-5 \
         named-guard identity"
    );
}

/// Fix round 6, item 5. The engine no longer rejects `FeatureVersion` at
/// policy build time (that rejection moved to the host's own
/// `variance_descriptor`, since "this host has no producer" is a fact
/// about the host, not the engine); this route's policy therefore builds
/// successfully, but every request against it must bypass the cache
/// entirely rather than publish a key that silently omits the declared
/// dimension.
#[tokio::test]
#[serial_test::serial]
async fn a_route_declaring_feature_version_always_bypasses_the_cache() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/feature-version-declared/1", &[]).await;
    dispatch_get(&harness, "/feature-version-declared/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "a route declaring a dimension this host cannot produce must bypass the cache \
         on every request, never publish a key that omits it"
    );
}

/// Fix round 7, finding 1. `FeatureMiddleware` and `DatabaseEvaluator`, both
/// shipped by the framework, with the team taken from a header through the
/// shipped `with_team_from_header` helper. The body is driven entirely by a
/// **team-scoped** flag read ambiently during the render.
///
/// Before this round, `fields.rs` extracted `UserIdField` and nothing else,
/// so a team-scoped decision recorded no observation at all, narrowed
/// nothing, and the render published under a key with no `Tenant` dimension
/// that the next team then hit. Verified failing by removing the
/// `if scopes.tenant` arm from `crate::features::fields::observe_identity`:
/// team beta was served `enabled=true` from team alpha's entry with the
/// render count unchanged.
#[tokio::test]
#[serial_test::serial]
async fn a_team_scoped_feature_flag_never_crosses_teams() {
    let harness = boot_with_render_cache().await;

    let alpha = dispatch_get(
        &harness,
        "/reads-team-scoped-flag/1",
        &[("x-test-team", "alpha")],
    )
    .await;
    let alpha_body = String::from_utf8_lossy(&alpha.body).to_string();
    let renders_after_alpha = counting_route::renders();

    let beta = dispatch_get(
        &harness,
        "/reads-team-scoped-flag/1",
        &[("x-test-team", "beta")],
    )
    .await;
    let beta_body = String::from_utf8_lossy(&beta.body).to_string();

    assert!(
        alpha_body.contains("enabled=true"),
        "the team-scoped flag must be on for team alpha - got {alpha_body:?}"
    );
    assert!(
        beta_body.contains("enabled=false"),
        "team beta must never be served team alpha's flag decision - got {beta_body:?}"
    );
    assert_eq!(
        counting_route::renders(),
        renders_after_alpha + 1,
        "team beta must be a genuine miss, not a hit on team alpha's entry"
    );
}

/// The same team-scoped flag on a route that correctly declares, and is
/// correctly keyed by, `Principal`, with one signed-in user. The key is
/// genuine and right; only the team axis was invisible. Same discriminating
/// line as the test above.
#[tokio::test]
#[serial_test::serial]
async fn a_team_scoped_feature_flag_never_crosses_teams_behind_one_principal() {
    let harness = boot_with_render_cache().await;

    let alpha = dispatch_get(
        &harness,
        "/principal-declared-reads-team-scoped-flag/1",
        &[("x-test-login", "alice"), ("x-test-team", "alpha")],
    )
    .await;
    let alpha_body = String::from_utf8_lossy(&alpha.body).to_string();
    let renders_after_alpha = counting_route::renders();

    let beta = dispatch_get(
        &harness,
        "/principal-declared-reads-team-scoped-flag/1",
        &[("x-test-login", "alice"), ("x-test-team", "beta")],
    )
    .await;
    let beta_body = String::from_utf8_lossy(&beta.body).to_string();

    assert!(alpha_body.contains("enabled=true"));
    assert!(
        beta_body.contains("enabled=false"),
        "one principal's team-alpha body must never be served for team beta - got {beta_body:?}"
    );
    assert_eq!(
        counting_route::renders(),
        renders_after_alpha + 1,
        "a route keyed only by Principal cannot represent two teams, so the second team \
         must be a genuine miss"
    );
}

/// Fix round 6, Leak 4, over real HTTP. A **user-scoped** flag read on a
/// route that declares nothing must never publish at all: not for a repeat
/// from the same visitor, and not across visitors.
///
/// Verified failing by removing the `if scopes.principal` arm from
/// `crate::features::fields::observe_identity`: alice's second request was a
/// hit (render count unchanged) and bob was then served `enabled=true`.
#[tokio::test]
#[serial_test::serial]
async fn a_user_scoped_feature_flag_never_publishes_a_shared_entry() {
    let harness = boot_with_render_cache().await;

    let alice = dispatch_get(
        &harness,
        "/reads-user-scoped-flag/1",
        &[("x-test-login", "alice")],
    )
    .await;
    let alice_body = String::from_utf8_lossy(&alice.body).to_string();
    let renders_after_alice = counting_route::renders();
    assert!(
        alice_body.contains("enabled=true"),
        "the user-scoped flag must be on for alice - got {alice_body:?}"
    );

    dispatch_get(
        &harness,
        "/reads-user-scoped-flag/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        renders_after_alice + 1,
        "the entry is not safe to publish even for alice herself: the route declares no \
         Principal dimension, so the key she would hit is the key everyone hits"
    );

    let bob = dispatch_get(
        &harness,
        "/reads-user-scoped-flag/1",
        &[("x-test-login", "bob")],
    )
    .await;
    let bob_body = String::from_utf8_lossy(&bob.body).to_string();
    assert!(
        bob_body.contains("enabled=false"),
        "bob must never be served alice's flag decision - got {bob_body:?}"
    );
    assert_eq!(counting_route::renders(), renders_after_alice + 2);
}

/// Fix round 7, finding 2. A **globally** scoped flag's answer does not
/// depend on who is reading it, so reading one must cost the cache nothing -
/// including for a signed-in visitor, whose id `FeatureMiddleware` puts in
/// the ambient context regardless.
///
/// Round 6 recorded that id at the top of `is_enabled`, before the evaluator
/// had decided anything, which disabled the cache for every signed-in
/// visitor of every page reading any flag in an application that installs
/// `FeatureMiddleware` globally - the reference application does. Verified
/// failing by restoring the unconditional call: alice's second request
/// rendered again instead of hitting.
#[tokio::test]
#[serial_test::serial]
async fn a_globally_scoped_feature_flag_still_caches_for_a_signed_in_visitor() {
    let harness = boot_with_render_cache().await;

    dispatch_get(&harness, "/reads-global-flag/1", &[]).await;
    let after_first_anonymous = counting_route::renders();
    dispatch_get(&harness, "/reads-global-flag/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        after_first_anonymous,
        "an anonymous repeat on a shared route reading a global flag is a cache hit"
    );

    let alice = dispatch_get(
        &harness,
        "/reads-global-flag/2",
        &[("x-test-login", "alice")],
    )
    .await;
    assert!(String::from_utf8_lossy(&alice.body).contains("enabled=true"));
    let after_first_alice = counting_route::renders();
    dispatch_get(
        &harness,
        "/reads-global-flag/2",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        after_first_alice,
        "a signed-in repeat must be a cache hit too: the flag is global, so the body does \
         not depend on who is asking"
    );
}

/// Fix round 7, finding 2, the half a naive fix gets wrong. The flag's only
/// identity rule is `user:bob`; alice falls through to the global rule, so
/// no `user:` key matched for her. Her answer is still a function of who she
/// is - bob's differs - so her page must not be published under a key bob
/// would hit.
///
/// This is what "record by flag scope, not by matched key" buys. Verified
/// failing by narrowing `Snapshot::record_scope` to only record a scope for
/// the reader whose key matched (recording `IdentityScopes::default()`
/// whenever the resolved scope key was the global one): alice's second
/// request became a hit, and bob was then served her body.
#[tokio::test]
#[serial_test::serial]
async fn a_flag_scoped_to_another_user_still_declines_for_everyone_else() {
    let harness = boot_with_render_cache().await;

    let alice = dispatch_get(
        &harness,
        "/reads-flag-with-another-users-override/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert!(
        String::from_utf8_lossy(&alice.body).contains("enabled=false"),
        "alice falls through to the global rule, which is off"
    );
    let after_first_alice = counting_route::renders();

    dispatch_get(
        &harness,
        "/reads-flag-with-another-users-override/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        after_first_alice + 1,
        "alice's page must never be published under a key bob hits, even though the rule \
         that answered her was the global one"
    );

    let bob = dispatch_get(
        &harness,
        "/reads-flag-with-another-users-override/1",
        &[("x-test-login", "bob")],
    )
    .await;
    assert!(
        String::from_utf8_lossy(&bob.body).contains("enabled=true"),
        "bob's own override is on, which is exactly why alice's page is not his"
    );
}

/// `Request::auth_user_id()` is a public identity accessor with no collector
/// instrumentation at all. The sixth review measured that it carries no
/// identity on the ordinary HTTP path - the only `with_auth_user_id` call
/// site is the WebSocket-upgrade terminator - so it is not a leak today.
/// This keeps that measurement standing: if a future change stamps the
/// identity on the HTTP path, this fails instead of leaking silently.
#[tokio::test]
#[serial_test::serial]
async fn request_auth_user_id_carries_no_identity_on_the_http_path() {
    let harness = boot_with_render_cache().await;

    let alice = dispatch_get(
        &harness,
        "/reads-auth-user-id-accessor/1",
        &[("x-test-login", "alice")],
    )
    .await;
    let alice_body = String::from_utf8_lossy(&alice.body).to_string();
    assert!(
        alice_body.contains("identity=none"),
        "if this is not `none`, Request::auth_user_id carries a real identity through an \
         accessor with no instrumentation at all, and this route caches it for everyone - \
         got {alice_body:?}"
    );
}

/// Fix round 7, finding 4, the documented limitation and its documented
/// remedy (see the middleware module doc's honest-boundary section).
/// `Gate::allows` records that a decision happened, never what it consulted,
/// so `AuthorizationRead` requires the `Principal` dimension even when the
/// gate is genuinely per-tenant. A route keyed by `Tenant` alone therefore
/// never caches; the same handler on a route that declares `Principal` as
/// well does, and stays partitioned by both.
///
/// This asserts the ruled behaviour rather than arguing with it: the guard
/// cannot tell a per-tenant gate from a per-user one, and treating every
/// decision as per-user is the safe default. Verified discriminating by
/// removing `VarianceDimension::Principal` from
/// `tenant_and_principal_declared_policy` in the support module: the second
/// half's repeat stopped being a hit.
#[tokio::test]
#[serial_test::serial]
async fn a_per_tenant_authorization_decision_caches_only_when_principal_is_declared_too() {
    let harness = boot_with_render_cache().await;
    ensure_per_tenant_authz_gate();

    let first = dispatch_get(
        &harness,
        "/tenant-declared-reads-per-tenant-authz/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    assert!(String::from_utf8_lossy(&first.body).contains("allowed=true"));
    let after_first = counting_route::renders();
    dispatch_get(
        &harness,
        "/tenant-declared-reads-per-tenant-authz/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        after_first + 1,
        "a route keyed by Tenant alone cannot satisfy AuthorizationRead's Principal \
         requirement, so it never publishes - the stated limitation"
    );

    let acme = dispatch_get(
        &harness,
        "/tenant-and-principal-declared-reads-per-tenant-authz/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    let acme_body = String::from_utf8_lossy(&acme.body).to_string();
    assert!(acme_body.contains("tenant=acme"));
    let after_acme = counting_route::renders();

    dispatch_get(
        &harness,
        "/tenant-and-principal-declared-reads-per-tenant-authz/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        after_acme,
        "declaring Principal alongside Tenant is the documented remedy: the repeat is a hit"
    );

    let globex = dispatch_get(
        &harness,
        "/tenant-and-principal-declared-reads-per-tenant-authz/1",
        &[("x-test-tenant", "globex")],
    )
    .await;
    let globex_body = String::from_utf8_lossy(&globex.body).to_string();
    assert!(
        !globex_body.contains("tenant=acme"),
        "globex must never be served acme's authorized body - got {globex_body:?}"
    );
    assert_eq!(
        counting_route::renders(),
        after_acme + 1,
        "the remedy still partitions by tenant: globex is a genuine miss"
    );
}

/// Fix round 7, finding 3, both arms of the guard's empty-set path. A
/// dimension whose required reason fired with no concrete value observed has
/// nothing to compare, so the check falls back to what the key says.
///
/// Positive: a fully anonymous request to a route declaring `Tenant` and
/// `Principal` whose render evaluates an authorization decision and reads
/// the tenant. `AuthorizationRead` requires `Principal` and observes no id;
/// `TenantObserved` fires with no tenant. Both key values are
/// `DimensionValue::Anonymous` - the render asked and found none, the key
/// says none, which is agreement - so the entry publishes and the repeat is
/// a hit. Before this round the path accepted only `Private(_)`, and
/// anonymous traffic, normally the bulk of what a render cache exists to
/// serve, was never cached on any route whose render touches auth.
///
/// Negative, which must survive: the same handler on a route that declares
/// `Tenant` only. `AuthorizationRead`'s `Principal` is undeclared, so it
/// still declines - otherwise the route would publish an anonymous page
/// under a key every signed-in visitor also hits.
///
/// Verified failing by reverting the empty-set match in
/// `key_used_different_values_than_the_render_saw` to
/// `Some(DimensionValue::Private(_))`: the positive half's repeat rendered
/// again. Verified the negative discriminates by widening the same match to
/// accept `None` as well: the negative half's repeat became a hit.
#[tokio::test]
#[serial_test::serial]
async fn anonymous_traffic_caches_where_the_key_declares_the_dimension_and_not_otherwise() {
    let harness = boot_with_render_cache().await;
    ensure_per_tenant_authz_gate();

    dispatch_get(
        &harness,
        "/tenant-and-principal-declared-reads-per-tenant-authz/1",
        &[],
    )
    .await;
    let after_first = counting_route::renders();
    dispatch_get(
        &harness,
        "/tenant-and-principal-declared-reads-per-tenant-authz/1",
        &[],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        after_first,
        "an anonymous repeat on a route that declares both dimensions its render \
         narrowed on is a cache hit: every key value says Anonymous and every observed \
         set is empty, which agrees"
    );

    dispatch_get(&harness, "/tenant-declared-reads-per-tenant-authz/1", &[]).await;
    let after_first_undeclared = counting_route::renders();
    dispatch_get(&harness, "/tenant-declared-reads-per-tenant-authz/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        after_first_undeclared + 1,
        "an empty observed set for an *undeclared* dimension must still decline: the key \
         this anonymous render would publish under is the key a signed-in visitor hits"
    );
}

/// The limitation the empty-set fix does not reach, asserted so it is a
/// recorded fact rather than a surprise (see the middleware module doc's
/// honest-boundary section). `Auth::id()` resolves an anonymous request by
/// falling through to `session()`, which records a *session* read, and
/// `classify` narrows any session read straight to `Uncacheable` - before
/// this guard ever runs. So an anonymous visitor of a route whose render
/// calls `Auth::id()` still never caches, whatever the key says.
///
/// Measured, not assumed: removing the `.or_else(|| session()...)` fallback
/// from `crate::session::middleware::auth_user_id` makes the first repeat
/// below a hit, with no other change - which is also the proof that the
/// empty-set fix is what stands behind it once the session read is out of
/// the way. That change is a much larger widening than fix round 7 was
/// scoped to make, so it is reported rather than taken.
#[tokio::test]
#[serial_test::serial]
async fn an_anonymous_render_that_resolves_identity_through_the_session_stays_uncacheable() {
    let harness = boot_with_render_cache().await;

    dispatch_get(&harness, "/private/1", &[]).await;
    let after_first_anonymous = counting_route::renders();
    dispatch_get(&harness, "/private/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        after_first_anonymous + 1,
        "anonymous identity resolution reads the session, and a session read is \
         Uncacheable - the empty-set path is never reached"
    );

    dispatch_get(&harness, "/private/2", &[("x-test-login", "alice")]).await;
    let after_first_alice = counting_route::renders();
    let alice = dispatch_get(&harness, "/private/2", &[("x-test-login", "alice")]).await;
    assert_eq!(alice.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        after_first_alice,
        "a signed-in visitor resolves through request state, never touching the session, \
         so the same route does cache for them"
    );
}

/// The positive control for the whole `Locale` axis, which the sixth review
/// found missing: every other locale test in this file asserts
/// `renders() == 2`, so if the `locale_material`-against-key comparison ever
/// disagreed spuriously, `Locale`-declared routes would silently never cache
/// and no test would notice. Verified discriminating by making that
/// comparison always return `true`: this test then failed while every other
/// locale test still passed.
#[tokio::test]
#[serial_test::serial]
async fn a_locale_declared_route_still_caches_when_nothing_switches() {
    let harness = boot_with_render_cache().await;

    let first = dispatch_get(&harness, "/late-locale/1", &[]).await;
    assert!(String::from_utf8_lossy(&first.body).contains("locale=en"));
    let after_first = counting_route::renders();
    dispatch_get(&harness, "/late-locale/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        after_first,
        "a Locale-declared route whose render observes exactly the key's locale must \
         still be a cache hit on the second request"
    );
}

/// Fix round 8, finding 5. `observe_identity` used to record an axis only
/// when the ambient context actually carried a field for it, so an
/// **anonymous** reader of an identity-scoped flag recorded nothing at all:
/// no value, and no bare read either. `classify` emitted no reason, the
/// class was never narrowed, and the anonymous body published under a
/// shared, principal-free key that bob - who has a `user:bob` override -
/// then hit, bypassing his own override. Note the orientation: a per-user
/// *restriction* (global `true`, `user:X false`) is bypassed the same way.
///
/// Verified failing by restoring the `if let Some(field)` shape to the
/// principal arm of `fields::observe_identity`, so an absent `UserIdField`
/// records nothing instead of `observe_principal_read()`.
#[tokio::test]
#[serial_test::serial]
async fn an_anonymous_reader_of_a_user_scoped_flag_never_publishes_a_shared_entry() {
    let harness = boot_with_render_cache().await;

    let anonymous = dispatch_get(&harness, "/reads-flag-with-another-users-override/1", &[]).await;
    let anonymous_body = String::from_utf8_lossy(&anonymous.body).to_string();
    let after_anonymous = counting_route::renders();
    assert!(
        anonymous_body.contains("enabled=false"),
        "an anonymous reader falls through to the global rule - got {anonymous_body:?}"
    );

    let bob = dispatch_get(
        &harness,
        "/reads-flag-with-another-users-override/1",
        &[("x-test-login", "bob")],
    )
    .await;
    let bob_body = String::from_utf8_lossy(&bob.body).to_string();
    assert!(
        bob_body.contains("enabled=true"),
        "bob has a per-user override and must never be served the anonymous reader's \
         flag decision from cache - got {bob_body:?}"
    );
    assert_eq!(
        counting_route::renders(),
        after_anonymous + 1,
        "bob must be a genuine miss: the anonymous render was never safe to publish \
         under a key bob also hits"
    );
}

/// The team half of the same hole (fix round 8, finding 5): a reader
/// carrying no `TeamField` recorded nothing for a team-scoped flag, so the
/// teamless body published shared and a reader in the scoped team hit it.
/// This is fix round 7's finding 1 reached through the absent field instead
/// of a different value.
///
/// Verified failing by restoring the `if let Some(field)` shape to the
/// tenant arm of `fields::observe_identity`.
#[tokio::test]
#[serial_test::serial]
async fn a_teamless_reader_of_a_team_scoped_flag_never_publishes_a_shared_entry() {
    let harness = boot_with_render_cache().await;

    let teamless = dispatch_get(&harness, "/reads-team-scoped-flag/1", &[]).await;
    let teamless_body = String::from_utf8_lossy(&teamless.body).to_string();
    let after_teamless = counting_route::renders();
    assert!(teamless_body.contains("enabled=false"));

    let alpha = dispatch_get(
        &harness,
        "/reads-team-scoped-flag/1",
        &[("x-test-team", "alpha")],
    )
    .await;
    let alpha_body = String::from_utf8_lossy(&alpha.body).to_string();
    assert!(
        alpha_body.contains("enabled=true"),
        "team alpha must never be served the teamless reader's flag decision - got \
         {alpha_body:?}"
    );
    assert_eq!(
        counting_route::renders(),
        after_teamless + 1,
        "the team alpha request must be a genuine miss"
    );
}

/// The same hole on a route that *does* declare `Principal`, which does not
/// help because the axis the flag is scoped by is `Tenant` (fix round 8,
/// finding 5). The bare tenant read the teamless reader now records lands on
/// the guard's empty-set path with `Tenant` undeclared, so the route
/// declines - the right answer, and the reason declaring the wrong dimension
/// buys nothing.
///
/// Verified failing by restoring the `if let Some(field)` shape to the
/// tenant arm of `fields::observe_identity`: the teamless render published,
/// its repeat became a hit, and team alpha was served `enabled=false`.
#[tokio::test]
#[serial_test::serial]
async fn declaring_principal_does_not_cover_a_flag_scoped_by_team() {
    let harness = boot_with_render_cache().await;

    let teamless = dispatch_get(
        &harness,
        "/principal-declared-reads-team-scoped-flag/1",
        &[],
    )
    .await;
    let teamless_body = String::from_utf8_lossy(&teamless.body).to_string();
    assert!(teamless_body.contains("enabled=false"));
    let after_teamless = counting_route::renders();

    dispatch_get(
        &harness,
        "/principal-declared-reads-team-scoped-flag/1",
        &[],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        after_teamless + 1,
        "the route declares Principal, but the dimension the observed tenant read \
         requires is Tenant, which it does not declare - so it must decline, even for \
         two identical anonymous requests"
    );
    let after_repeat = counting_route::renders();

    let alpha = dispatch_get(
        &harness,
        "/principal-declared-reads-team-scoped-flag/1",
        &[("x-test-team", "alpha")],
    )
    .await;
    let alpha_body = String::from_utf8_lossy(&alpha.body).to_string();
    assert!(
        alpha_body.contains("enabled=true"),
        "team alpha must not be served the teamless body - got {alpha_body:?}"
    );
    assert_eq!(
        counting_route::renders(),
        after_repeat + 1,
        "team alpha is a genuine miss"
    );
}

// ---------------------------------------------------------------------
// Closing fix round (final review F2, ruling R118): the query-builder and
// raw read seams.
// ---------------------------------------------------------------------

/// Final review, F2 (the review's Probe B, promoted): a render whose only
/// read is `DB::table("posts").get()` observes the `posts` table, so an ORM
/// write to `posts` after publication makes the next request render again
/// rather than serve the stale row count. Before the fix the builder facade
/// recorded nothing, the entry observed only `Broad`, and the second
/// dispatch below was served stale with `renders()` still `1`.
///
/// Proven by revert: with `observe_table_read` removed from
/// `DbTableBuilder::get`, this fails at the "reconciled" assertion with
/// `left: 1, right: 2` and the stale `0 posts` body.
#[tokio::test]
#[serial_test::serial]
async fn a_query_builder_read_is_reconciled_by_an_orm_write() {
    let harness = boot_with_render_cache().await;

    let first = dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);
    assert!(
        String::from_utf8_lossy(&first.body).contains("sees 0 posts"),
        "precondition: the table is empty at the first render"
    );
    let repeat = dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "precondition: a builder-only read is cacheable and the repeat is a hit"
    );
    assert_eq!(repeat.header("age"), Some("0"));

    advance_posts(&harness).await;

    let after_write = dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "an ORM write to posts landed after publication; a render that read posts through \
         DB::table must be reconciled, not served stale"
    );
    assert!(
        String::from_utf8_lossy(&after_write.body).contains("sees 1 posts"),
        "the re-render shows the written row - got {:?}",
        String::from_utf8_lossy(&after_write.body)
    );

    dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "control: the re-render published and this repeat is a hit"
    );
}

/// Final review, F2: a render whose only read is raw SQL (`DB::select`,
/// `DB::select_one`, or `DB::scalar`; one key per shape) is never stored.
/// The framework cannot name the tables such a statement read, so it marks
/// the collector report incomplete and the render is declined, the same way
/// an overflowed report is: served, never cached, and never served stale.
///
/// Proven by revert: with `observe_unobservable_read` removed from the three
/// facade methods, the first shape's repeat is a hit (`renders()` stays at
/// `1` where `2` is required) and `inspect` finds a stored entry.
#[tokio::test]
#[serial_test::serial]
async fn a_raw_sql_read_is_never_stored() {
    let harness = boot_with_render_cache().await;

    let mut expected_renders = 0;
    for kind in ["select", "select-one", "scalar"] {
        let path = format!("/raw-read/{kind}");
        let first = dispatch_get(&harness, &path, &[]).await;
        expected_renders += 1;
        assert_eq!(
            first.status,
            StatusCode::OK,
            "{kind}: the raw read itself works"
        );
        assert_eq!(
            counting_route::renders(),
            expected_renders,
            "{kind}: first render"
        );
        let key = RenderCache::key_for_route_for_test("/raw-read/{kind}", &[("kind", kind)], None);
        assert!(
            RenderCache::inspect(&key).await.expect("inspect").is_none(),
            "{kind}: a render that read through raw SQL is never published"
        );

        dispatch_get(&harness, &path, &[]).await;
        expected_renders += 1;
        assert_eq!(
            counting_route::renders(),
            expected_renders,
            "{kind}: the repeat renders again; a raw-SQL render is never a hit"
        );
    }
}

/// Final review, F2: `Auth::user()` resolves through `DatabaseUserProvider`,
/// which reads the `users` table through `DB::table(..).first()`; with the
/// builder facade observed, a `PrivateCached` render that shows the
/// signed-in user's own row is invalidated by an ORM write to that row. The
/// provider itself needed no change. Before the fix the entry observed only
/// `Broad` and the renamed user kept seeing their old name for `fresh_ms`.
///
/// Proven by revert: with `observe_table_read` removed from
/// `DbTableBuilder::get`, this fails at the "invalidated" assertion with
/// `left: 1, right: 2` and the body still naming `alice`.
#[tokio::test]
#[serial_test::serial]
async fn a_private_render_showing_auth_user_is_invalidated_by_an_orm_write_to_that_users_row() {
    let harness = boot_with_render_cache().await;
    let id = create_user(&harness, "alice").await;
    let id_text = id.to_string();
    let login = [("x-test-provider-login", id_text.as_str())];

    let first = dispatch_get(&harness, "/shows-auth-user", &login).await;
    assert_eq!(first.status, StatusCode::OK);
    let first_body = String::from_utf8_lossy(&first.body).to_string();
    assert!(
        first_body.contains(&format!("user {id} named alice")),
        "precondition: Auth::user() resolved the row through the provider - got {first_body:?}"
    );
    assert_eq!(counting_route::renders(), 1);
    dispatch_get(&harness, "/shows-auth-user", &login).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "precondition: the private render is cacheable for its own visitor and the repeat is \
         a hit"
    );

    rename_user(&harness, id, "alicia").await;

    let after_write = dispatch_get(&harness, "/shows-auth-user", &login).await;
    let after_body = String::from_utf8_lossy(&after_write.body).to_string();
    assert_eq!(
        counting_route::renders(),
        2,
        "an ORM write to the user's own row must invalidate a render that showed it"
    );
    assert!(
        after_body.contains(&format!("user {id} named alicia")),
        "the re-render shows the renamed row - got {after_body:?}"
    );
}
