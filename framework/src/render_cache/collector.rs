//! Request-scoped dependency collector: a Tokio task-local that framework
//! reads register into. Absent outside a scope, so ordinary requests pay
//! one `try_with` per read and nothing else.

use std::sync::{Arc, Mutex};

use suprnova_live::render_cache::generation::{DependencyIdentity, MAX_OBSERVATIONS};

/// Context flags the collector accumulates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CollectedContext {
    /// A principal was resolved or checked.
    pub principal_read: bool,
    /// A session value was read.
    pub session_read: bool,
    /// An authorization decision was evaluated.
    pub authorization_read: bool,
    /// Secret configuration was read.
    pub secret_context_read: bool,
    /// Observation bound exceeded; the response must not be stored.
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
#[must_use]
pub fn is_active() -> bool {
    COLLECTOR.try_with(|_| ()).is_ok()
}

/// Records a typed dependency; bounded, idempotent, no-op outside a scope.
pub fn observe(identity: DependencyIdentity) {
    with_state(|state| {
        if state.seen.contains(&identity) {
            return;
        }
        if state.seen.len() >= MAX_OBSERVATIONS {
            state.report.context.overflowed = true;
            return;
        }
        state.seen.insert(identity.clone());
        state.report.observed.push(identity);
    });
}

/// A read of one table.
pub fn observe_table_read(table: &str) {
    if let Ok(identity) = DependencyIdentity::try_table(table) {
        observe(identity);
    }
}

/// A record read by primary key bytes.
pub fn observe_record_read(table: &str, key: &[u8]) {
    if let Ok(identity) = DependencyIdentity::try_record(table, key) {
        observe(identity);
    }
}

/// A record read identified by its JSON-encoded primary key value.
///
/// This is the only place a record's primary key becomes bytes: the write
/// side advances a record's generation using
/// `model.primary_key_value_json().to_string()`, so the read side must
/// encode the same value the same way or the two identities never match and
/// record-level invalidation silently never fires.
pub fn observe_record_read_json(table: &str, key: &serde_json::Value) {
    observe_record_read(table, key.to_string().as_bytes());
}

/// The principal was resolved or checked.
pub fn observe_principal_read() {
    with_state(|state| state.report.context.principal_read = true);
}
/// A session value was read.
pub fn observe_session_read() {
    with_state(|state| state.report.context.session_read = true);
}
/// An authorization decision was evaluated.
pub fn observe_authorization_read() {
    with_state(|state| state.report.context.authorization_read = true);
}
/// Secret configuration was read.
pub fn observe_secret_context_read() {
    with_state(|state| state.report.context.secret_context_read = true);
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
