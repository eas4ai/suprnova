//! Request-scoped dependency collector: a Tokio task-local that framework
//! reads register into. Absent outside a scope, so ordinary requests pay
//! one `try_with` per read and nothing else.
//!
//! # Attribution
//!
//! Every read lands in one of three buckets, chosen by where the request
//! was when it happened. A scope starts in the **gate** bucket: whatever
//! ran before the route handler - an authorization guard, a tenant
//! middleware - reads there, and those reads run again on every request,
//! hit or miss. [`begin_handler`] switches the scope to the **content**
//! bucket, which holds what the handler itself read to build the body.
//! [`slot_scope`] runs an identity-bound island's mount in the **slot**
//! bucket, whose reads are counted into [`CollectorReport::slot_reads`]
//! and recorded nowhere else, because those islands are re-rendered on
//! every hit.
//!
//! Only a [`crate::render_cache::RepresentationClass::PublicShellStitched`]
//! route classifies from the content bucket alone. Three things together
//! are what make that sound. The stored shell holds only what the handler
//! rendered after [`begin_handler`], so a gate's own bytes are never in it,
//! and a gate that rewrites the body after the handler returned is caught
//! by the body digest and declines the store. Every hit on that class runs
//! the route's gate again before anything is served, so the gate's decision
//! is taken fresh per request rather than read back out of the shell. And a
//! gate value that also reaches the handler through an instrumented seam -
//! `Auth::user()`, a cookie, a query-builder read - is observed again in
//! the content bucket when the handler consumes it, so it still reaches the
//! key. What is left is a gate value reaching the handler through an
//! uninstrumented seam, a request header or an application task-local:
//! the pre-existing boundary "Limitations, by design" below already
//! describes, which the gate bucket never closed and this exemption does
//! not widen. Every other class folds the gate bucket back into content
//! ([`CollectorReport::fold_gate_into_content`], called by the render
//! cache middleware for every non-stitched route) and classifies from
//! exactly the undivided report it produced before attribution existed.
//!
//! A stitched route whose chain answered before the handler ever ran has
//! an empty content bucket, and classifying from it would publish the
//! gate's own response as the shared shell. The report records that in
//! [`CollectorReport::handler_began`] and the middleware declines to
//! store the representation when it is `false`.
//!
//! # Limitations, by design
//!
//! - **Config and Feature identities have no automatic producer.** No
//!   framework read observes a config or feature generation and no write
//!   path advances one, so observing them would spend the bounded
//!   observation budget while contributing nothing to invalidation - the
//!   same reasoning as ruling R24 on query classes. `Config::get::<T>()` is
//!   also type-keyed rather than name-keyed, so there is no stable name to
//!   build an identity from at that seam. The one reserved exception is
//!   [`permission_version_identity`], a `Config` identity this crate itself
//!   both observes (for every render whose key carries a resolved
//!   `Principal`) and advances
//!   ([`crate::render_cache::RenderCache::bump_permission_version`]).
//! - **Raw SQL reads mark the report incomplete.** `DB::select`,
//!   `DB::select_one`, `DB::scalar`, and `DB::select_on` cannot name the
//!   tables they read, so they call [`observe_unobservable_read`] and the
//!   render is never stored (final review, F2). The query-builder facade
//!   (`DB::table(..).get()`, `first()`, `count()`) knows its table and
//!   records it through [`observe_table_read`] instead. Inside a slot the
//!   same read is counted as a slot read and marks nothing, for the reason
//!   `mark_incomplete` gives (a private function; see its own comment).
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
//! - **Uninstrumented seams are invisible to every bucket.** A gate value
//!   that reaches the handler through a request header or an application
//!   task-local is never observed, so it reaches neither the key nor the
//!   classification. That boundary predates attribution, and the stitched
//!   class's gate exemption does not widen it; the implementation doc's
//!   "The honest boundary of what the guards can see" records it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use suprnova_live::render_cache::composite::{MAX_SHELL_ISLANDS, MAX_STITCH_SLOTS, ShellIsland};
use suprnova_live::render_cache::generation::{DependencyIdentity, MAX_OBSERVATIONS};

/// The reserved `Config` key behind [`permission_version_identity`].
///
/// A dotted, crate-prefixed name no application configuration key would
/// collide with; the identity's digest, not this text, is what the ledger
/// stores.
pub const PERMISSION_VERSION_CONFIG_KEY: &str = "suprnova.render_cache.permission_version";

/// The dependency identity whose generation is the permission version.
///
/// Every render whose lookup key carries a resolved `Principal` value
/// observes this identity (the middleware adds it to the collector at the
/// start of the render), and
/// [`crate::render_cache::RenderCache::bump_permission_version`] advances it
/// through the same ledger path an ORM write uses, so a bump is persisted in
/// `suprnova_render_generations` and logged like any other advance. A
/// cached private representation published before a bump therefore fails
/// its coherence check at the next lookup, and keeps failing it after a
/// process restart, which the earlier process-local counter (a `static
/// AtomicU64` that reset to 0 while an L1 entry survived on disk) did not
/// guarantee (final review, F3).
///
/// Nothing in the framework bumps this on its own: role and permission
/// changes are application-defined (a `roles` table update, a policy
/// reassignment), so only the application knows when they happen. An
/// application that grants or revokes permissions and cares about
/// RenderCache must call `bump_permission_version` when it does. Without
/// that call, a user whose permissions were just revoked keeps matching the
/// cache key their prior permission set produced, and keeps being served
/// whatever was cached under it.
#[must_use]
pub fn permission_version_identity() -> DependencyIdentity {
    DependencyIdentity::config(PERMISSION_VERSION_CONFIG_KEY)
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
    /// Every distinct locale value (`Lang::locale`, a localization-only
    /// item) returned during
    /// the render, recorded at the same observation point that already
    /// emits a [`DependencyIdentity::Locale`] dependency (fix round 6).
    ///
    /// Round 5 instead re-read `Lang::locale()` a second time, after the
    /// render, reasoning that the task-local was "still installed" then.
    /// That is only true for the outer scope: a handler that renders inside
    /// `scope_locale` (the framework's own documented, supported
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

/// Which bucket a read lands in. A scope starts in `Gate`; the Live
/// completion middleware (the last middleware before the handler)
/// switches it to `Content`; an identity-bound mount runs in `Slot`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Attribution {
    #[default]
    Gate,
    Content,
    Slot,
}

/// Reads made by middleware before the handler started.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GateReport {
    /// Context flags from gate reads.
    pub context: CollectedContext,
    /// Dependencies from gate reads.
    pub observed: Vec<DependencyIdentity>,
}

/// The collector's report.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectorReport {
    /// Observed identities in first-seen order, deduplicated.
    pub observed: Vec<DependencyIdentity>,
    /// Context flags.
    pub context: CollectedContext,
    /// Reads made by middleware before the handler started; see
    /// [`begin_handler`].
    pub gate: GateReport,
    /// Reads made inside identity-bound mounts; counted, never recorded,
    /// because those islands are re-rendered on every hit.
    pub slot_reads: usize,
    /// Whether [`begin_handler`] ever ran in this scope, and therefore
    /// whether the content bucket is an account of what the handler read
    /// or merely of nothing having happened yet.
    ///
    /// A stitched shell is classified from content reads alone, so a
    /// request whose chain answered before the handler ever started - an
    /// authorization guard that returns a page instead of calling the
    /// next layer, a tenant refusal - would be classified from an empty
    /// content bucket and published as the shared shell for that route,
    /// which is the gate's own response. The render cache middleware
    /// declines to store a stitched representation whenever this is
    /// `false`. [`fold_gate_into_content`](Self::fold_gate_into_content)
    /// deliberately leaves it alone: it records what happened during the
    /// request, not which bucket a read ended up in.
    pub handler_began: bool,
    /// Undeclared request context names that affected rendering.
    pub undeclared: Vec<String>,
    /// Facts a rendered Live document recorded, if the render mounted one.
    pub live_document: Option<super::live::LiveDocumentFacts>,
    /// Test-only: see [`strip_classification_reasons_for_test`]'s own doc.
    /// Lives on the report itself, not a separate collector field, because
    /// `lead_render` reads it from the *extracted* report after the
    /// collector scope has already closed - the same reason every other
    /// field here is captured this way rather than re-queried later.
    #[cfg(any(test, feature = "testing"))]
    pub strip_classification_reasons: bool,
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

    /// Folds gate reads into the content buckets (gate first,
    /// deduplicated), producing the undivided report every non-stitched
    /// route classifies from.
    ///
    /// The gate bucket is emptied, so folding twice is a no-op the second
    /// time. Gate first because these reads genuinely happened first, and
    /// the observed order is what [`storable`](Self::storable) hands the
    /// observation window. [`handler_began`](Self::handler_began) is left
    /// as it was: it is a fact about the request, not about bucketing.
    pub fn fold_gate_into_content(&mut self) {
        let gate = std::mem::take(&mut self.gate);
        let content = &mut self.context;
        content.principal_read |= gate.context.principal_read;
        content
            .principal_material
            .extend(gate.context.principal_material);
        content.tenant_read |= gate.context.tenant_read;
        content.tenant_material.extend(gate.context.tenant_material);
        content.locale_material.extend(gate.context.locale_material);
        content.session_read |= gate.context.session_read;
        content.authorization_read |= gate.context.authorization_read;
        content.secret_context_read |= gate.context.secret_context_read;
        // No `overflowed` fold: the gate context has no overflow state to
        // carry. Both producers - `mark_incomplete` and `observe`'s bound
        // check - write `report.context.overflowed` whatever bucket the
        // read was attributed to, precisely so one overflow marks the whole
        // report no matter where it happened.
        let mut merged = gate.observed;
        let seen: BTreeSet<DependencyIdentity> = merged.iter().cloned().collect();
        merged.extend(
            self.observed
                .drain(..)
                .filter(|identity| !seen.contains(identity)),
        );
        self.observed = merged;
    }
}

#[derive(Default)]
struct State {
    report: CollectorReport,
    seen: std::collections::BTreeSet<DependencyIdentity>,
    /// Which bucket the next read lands in; see the module documentation.
    attribution: Attribution,
    /// The gate bucket's own deduplication set, kept separate from `seen`
    /// so a table read by both the gate and the handler is recorded once
    /// in each bucket and once after they are folded together.
    seen_gate: std::collections::BTreeSet<DependencyIdentity>,
    /// How many *distinct* identities the two buckets hold between them:
    /// `|seen_gate union seen|`, maintained incrementally rather than
    /// recomputed, since `observe` runs on every framework read.
    ///
    /// This, and not `seen_gate.len() + seen.len()`, is what
    /// [`MAX_COLLECTED`] bounds. An identity the gate and the handler both
    /// read occupies a slot in each set but folds into one observation, so
    /// counting the two sums would spend two units of the budget on one
    /// observation and overflow a report whose folded set is well inside
    /// the window.
    distinct: usize,
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

/// Runs `f` against the context of whichever bucket the current
/// attribution selects, or counts a slot read and does nothing when the
/// request is inside an identity-bound mount. `None` outside a scope, and
/// `None` for a slot read, which has no context to hand back.
fn with_context<R>(f: impl FnOnce(&mut CollectedContext) -> R) -> Option<R> {
    with_state(|state| match state.attribution {
        Attribution::Gate => Some(f(&mut state.report.gate.context)),
        Attribution::Content => Some(f(&mut state.report.context)),
        Attribution::Slot => {
            state.report.slot_reads += 1;
            None
        }
    })
    .flatten()
}

/// Marks the start of the route handler: every later read is a content
/// read. Idempotent; a no-op outside a scope. Called by the Live
/// completion middleware, the last middleware before any Live route's
/// handler.
///
/// Framework-internal, and `pub` only because the framework's own
/// integration tests drive it: calling it from application code moves the
/// gate/content boundary to wherever the call is, so reads that ran before
/// the handler are recorded as ones the shared shell depends on and the
/// route classifies from a bucket that no longer describes the body.
#[doc(hidden)]
pub fn begin_handler() {
    with_state(|state| {
        state.report.handler_began = true;
        if state.attribution == Attribution::Gate {
            state.attribution = Attribution::Content;
        }
    });
}

/// Runs `future` with reads attributed to an identity-bound island slot.
/// Restores the previous attribution afterwards, including on an early
/// return, a panic, or the future being dropped part-way.
///
/// Nested calls stay in the slot bucket rather than being rejected: the
/// inner scope's "previous" attribution is `Slot` itself, so a mount that
/// nests another mount keeps counting reads the same way and the
/// outermost scope restores gate or content exactly once. A slot read is
/// recorded nowhere, so nesting cannot leak one into a recorded bucket.
///
/// A scope that found the gate bucket restores the *content* bucket when
/// [`begin_handler`] ran while it was open: the handler boundary was
/// genuinely crossed inside the slot, so restoring gate would record the
/// handler's own later reads as gate reads it never made, moving them to
/// the wrong side of the boundary a stitched route classifies from.
///
/// Framework-internal, and `pub` only because the framework's own
/// integration tests drive it: calling it from application code suppresses
/// dependency recording for everything inside it, since a slot read is
/// counted and recorded nowhere, so a route classifies as depending on
/// less than it actually read.
#[doc(hidden)]
pub async fn slot_scope<F: std::future::Future>(future: F) -> F::Output {
    struct Restore(Option<Attribution>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.0 {
                with_state(|state| {
                    state.attribution =
                        if previous == Attribution::Gate && state.report.handler_began {
                            Attribution::Content
                        } else {
                            previous
                        };
                });
            }
        }
    }
    let previous = with_state(|state| std::mem::replace(&mut state.attribution, Attribution::Slot));
    let _restore = Restore(previous);
    future.await
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
///
/// Except inside a slot, where it counts a slot read like every other read
/// there and marks nothing. A slot's reads are recorded in no bucket and
/// re-run on every hit, so a read the collector cannot name inside one says
/// nothing about what the stored shell depends on - and the shell is the
/// only thing the report decides the fate of. Marking the report from here
/// would mean one island reaching for raw SQL silently stopped a whole
/// route from ever being stored, which is not a conservatism that buys
/// anything: the island re-renders per hit either way. The route that keeps
/// an identity-bound island *without* stitching is already declined whole,
/// by [`super::live::document_declines`], so no path stores a body carrying
/// a slot's unnamed read. One path reaches storage without that decline and
/// is safe anyway: `LiveDocument::mount` records the identity-bound fact
/// only *after* the mount, so a mount that fails returns before anything is
/// recorded and leaves the report storable - and the body it leaves behind
/// carries no island bytes at all, because the mount that would have
/// produced them is the one that failed
/// (`render_cache::live::a_failed_identity_bound_mount_publishes_a_shell_with_no_island_bytes`).
fn mark_incomplete() {
    with_state(|state| match state.attribution {
        Attribution::Slot => state.report.slot_reads += 1,
        Attribution::Gate | Attribution::Content => state.report.context.overflowed = true,
    });
}

/// Records a typed dependency into the bucket the current attribution
/// selects; bounded, idempotent within a bucket, no-op outside a scope.
///
/// The bound is shared across the gate and content buckets, because both
/// are folded into one observation window on every non-stitched route. A
/// per-bucket bound would let a report sit under the cap in each bucket
/// separately, overflow only once the two were folded, and be stored on a
/// dependency set that had already dropped identities. It is shared as a
/// count of *distinct* identities, since that is what the fold produces:
/// see the private `State::distinct` field's own comment.
pub fn observe(identity: DependencyIdentity) {
    with_state(|state| match state.attribution {
        Attribution::Slot => state.report.slot_reads += 1,
        Attribution::Gate => {
            if state.seen_gate.contains(&identity) {
                return;
            }
            // New to this bucket, but the budget is spent per distinct
            // identity: one the content bucket already holds folds into the
            // same observation and costs nothing more. See `State::distinct`.
            let new_to_report = !state.seen.contains(&identity);
            if new_to_report && state.distinct >= MAX_COLLECTED {
                state.report.context.overflowed = true;
                return;
            }
            state.distinct += usize::from(new_to_report);
            state.seen_gate.insert(identity.clone());
            state.report.gate.observed.push(identity);
        }
        Attribution::Content => {
            if state.seen.contains(&identity) {
                return;
            }
            let new_to_report = !state.seen_gate.contains(&identity);
            if new_to_report && state.distinct >= MAX_COLLECTED {
                state.report.context.overflowed = true;
                return;
            }
            state.distinct += usize::from(new_to_report);
            state.seen.insert(identity.clone());
            state.report.observed.push(identity);
        }
    });
}

/// A read whose dependencies cannot be named at all: raw SQL through
/// `DB::select`, `DB::select_one`, `DB::scalar`, or `DB::select_on`, whose
/// statement text is opaque to the framework.
///
/// Marks the report incomplete, the same way an overflowed report or an
/// unencodable identity does, so the representation is never stored: a
/// render that read something the collector cannot account for is not one
/// whose stored dependency set could ever be trusted to invalidate it (final
/// review, F2; the 005 contract's "insufficiently observable reads bypass
/// caching"). A no-op outside a scope, like every other observer here, and
/// inside a slot a counted slot read that marks nothing - see the private
/// `mark_incomplete`'s own comment.
pub fn observe_unobservable_read() {
    if !is_active() {
        return;
    }
    mark_incomplete();
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
    with_context(|context| context.principal_read = true);
}
/// The principal was resolved to a concrete value. Fix round 5: records
/// what was actually read, not merely that something was; fix round 6:
/// added to the observed *set* rather than overwriting a single slot - see
/// `CollectedContext::principal_material`'s own doc for why the collapse
/// itself was a leak.
pub fn observe_principal_value(id: &str) {
    with_context(|context| {
        context.principal_read = true;
        context.principal_material.insert(id.to_owned());
    });
}
/// The tenant was resolved or checked.
pub fn observe_tenant_read() {
    with_context(|context| context.tenant_read = true);
}
/// The tenant was resolved to a concrete value. Fix round 5: see
/// `observe_principal_value`'s own doc; the same reasoning applies.
pub fn observe_tenant_value(id: &str) {
    with_context(|context| {
        context.tenant_read = true;
        context.tenant_material.insert(id.to_owned());
    });
}
/// The locale was resolved to a concrete value, at the same
/// `Lang::locale` call (a localization-only item) that already emits a
/// [`DependencyIdentity::Locale`] dependency. Fix round 6: see
/// `CollectedContext::locale_material`'s own doc for why re-deriving the
/// locale after the render (round 5's approach) cannot substitute for
/// recording it at the point of every read.
pub fn observe_locale_value(locale: &str) {
    with_context(|context| {
        context.locale_material.insert(locale.to_owned());
    });
}
/// A session value was read.
pub fn observe_session_read() {
    with_context(|context| context.session_read = true);
}
/// An authorization decision was evaluated.
pub fn observe_authorization_read() {
    with_context(|context| context.authorization_read = true);
}
/// Secret configuration was read. No framework read hooks this
/// automatically (see the module documentation); application code and
/// later adapters call it explicitly to mark a representation as having
/// touched secret configuration.
pub fn observe_secret_context_read() {
    with_context(|context| context.secret_context_read = true);
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

/// Records one identity-bound island and its emitted bytes; a no-op
/// outside a scope. Recording more than [`MAX_STITCH_SLOTS`] slots marks
/// the capture invalid and records nothing further, because a shell that
/// silently dropped an island would be published with that island's
/// contents baked into shared bytes. Called by
/// [`super::live::record_stitch_slot`], never directly by application code.
pub fn observe_live_document_stitch_slot(slot: super::live::CapturedSlot) {
    with_state(|state| {
        let capture = &mut state
            .report
            .live_document
            .get_or_insert_with(Default::default)
            .stitch;
        if capture.slots.len() >= MAX_STITCH_SLOTS {
            capture.invalid = true;
            return;
        }
        capture.slots.push(slot);
    });
}

/// Records one public-seed island as remaining inside the shell; a no-op
/// outside a scope. Bounded by [`MAX_SHELL_ISLANDS`], the same way and for
/// the same reason as [`observe_live_document_stitch_slot`]. Called by
/// [`super::live::record_shell_island`], never directly by application code.
pub fn observe_live_document_shell_island(island: ShellIsland) {
    with_state(|state| {
        let capture = &mut state
            .report
            .live_document
            .get_or_insert_with(Default::default)
            .stitch;
        if capture.shell_islands.len() >= MAX_SHELL_ISLANDS {
            capture.invalid = true;
            return;
        }
        capture.shell_islands.push(island);
    });
}

/// Records the nonce one document's bootstrap markup stamped; a no-op
/// outside a scope. A second bootstrap carrying a *different* nonce marks
/// the capture invalid: one shell can only hold one nonce, so two disagree
/// about what the holes it cuts should be filled with. A second bootstrap
/// that stamps no nonce at all is not a conflict - it cut no holes. Called
/// by [`super::live::record_bootstrap_nonce`], never directly by
/// application code.
pub fn observe_live_document_bootstrap_nonce(nonce: Option<&str>) {
    let Some(fresh) = nonce else {
        return;
    };
    with_state(|state| {
        let capture = &mut state
            .report
            .live_document
            .get_or_insert_with(Default::default)
            .stitch;
        match capture.nonce.as_deref() {
            None => capture.nonce = Some(fresh.to_owned()),
            Some(existing) if existing != fresh => capture.invalid = true,
            Some(_) => {}
        }
    });
}

/// Records the digest of one rendered document body; a no-op outside a
/// scope. A second digest marks the capture invalid and keeps the first:
/// two rendered documents in one request means the recorded mounts belong
/// to two different bodies, and no single shell can be cut from both.
/// Called by [`super::live::record_document_digest`], never directly by
/// application code.
pub fn observe_live_document_digest(digest: [u8; 32]) {
    with_state(|state| {
        let capture = &mut state
            .report
            .live_document
            .get_or_insert_with(Default::default)
            .stitch;
        if capture.document_digest.is_some() {
            capture.invalid = true;
            return;
        }
        capture.document_digest = Some(digest);
    });
}

/// Marks the capture as one no shell can be built from; a no-op outside a
/// scope. Called by [`super::live::record_stitch_capture_invalid`], never
/// directly by application code.
pub fn observe_live_document_stitch_invalid() {
    with_state(|state| {
        state
            .report
            .live_document
            .get_or_insert_with(Default::default)
            .stitch
            .invalid = true;
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

/// Test-only: marks the active collector scope so a *copy* of the
/// classification this request produces - never the real one, which still
/// reaches the value guard and `entry_header` untouched (R90) - has its
/// `reasons` cleared immediately after `classify` runs, right before the
/// R86 invariant (`is_unreasoned_private_class` in `middleware.rs`) checks
/// that copy; because the real classification is never mutated, calling
/// this can only ever make the invariant decline, never weaken any guard
/// downstream of it.
///
/// Simulates a class `classify` genuinely narrowed to `PrivateCached` but
/// that somehow carries no attached reason - a shape `classify`'s own
/// implementation cannot produce today (every narrowing call pushes its
/// reason unconditionally, so a `PrivateCached` result with empty reasons
/// only ever arises when the route's own declared class was already
/// `PrivateCached`, which the R89 scoping condition now exempts). This
/// exists only so a test can exercise `is_unreasoned_private_class`'s call
/// site through `lead_render`'s real control flow - reading an identity so
/// `classify` narrows and attaches `PrincipalObserved`, then stripping that
/// reason here - rather than constructing a `ClassificationOutcome`
/// directly and calling the predicate in isolation, which the delivered
/// suite already does separately. A no-op outside a collector scope, like
/// every other observer in this module.
///
/// Sets the flag on the report itself rather than a separate collector
/// field: `lead_render` calls `classify` using a `CollectorReport` already
/// *extracted* from the collector (the scope has closed by then, the same
/// reason every other field on this report is captured this way), so the
/// flag has to travel with it to be readable at that point.
#[cfg(any(test, feature = "testing"))]
pub fn strip_classification_reasons_for_test() {
    with_state(|state| state.report.strip_classification_reasons = true);
}
