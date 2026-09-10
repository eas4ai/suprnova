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
use crate::render_cache_operations_support;
use crate::render_cache_privacy_support;
use render_cache_operations_support::{
    NOT_FOUND_ROUTE, boot_with_file_l1, boot_with_render_cache, clock, counting_route, dispatch_get,
};
use suprnova::Model;
use suprnova::StatusCode;
use suprnova::attrs;
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::console::{epoch_advance_report_for_test, inspect_report_for_test};
use suprnova::render_cache::telemetry;
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

/// Fix round 2 (R99/N4): `store_entry` frames a class-aware retention, so a
/// `PrivateCached` entry is swept at its own Dead point (`fresh_ms` alone,
/// per spec 16 line 78 - private entries have bounded retention and
/// eviction independent of public entries) while a `PublicShared` entry
/// with the identical freshness numbers survives until the wider
/// `fresh_ms + max(stale_servable_ms, stale_on_error_ms)` edge.
/// `/private-l1/{id}` and `/stale/{id}` declare the same freshness
/// (60_000, 60_000, 120_000); only their class differs. Reverting
/// `store_entry` to the class-blind `dead_after_ms()` call (fix round 1's
/// shape) makes the private entry's file survive to 180_000 too, failing
/// the first sweep assertion below.
#[tokio::test]
#[serial_test::serial]
async fn sweep_the_dead_edge_is_class_aware_private_entries_die_at_fresh_ms_alone() {
    let (harness, dir) = boot_with_file_l1().await;
    dispatch_get(&harness, "/stale/1", &[]).await;
    dispatch_get(&harness, "/private-l1/1", &[("x-test-login", "user-7")]).await;
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("dir").count(),
        2,
        "both routes use StorageLayers::l0_and_l1, so both publish a file"
    );

    // Past the private Dead edge (fresh_ms alone, 60_000) but nowhere near
    // the public one (180_000).
    clock(&harness).advance_ms(60_000 + 1);
    let at_private_edge = RenderCache::sweep().await.expect("sweep");
    assert_eq!(
        at_private_edge.removed, 1,
        "only the PrivateCached entry is dead at fresh_ms alone (60_000)"
    );
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 1);

    // Past the public Dead edge too.
    clock(&harness).advance_ms(120_000);
    let at_public_edge = RenderCache::sweep().await.expect("sweep");
    assert_eq!(
        at_public_edge.removed, 1,
        "the PublicShared entry with the same numbers is dead only at \
         fresh_ms + max(stale_servable_ms, stale_on_error_ms) (180_000)"
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

/// Fix round 2 (N1): a ledger read failing after `render-cache:inspect`'s
/// primary lookup already succeeded must not print the false epoch `0`; it
/// prints an honest "unavailable" and still returns the inspection the
/// caller asked for.
///
/// The ledger read and the epoch advance's own write both execute against
/// the same `suprnova_render_epochs` table, so dropping it fails both
/// identically - this harness cannot isolate "the advance succeeds and only
/// the immediately-following read fails" as a distinct SQL-level fault (see
/// the next test's own doc for how that half is covered instead). What it
/// can isolate cleanly is this command's read, since `RenderCache::inspect`
/// itself is a pure L0 lookup with no ledger dependency at all, so its
/// primary result is unaffected by the table being gone.
#[tokio::test]
#[serial_test::serial]
async fn console_inspect_reports_the_epoch_as_unavailable_when_the_ledger_read_fails() {
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/private/1", &[("x-test-login", "user-7")]).await;
    let key = RenderCache::key_for_route_for_test("/private/{id}", &[("id", "1")], Some("user-7"));

    suprnova::DB::unprepared("DROP TABLE suprnova_render_epochs")
        .await
        .expect("drop the epoch table to force the ledger read to fail");

    let report = inspect_report_for_test(&key)
        .await
        .expect("the primary inspection succeeds independently of the ledger");
    assert!(
        report.contains("current epoch: unavailable"),
        "N1: an unreadable epoch must be reported honestly, not as a false 0: {report}"
    );
}

/// Fix round 2 (N1): `render-cache:epoch-advance` must propagate rather
/// than print a false `epoch advanced to 0` when the ledger is unavailable.
///
/// This harness cannot isolate the exact sequence N1 describes ("the advance
/// succeeds and only the immediately-following read fails"): `advance_epoch`'s
/// own `UPDATE` and the read `current_epoch_for_display` performs afterward
/// both execute against the same `suprnova_render_epochs` table and the same
/// `epoch` column, so any corruption severe enough to fail the read also
/// fails the write - there is no way to break only the second statement
/// with a single-connection SQLite harness. Dropping the table instead
/// makes `advance_epoch` itself fail first, which the pre-fix-round code
/// already propagated correctly; what this test actually proves is the
/// weaker but still meaningful claim the code now guarantees either way:
/// on any ledger failure reachable from this command, it errors out and
/// never reaches a codepath that could print a fabricated epoch value.
/// `current_epoch_for_display`'s own `?` in `epoch_advance_report` (see
/// `console.rs`) is what makes the specific N1 sequence impossible to
/// observe printing `0` even though it cannot be independently triggered
/// here.
#[tokio::test]
#[serial_test::serial]
async fn console_epoch_advance_propagates_when_the_ledger_is_unavailable() {
    let _harness = boot_with_render_cache().await;

    suprnova::DB::unprepared("DROP TABLE suprnova_render_epochs")
        .await
        .expect("drop the epoch table to force the ledger to be unavailable");

    let result = epoch_advance_report_for_test().await;
    assert!(
        result.is_err(),
        "N1: a ledger failure must fail the command, never resolve with a \
         printed epoch value"
    );
}

/// Iteration 006, definition-of-done item 4. A write made by a process that
/// never ran `RenderCache::install` - a queue worker, a scheduled task, a
/// console command - advances the same generations the serving process
/// advances, so the entry that depended on the write rebuilds.
///
/// Verified failing by reverting `orm::advance` to `super::is_installed()`:
/// the `posts` generation stood still and the next request was a hit
/// serving the pre-write body.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_model_write_with_no_runtime_installed_advances_the_ledger() {
    use render_cache_operations_support::Post;
    use suprnova::render_cache::ledger::SqlGenerationLedger;
    use suprnova_live::render_cache::generation::GenerationLedger as _;

    let harness = boot_with_render_cache().await;
    let table = DependencyIdentity::table("posts");
    let ledger = SqlGenerationLedger::new();

    dispatch_get(&harness, "/posts/1", &[]).await;
    let after_first = counting_route::renders();
    dispatch_get(&harness, "/posts/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        after_first,
        "the route is cached before the out-of-process write"
    );
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table)
        .unwrap_or(0);

    // From here on this process is not a serving process: no runtime
    // installed, exactly like a queue worker.
    suprnova::render_cache::RenderCache::uninstall_for_test();
    Post::create(attrs! { title: "written outside the server" })
        .await
        .expect("write a post");

    let after = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table)
        .unwrap_or(0);
    assert!(
        after > before,
        "an uninstalled process advances the generation its write changed"
    );

    // Re-open the serving gate; the runtime itself was never torn down.
    suprnova::render_cache::mark_installed();
    dispatch_get(&harness, "/posts/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        after_first + 1,
        "the entry that depended on the write rebuilds"
    );
}

/// The permission generation is the same rule: a console command that
/// revokes a role advances it from wherever it runs.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn bump_permission_version_with_no_runtime_installed_advances_the_permission_generation() {
    use suprnova::render_cache::ledger::SqlGenerationLedger;
    use suprnova_live::render_cache::generation::GenerationLedger as _;

    let _harness = boot_with_render_cache().await;
    let identity = suprnova::render_cache::collector::permission_version_identity();
    let ledger = SqlGenerationLedger::new();
    let before = ledger
        .current(&[identity.digest()])
        .await
        .expect("current")
        .get(&identity)
        .unwrap_or(0);

    suprnova::render_cache::RenderCache::uninstall_for_test();
    suprnova::render_cache::RenderCache::bump_permission_version()
        .await
        .expect("bump from an uninstalled process");
    suprnova::render_cache::mark_installed();

    let after = ledger
        .current(&[identity.digest()])
        .await
        .expect("current")
        .get(&identity)
        .unwrap_or(0);
    assert_eq!(
        after,
        before + 1,
        "bump_permission_version works from any process that writes"
    );
}

/// A process with RenderCache disabled by configuration opens nothing and
/// writes nothing to the ledger, which is what keeps every application that
/// does not use RenderCache paying no RenderCache SQL at all.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_process_with_render_cache_disabled_writes_nothing() {
    use render_cache_operations_support::Post;
    use suprnova::render_cache::ledger::SqlGenerationLedger;
    use suprnova::render_cache::write_side::WriteSideDecision;
    use suprnova_live::render_cache::generation::GenerationLedger as _;

    let _harness = boot_with_render_cache().await;
    let table = DependencyIdentity::table("posts");
    let ledger = SqlGenerationLedger::new();
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table)
        .unwrap_or(0);

    suprnova::render_cache::RenderCache::uninstall_for_test();
    suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(Some(false));

    Post::create(attrs! { title: "written with the cache disabled" })
        .await
        .expect("write a post");

    assert_eq!(
        suprnova::render_cache::RenderCache::write_side_decision_for_test(),
        WriteSideDecision::Closed,
        "a disabled process fixes Closed without probing the schema"
    );
    let after = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table)
        .unwrap_or(0);
    assert_eq!(after, before, "and advances nothing");

    suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(None);
    suprnova::render_cache::mark_installed();
}

/// Every row of the probe's decision table, against the pure function that
/// holds it, so no row needs a database to be proven.
#[test]
fn the_write_side_decision_table() {
    use suprnova::render_cache::write_side::{WriteSideDecision, decide};

    // Installed wins over everything: the serving process advances.
    assert_eq!(decide(true, false, false, None), WriteSideDecision::Open);
    assert_eq!(
        decide(true, true, true, Some(false)),
        WriteSideDecision::Open
    );
    // Disabled by configuration fixes Closed without a probe.
    assert_eq!(
        decide(false, false, true, Some(true)),
        WriteSideDecision::Closed
    );
    // Enabled but disconnected decides nothing: the write itself needs a
    // connection, so the next write probes again.
    assert_eq!(
        decide(false, true, false, None),
        WriteSideDecision::Undecided
    );
    // Enabled and connected: the schema decides.
    assert_eq!(
        decide(false, true, true, Some(true)),
        WriteSideDecision::Open
    );
    assert_eq!(
        decide(false, true, true, Some(false)),
        WriteSideDecision::Closed
    );
    // A probe that failed decides nothing; the error propagates instead.
    assert_eq!(
        decide(false, true, true, None),
        WriteSideDecision::Undecided
    );
}

/// A fixed decision is not probed again: two writes after `Closed` issue no
/// statement beyond the writes themselves.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_closed_write_side_is_not_probed_again() {
    use render_cache_operations_support::{Post, statements};

    let _harness = boot_with_render_cache().await;
    suprnova::render_cache::RenderCache::uninstall_for_test();
    suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(Some(false));

    // The first write is what fixes the decision.
    Post::create(attrs! { title: "first" })
        .await
        .expect("first write");

    statements::reset();
    Post::create(attrs! { title: "second" })
        .await
        .expect("second write");
    let one_write = statements::count();

    statements::reset();
    Post::create(attrs! { title: "third" })
        .await
        .expect("third write");
    Post::create(attrs! { title: "fourth" })
        .await
        .expect("fourth write");
    let two_writes = statements::count();

    suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(None);
    suprnova::render_cache::mark_installed();

    // Whatever one `INSERT` costs on this backend, two of them cost exactly
    // twice as much: no probe, no ledger read, no ledger write, and nothing
    // that happens only once.
    assert!(one_write > 0, "a write runs at least one statement");
    assert_eq!(
        two_writes,
        one_write * 2,
        "a fixed decision is never probed again"
    );
}

/// The eight closed telemetry names, each naming its own counter. The
/// rewind counter joins the others here rather than in its own test,
/// because what matters is that the set stays closed and distinct.
#[test]
fn the_render_cache_telemetry_names_are_closed_and_distinct() {
    let names = [
        telemetry::LOOKUPS,
        telemetry::HITS,
        telemetry::PUBLICATIONS,
        telemetry::REBUILDS,
        telemetry::STITCH_ASSEMBLIES,
        telemetry::STITCH_SLOTS,
        telemetry::STITCH_NESTED,
        telemetry::EPOCH_REWINDS,
    ];
    let mut sorted = names.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "no two counters share a name");
    for name in names {
        assert!(
            name.starts_with("suprnova.render_cache."),
            "{name} is namespaced"
        );
    }
    assert_eq!(
        telemetry::EPOCH_REWINDS,
        "suprnova.render_cache.epoch_rewinds",
        "the operations chapter's telemetry table quotes this name"
    );
}

// ── Plan F: declined lookups record a reason ────────────────────────────
//
// Every test below resets the recorder first, so it only ever inspects
// what its own one or two dispatches produced. Each dispatches once (or
// twice, for the outcome/reason correspondence) and reads back the
// `reason` label the framework actually attached, rather than inferring it
// from the response.

/// Reads a session value on a route that declares no variance at all: the
/// classification narrows to `Uncacheable` through `SessionValueRead`, and
/// nothing else in `reasons` could have forced it, so `lead_render` records
/// exactly that reason.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_session_value_read_declines_with_reason_session_value_read() {
    telemetry::reset_recorded_lookups_for_test();
    let harness = render_cache_privacy_support::boot_with_render_cache().await;
    render_cache_privacy_support::dispatch_get(
        &harness,
        render_cache_privacy_support::READS_SESSION_MUT_ROUTE,
        &[],
    )
    .await;

    let declined: Vec<_> = telemetry::recorded_lookups_for_test()
        .into_iter()
        .filter(|lookup| lookup.outcome == "declined")
        .collect();
    assert_eq!(declined.len(), 1, "exactly one declined lookup");
    assert_eq!(declined[0].reason, Some("session_value_read"));
}

/// The existing header-driven gate from the privacy support: a body driven
/// by an authorization decision alone, on a route that declares no
/// `Principal` variance. After Plan E the unresolved consult still requires
/// `Principal`, so the key guard's empty-set arm finds no declared
/// dimension for it and declines with `PrincipalUndeclared`.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_principal_gate_without_principal_variance_declines_with_reason_principal_undeclared() {
    telemetry::reset_recorded_lookups_for_test();
    render_cache_privacy_support::ensure_role_gate();
    let harness = render_cache_privacy_support::boot_with_render_cache().await;
    render_cache_privacy_support::dispatch_get(
        &harness,
        render_cache_privacy_support::AUTHZ_DRIVEN_ROUTE,
        &[("x-test-role", "admin")],
    )
    .await;

    let declined: Vec<_> = telemetry::recorded_lookups_for_test()
        .into_iter()
        .filter(|lookup| lookup.outcome == "declined")
        .collect();
    assert_eq!(declined.len(), 1, "exactly one declined lookup");
    assert_eq!(declined[0].reason, Some("principal_undeclared"));
}

/// A route that reads the negotiated locale without declaring the `Locale`
/// dimension: the key guard's locale check runs unconditionally, before
/// the classification-reasons loop, and declines with `LocaleUndeclared`
/// because the route names no `Locale` value for the key to compare
/// against at all.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn an_undeclared_locale_declines_with_reason_locale_undeclared() {
    telemetry::reset_recorded_lookups_for_test();
    let harness = render_cache_privacy_support::boot_with_render_cache().await;
    render_cache_privacy_support::dispatch_get(
        &harness,
        render_cache_privacy_support::UNDECLARED_LOCALE_ROUTE,
        &[("x-test-locale", "de")],
    )
    .await;

    let declined: Vec<_> = telemetry::recorded_lookups_for_test()
        .into_iter()
        .filter(|lookup| lookup.outcome == "declined")
        .collect();
    assert_eq!(declined.len(), 1, "exactly one declined lookup");
    assert_eq!(declined[0].reason, Some("locale_undeclared"));
}

/// A 404 on an otherwise ordinary cached route: eligibility's own `Status`
/// check declines before classification or the key guard ever run.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn an_ineligible_status_declines_with_reason_status() {
    telemetry::reset_recorded_lookups_for_test();
    let harness = boot_with_render_cache().await;
    let path = NOT_FOUND_ROUTE.replace("{id}", "1");
    let response = dispatch_get(&harness, &path, &[]).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    let declined: Vec<_> = telemetry::recorded_lookups_for_test()
        .into_iter()
        .filter(|lookup| lookup.outcome == "declined")
        .collect();
    assert_eq!(declined.len(), 1, "exactly one declined lookup");
    assert_eq!(declined[0].reason, Some("status"));
}

/// The four declines above name four distinct reasons - proof the closed
/// set actually distinguishes contracts rather than collapsing them behind
/// one shared label.
#[test]
fn the_four_named_declines_are_four_distinct_reasons() {
    let reasons = [
        "session_value_read",
        "principal_undeclared",
        "locale_undeclared",
        "status",
    ];
    let mut sorted = reasons.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        reasons.len(),
        "no two of the four share a label"
    );
}

/// A hit and a miss both record `reason: None`; only a decline ever carries
/// one.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn declined_is_the_only_outcome_that_carries_a_reason() {
    telemetry::reset_recorded_lookups_for_test();
    let harness = boot_with_render_cache().await;
    dispatch_get(&harness, "/cached/1", &[]).await;
    dispatch_get(&harness, "/cached/1", &[]).await;

    let recorded = telemetry::recorded_lookups_for_test();
    assert!(
        recorded.iter().any(|lookup| lookup.outcome == "miss"),
        "sanity: the first request missed"
    );
    assert!(
        recorded.iter().any(|lookup| lookup.outcome == "l0"),
        "sanity: the second request hit"
    );
    for lookup in &recorded {
        if lookup.outcome == "declined" {
            assert!(
                lookup.reason.is_some(),
                "a declined outcome must carry a reason"
            );
        } else {
            assert!(
                lookup.reason.is_none(),
                "outcome {} unexpectedly carried a reason",
                lookup.outcome
            );
        }
    }
}

/// Every closed `reason` label appears, backticked, in the operations
/// manual's Telemetry section - the chapter an operator actually reads
/// when a route's `declined` rate is high.
#[cfg(feature = "testing")]
#[test]
fn every_decline_reason_is_documented_in_the_operations_chapter() {
    let manual_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../manual/render-cache-operations.md"
    );
    let manual = std::fs::read_to_string(manual_path)
        .unwrap_or_else(|error| panic!("read {manual_path}: {error}"));
    let missing: Vec<&str> = telemetry::decline_reason_labels_for_test()
        .into_iter()
        .filter(|label| !manual.contains(&format!("`{label}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "the operations chapter is missing these reason labels: {missing:?}"
    );
}
