//! Credible generation hints: what one does to a validation lease, the
//! several things it cannot do, and the proof that a deployment with hints
//! off and a deployment whose channel is dead behave identically.
//!
//! Every test here runs without a Redis. The seam
//! `RenderCache::deliver_hint_for_test` hands a message body to the same
//! decode, the same bound, the same application, and the same single
//! telemetry outcome the subscriber's own applier runs, so what these
//! assert about a payload is what a subscribing node would do with it. The
//! two tests that need a live Redis - a hint crossing between two nodes,
//! and a subscriber dropped for falling behind - are `#[ignore]`d in
//! `tiers/redis.rs` with the rest of the Tier 2 suite.
//!
//! `/short-leased/{id}` is the route throughout: five fresh minutes against
//! a ten second lease, so an authority read within the fresh window can
//! only be the lease having ended, never the entry having aged out. The
//! statement counter is what sees that read; `counting_route::renders()`
//! is what sees a rebuild.
//!
//! Gated on the `testing` feature like `bypass.rs` and for the same
//! reason: the seams these drive only exist in the library under it.
#![cfg(feature = "testing")]

use suprnova::StatusCode;
use suprnova::render_cache::hints::MAX_HINT_DIGESTS;
use suprnova::render_cache::{DependencyIdentity, HintsConfig, RenderCache, telemetry};

use crate::render_cache_middleware_support;
use crate::render_cache_tiers_support;
use render_cache_middleware_support::{
    advance_posts, boot_with_render_cache_and_hints_for_test, clock, counting_route, dispatch_get,
    statements,
};
use render_cache_tiers_support::redis_config_on_a_closed_port;

/// The lease-mode route every test here drives.
const LEASED: &str = "/short-leased/1";

/// A digest `/short-leased/{id}`'s render genuinely observes: its handler
/// calls `Post::find`, and every `Model::find` observes its table.
fn observed_digest() -> [u8; 32] {
    DependencyIdentity::table("posts").digest()
}

/// A digest no entry in this harness observes.
fn unobserved_digest() -> [u8; 32] {
    DependencyIdentity::config("nothing-in-this-harness-reads-this").digest()
}

/// The hint outcomes recorded so far, with `subscriber_dropped` removed.
///
/// A runtime installed by an earlier test in this binary keeps its
/// subscriber until the next install replaces it, and a subscriber pointed
/// at nothing records that label as it retries. It is not an outcome of
/// applying a message, so filtering it is what keeps these assertions about
/// the message under test rather than about test ordering.
fn message_outcomes() -> Vec<&'static str> {
    telemetry::recorded_hints_for_test()
        .into_iter()
        .filter(|outcome| *outcome != "subscriber_dropped")
        .collect()
}

/// Delivers one hint naming `digests`, as a subscriber would.
fn deliver(digests: &[[u8; 32]]) {
    RenderCache::deliver_hint_for_test(&RenderCache::hint_body_for_test(digests));
}

/// A hint naming a digest the entry observes makes the very next lookup
/// reread the authority, on a request that would otherwise have been served
/// under a still-valid lease and no statement at all.
///
/// Discriminating in both directions: the dispatch before the hint is
/// asserted to issue zero statements, so the one after it issuing exactly
/// one is the hint's doing and not the route's baseline. Nothing has
/// actually moved in the database, so the reread finds the entry coherent
/// and it still serves - the hint changed *when* this node revalidated and
/// nothing else, which is the whole of what a hint may do.
#[tokio::test]
#[serial_test::serial]
async fn a_hint_naming_an_observed_digest_makes_the_next_lookup_revalidate_earlier() {
    telemetry::reset_recorded_hints_for_test();
    let harness = boot_with_render_cache_and_hints_for_test(HintsConfig::Disabled).await;

    dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(counting_route::renders(), 1, "the first request renders");
    dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the second request grants the lease"
    );

    statements::reset();
    let leased = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(leased.status, StatusCode::OK);
    assert_eq!(
        statements::count(),
        0,
        "baseline: within the lease this route reaches the database not at all"
    );

    deliver(&[observed_digest()]);
    assert_eq!(
        message_outcomes(),
        vec!["applied"],
        "the hint named a digest a lease here observes"
    );

    statements::reset();
    let after_hint = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(after_hint.status, StatusCode::OK);
    assert_eq!(
        statements::count(),
        1,
        "the hint shortened the lease, so this request rereads the authority - one \
         statement, the batched generation-and-epoch read"
    );
    assert_eq!(
        counting_route::renders(),
        1,
        "nothing had actually moved, so the reread found the entry coherent and it served"
    );
}

/// No hint causes an entry to be served that the coherence check would have
/// refused.
///
/// The dependency really moves, so the check genuinely refuses. Three
/// messages are delivered across that refusal - one naming the digest that
/// moved, one naming a digest nothing here observes, and one naming no
/// digest at all - and none of them makes the superseded entry serve: the
/// route rebuilds and the body that comes back is the new render's.
#[tokio::test]
#[serial_test::serial]
async fn no_hint_makes_a_refused_entry_serve() {
    telemetry::reset_recorded_hints_for_test();
    let harness = boot_with_render_cache_and_hints_for_test(HintsConfig::Disabled).await;

    dispatch_get(&harness, LEASED, &[]).await;
    let first = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(counting_route::renders(), 1);
    assert_eq!(first.body, "cached render 1");

    // The entry's dependency moves under it. Its lease is still valid, so
    // only a revalidation can discover that - which is exactly what the
    // first hint below provokes.
    advance_posts(&harness).await;

    deliver(&[unobserved_digest()]);
    deliver(&[]);
    deliver(&[observed_digest()]);

    let after = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        2,
        "the coherence check refused the stored entry and the route rebuilt"
    );
    assert_eq!(
        after.body, "cached render 2",
        "the response is the rebuild, never the entry the check refused"
    );
}

/// A message carrying more than `MAX_HINT_DIGESTS` digests is dropped
/// whole, never truncated.
///
/// The digest this node genuinely observes is the *first* one in the
/// message, so a receiver that read the message up to its bound and stopped
/// would have applied it. Asserting that the lease is still trusted
/// afterwards - zero statements on the next request - is what proves none of
/// its digests was applied, rather than merely that some were not.
#[tokio::test]
#[serial_test::serial]
async fn a_message_over_the_digest_bound_is_dropped_whole() {
    telemetry::reset_recorded_hints_for_test();
    let harness = boot_with_render_cache_and_hints_for_test(HintsConfig::Disabled).await;

    dispatch_get(&harness, LEASED, &[]).await;
    dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(counting_route::renders(), 1, "the lease is granted");

    let mut digests = vec![observed_digest()];
    for filler in 0..MAX_HINT_DIGESTS {
        digests.push([u8::try_from(filler % 251).expect("a byte"); 32]);
    }
    assert_eq!(digests.len(), MAX_HINT_DIGESTS + 1, "one past the bound");
    deliver(&digests);

    assert_eq!(
        message_outcomes(),
        vec!["dropped_over_bound"],
        "one message, one outcome, and it is the drop"
    );

    statements::reset();
    let after = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(
        statements::count(),
        0,
        "the observed digest at the head of the over-bound message was not applied either: \
         the lease is still trusted and the authority is not consulted"
    );
    assert_eq!(counting_route::renders(), 1);
}

/// A hint naming a digest no lease here observes records
/// `ignored_unknown_key` and changes nothing, and neither does a message
/// this node cannot read at all.
#[tokio::test]
#[serial_test::serial]
async fn a_hint_naming_nothing_this_node_holds_is_ignored() {
    telemetry::reset_recorded_hints_for_test();
    let harness = boot_with_render_cache_and_hints_for_test(HintsConfig::Disabled).await;

    dispatch_get(&harness, LEASED, &[]).await;
    dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(counting_route::renders(), 1, "the lease is granted");

    deliver(&[unobserved_digest()]);
    // Not one of ours at all: a foreign publisher on a shared prefix, or a
    // build that speaks a wire this one does not.
    RenderCache::deliver_hint_for_test("not a hint at all");
    assert_eq!(
        message_outcomes(),
        vec!["ignored_unknown_key", "ignored_unknown_key"],
        "neither message named anything this node holds a lease against"
    );

    statements::reset();
    let after = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(
        statements::count(),
        0,
        "nothing changed: the lease is still trusted and no authority read happened"
    );
    assert_eq!(counting_route::renders(), 1);
}

/// A hint never extends a lease and never creates one.
///
/// The lease is allowed to expire on its own - eleven seconds against a ten
/// second lease, and nowhere near the route's five fresh minutes - and a
/// hint naming the digest that entry observes is then delivered. It finds no
/// live lease, records `ignored_unknown_key`, and leaves the table empty, so
/// the next request rereads the authority exactly as it would have with no
/// hint at all. Had the hint been able to write an expiry rather than only
/// lower one, this is where it would show.
///
/// The complementary half - a hint offered an instant *later* than a lease's
/// held expiry - is asserted directly against the table in
/// `render_cache::hints`'s own unit tests, because a message carries no
/// instant to offer: the applier always uses this node's own clock, for the
/// reason that module's documentation gives.
#[tokio::test]
#[serial_test::serial]
async fn a_hint_never_extends_or_creates_a_lease() {
    telemetry::reset_recorded_hints_for_test();
    let harness = boot_with_render_cache_and_hints_for_test(HintsConfig::Disabled).await;

    dispatch_get(&harness, LEASED, &[]).await;
    dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(counting_route::renders(), 1, "the lease is granted");
    assert_eq!(RenderCache::lease_count_for_test(), 1);

    // Past the ten second lease, far short of the five minute fresh window.
    clock(&harness).advance_ms(11_000);
    deliver(&[observed_digest()]);
    assert_eq!(
        message_outcomes(),
        vec!["ignored_unknown_key"],
        "there was no live lease left for it to name"
    );
    assert_eq!(
        RenderCache::lease_count_for_test(),
        0,
        "a hint cannot leave a lease behind where there was none"
    );

    statements::reset();
    let after = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(
        statements::count(),
        1,
        "the expired lease is gone, so this request rereads the authority once"
    );
    assert_eq!(
        counting_route::renders(),
        1,
        "and the entry is still fresh and still coherent, so it serves"
    );
}

/// Hints off and a hint channel that is dead are the same deployment.
///
/// The same sequence of requests and writes runs against a runtime with
/// `HintsConfig::Disabled` and against one configured to reach a Redis that
/// is not there, and the two runs are compared whole: the same bodies, the
/// same statuses, the same rebuilds admitted, and the same number of
/// authority reads at each step. This is the test that proves the feature is
/// optional.
#[tokio::test]
#[serial_test::serial]
async fn a_dead_hint_channel_behaves_exactly_like_hints_switched_off() {
    /// One step's observable result: what was served, and what it cost.
    #[derive(Debug, Eq, PartialEq)]
    struct Step {
        status: StatusCode,
        body: bytes::Bytes,
        renders: u64,
        statements: u64,
    }

    async fn run(hints: HintsConfig) -> Vec<Step> {
        let harness = boot_with_render_cache_and_hints_for_test(hints).await;
        let mut steps = Vec::new();
        let step = async |path: &str| {
            statements::reset();
            let response = dispatch_get(&harness, path, &[]).await;
            Step {
                status: response.status,
                body: response.body.clone(),
                renders: counting_route::renders(),
                statements: statements::count(),
            }
        };
        // A miss, the hit that grants the lease, and a hit inside it.
        steps.push(step(LEASED).await);
        steps.push(step(LEASED).await);
        steps.push(step(LEASED).await);
        // A write moves the dependency; within the lease this node cannot
        // see it yet, and once the lease ends it rebuilds.
        advance_posts(&harness).await;
        steps.push(step(LEASED).await);
        clock(&harness).advance_ms(11_000);
        steps.push(step(LEASED).await);
        steps.push(step(LEASED).await);
        steps
    }

    let without = run(HintsConfig::Disabled).await;
    let closed = redis_config_on_a_closed_port();
    let dead = run(HintsConfig::Redis {
        url: closed.url,
        prefix: closed.prefix,
    })
    .await;

    assert_eq!(
        without, dead,
        "a node whose hint channel is dead serves the same entries and admits the same \
         rebuilds as a node built without hints at all"
    );
}

/// A hint channel that cannot be reached is not a request failure.
///
/// The endpoint is a loopback port nothing listens on, so the boot succeeds
/// (the hint channel is deliberately not one of the endpoints install proves
/// with a `PING` - see `redis_endpoints`), the subscriber records
/// `subscriber_dropped` as it fails to establish itself, and requests are
/// served throughout exactly as they would be with no channel configured.
///
/// The wait is on the telemetry barrier, not on a clock: it resolves when
/// the label has actually been recorded.
#[tokio::test]
#[serial_test::serial]
async fn an_unreachable_hint_channel_degrades_in_telemetry_and_not_in_the_response() {
    telemetry::reset_recorded_hints_for_test();
    let closed = redis_config_on_a_closed_port();
    let harness = boot_with_render_cache_and_hints_for_test(HintsConfig::Redis {
        url: closed.url,
        prefix: closed.prefix,
    })
    .await;

    telemetry::await_hint_for_test("subscriber_dropped", 1).await;

    let miss = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(miss.status, StatusCode::OK);
    let hit = dispatch_get(&harness, LEASED, &[]).await;
    assert_eq!(hit.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        1,
        "the second request was served from the cache, unaffected by the dead channel"
    );
    assert!(
        message_outcomes().is_empty(),
        "a channel that never connected delivered no message, so no message outcome was \
         recorded: the degradation is the subscription's, and it is only in telemetry"
    );
}
