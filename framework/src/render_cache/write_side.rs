//! Whether this process advances RenderCache generations.
//!
//! `INSTALLED` answers "is a serving runtime installed here", which
//! is the right question for the middleware and the wrong one for a write:
//! a queue worker, a scheduled task, and a console command all write through
//! the same ORM the server does, share the same database, and have no
//! `Router` to hand `RenderCache::install`. Before this module their writes
//! advanced nothing, so a page depending on such a write went on being
//! served until its freshness window ran out.
//!
//! The rule this module implements: a process advances generations when a
//! runtime is installed, or when its configuration enables RenderCache and
//! its database holds the RenderCache migration. It probes at most once and
//! then costs nothing, so an application that never uses RenderCache still
//! pays no RenderCache SQL on any write - the property `INSTALLED` was
//! introduced to guarantee and that this must not lose.
//!
//! The probe itself runs on a connection taken directly from the pool
//! (`ledger::migration_present_off_transaction`), never on a caller's
//! transaction - a failing probe must not poison a write the caller is
//! also making. That means the probe needs a connection of its own, which
//! a caller already inside a transaction cannot safely wait for: it is
//! already holding the one connection its transaction was granted, so
//! asking the pool for a second blocks until the pool's acquire timeout
//! fires, on any backend, not only a single-connection test database. See
//! `write_side_open`'s `caller_holds_pool_connection` parameter: when
//! that is set, this module skips the probe rather than wait for a
//! connection it cannot be sure exists, and stays `Undecided` until a
//! write asks again from outside a transaction.

use std::sync::atomic::{AtomicU8, Ordering};

use suprnova_live::render_cache::generation::DependencyIdentity;

use crate::FrameworkError;
use crate::database::DB;

/// What the probe concluded about this process from one set of facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteSideDecision {
    /// This process advances generations, for the rest of its life.
    Open,
    /// This process never advances generations, for the rest of its life.
    Closed,
    /// Nothing is decided yet: without a connection there is neither a
    /// schema to probe nor a write to advance against, so the next write
    /// asks again.
    Undecided,
}

/// The tri-state, encoded so it fits one relaxed atomic. Relaxed is enough:
/// every transition is idempotent and monotone (`UNKNOWN` to one fixed
/// answer, and the two fixed answers can never disagree, since they are
/// computed from process-lifetime facts), so a racing pair of writes can
/// only ever store the same value twice.
///
/// Whether a runtime is installed is not one of those facts: a process can
/// write once before it installs RenderCache. So `installed` is read before
/// the fixed answer, never cached in it, and a `CLOSED` fixed earlier cannot
/// outlive an install (DATA-028).
const UNKNOWN: u8 = 0;
const OPEN: u8 = 1;
const CLOSED: u8 = 2;

static STATE: AtomicU8 = AtomicU8::new(UNKNOWN);

/// The probe's decision as a pure function of the four facts it reads.
///
/// Kept apart from the I/O deliberately: every row of this table matters
/// for correctness and none of them needs a database to be proven, so
/// `the_write_side_decision_table` pins all seven directly.
///
/// `migration` is `None` when the schema was not read - because the probe
/// did not get that far, or because reading it failed - and the caller
/// propagates that failure rather than fixing a decision on it.
#[must_use]
pub const fn decide(
    installed: bool,
    enabled: bool,
    connected: bool,
    migration: Option<bool>,
) -> WriteSideDecision {
    if installed {
        return WriteSideDecision::Open;
    }
    if !enabled {
        return WriteSideDecision::Closed;
    }
    if !connected {
        return WriteSideDecision::Undecided;
    }
    match migration {
        Some(true) => WriteSideDecision::Open,
        Some(false) => WriteSideDecision::Closed,
        None => WriteSideDecision::Undecided,
    }
}

/// Whether configuration enables RenderCache in this process.
fn enabled() -> bool {
    #[cfg(any(test, feature = "testing"))]
    if let Some(forced) = enabled_override_for_test() {
        return forced;
    }
    super::config::RenderCacheConfig::enabled_from_env()
}

/// True when this process advances generations. Probes at most once, then
/// answers from the fixed tri-state.
///
/// This is [`decide`] applied to the four facts and nothing more: it
/// gathers `installed`, `enabled`, `connected`, and the schema answer, hands
/// them to `decide`, and acts on the one value that comes back. The table
/// `the_write_side_decision_table` pins is therefore the code path itself
/// and cannot drift from it.
///
/// The schema is read only when it can decide anything - not installed,
/// enabled, and connected - so an installed, a disabled, or a disconnected
/// process issues no statement at all.
///
/// A runtime installed here answers `true` *without* fixing the state, so a
/// test that uninstalls the runtime gets the probe it would get in a worker
/// rather than the serving process's answer frozen in. It is also read
/// before the fixed state, so a `Closed` fixed by a write made before the
/// install does not keep an installed, serving process from advancing.
/// `Undecided` fixes nothing either: the next write asks again.
///
/// `caller_holds_pool_connection` is true when the call is made from inside
/// a transaction - ambient (`CURRENT_TX`) or an explicit `_with_tx` /
/// `with_tx` handle - that already holds the one connection it was granted
/// from the pool. The schema probe (`migration_present_off_transaction`)
/// needs a *second*, separate pooled connection: on a pool with no spare
/// connection - a single-connection test database is the extreme case, but
/// any pool can be briefly exhausted - asking for one blocks until the
/// pool's acquire timeout fires, which stalls the caller's write and can
/// exhaust the pool for everyone else waiting on it. So when this call
/// would otherwise probe the schema and the caller already holds a pool
/// connection, the probe is skipped and the process stays `Undecided`:
/// the write itself still runs, and the next call - from this caller's next
/// write, or from any write made outside a transaction - asks again. This
/// never blocks `installed`, `disabled`, or `disconnected` answers, none of
/// which touch the database.
///
/// # Errors
///
/// Propagates the schema probe's own database error, without fixing a
/// decision on it: a database that could not be reached is not the same
/// answer as one without the migration.
pub(crate) async fn write_side_open(
    caller_holds_pool_connection: bool,
) -> Result<bool, FrameworkError> {
    // `decide` answers Open for an installed process whatever the other
    // facts are, so that row is taken here, before the fixed state an
    // earlier, pre-install write may have left behind.
    if super::is_installed() {
        return Ok(true);
    }
    match STATE.load(Ordering::Relaxed) {
        OPEN => return Ok(true),
        CLOSED => return Ok(false),
        _ => {}
    }
    let installed = false;
    let enabled = enabled();
    let connected = DB::is_connected();
    let needs_schema_probe = enabled && connected;
    if needs_schema_probe && caller_holds_pool_connection {
        return Ok(false);
    }
    let migration = if needs_schema_probe {
        Some(super::ledger::migration_present_off_transaction().await?)
    } else {
        None
    };
    match decide(installed, enabled, connected, migration) {
        WriteSideDecision::Open => {
            STATE.store(OPEN, Ordering::Relaxed);
            Ok(true)
        }
        WriteSideDecision::Closed => {
            STATE.store(CLOSED, Ordering::Relaxed);
            Ok(false)
        }
        WriteSideDecision::Undecided => Ok(false),
    }
}

/// The decision this process has fixed, or `Undecided` before it probes.
#[must_use]
pub fn decision() -> WriteSideDecision {
    match STATE.load(Ordering::Relaxed) {
        OPEN => WriteSideDecision::Open,
        CLOSED => WriteSideDecision::Closed,
        _ => WriteSideDecision::Undecided,
    }
}

/// CACHE-009: set when an advancement that could not share its row write's
/// transaction (a write on a named connection, whose ledger lives on the
/// primary) failed, or was dropped part-way, after the row landed. While
/// set, every lookup in this process misses, so no entry whose invalidation
/// is uncertain is served. Process-local: another node learns nothing from
/// it, which Live spec 17 records as the limit of this fallback.
///
/// Cleared only by [`resolve`], once every identity in [`UNRESOLVED`] has
/// been advanced. An unrelated success used to clear it, which put the
/// entries the failed advance missed back in service on their old
/// generations (DATA-029).
static SERVING_SUSPENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The identities whose advancement this process could not record after
/// their rows committed, each with the sequence number of its latest
/// failure. The next advancement that can land carries them along with its
/// own, so the missed invalidation is repaired rather than forgotten.
///
/// The sequence number is what lets an advancement resolve only the
/// failures it carried. An advancement that lands has covered the writes
/// that committed before it, which are the failures recorded before it
/// took its snapshot. A failure recorded later can belong to a write that
/// committed after the advancement did, so it stays (DATA-029).
static UNRESOLVED: std::sync::Mutex<Unresolved> = std::sync::Mutex::new(Unresolved {
    recorded: 0,
    failures: std::collections::BTreeMap::new(),
});

/// The state behind [`UNRESOLVED`], kept under one lock so a snapshot and
/// its mark always agree.
struct Unresolved {
    /// How many failures this process has recorded; the latest one's
    /// sequence number.
    recorded: u64,
    /// Each unresolved identity and the sequence number of its latest
    /// failure.
    failures: std::collections::BTreeMap<DependencyIdentity, u64>,
}

impl Unresolved {
    /// Records one failure that missed `missed`, under the next sequence
    /// number.
    ///
    /// The set is bounded by the most identities one representation may
    /// observe: past that, it collapses to the broad identity every
    /// representation observes, so one advance of it still repairs
    /// everything the missed ones would have invalidated. The broad identity
    /// takes this failure's sequence number, the newest, so it stands for
    /// the newest failure it replaced.
    fn record(&mut self, missed: &[DependencyIdentity]) {
        self.recorded = self.recorded.saturating_add(1);
        let sequence = self.recorded;
        for identity in missed {
            self.failures.insert(identity.clone(), sequence);
        }
        if self.failures.len() > suprnova_live::render_cache::generation::MAX_OBSERVATIONS {
            self.failures.clear();
            self.failures.insert(DependencyIdentity::broad(), sequence);
        }
    }

    /// The unresolved identities, and the mark that says which failures
    /// they are.
    fn snapshot(&self) -> (Vec<DependencyIdentity>, UnresolvedMark) {
        (
            self.failures.keys().cloned().collect(),
            UnresolvedMark(self.recorded),
        )
    }

    /// Removes each of `advanced` whose latest failure is at or before
    /// `mark`, and reports whether nothing is left.
    fn resolve(&mut self, advanced: &[DependencyIdentity], mark: UnresolvedMark) -> bool {
        for identity in advanced {
            if self
                .failures
                .get(identity)
                .is_some_and(|&sequence| sequence <= mark.0)
            {
                self.failures.remove(identity);
            }
        }
        self.failures.is_empty()
    }
}

/// Which failures an advancement carried: every one recorded at or before
/// this mark. Returned by [`unresolved`] with the identities it read, and
/// handed back to [`resolve`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnresolvedMark(u64);

fn unresolved_set() -> std::sync::MutexGuard<'static, Unresolved> {
    UNRESOLVED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Stops serving stored entries until every one of `missed` is advanced.
/// See `Unresolved::record` for the bound on what is kept.
pub(crate) fn suspend_serving(missed: &[DependencyIdentity]) {
    let mut unresolved = unresolved_set();
    unresolved.record(missed);
    SERVING_SUSPENDED.store(true, Ordering::Relaxed);
}

/// The identities a failed advancement left behind, for the next
/// advancement to carry, and the mark that says which failures they are.
pub(crate) fn unresolved() -> (Vec<DependencyIdentity>, UnresolvedMark) {
    unresolved_set().snapshot()
}

/// `advanced` landed, carrying the failures recorded at or before `mark`:
/// those leave the unresolved set, and serving resumes once nothing is left
/// in it. An identity whose latest failure came after `mark` stays, because
/// the write behind that failure may have committed after this advancement
/// did (DATA-029).
pub(crate) fn resolve(advanced: &[DependencyIdentity], mark: UnresolvedMark) {
    let mut unresolved = unresolved_set();
    if unresolved.resolve(advanced, mark) {
        SERVING_SUSPENDED.store(false, Ordering::Relaxed);
    }
}

/// Whether [`suspend_serving`] is in force.
#[must_use]
pub(crate) fn serving_suspended() -> bool {
    SERVING_SUSPENDED.load(Ordering::Relaxed)
}

/// Returns the probe to `Unknown`, so the next write decides again.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn reset_for_test() {
    STATE.store(UNKNOWN, Ordering::Relaxed);
}

/// `-1` unset, `0` forced disabled, `1` forced enabled.
#[cfg(any(test, feature = "testing"))]
static ENABLED_OVERRIDE: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

/// Forces the configuration answer the probe reads.
///
/// A test cannot set `RENDER_CACHE_ENABLED` itself: `std::env::set_var` is
/// `unsafe` in Rust 2024 and this crate forbids `unsafe`. This seam reads
/// exactly where the variable would be read, and nothing else consults it.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn set_enabled_for_test(enabled: Option<bool>) {
    let encoded = match enabled {
        None => -1,
        Some(false) => 0,
        Some(true) => 1,
    };
    ENABLED_OVERRIDE.store(encoded, Ordering::Relaxed);
}

#[cfg(any(test, feature = "testing"))]
fn enabled_override_for_test() -> Option<bool> {
    match ENABLED_OVERRIDE.load(Ordering::Relaxed) {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! The unresolved set's own rules, on a local value rather than the
    //! process-wide one, which lookups in other tests of this binary read.
    use super::*;

    fn empty() -> Unresolved {
        Unresolved {
            recorded: 0,
            failures: std::collections::BTreeMap::new(),
        }
    }

    /// DATA-029: an advancement resolves the failures it carried and leaves
    /// a newer failure of the same identity in place.
    #[test]
    fn a_failure_recorded_after_the_mark_survives_resolve() {
        let posts = DependencyIdentity::table("posts");
        let users = DependencyIdentity::table("users");
        let mut unresolved = empty();
        unresolved.record(std::slice::from_ref(&posts));
        let (carried, mark) = unresolved.snapshot();
        assert_eq!(carried, vec![posts.clone()]);

        unresolved.record(&[posts.clone(), users.clone()]);
        assert!(
            !unresolved.resolve(&[posts.clone(), users.clone()], mark),
            "both failures recorded after the mark are still unresolved"
        );
        let (left, newer) = unresolved.snapshot();
        assert_eq!(left, vec![posts.clone(), users.clone()]);

        assert!(
            unresolved.resolve(&[posts, users], newer),
            "an advancement carrying the newer failures resolves them"
        );
    }

    /// A collapse to the broad identity takes the newest sequence number, so
    /// an advancement that carried only the older failures cannot resolve
    /// it.
    #[test]
    fn a_collapse_after_the_mark_is_not_resolved_by_the_older_advancement() {
        let posts = DependencyIdentity::table("posts");
        let mut unresolved = empty();
        unresolved.record(std::slice::from_ref(&posts));
        let (_, mark) = unresolved.snapshot();

        let many: Vec<DependencyIdentity> = (0
            ..=suprnova_live::render_cache::generation::MAX_OBSERVATIONS)
            .map(|index| DependencyIdentity::table(&format!("table_{index}")))
            .collect();
        unresolved.record(&many);
        let broad = DependencyIdentity::broad();
        assert_eq!(unresolved.snapshot().0, vec![broad.clone()]);

        assert!(
            !unresolved.resolve(&[posts, broad.clone()], mark),
            "the collapsed failure is newer than the mark"
        );
        let (_, newer) = unresolved.snapshot();
        assert!(unresolved.resolve(&[broad], newer));
    }
}
