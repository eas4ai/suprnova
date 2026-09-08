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
/// Attribute `outcome` values, emitted only on `LOOKUPS` and `HITS` (see
/// `middleware.rs`'s `LookupOutcome::as_str`): `l0`, `l1`, `conditional`,
/// `stale`, `miss`, `bypass`, `moved`, `declined`. `PUBLICATIONS` and
/// `REBUILDS` carry no `outcome` attribute at all - each has exactly one
/// outcome. `STITCH_ASSEMBLIES` and `STITCH_SLOTS` carry their own closed
/// value sets, documented on each.
pub const OUTCOME: &str = "outcome";
