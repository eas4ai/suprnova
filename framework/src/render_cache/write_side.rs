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
//! [`write_side_open`]'s `caller_holds_pool_connection` parameter: when
//! that is set, this module skips the probe rather than wait for a
//! connection it cannot be sure exists, and stays `Undecided` until a
//! write asks again from outside a transaction.

use std::sync::atomic::{AtomicU8, Ordering};

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
/// rather than the serving process's answer frozen in. `Undecided` fixes
/// nothing either: the next write asks again.
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
    match STATE.load(Ordering::Relaxed) {
        OPEN => return Ok(true),
        CLOSED => return Ok(false),
        _ => {}
    }
    let installed = super::is_installed();
    let enabled = enabled();
    let connected = DB::is_connected();
    let needs_schema_probe = !installed && enabled && connected;
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
            if !installed {
                STATE.store(OPEN, Ordering::Relaxed);
            }
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
///
/// Compiled unconditionally, because the missing-table warning (ruling R65)
/// reads it from a synchronous match arm with no place to await a probe: it
/// reports a schema regression only for a process that had already decided
/// it advances generations, which is exactly the process for which a
/// vanished table is a regression rather than the ordinary uninstalled
/// case.
#[must_use]
pub fn decision() -> WriteSideDecision {
    match STATE.load(Ordering::Relaxed) {
        OPEN => WriteSideDecision::Open,
        CLOSED => WriteSideDecision::Closed,
        _ => WriteSideDecision::Undecided,
    }
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
