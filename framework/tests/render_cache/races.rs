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

use crate::render_cache_middleware_support;
use render_cache_middleware_support::{
    boot_with_render_cache, clock, counting_route, dispatch_get, race,
    wait_until_background_finished,
};
use suprnova::StatusCode;
use suprnova::render_cache::RenderCache;

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

/// A write that commits *before* the render's consistent read view even
/// opens is not a race at all, and the middleware must not mistake it for
/// one: the render reads the written row, the window it closes records the
/// generation that write produced, and the fresh reread agrees with it. The
/// entry publishes and the next request is served from it.
///
/// This is the lower boundary of the race window `AFTER_REREAD` and
/// `AFTER_VIEW_CLOSE` bound from the other side. Without it, "a write near
/// a render discards the candidate" would be satisfied by a middleware that
/// discards on *every* nearby write, which would make the cache useless on
/// any write-active table rather than merely correct.
///
/// `/builder-read` rather than `/cached/{id}`: its body carries the row
/// count (`sees N posts`), so the served bytes themselves show which side
/// of the view the write landed on - the render count alone cannot.
#[tokio::test]
#[serial_test::serial]
async fn a_write_before_the_read_view_opens_is_rendered_into_the_entry_and_served() {
    let harness = boot_with_render_cache().await;
    race::write_posts_before_view(&harness);

    let rendered = dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the first dispatch is a plain miss and renders"
    );
    assert!(
        String::from_utf8_lossy(&rendered.body).contains("sees 1 posts"),
        "the write landed before the read view opened, so the render read it - got {:?}",
        String::from_utf8_lossy(&rendered.body)
    );

    let served = dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "nothing moved between the render and its own reread, so the candidate published \
         and this dispatch is a hit"
    );
    assert_eq!(
        served.body, rendered.body,
        "the stored body is the one that saw the write"
    );
}

/// A write that commits after the render's read view has closed but before
/// the fresh reread runs is caught by that reread: the candidate is
/// discarded and nothing is published at all - not published and then
/// missed on the next lookup, the way a write landing *after* the reread is
/// (see the first test in this file). Proven by inspecting the store
/// directly.
#[tokio::test]
#[serial_test::serial]
async fn a_write_after_the_read_view_closes_is_caught_by_the_reread_and_discards_the_candidate() {
    let harness = boot_with_render_cache().await;
    race::write_posts_after_view_close(&harness);

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the render still runs; only publication is declined"
    );

    let key = RenderCache::key_for_route_for_test("/cached/{id}", &[("id", "1")], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_none(),
        "the reread saw the write and the candidate was never published"
    );

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "nothing was stored, so the next request renders again"
    );
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_some(),
        "the race hook was one-shot; this second render published"
    );

    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "control: the second render did publish, so this dispatch is a hit"
    );
}

/// A write that lands *inside* the fresh reread - after it has read the
/// ledger, before it compares what it read against what the render observed
/// - is on the far side of the reread's own snapshot, so the comparison
/// passes and the candidate publishes carrying observations that are
/// already behind. The lookup-time coherence check is where it is caught:
/// the next request finds the stored entry moved and renders instead of
/// serving it.
///
/// The upper boundary of the same window `AFTER_REREAD` covers from just
/// outside it: this proves the comparison genuinely judges the values the
/// reread read, not values re-read at comparison time (which would catch
/// this write and publish nothing).
#[tokio::test]
#[serial_test::serial]
async fn a_write_during_the_reread_publishes_a_stale_entry_that_the_next_lookup_misses() {
    let harness = boot_with_render_cache().await;
    race::write_posts_during_reread(&harness);

    dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the first dispatch is a plain miss and renders"
    );

    // Task 5b review: what separates this seam from `AFTER_VIEW_CLOSE` is
    // that the candidate *is* published here and caught at the next lookup,
    // rather than discarded before it is ever stored. Without this
    // assertion the two seams are indistinguishable from the render counts
    // alone - the sibling test above asserts `is_none()` at exactly this
    // point for exactly that reason.
    let key = RenderCache::key_for_route_for_test("/builder-read", &[], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_some(),
        "the write landed inside the reread, so the comparison passed and the stale entry \
         was published rather than discarded"
    );

    let rebuilt = dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "whatever the first dispatch left behind, this lookup does not serve it: the write \
         landed inside the reread, so any entry it published carries observations the \
         ledger has already moved past"
    );
    assert!(
        String::from_utf8_lossy(&rebuilt.body).contains("sees 1 posts"),
        "the re-render reads the raced write - got {:?}",
        String::from_utf8_lossy(&rebuilt.body)
    );

    let served = dispatch_get(&harness, "/builder-read", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "the race hook was one-shot; the rebuilt entry is coherent and this dispatch is a hit"
    );
    assert_eq!(
        served.body, rebuilt.body,
        "and it serves the body that saw the write"
    );
}

/// Ruling R18: a singleflight waiter on a `StaleOnError` entry is answered
/// the same way a request that had arrived a moment earlier would have
/// been. Both rebuild; both, when that rebuild fails, serve the stale bytes
/// under `Warning` rather than handing the failure to the client.
///
/// This is the client-visible half of the ruling, and it holds because both
/// requests entered through `serve`'s own `StaleOnError` arm, so the
/// fallback that wraps that arm wraps the waiting one too. It passes before
/// and after the fix and is a guard, not its proof; the arm the fix
/// actually changed - a waiter that re-evaluates *onto* a stale-on-error
/// entry it did not arrive on - is reached by
/// [`a_waiter_that_re_evaluates_onto_a_stale_on_error_entry_falls_back_to_it`]
/// below.
///
/// Both renders are armed to fail through `fail_next_render`, which is
/// one-shot and, in `stale_handler`, consumed *after* `on_render_start`
/// returns - so a held render consumes its arming only once released. Each
/// render is therefore held first and armed second: the leader while it is
/// held, then the waiter's own rebuild while *it* is held. Every wait here
/// is a state barrier - the render-started count, the coordinator's waiter
/// count, and its release count - and none of it waits on time.
#[tokio::test]
#[serial_test::serial]
async fn a_waiter_behind_a_failed_leader_is_served_the_stale_entry_it_was_waiting_on() {
    let harness = boot_with_render_cache().await;
    let original = dispatch_get(&harness, "/stale/1", &[]).await;
    assert_eq!(original.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);
    assert!(
        original.header("warning").is_none(),
        "precondition: a fresh publish is not stale"
    );

    // `/stale/{id}` is fresh 60_000, stale-servable 60_000, stale-on-error
    // 120_000. age 130_000 gives past_fresh = 70_000, inside
    // [stale_servable_ms, stale_on_error_ms): the StaleOnError band, which
    // is the one band where a request rebuilds in the foreground and still
    // has somewhere to fall back to.
    clock(&harness).advance_ms(130_000);

    counting_route::hold_next_render(&harness);
    let leader = {
        let h = harness.clone();
        tokio::spawn(async move { dispatch_get(&h, "/stale/1", &[]).await })
    };
    // The `original` dispatch already rendered once, so the leader's held
    // render is the second; see `wait_until_rendering_count`'s own doc for
    // why the count has to be explicit.
    counting_route::wait_until_rendering_count(&harness, 2).await;

    let waiter = {
        let h = harness.clone();
        tokio::spawn(async move { dispatch_get(&h, "/stale/1", &[]).await })
    };
    counting_route::wait_until_waiting(&harness, 1).await;

    // The leader is held and the waiter is parked behind it. Arm the
    // leader's failure and the hold that will catch the waiter's own
    // rebuild, then let the leader go: it fails, publishes nothing, and
    // releases the lease the waiter is waiting on.
    counting_route::fail_next_render(&harness);
    counting_route::hold_next_render(&harness);
    counting_route::release_render(&harness);

    // The waiter re-admitted itself and its own rebuild is now held, before
    // it has decided anything. Arm its failure and let it go.
    counting_route::wait_until_rendering_count(&harness, 3).await;
    counting_route::fail_next_render(&harness);
    counting_route::release_render(&harness);

    let (leader_response, waiter_response) =
        (leader.await.expect("leader"), waiter.await.expect("waiter"));

    assert_eq!(
        leader_response.status,
        StatusCode::OK,
        "the leader's own failed rebuild falls back to the stale entry"
    );
    assert_eq!(
        waiter_response.status,
        StatusCode::OK,
        "and so does the waiter's: a client that queued behind a failing leader must not \
         receive the failure the client ahead of it was shielded from"
    );
    assert_eq!(
        waiter_response.header("warning"),
        Some("110 - \"Response is Stale\""),
        "and it is answered honestly, under the same Warning the leader got"
    );
    assert_eq!(
        leader_response.header("warning"),
        Some("110 - \"Response is Stale\"")
    );
    assert_eq!(
        waiter_response.body, original.body,
        "the bytes served are the stale entry's own, not a fresh render's"
    );
    assert_eq!(leader_response.body, original.body);

    // Three leads have now run and released: the original publish, the
    // leader's failed rebuild, and the waiter's own. Waiting on the release
    // count is what makes the assertions below read a settled store rather
    // than one with a publish decision still in flight.
    wait_until_background_finished(&harness, 3).await;
    assert_eq!(
        counting_route::renders(),
        3,
        "both rebuild attempts really ran: neither request was served the stale entry \
         without first trying to replace it"
    );
    let key = RenderCache::key_for_route_for_test("/stale/{id}", &[("id", "1")], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_some(),
        "and the stale entry both fell back to is still the stored one: a failed rebuild \
         publishes nothing"
    );
}

/// Ruling R18, the arm the fix actually changed: a singleflight waiter that
/// arrived on *no* entry, and whose re-evaluation after the wait lands on a
/// `StaleOnError` one, falls back to that entry when its own rebuild fails.
///
/// Reaching the waiter's own `StaleOnError` arm takes a shape the plain
/// stale route cannot produce, because a request that is already
/// stale-on-error at its first lookup enters through `serve`'s arm and is
/// wrapped there. `/stale-error-only/{id}` declares no stale-servable
/// window at all, so an entry whose observations are behind the ledger -
/// floored to `past_fresh == 0` by `freshness_state` - lands in the
/// stale-on-error band rather than the servable one. The leader here
/// publishes exactly such an entry (`AFTER_REREAD` lands a write between
/// its coherent reread and its store), so the waiter, which found nothing
/// on its own first lookup, re-evaluates onto a stale-on-error entry it
/// never arrived on.
///
/// Before the fix the waiter called `render_and_publish` there with no
/// fallback, and its 500 reached the client; the stale bytes it was sitting
/// on were never offered. Proven by revert: dropping the `StaleOnError`
/// branch from the waiter arm leaves the last assertions below reading
/// `500` where they require `200` and the stale `Warning`.
#[tokio::test]
#[serial_test::serial]
async fn a_waiter_that_re_evaluates_onto_a_stale_on_error_entry_falls_back_to_it() {
    let harness = boot_with_render_cache().await;

    // Cold: neither request finds an entry, so both reach admission through
    // the miss path rather than through `serve`'s stale-on-error arm.
    counting_route::hold_next_render(&harness);
    let leader = {
        let h = harness.clone();
        tokio::spawn(async move { dispatch_get(&h, "/stale-error-only/1", &[]).await })
    };
    counting_route::wait_until_rendering_count(&harness, 1).await;

    let waiter = {
        let h = harness.clone();
        tokio::spawn(async move { dispatch_get(&h, "/stale-error-only/1", &[]).await })
    };
    counting_route::wait_until_waiting(&harness, 1).await;

    // The leader publishes an entry that is already behind the ledger, and
    // the hold is re-armed so the waiter's own rebuild can be caught before
    // it decides anything.
    race::write_posts_after_reread(&harness);
    counting_route::hold_next_render(&harness);
    counting_route::release_render(&harness);

    counting_route::wait_until_rendering_count(&harness, 2).await;
    counting_route::fail_next_render(&harness);
    counting_route::release_render(&harness);

    let (leader_response, waiter_response) =
        (leader.await.expect("leader"), waiter.await.expect("waiter"));

    assert_eq!(
        leader_response.status,
        StatusCode::OK,
        "the leader's own render succeeded and published"
    );
    assert!(
        leader_response.header("warning").is_none(),
        "precondition: the leader served its own fresh render, not a stale entry"
    );
    assert_eq!(
        waiter_response.status,
        StatusCode::OK,
        "the waiter's rebuild failed, and a stale-on-error window is exactly what a failed \
         rebuild is supposed to fall back into - whichever path reached it"
    );
    assert_eq!(
        waiter_response.header("warning"),
        Some("110 - \"Response is Stale\""),
        "and it says so"
    );
    assert_eq!(
        waiter_response.body, leader_response.body,
        "the bytes served are the entry the leader published, which is what the waiter was \
         waiting on all along"
    );
    assert_eq!(
        counting_route::renders(),
        2,
        "one render each: the leader's publish and the waiter's failed rebuild"
    );
}
