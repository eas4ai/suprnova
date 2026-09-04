//! Request-scoped dependency collector: a Tokio task-local that framework
//! reads register into. Absent outside a scope, so ordinary requests pay
//! one `try_with` per read and nothing else.
//!
//! # Limitations, by design
//!
//! - **Config and Feature identities have no producer.** No write path
//!   advances a config or feature generation, so observing them would spend
//!   the bounded observation budget while contributing nothing to
//!   invalidation - the same reasoning as ruling R24 on query classes.
//!   `Config::get::<T>()` is also type-keyed rather than name-keyed, so
//!   there is no stable name to build an identity from at that seam.
//! - **[`observe_secret_context_read`] has no automatic producer.**
//!   `Config::get::<T>()` returns whole typed structs, so a secret read is
//!   indistinguishable from any other configuration read at that seam;
//!   hooking it there would mark either everything or nothing as
//!   secret-touching. This flag exists for application code and later
//!   adapters to set explicitly - it is not automatic protection.
//! - **Per-connection reads collapse into one table identity.**
//!   [`observe_table_read`] ignores the connection name, so the same table
//!   read on two different connections shares one identity. This
//!   over-invalidates, which is safe; tenancy is handled at the key level
//!   through the Tenant variance dimension, not here.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use suprnova_live::render_cache::generation::{DependencyIdentity, MAX_OBSERVATIONS};

/// Process-wide permission-version counter fed into `Principal` variance
/// (`PrivateMaterial::principal`) so a cached private representation stops
/// matching a user whose permissions changed since it was published.
///
/// Starts at 0. Nothing in the framework bumps this on its own: role and
/// permission changes are application-defined (a `roles` table update, a
/// policy reassignment), so only the application knows when they happen.
/// An application that grants or revokes permissions and cares about
/// RenderCache must call [`crate::render_cache::RenderCache::bump_permission_version`]
/// when it does - documented on that function and in the manual. Without
/// that call, a user whose permissions were just revoked keeps matching the
/// cache key their prior permission set produced, and keeps being served
/// whatever was cached under it.
static PERMISSION_VERSION: AtomicU64 = AtomicU64::new(0);

/// The current process-wide permission version. Read by the middleware
/// while building `Principal` variance material; see
/// [`crate::render_cache::RenderCache::bump_permission_version`] for what
/// advances it.
#[must_use]
pub fn permission_version() -> u64 {
    PERMISSION_VERSION.load(Ordering::SeqCst)
}

/// Advances the permission version. `pub(crate)`: the public entry point is
/// [`crate::render_cache::RenderCache::bump_permission_version`], which this
/// backs.
pub(crate) fn bump_permission_version() {
    PERMISSION_VERSION.fetch_add(1, Ordering::SeqCst);
}

/// The collector's own cap, one below [`MAX_OBSERVATIONS`].
///
/// [`suprnova_live::render_cache::generation::ObservationWindow::open`]
/// always seeds `DependencyIdentity::Broad` before a representation's own
/// observations are added, and that seed counts toward the same
/// [`MAX_OBSERVATIONS`] budget. A collector report holding exactly
/// `MAX_OBSERVATIONS` identities (none of them `Broad`) would therefore
/// overflow the window the moment `Broad` is folded in, and could never be
/// closed. Reserving one slot here keeps a non-overflowed report always
/// closeable. Do not raise this back to `MAX_OBSERVATIONS`.
const MAX_COLLECTED: usize = MAX_OBSERVATIONS - 1;

/// Context flags the collector accumulates.
///
/// No longer `Copy` as of fix round 5: `principal_material`/`tenant_material`
/// carry owned `String`s.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectedContext {
    /// A principal was resolved or checked.
    pub principal_read: bool,
    /// Every distinct principal id the render actually observed, across
    /// every accessor that touched one.
    ///
    /// Fix round 5 recorded only the most recent value, in a single
    /// `Option<String>` slot; fix round 6 replaced that with a set after the
    /// reviewer proved the collapse itself was a leak: a handler that reads
    /// a named guard's identity to build the body, then separately touches
    /// the default accessor (`Auth::id()`, say, for an unrelated check),
    /// recorded only the second value, and the guard compared that second
    /// value against the key - passing, because the key and the *last*
    /// observation happened to agree, even though the *body* was built from
    /// a different, unrecorded one. A representation can only be published
    /// under one key, so if the render observed two different principal
    /// values in the same request, the key cannot represent both correctly.
    /// The guard declines on any member of this set disagreeing with the
    /// key, not just the last one written. Re-deriving `Auth::id()` after
    /// the render (round 4's approach, before round 5 removed it) cannot
    /// substitute for this: it only ever sees the *default* guard's slot,
    /// never a named one, so a read through any other accessor stays
    /// invisible before or after the render.
    pub principal_material: BTreeSet<String>,
    /// A tenant was resolved or checked. Fix round 4:
    /// `Request::live_tenant()` records this on every call, the same way
    /// `Lang::locale()` records a locale observation - `ObservedContext.tenant`
    /// was previously always `None` because nothing produced this.
    pub tenant_read: bool,
    /// Every distinct tenant id the render actually observed. See
    /// `principal_material`'s own doc; the same reasoning applies.
    pub tenant_material: BTreeSet<String>,
    /// Every distinct locale value [`crate::Lang::locale`] returned during
    /// the render, recorded at the same observation point that already
    /// emits a [`DependencyIdentity::Locale`] dependency (fix round 6).
    ///
    /// Round 5 instead re-read `Lang::locale()` a second time, after the
    /// render, reasoning that the task-local was "still installed" then.
    /// That is only true for the outer scope: a handler that renders inside
    /// [`crate::scope_locale`] (the framework's own documented, supported
    /// API for a mid-render locale switch) has its nested scope pop the
    /// instant that future resolves, before the guard ever gets to re-read
    /// it - so the re-read silently saw the *outer*, pre-switch locale
    /// again, the same value the key was already built from, and always
    /// agreed with it. The same gap reproduces without any nested-scope API
    /// at all: a per-route locale middleware installed after
    /// [`super::RenderCache::install`] (the only position such a middleware
    /// can occupy, since `install` appends) also sets the task-local before
    /// the handler runs and it, too, is scoped no wider than that
    /// middleware's own `next(request)` call - gone by the time a
    /// post-render re-read outside it would look. Recording every observed
    /// value at the moment `Lang::locale()` is actually called, the same
    /// mechanism already used for identity, closes both: there is no
    /// "after the render" read to get wrong.
    pub locale_material: BTreeSet<String>,
    /// A session value was read.
    pub session_read: bool,
    /// An authorization decision was evaluated.
    pub authorization_read: bool,
    /// Secret configuration was read.
    pub secret_context_read: bool,
    /// Observation bound exceeded, or a dependency could not be encoded
    /// into an identity at all; the report is incomplete either way and
    /// the response it describes must not be stored.
    pub overflowed: bool,
}

/// The collector's report.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectorReport {
    /// Observed identities in first-seen order, deduplicated.
    pub observed: Vec<DependencyIdentity>,
    /// Context flags.
    pub context: CollectedContext,
    /// Undeclared request context names that affected rendering.
    pub undeclared: Vec<String>,
    /// Facts a rendered Live document recorded, if the render mounted one.
    pub live_document: Option<super::live::LiveDocumentFacts>,
}

impl CollectorReport {
    /// The observed identities, or `None` when the report overflowed (bound
    /// exceeded, or a dependency could not be encoded).
    ///
    /// This is the only way callers should read `observed` for the purpose
    /// of deciding whether to store a response: an overflowed report's
    /// `observed` field still holds a full-looking list, but it is missing
    /// whichever identities did not fit or could not be built, and those can
    /// include a broader identity (a whole-table read) than anything that
    /// did fit. Storing on that partial list risks a cached response that
    /// no write can ever invalidate. Prefer `storable().is_some()` to
    /// `!context.overflowed` so a future field never has to be
    /// remembered separately.
    #[must_use]
    pub fn storable(&self) -> Option<&[DependencyIdentity]> {
        if self.context.overflowed {
            None
        } else {
            Some(&self.observed)
        }
    }
}

#[derive(Default)]
struct State {
    report: CollectorReport,
    seen: std::collections::BTreeSet<DependencyIdentity>,
}

/// A scope's collector.
#[derive(Clone, Default)]
pub struct Collector {
    state: Arc<Mutex<State>>,
}

tokio::task_local! {
    static COLLECTOR: Collector;
}

impl Collector {
    /// Runs `future` with a fresh collector; the report is readable inside via
    /// [`current_report`] and dropped with the scope.
    pub async fn scope<F: std::future::Future>(future: F) -> F::Output {
        COLLECTOR.scope(Self::default(), future).await
    }
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    COLLECTOR
        .try_with(|collector| f(&mut collector.state.lock().unwrap_or_else(|p| p.into_inner())))
        .ok()
}

/// Whether a collector is active on this task.
///
/// Read-site hooks check this before doing any work to build an identity
/// (a bounds scan, a `String` allocation, a JSON encode) so that on an
/// ordinary request - no [`Collector::scope`] active - a framework read
/// costs exactly one cheap `try_with` and nothing else.
#[must_use]
pub fn is_active() -> bool {
    COLLECTOR.try_with(|_| ()).is_ok()
}

/// Marks the current report incomplete: either the observation bound was
/// reached, or a dependency could not be encoded into an identity at all.
/// Either way the report must not be treated as a complete accounting of
/// what the representation depended on.
fn mark_incomplete() {
    with_state(|state| state.report.context.overflowed = true);
}

/// Records a typed dependency; bounded, idempotent, no-op outside a scope.
pub fn observe(identity: DependencyIdentity) {
    with_state(|state| {
        if state.seen.contains(&identity) {
            return;
        }
        if state.seen.len() >= MAX_COLLECTED {
            state.report.context.overflowed = true;
            return;
        }
        state.seen.insert(identity.clone());
        state.report.observed.push(identity);
    });
}

/// A read of one table.
///
/// A table or record name that fails [`DependencyIdentity`]'s bounds marks
/// the report incomplete instead of silently recording nothing: a
/// dependency that cannot be named is not a dependency that can be safely
/// ignored.
pub fn observe_table_read(table: &str) {
    if !is_active() {
        return;
    }
    match DependencyIdentity::try_table(table) {
        Ok(identity) => observe(identity),
        Err(_) => mark_incomplete(),
    }
}

/// A record read by primary key bytes. See [`observe_table_read`] for the
/// bound-failure behaviour.
///
/// `key` is used verbatim - this function does not encode it. If the
/// caller's read observed a JSON-typed primary key value, encode it
/// through [`record_identity`] first (or call
/// [`observe_record_read_json`] directly) rather than hand-rolling the
/// bytes: the write side always builds its identities through
/// `record_identity`'s exact encoding (JSON `Display` form, quotes
/// included for strings), and any other encoding here - trimming quotes,
/// using a different number format, and so on - silently breaks
/// record-level invalidation for that row, the same drift ruling R45
/// fixed on the write side. This is the one remaining seam where
/// application code can reintroduce it.
pub fn observe_record_read(table: &str, key: &[u8]) {
    if !is_active() {
        return;
    }
    match DependencyIdentity::try_record(table, key) {
        Ok(identity) => observe(identity),
        Err(_) => mark_incomplete(),
    }
}

/// The canonical [`DependencyIdentity::Record`] for a table and a
/// JSON-encoded primary key value.
///
/// This is the only place a record's primary key becomes bytes:
/// `key.to_string()` (the JSON value's `Display` form, e.g. `42` for a
/// number or `"abc"` with the quotes retained for a string), UTF-8 encoded.
/// The write side that advances a record's generation on a model write
/// **must** build its identity through this same function - encoding the
/// key any other way means a write's identity never matches a read's
/// observed identity, and record-level invalidation silently never fires
/// for that table. See ruling R29.
///
/// Returns `None` when the table name or the encoded key falls outside
/// [`DependencyIdentity::try_record`]'s bounds.
#[must_use]
pub fn record_identity(table: &str, key: &serde_json::Value) -> Option<DependencyIdentity> {
    DependencyIdentity::try_record(table, key.to_string().as_bytes()).ok()
}

/// A record read identified by its JSON-encoded primary key value. See
/// [`record_identity`] for the encoding and [`observe_table_read`] for the
/// bound-failure behaviour.
pub fn observe_record_read_json(table: &str, key: &serde_json::Value) {
    if !is_active() {
        return;
    }
    match record_identity(table, key) {
        Some(identity) => observe(identity),
        None => mark_incomplete(),
    }
}

/// The principal was resolved or checked.
pub fn observe_principal_read() {
    with_state(|state| state.report.context.principal_read = true);
}
/// The principal was resolved to a concrete value. Fix round 5: records
/// what was actually read, not merely that something was; fix round 6:
/// added to the observed *set* rather than overwriting a single slot - see
/// `CollectedContext::principal_material`'s own doc for why the collapse
/// itself was a leak.
pub fn observe_principal_value(id: &str) {
    with_state(|state| {
        state.report.context.principal_read = true;
        state
            .report
            .context
            .principal_material
            .insert(id.to_owned());
    });
}
/// The tenant was resolved or checked.
pub fn observe_tenant_read() {
    with_state(|state| state.report.context.tenant_read = true);
}
/// The tenant was resolved to a concrete value. Fix round 5: see
/// `observe_principal_value`'s own doc; the same reasoning applies.
pub fn observe_tenant_value(id: &str) {
    with_state(|state| {
        state.report.context.tenant_read = true;
        state.report.context.tenant_material.insert(id.to_owned());
    });
}
/// The locale was resolved to a concrete value, at the same
/// [`crate::Lang::locale`] call that already emits a
/// [`DependencyIdentity::Locale`] dependency. Fix round 6: see
/// `CollectedContext::locale_material`'s own doc for why re-deriving the
/// locale after the render (round 5's approach) cannot substitute for
/// recording it at the point of every read.
pub fn observe_locale_value(locale: &str) {
    with_state(|state| {
        state
            .report
            .context
            .locale_material
            .insert(locale.to_owned());
    });
}
/// A session value was read.
pub fn observe_session_read() {
    with_state(|state| state.report.context.session_read = true);
}
/// An authorization decision was evaluated.
pub fn observe_authorization_read() {
    with_state(|state| state.report.context.authorization_read = true);
}
/// Secret configuration was read. No framework read hooks this
/// automatically (see the module documentation); application code and
/// later adapters call it explicitly to mark a representation as having
/// touched secret configuration.
pub fn observe_secret_context_read() {
    with_state(|state| state.report.context.secret_context_read = true);
}
/// Records one successful Live island mount; a no-op outside a scope.
/// Accumulates rather than replaces, because a request can mount more than
/// one island (or more than one `LiveDocument`): counts add, and the seed
/// deadline takes the minimum of what was already recorded and this
/// mount's own. Called by [`super::live::record_mount`], never directly by
/// application code.
pub fn observe_live_document_mount(
    kind: crate::live::LiveMountKind,
    seed_deadline_ms: Option<u64>,
) {
    with_state(|state| {
        let facts = state
            .report
            .live_document
            .get_or_insert_with(Default::default);
        match kind {
            crate::live::LiveMountKind::PublicSeed => facts.public_seed_islands += 1,
            crate::live::LiveMountKind::IdentityBound => facts.identity_bound_islands += 1,
        }
        if let Some(deadline) = seed_deadline_ms {
            facts.seed_deadline_ms = Some(
                facts
                    .seed_deadline_ms
                    .map_or(deadline, |current| current.min(deadline)),
            );
        }
    });
}

/// Records that a rendered Live document declared `NoStore`; a no-op
/// outside a scope. Sticky: once set by any document in the request, stays
/// set regardless of what a later document in the same request declares.
/// Called by [`super::live::record_document_intent`], never directly by
/// application code.
pub fn observe_live_document_no_store() {
    with_state(|state| {
        state
            .report
            .live_document
            .get_or_insert_with(Default::default)
            .no_store = true;
    });
}
/// Undeclared request context affected rendering.
pub fn observe_undeclared(name: &str) {
    with_state(|state| {
        if state.report.undeclared.len() < 32 {
            state
                .report
                .undeclared
                .push(name.chars().take(64).collect());
        }
    });
}

/// The current report, or `None` outside a scope.
#[must_use]
pub fn current_report() -> Option<CollectorReport> {
    with_state(|state| state.report.clone())
}
