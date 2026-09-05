//! Task 17: a deterministic race suite for the render-cache middleware -
//! a write landing after the fresh reread, an epoch advance during a
//! render, a background rebuild raced by a write, and singleflight
//! leader/waiter interleaving. Every synchronization point here is a
//! `Notify`-based state barrier (`counting_route`'s existing hooks, plus
//! `race`'s two new race points and its background-finished barrier - see
//! `render_cache_middleware_support::race`'s own doc); none of it waits on
//! wall-clock time.
//!
//! Ruling R72: gated on the `testing` feature, not `cfg(test)` - this file
//! is a separate crate from the library and never gets the library's own
//! `cfg(test)`, so without this gate a feature-matrix build that turns
//! default features off would try to compile against
//! `suprnova::render_cache::middleware::race_points`, which does not exist
//! there, and fail for a reason nobody would connect to this suite. With
//! the gate, such a build compiles this file to nothing instead.
//!
//! Every test is `#[serial_test::serial]` and plain `#[tokio::test]`
//! (current-thread), matching `render_cache_middleware.rs`'s own choice
//! and for the same reasons: `RenderCache::install`'s runtime and the
//! global middleware registry are process-global, and `TestContainer::fake()`
//! writes a thread-local a multi-thread runtime could migrate away from
//! between polls. Singleflight and background-rebuild interleaving are
//! exercised correctly on a single thread, cooperatively, the same way
//! `render_cache_middleware.rs`'s own singleflight tests already prove.
#![cfg(feature = "testing")]

use suprnova::StatusCode;
use suprnova::render_cache::RenderCache;

mod render_cache_middleware_support;
use render_cache_middleware_support::{
    boot_with_render_cache, clock, counting_route, dispatch_get, race,
};

/// A write that lands after the fresh reread already found a render
/// coherent - but before that render's candidate is built and stored -
/// still carries the render's now-stale observations into the store. The
/// *next* lookup, not this one, is where it is caught: `coherence` reads
/// the ledger fresh again and finds the stored entry's observations behind
/// it, so the entry is a miss rather than served as current.
///
/// Fix round 1, R98/F1: this is the production mechanism the test targets,
/// and the two ways to disable it fail differently. With `AFTER_REREAD`
/// never fired (the *test's own* write injection removed, e.g. by making
/// `race::write_posts_after_reread` arm nothing), the extra write never
/// lands at all, so the second dispatch below is a hit and `renders()`
/// stays `1` where the assertion requires `2` - fails at that assertion.
/// With the *production* lookup-time coherence check short-circuited to
/// always report coherent (`authority_coherence` returning `Coherent`
/// unconditionally, which is the mechanism `coherence()` calls on every
/// hit), the stale entry is served as fresh regardless of what the write
/// did, and `renders()` stays `1` at the same assertion for a different
/// reason. Both were run against the production sabotage and both fail
/// there; see the task report's R74 table for the exact lines and the
/// masking checks (`no_publish`, `bypass`) that also correctly fail this
/// test rather than passing vacuously.
#[tokio::test]
#[serial_test::serial]
async fn a_write_between_the_fresh_reread_and_publication_is_caught_at_the_next_lookup() {
    let harness = boot_with_render_cache().await;
    race::write_posts_after_reread(&harness);

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the first dispatch is a plain miss and renders"
    );

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "the published entry's observed generations are behind the ledger (the race hook's \
         write landed after this render's own fresh reread), so it is a miss"
    );

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "the rebuilt entry's own reread saw no further race, so it is coherent and served \
         as a fresh hit"
    );
}

/// An epoch advance that lands between a request capturing the epoch for
/// its `RenderJob` and that render's own fresh reread bakes staleness into
/// the render by construction: the fresh reread reads the epoch again,
/// finds it moved, and the candidate is never published at all - not
/// merely served once and then missed, the way a moved dependency
/// generation is in the test above. Proven by directly inspecting the
/// store: no entry exists for the route after the raced render.
#[tokio::test]
#[serial_test::serial]
async fn an_epoch_advance_during_a_render_discards_the_candidate() {
    let harness = boot_with_render_cache().await;
    race::advance_epoch_during_next_render(&harness);

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the render still runs; only publication is declined"
    );

    let key = RenderCache::key_for_route_for_test("/cached/{id}", &[("id", "1")], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_none(),
        "a candidate rendered under an old epoch is never published"
    );

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "the epoch hook was one-shot; this second render sees no further race and publishes"
    );

    // Fix round 1, R98/F2: a positive control. Without it, this test is
    // satisfied by any run that never publishes anything at all (a broken
    // `store_entry`, or the middleware removed from the request path
    // entirely both leave the `inspect` above `None` and `renders()` at 1
    // for reasons that have nothing to do with the epoch race). Requiring a
    // *hit* here means the un-raced render really did publish, so a build
    // that cannot publish turns this red instead of green.
    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "control: the second render did publish, so this dispatch is a hit"
    );
}

/// A background rebuild of a stale-servable entry actually refreshes the
/// stored output (proven by the `Warning`/`Age` headers and the render
/// count, not merely by the rebuild having started); a write raced into a
/// *second* background rebuild's render window discards that candidate
/// instead of overwriting the entry the first rebuild published, the same
/// "moved generation discards the candidate" mechanism
/// `render_cache_middleware.rs`'s own foreground tests already prove,
/// exercised here through the background path instead.
#[tokio::test]
#[serial_test::serial]
async fn a_background_rebuild_publishes_fresh_output_and_a_write_during_it_discards_the_candidate()
{
    let harness = boot_with_render_cache().await;
    let key = RenderCache::key_for_route_for_test("/stale/{id}", &[("id", "1")], None);

    dispatch_get(&harness, "/stale/1", &[]).await;
    assert_eq!(counting_route::renders(), 1);
    // Fix round 1, R98/F5: a precondition, not a race hook, but load-bearing
    // for failure mode: without it, a build that cannot publish at all
    // turns the *next* dispatch below into a foreground miss instead of a
    // stale hit, which would then render synchronously inside the armed
    // hold with nothing else left to call `release_render` - a hang, not a
    // red test. Asserting here instead makes that failure mode a clean,
    // immediate red.
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_some(),
        "precondition: the first render published"
    );
    clock(&harness).advance_ms(70_000);

    // A clean background rebuild: no race, just proof that it refreshes.
    counting_route::hold_next_render(&harness);
    let stale = dispatch_get(&harness, "/stale/1", &[]).await;
    assert!(
        stale.header("warning").is_some(),
        "the client-visible dispatch is served from the entry the rebuild is about to replace"
    );
    // `wait_until_rendering_count` counts renders started; `wait_until_background_finished`
    // counts leases released. They agree at `2` here only because every
    // render in this test is a lead's own render (no plain hit or `Wait`
    // dispatch is mixed in) - see fix round 1, R98/F9. Don't assume they
    // stay in lockstep if this test grows one.
    counting_route::wait_until_rendering_count(&harness, 2).await;
    counting_route::release_render(&harness);
    race::wait_until_background_finished(&harness, 2).await;

    let first_rebuild = RenderCache::inspect(&key)
        .await
        .expect("inspect")
        .expect("the background rebuild published");

    let fresh = dispatch_get(&harness, "/stale/1", &[]).await;
    assert!(fresh.header("warning").is_none());
    assert_eq!(fresh.header("age"), Some("0"));
    assert_eq!(
        counting_route::renders(),
        2,
        "the rebuild rendered once; this dispatch is a hit"
    );

    // Race a write into a second background rebuild: the candidate must be
    // discarded, leaving the first rebuild's own publish authoritative.
    clock(&harness).advance_ms(70_000);
    counting_route::hold_next_render(&harness);
    dispatch_get(&harness, "/stale/1", &[]).await;
    // See the `2`/`2` pairing above (R98/F9): same coincidence, same
    // caveat, now at `3`.
    counting_route::wait_until_rendering_count(&harness, 3).await;
    counting_route::write_during_next_render(&harness);
    counting_route::release_render(&harness);
    race::wait_until_background_finished(&harness, 3).await;

    // `published_at_ms` unchanged, not a header, is the proof here: a
    // dispatch after a raced-and-discarded background rebuild is a *stale
    // hit* on the entry the first rebuild published (the moved coherence
    // check floors the effective age at `fresh_ms`, and `/stale/{id}`'s
    // policy still has this age within its stale-servable window), not a
    // foreground render - so it carries `Age`/`Warning` like any other
    // stale hit and neither header distinguishes "discarded" from "never
    // raced". `published_at_ms` does, and only because the clock advanced
    // 70_000ms between the two rebuilds: a publish would have written a
    // different value, which is exactly what the sabotage build below (see
    // the task report's R74 table) produces when the discard is disabled.
    let after_race = RenderCache::inspect(&key)
        .await
        .expect("inspect")
        .expect("the earlier publish is still there");
    assert_eq!(
        after_race.published_at_ms, first_rebuild.published_at_ms,
        "a candidate raced by a write during its render is discarded; the entry the first \
         rebuild published stays authoritative"
    );
}

/// A leader holding one key's render never blocks a request for a
/// different key: the coordinator admits by key, not globally. Proven by
/// dispatching the second key's request while the first is deliberately
/// held, and requiring it to complete before the first is ever released.
#[tokio::test]
#[serial_test::serial]
async fn two_keys_rebuild_independently_while_one_leader_is_held() {
    let harness = boot_with_render_cache().await;

    counting_route::hold_next_render(&harness);
    let held = tokio::spawn({
        let h = harness.clone();
        async move { dispatch_get(&h, "/cached/1", &[]).await }
    });
    counting_route::wait_until_rendering_count(&harness, 1).await;

    let other = dispatch_get(&harness, "/cached/2", &[]).await;
    assert_eq!(
        other.status,
        StatusCode::OK,
        "a different key is never blocked by another key's leader"
    );

    counting_route::release_render(&harness);
    held.await.expect("held");
    assert_eq!(counting_route::renders(), 2);

    // Fix round 1, R98/F3: a positive control. Without it, this test is
    // satisfied even with the cache removed from the request path
    // entirely - two independent renders that never touch the coordinator
    // also produce two 200s and `renders() == 2`. Requiring both keys to
    // actually be published ties the test to the cache genuinely being
    // involved, not merely to two ordinary handlers not blocking each
    // other.
    for id in ["1", "2"] {
        let key = RenderCache::key_for_route_for_test("/cached/{id}", &[("id", id)], None);
        assert!(
            RenderCache::inspect(&key).await.expect("inspect").is_some(),
            "control: /cached/{id} really went through the cache and published"
        );
    }
}
