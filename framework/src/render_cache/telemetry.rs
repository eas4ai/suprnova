//! Closed telemetry names; attributes are bounded enumerations.

/// Counter: RenderCache lookups attempted.
pub const LOOKUPS: &str = "suprnova.render_cache.lookups";
/// Counter: RenderCache lookups that returned a stored representation,
/// tallied by outcome.
///
/// **A tally of outcome labels, not a count of requests** (R54). One lookup
/// records every outcome that is true of it, and a conditional hit has two:
/// the tier that answered it (`l0` or `l1`) and `conditional`, which says
/// the answer was a 304. Both facts are worth having - which tier served,
/// and how many hits cost no body - and one label per request could only
/// report one of them. Summing this counter over its `outcome` values
/// therefore over-counts requests, and so does summing [`LOOKUPS`], which
/// takes one increment from each of the same records. Read either counter
/// one label at a time; for a request count, use a single label such as
/// `l0`.
pub const HITS: &str = "suprnova.render_cache.hits";
/// Counter: RenderCache publications accepted.
pub const PUBLICATIONS: &str = "suprnova.render_cache.publications";
/// Counter: RenderCache rebuilds coordinated.
pub const REBUILDS: &str = "suprnova.render_cache.rebuilds";
/// Counter: composite assemblies attempted on a stitched route's hit.
///
/// Attribute `outcome` values: `fail_document` (the shell was not used and
/// the route's own handler answered the request uncached), `assembled` (the
/// document was assembled from re-mounted islands).
pub const STITCH_ASSEMBLIES: &str = "suprnova.render_cache.stitch.assemblies";
/// Counter: island slots resolved while assembling a stitched hit.
///
/// Attribute `outcome` values: `rendered` (the island mounted and its markup
/// replaced the slot), `omitted` (the slot's declared policy dropped it),
/// `fallback` (the slot's declared fragment took its place), `failed` (the
/// slot could not be resolved and the whole document falls back to the
/// handler).
pub const STITCH_SLOTS: &str = "suprnova.render_cache.stitch.slots";
/// Counter: nested cached segments resolved while assembling a stitched hit.
///
/// Distinguishes an inner segment's outcomes from an island slot's under a
/// closed pair of attributes, per spec: `outcome` takes exactly one value
/// from `resolved`, `omitted`, `fallback`, `failed`; `cause` takes exactly
/// one value from `none`, `fetch_failed`, `version_mismatch`,
/// `length_mismatch`, `depth_exceeded`, `cycle`, `unauthorized`, with `none`
/// used exactly when `outcome` is `resolved`.
pub const STITCH_NESTED: &str = "suprnova.render_cache.stitch.nested";
/// Attribute `cause`, emitted only on [`STITCH_NESTED`] beside `outcome`;
/// see that constant's own doc for the closed value set.
pub const CAUSE: &str = "cause";
/// Counter: authority epoch rewinds detected and lifted past.
///
/// Incremented once per detection, by the node that detected it: a stamp
/// above the authority - an entry's, or this node's own leased epoch -
/// which the node then lifts the ledger's epoch above. Carries no
/// attribute, like [`PUBLICATIONS`] and [`REBUILDS`]: there is one outcome,
/// and the two epoch numbers involved go to the warning log, never to a
/// metric label, which is what keeps the label set closed and
/// low-cardinality. One authority read that meets both a rewound lease and
/// a rewound entry counts once.
///
/// A non-zero value after a database restore is the expected signal that
/// the restore was noticed. A non-zero value at any other time means an
/// authority moved backwards for a reason nobody intended.
pub const EPOCH_REWINDS: &str = "suprnova.render_cache.epoch_rewinds";
/// Attribute `outcome` values, emitted only on `LOOKUPS` and `HITS` (see
/// `middleware.rs`'s `LookupOutcome::as_str`): `l0`, `l1`, `conditional`,
/// `stale`, `miss`, `bypass`, `moved`, `declined`. `PUBLICATIONS` and
/// `REBUILDS` carry no `outcome` attribute at all - each has exactly one
/// outcome. `STITCH_ASSEMBLIES`, `STITCH_SLOTS`, and `STITCH_NESTED` carry
/// their own closed value sets, documented on each.
pub const OUTCOME: &str = "outcome";
/// Attribute `reason`, emitted only on `LOOKUPS` and only alongside
/// `outcome="declined"` (see `decline::LookupDeclineReason::as_str`): every
/// other outcome carries no `reason`. The value is `snake_case` of a closed
/// compile-time enum variant, never anything derived from a route name, a
/// key, an identity digest, or a header value.
///
/// The closed value set, grouped as the reason's own definition groups it:
///
/// - Eligibility: `policy_uncacheable`, `method`, `status`, `streaming`,
///   `sets_cookie`, `unsafe_header_name`.
/// - Observation: `observation_overflowed`, `ledger_read_failed`,
///   `handler_not_begun`.
/// - Classification narrowed to `Uncacheable`: `session_value_read`,
///   `secret_context_read`, `undeclared_context`.
/// - Live document facts: `identity_bound_without_stitching`,
///   `invalid_stitch_capture`, `no_store_intent`,
///   `unresolvable_seed_deadline`.
/// - Invariants over the key: `unreasoned_private_class`,
///   `principal_undeclared`, `principal_divergent`, `tenant_undeclared`,
///   `tenant_divergent`, `locale_undeclared`, `locale_divergent`.
/// - Publication: `seed_deadline_elapsed`, `unsafe_header_value`,
///   `composite_capture_invalid`, `composite_slot_count_mismatch`,
///   `composite_too_many_slots`, `composite_digest_mismatch`,
///   `composite_empty_slot`, `composite_slot_not_found`,
///   `composite_slot_ambiguous`, `composite_nested_wider_class`,
///   `composite_nested_longer_freshness`, `composite_nested_depth_exceeded`,
///   `composite_nested_cycle`, `composite_nested_unresolvable`.
pub const REASON: &str = "reason";

/// One lookup recorded for a test: the `outcome` label, and, for a decline,
/// the `reason` label beside it.
///
/// Compiled only under `cfg(test)` or the `testing` feature, the same seam
/// `middleware::race_points` uses: an integration test under
/// `framework/tests/` is a separate crate with no `cfg(test)` of its own
/// reaching this library, so it can only see this recorder through the
/// feature.
#[cfg(any(test, feature = "testing"))]
pub struct RecordedLookup {
    /// The `outcome` label this lookup recorded.
    pub outcome: &'static str,
    /// The `reason` label, present only when `outcome` is `"declined"`.
    pub reason: Option<&'static str>,
}

#[cfg(any(test, feature = "testing"))]
mod recorder {
    use std::sync::Mutex;

    use super::RecordedLookup;

    /// Bounds the recorder the same way a test-only channel is bounded
    /// anywhere else in this crate: a runaway test cannot grow this without
    /// limit, and 4096 is far past what any single test in this suite
    /// dispatches.
    const MAX_RECORDED: usize = 4096;

    static RECORDED: Mutex<Vec<RecordedLookup>> = Mutex::new(Vec::new());

    pub(super) fn push(outcome: &'static str, reason: Option<&'static str>) {
        let mut recorded = RECORDED.lock().unwrap_or_else(|e| e.into_inner());
        if recorded.len() < MAX_RECORDED {
            recorded.push(RecordedLookup { outcome, reason });
        }
    }

    pub(super) fn drain() -> Vec<RecordedLookup> {
        let recorded = RECORDED.lock().unwrap_or_else(|e| e.into_inner());
        recorded
            .iter()
            .map(|lookup| RecordedLookup {
                outcome: lookup.outcome,
                reason: lookup.reason,
            })
            .collect()
    }

    pub(super) fn clear() {
        let mut recorded = RECORDED.lock().unwrap_or_else(|e| e.into_inner());
        recorded.clear();
    }
}

/// Records one lookup outcome (and, for a decline, its reason) into the
/// test-only recorder; a no-op outside `cfg(any(test, feature = "testing"))`.
/// Called from [`super::middleware::LookupOutcome::record`], alongside the
/// real metric increment, never instead of it.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn record_for_test(outcome: &'static str, reason: Option<&'static str>) {
    recorder::push(outcome, reason);
}

/// The lookups recorded since the last [`reset_recorded_lookups_for_test`],
/// oldest first, bounded to 4096.
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub fn recorded_lookups_for_test() -> Vec<RecordedLookup> {
    recorder::drain()
}

/// Clears the recorder. Test-only cleanup; production code never calls
/// this.
#[cfg(any(test, feature = "testing"))]
pub fn reset_recorded_lookups_for_test() {
    recorder::clear();
}

/// Every closed `reason` label, `snake_case`, in the same order
/// [`REASON`]'s own doc groups them. For the documentation test that pins
/// each one appears in the operations manual chapter; not itself an
/// `outcome` or `reason` value recorded anywhere.
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub fn decline_reason_labels_for_test() -> Vec<&'static str> {
    super::decline::LookupDeclineReason::ALL
        .iter()
        .map(|reason| reason.as_str())
        .collect()
}
