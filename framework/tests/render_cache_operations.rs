//! Task 16: operator control over the RenderCache - an epoch advance makes
//! every existing entry unreachable at once, inspection exposes metadata
//! only, and sweep removes both dead-by-retention and dead-by-epoch L1
//! files.
//!
//! Every test here is `#[serial_test::serial]`: `RenderCache`'s installed
//! runtime and the process-wide global middleware registry are both
//! process-global state (see `RenderCache::install`'s own doc), so two of
//! these tests running concurrently in the same test binary would install
//! over each other. Plain `#[tokio::test]` (current-thread), not
//! `flavor = "multi_thread"`: `render_cache_operations_support::boot` uses
//! `TestContainer::fake()`, which writes a thread-local, and a
//! multi-thread runtime can migrate a future between worker threads
//! between polls, making that registration invisible to whichever thread
//! resumes the test - the exact reason
//! `render_cache_middleware_support`'s own doc gives for the same choice.

mod render_cache_operations_support;
use render_cache_operations_support::{
    boot_with_file_l1, boot_with_render_cache, clock, counting_route, dispatch_get,
};
use suprnova::render_cache::{RenderCache, RepresentationClass};

#[tokio::test]
#[serial_test::serial]
async fn an_epoch_advance_makes_every_existing_entry_unreachable() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/cached/1", &[]).await;
    dispatch_get(&harness, "/cached/2", &[]).await;
    assert_eq!(counting_route::renders(), 2);

    let old_key = RenderCache::key_for_route_for_test("/cached/{id}", &[("id", "1")], None);
    assert!(
        RenderCache::inspect(&old_key)
            .await
            .expect("inspect")
            .is_some(),
        "sanity: the entry is reachable before the advance"
    );

    RenderCache::advance_epoch().await.expect("advance");

    dispatch_get(&harness, "/cached/1", &[]).await;
    dispatch_get(&harness, "/cached/2", &[]).await;
    assert_eq!(
        counting_route::renders(),
        4,
        "keys are epoch-namespaced; the pre-advance entries are unreachable \
         by ordinary lookup, so both routes render again - the observable \
         consequence ruling R81 points to, since RenderKey::derive bakes \
         the epoch into the lookup key itself and no request-driven code \
         path can distinguish this miss from any other"
    );

    // `key_for_route_for_test` hardcodes `epoch: 1` (see its own doc at
    // `key_input_for_test` in `middleware.rs`: "this test helper never
    // advances the epoch, so every call ... always lands on the same key
    // regardless of when it runs in a test") - it cannot recompute a
    // post-advance key, and `RenderCache::inspect` is a raw store lookup
    // by exact key with no freshness or epoch check of its own (see its
    // own doc: "Body-free inspection of a stored L0 entry"). So `old_key`
    // still names a real L0 entry after the advance - nothing removes it -
    // and asserting `inspect(&old_key).is_none()` here would be asserting
    // something false, not a stronger proof. The render-count assertion
    // above is the proof this test can honestly make; see the report's
    // Deviations section.
    assert!(
        RenderCache::inspect(&old_key)
            .await
            .expect("inspect")
            .is_some(),
        "the pre-advance entry is still physically present in L0 under its \
         old key - advance_epoch never evicts L0 entries, it only changes \
         what future lookups compute as the key for this route"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn inspection_exposes_metadata_and_never_a_body_or_raw_identity() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/private/1", &[("x-test-login", "user-7")]).await;

    let key = RenderCache::key_for_route_for_test("/private/{id}", &[("id", "1")], Some("user-7"));
    let inspection = RenderCache::inspect(&key)
        .await
        .expect("inspect")
        .expect("entry");
    assert_eq!(inspection.class, RepresentationClass::PrivateCached);
    assert!(inspection.body_bytes > 0);
    let text = format!("{inspection:?}");
    assert!(
        !text.contains("user-7") && !text.contains("<html"),
        "EntryInspection carries no raw identity and no body: {text}"
    );
    assert!(key.starts_with("rk1."));

    let store = RenderCache::store_inspection().await.expect("store");
    assert_eq!(store.entries, 1);
    assert!(store.bytes > 0);
}

#[tokio::test]
#[serial_test::serial]
async fn sweep_removes_dead_files_and_files_from_older_epochs() {
    let (harness, dir) = boot_with_file_l1().await;
    dispatch_get(&harness, "/stale/1", &[]).await;
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("dir").count(),
        1,
        "the stale route uses StorageLayers::l0_and_l1, so the first render \
         publishes one file"
    );

    // `/stale/{id}`'s policy is fresh 60_000, stale-servable 60_000,
    // stale-on-error 120_000 (see the support module); the middleware
    // frames the L1 retention as fresh_ms + stale_on_error_ms = 180_000
    // (see `store_entry` in `middleware.rs` and
    // `coherence::evaluate_freshness`, which reaches `Dead` at exactly that
    // age for a non-private class).
    clock(&harness).advance_ms(180_000 + 1);
    assert_eq!(
        RenderCache::sweep().await.expect("sweep"),
        1,
        "past its retention window the file is dead"
    );
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);

    dispatch_get(&harness, "/stale/1", &[]).await;
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("dir").count(),
        1,
        "the L0 entry is stale too (evaluate_freshness reaches Dead at the \
         same age), so this dispatch is a fresh render that republishes"
    );

    RenderCache::advance_epoch().await.expect("advance");
    assert_eq!(
        RenderCache::sweep().await.expect("sweep"),
        1,
        "the republished file's fence epoch now predates the ledger's \
         epoch, so sweep removes it even though its retention window has \
         not elapsed"
    );
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
}
