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
    advance_posts, boot_with_render_cache,
    boot_with_render_cache_preserving_global_middleware_for_test, clock, counting_route,
    dispatch_get, dispatch_head,
};

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

    let conditional = dispatch_get(&harness, "/cached/1", &[("if-none-match", &etag)]).await;
    assert_eq!(conditional.status, StatusCode::NOT_MODIFIED);
    assert!(conditional.body.is_empty());

    let head = dispatch_head(&harness, "/cached/1").await;
    assert_eq!(head.status, StatusCode::OK);
    assert!(head.body.is_empty());
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
    counting_route::wait_until_rendering(&harness).await;
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
    counting_route::wait_until_rendering(&harness).await;
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
