//! Compilation-failure probe for the production build shape (design doc
//! `2026-09-08-render-cache-observability-build-shape-design.md`, section
//! 5.3, "the vocabulary section" for "test seam"). Every `pub` item under
//! `framework/src` gated by `cfg(any(test, feature = "testing"))` is
//! referenced below by path.
//!
//! `cargo check` with no features: every reference to a genuinely gated
//! item must fail to resolve. `cargo check --features with-testing`: every
//! reference must resolve, proving the list names real items rather than
//! typos that would "pass" step 1 for the wrong reason.
//!
//! The inventory is `grep -rn 'cfg(any(test, feature = "testing"))'
//! framework/src`, followed by checking each hit for `pub`-ness (a
//! `pub(crate)` or private item is invisible outside the framework crate
//! regardless of features, so it is not part of this crate's surface and is
//! not listed here).
//!
//! # Two error classes, not one
//!
//! The design text says a missing reference fails with `E0425` or `E0433`.
//! That holds for a free function gated inside an otherwise-always-present
//! module (`E0425`, "cannot find function ... in module ...") and for a
//! module gated as a whole (`E0433`, "cannot find `X` in `Y`" - resolution
//! fails at the missing module segment before it ever looks at what comes
//! after it, so it does not matter which item past that segment is named).
//! It does not hold for an associated function or method gated inside an
//! `impl` block on a type that itself always exists: missing that produces
//! `E0599` ("no function or associated item named ... found for
//! struct/type ..."), confirmed empirically (a throwaway single-file
//! `rustc` compile of the three shapes) before writing this file. Most of
//! the framework's actual test seams take this third shape
//! (`RenderCache::*_for_test`, `RenderCacheConfig::with_*_for_test`,
//! `Context::test_*`, `Storage::fake`, `DbConnection::observe_statements_for_test`),
//! so `scripts/check-production-build.sh` accepts `E0599` alongside
//! `E0425`/`E0433` rather than the two the design names - a deviation from
//! the literal design text, made because the alternative (rewriting every
//! associated-function seam as a free function to fit two error codes)
//! would be a framework API change section 5 does not ask for.
//!
//! # Two pairs of items the design lists that are not actually gated
//!
//! `RenderCacheConfig::with_clock_for_test` / `with_coordinator_for_test`
//! (`framework/src/render_cache/config.rs`) and
//! `render_cache::console::epoch_advance_report_for_test` /
//! `inspect_report_for_test` (`framework/src/render_cache/console.rs`) carry
//! no `cfg(any(test, feature = "testing"))` gate at all - `#[doc(hidden)]`
//! only. They compile identically with or without `with-testing` and are
//! referenced below anyway, for completeness against the design's list;
//! they contribute nothing to this probe's compile-failure proof, which
//! rests on the genuinely gated items around them. This is a gap in the
//! framework's own test-seam hygiene relative to the design's intent, not
//! something section 5 asks this crate to close.

use suprnova::context::Context;
use suprnova::database::DbConnection;
use suprnova::features::evaluators::database::DatabaseEvaluator;
use suprnova::filesystem::Storage;
use suprnova::render_cache::{RenderCache, RenderCacheConfig};

/// `filesystem::testing` is gated at its `mod` declaration
/// (`framework/src/filesystem/mod.rs`), so any path into it fails to
/// resolve at the `testing` segment itself, regardless of which item past
/// it is named.
fn filesystem_testing_guard() -> Option<suprnova::filesystem::testing::StorageFakeGuard> {
    None
}

fn main() {
    // --- Free functions gated inside an otherwise-always-present module
    // (E0425 without `with-testing`: "cannot find function ... in module
    // ...") ---
    let _ = suprnova::crypto::_test_install_key;
    let _ = suprnova::crypto::_test_install_keyring;
    let _ = suprnova::crypto::_test_encrypt_with;
    let _ = suprnova::crypto::_test_encrypt_with_for;
    let _ = suprnova::crypto::_test_force_next_encrypt_failure;
    let _ = suprnova::rbac::observed_rbac_statements_for_test;
    let _ = suprnova::testing::install_test_encryption_key;
    let _ = suprnova::testing::install_test_encryption_keyring;
    let _ = suprnova::render_cache::collector::strip_classification_reasons_for_test;
    let _ = suprnova::render_cache::telemetry::recorded_lookups_for_test;
    let _ = suprnova::render_cache::telemetry::reset_recorded_lookups_for_test;
    let _ = suprnova::render_cache::telemetry::decline_reason_labels_for_test;
    // Generic over its edit closure. Calling it - never polling or awaiting
    // the returned future, so nothing actually runs - lets the closure's
    // parameter type (a `suprnova-live` type this crate never names
    // directly, since it depends only on `suprnova`) be inferred from
    // `rewrite_composite_for_test`'s own signature instead of spelled out
    // here.
    let _future = suprnova::render_cache::testing::rewrite_composite_for_test(
        "probe-pattern",
        |_graph| {},
    );

    // --- A module gated as a whole (E0433 without `with-testing`) ---
    let _ = filesystem_testing_guard;
    let _ = suprnova::render_cache::middleware::race_points::arm;
    let _ = suprnova::render_cache::middleware::race_points::disarm;
    let _ = &suprnova::render_cache::middleware::race_points::EPOCH_CAPTURED;

    // --- Associated functions/methods on a type that itself always exists
    // (E0599 without `with-testing` - see this file's header comment) ---
    // `test_set_query` and `test_query_guard` take `impl Into<String>`
    // parameters; with `testing` on, more than one crate in the dependency
    // graph implements `Into<String>` for a `String` argument (an
    // ambiguity `unicase` introduces elsewhere in the tree), so taking
    // either as a bare, uncalled path value leaves the compiler unable to
    // pick a concrete type (E0283). Calling them instead - never running,
    // since `cargo check` type-checks `main` without executing it -
    // supplies concrete `&str` arguments and sidesteps the ambiguity.
    Context::test_set_query("probe-name", "probe-value");
    let _query_guard = Context::test_query_guard("probe-name", "probe-value");
    let _ = Context::test_clear_query;
    let _ = RenderCache::shell_for_test;
    let _ = RenderCache::l0_hot_for_test;
    let _ = RenderCache::l0_body_ptr_for_test;
    let _ = RenderCache::hot_response_body_ptr_for_test;
    let _ = RenderCache::l0_frame_ptr_for_test;
    let _ = RenderCache::hot_serves_for_test;
    let _ = RenderCache::background_rebuilds_for_test;
    let _ = RenderCache::uninstall_for_test;
    let _ = RenderCache::set_write_side_enabled_for_test;
    let _ = RenderCache::write_side_decision_for_test;
    let _ = Storage::fake;
    let _ = DbConnection::observe_statements_for_test::<fn()>;
    let _ = DatabaseEvaluator::execute_unprepared_for_test;

    // --- Named by the design as seams, but not actually gated today (see
    // this file's header comment); these resolve either way. ---
    let _ = RenderCacheConfig::with_clock_for_test;
    let _ = RenderCacheConfig::with_coordinator_for_test;
    let _ = suprnova::render_cache::console::epoch_advance_report_for_test;
    let _ = suprnova::render_cache::console::inspect_report_for_test;
}
