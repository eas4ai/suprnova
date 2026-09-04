//! Closed telemetry names; attributes are bounded enumerations.

/// Counter: RenderCache lookups attempted.
pub const LOOKUPS: &str = "suprnova.render_cache.lookups";
/// Counter: RenderCache lookups that returned a stored representation.
pub const HITS: &str = "suprnova.render_cache.hits";
/// Counter: RenderCache publications accepted.
pub const PUBLICATIONS: &str = "suprnova.render_cache.publications";
/// Counter: RenderCache rebuilds coordinated.
pub const REBUILDS: &str = "suprnova.render_cache.rebuilds";
/// Attribute `outcome` values, emitted only on `LOOKUPS` and `HITS` (see
/// `middleware.rs`'s `LookupOutcome::as_str`): `l0`, `l1`, `conditional`,
/// `stale`, `miss`, `bypass`, `moved`, `declined`. `PUBLICATIONS` and
/// `REBUILDS` carry no `outcome` attribute at all - each has exactly one
/// outcome.
pub const OUTCOME: &str = "outcome";
