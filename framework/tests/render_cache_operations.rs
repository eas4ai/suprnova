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
use suprnova::render_cache::console::{epoch_advance_report_for_test, inspect_report_for_test};
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

    // Fix round 1 (R94/F11): `advance_epoch` now clears L0 outright, so
    // the pre-advance entry is gone regardless of what key names it -
    // `key_for_route_for_test` hardcoding `epoch: 1` (see its own doc at
    // `key_input_for_test` in `middleware.rs`) no longer matters here: a
    // cleared store has nothing under any key. Before this fix round,
    // `old_key` still named a real, merely-unreachable-by-ordinary-lookup
    // L0 entry after an advance (see the superseded round-1 report and its
    // Deviations section); that is no longer true.
    assert!(
        RenderCache::inspect(&old_key)
            .await
            .expect("inspect")
            .is_none(),
        "the pre-advance entry is gone: advance_epoch clears L0 outright"
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
    // frames the L1 retention as `policy.freshness().dead_after_ms()` =
    // `fresh_ms + max(stale_servable_ms, stale_on_error_ms)` = 180_000
    // (see `store_entry` in `middleware.rs` and
    // `coherence::evaluate_freshness`, which reaches `Dead` at exactly that
    // age for a non-private class - this route's stale_on_error_ms is
    // already the wider band, so this matches the pre-fix-round formula's
    // number too; `sweep_the_dead_edge_uses_the_widest_stale_band_not_stale_on_error_alone`
    // below is the test that only the correct formula can pass).
    clock(&harness).advance_ms(180_000 + 1);
    let outcome = RenderCache::sweep().await.expect("sweep");
    assert_eq!(
        outcome.removed, 1,
        "past its retention window the file is dead"
    );
    assert!(!outcome.more_remain);
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
        RenderCache::sweep().await.expect("sweep").removed,
        1,
        "the republished file's fence epoch now predates the ledger's \
         epoch, so sweep removes it even though its retention window has \
         not elapsed"
    );
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
}

/// Fix round 1 (R93/F2, F3): pins `store_entry`'s retention to the
/// *policy's* Dead edge, not to `0`, `u64::MAX`, or the pre-fix-round
/// `fresh_ms + stale_on_error_ms` formula. `/inverted/{id}` declares
/// `stale_servable_ms` (120_000) wider than `stale_on_error_ms` (0), a
/// shape `FreshnessPolicy::new` explicitly permits; the old formula would
/// give 60_000, `0` would sweep it immediately, and `u64::MAX` would never
/// sweep it - only `fresh_ms + max(stale_servable_ms, stale_on_error_ms)`
/// (180_000) gets both checks below right.
#[tokio::test]
#[serial_test::serial]
async fn sweep_the_dead_edge_uses_the_widest_stale_band_not_stale_on_error_alone() {
    let (harness, dir) = boot_with_file_l1().await;
    dispatch_get(&harness, "/inverted/1", &[]).await;
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 1);

    // Past the old, wrong formula's edge (60_000) but nowhere near the
    // true one (180_000): reverting `store_entry` to `fresh_ms +
    // stale_on_error_ms`, or to a `0` retention, would already have this
    // file swept by now.
    clock(&harness).advance_ms(120_000);
    let too_early = RenderCache::sweep().await.expect("sweep");
    assert_eq!(
        too_early.removed, 0,
        "at 120_000 elapsed the entry is still StaleServable, not Dead - a \
         wrong or zero retention would have swept it already"
    );
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 1);

    // Past the true Dead edge: reverting `store_entry` to `u64::MAX`
    // (never age-swept) would leave this file alive forever.
    clock(&harness).advance_ms(60_001);
    let at_dead_edge = RenderCache::sweep().await.expect("sweep");
    assert_eq!(
        at_dead_edge.removed, 1,
        "at fresh_ms + max(stale_servable_ms, stale_on_error_ms) (180_000 \
         elapsed) the entry is genuinely Dead"
    );
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
}

/// Fix round 1 (R94/F11): an epoch advance clears L0 immediately rather
/// than leaving unreachable entries to age out.
#[tokio::test]
#[serial_test::serial]
async fn advance_epoch_clears_l0() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/cached/1", &[]).await;
    assert_eq!(
        RenderCache::store_inspection()
            .await
            .expect("store")
            .entries,
        1
    );

    RenderCache::advance_epoch().await.expect("advance");

    let store = RenderCache::store_inspection().await.expect("store");
    assert_eq!(store.entries, 0, "L0 is emptied by the epoch advance");
    assert_eq!(store.bytes, 0);
}

/// Fix round 1 (R95/F4, ported from the review's console probe): both
/// hidden commands are actually registered and actually hidden from
/// `--help`, not merely present as source that nothing links in.
#[test]
fn both_console_commands_are_registered_and_hidden() {
    for name in ["render-cache:epoch-advance", "render-cache:inspect"] {
        let entry = suprnova::console::list()
            .into_iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        let command = (entry.clap_builder)();
        assert!(command.is_hide_set(), "{name} must be hidden from --help");
    }
}

/// Fix round 1 (R95/F4, F10): `render-cache:epoch-advance` runs end to end
/// through the real console dispatcher and its printed report names the
/// new epoch value, not just the word "advanced".
#[tokio::test]
#[serial_test::serial]
async fn console_epoch_advance_reports_the_new_epoch_value() {
    let _harness = boot_with_render_cache().await;

    let report = epoch_advance_report_for_test()
        .await
        .expect("epoch advance succeeds with a runtime installed");
    let epoch_now = RenderCache::store_inspection().await.expect("store").epoch;
    assert!(
        report.contains(&epoch_now.to_string()),
        "the printed report must name the epoch it advanced to: {report:?}"
    );

    suprnova::console::dispatch_argv(vec![
        "console".to_owned(),
        "render-cache:epoch-advance".to_owned(),
    ])
    .await
    .expect("the real command dispatcher runs the command and succeeds");
}

/// Fix round 1 (R95/F4, F10): `render-cache:inspect` runs end to end, its
/// report is bounded and carries no raw identity or body, and it names the
/// current epoch alongside the entry's own.
#[tokio::test]
#[serial_test::serial]
async fn console_inspect_reports_metadata_and_current_epoch_bounded_and_body_free() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/private/1", &[("x-test-login", "user-7")]).await;
    let key = RenderCache::key_for_route_for_test("/private/{id}", &[("id", "1")], Some("user-7"));

    let report = inspect_report_for_test(&key)
        .await
        .expect("inspect succeeds for a real key");
    assert!(report.len() < 512, "bounded text: {report}");
    assert!(!report.contains("user-7"), "no raw identity: {report}");
    assert!(!report.contains("<html"), "no body: {report}");
    // `EntryInspection`'s own `Debug` output already contains the word
    // "epoch" (its `epoch: u64` field), so asserting on that literal
    // substring alone would pass whether or not this report also names
    // the *current* ledger epoch - not what F10 actually added. Assert on
    // the specific "current epoch: <value>" text instead, using a value
    // read independently through `store_inspection`.
    let current_epoch = RenderCache::store_inspection().await.expect("store").epoch;
    assert!(
        report.contains(&format!("current epoch: {current_epoch}")),
        "the current epoch (distinct from the entry's own) is visible: {report}"
    );

    suprnova::console::dispatch_argv(vec![
        "console".to_owned(),
        "render-cache:inspect".to_owned(),
        key,
    ])
    .await
    .expect("the real command dispatcher runs the command and succeeds for a real key");
}

/// Fix round 1 (R95/F9, ported from the review's console probe):
/// `render-cache:inspect` used to swallow every failure into a printed
/// message and `Ok(())`, making an unparseable key indistinguishable from
/// success at the exit code. It must now propagate, the way
/// `render-cache:epoch-advance` always did.
#[tokio::test]
#[serial_test::serial]
async fn console_inspect_propagates_failure_for_an_unparseable_key() {
    let _harness = boot_with_render_cache().await;

    let result = suprnova::console::dispatch_argv(vec![
        "console".to_owned(),
        "render-cache:inspect".to_owned(),
        "not-a-key".to_owned(),
    ])
    .await;
    assert!(
        result.is_err(),
        "an unparseable key must fail the command, not resolve Ok(())"
    );
}
