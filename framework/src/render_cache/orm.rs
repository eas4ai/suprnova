//! Generation advancement for every supported write path, inside the
//! owning transaction.
//!
//! Task 10 shipped the request-scoped collector (what a render read) and
//! Task 11 the database-authoritative ledger (how many times a dependency
//! changed). This module is the write side that connects them: every
//! supported write path calls one of the four functions below right after
//! its row(s) land and before it dispatches any non-cancellable lifecycle
//! event, so a page that depended on the changed data stops being served
//! stale the moment the write commits. A write path that never calls one
//! of these advances nothing, and pages that depended on it keep being
//! served after it changes - see ruling R48 for exactly that failure mode
//! on the soft-delete `restore` override. Which *process* ran the write
//! makes no difference: every process whose configuration enables
//! RenderCache and whose database holds the migration advances generations
//! (see `super::write_side`).

use sea_orm::{EntityTrait, IntoActiveModel, PrimaryKeyTrait};
use serde::Serialize;
use suprnova_live::render_cache::generation::DependencyIdentity;

use crate::database::Transaction;
use crate::database::after_commit::in_transaction;
use crate::eloquent::Model;
use crate::{DB, FrameworkError};

/// Advances `identities` inside the current ambient transaction, or opens
/// one around just the advance when none is active.
///
/// Returns `Ok(())` immediately, issuing no SQL at all, when this process
/// does not advance generations (`super::write_side_open`): either its
/// configuration disables RenderCache, or its database does not hold the
/// RenderCache migration. That keeps the property this gate was introduced
/// for - an application that never uses RenderCache performs zero
/// RenderCache SQL on any write - while letting a queue worker, a scheduled
/// task, or a console command against a RenderCache database advance
/// exactly what the same write advances in the server. The probe runs at
/// most once per process and off any caller transaction; see
/// `super::write_side`.
///
/// The common case - a write issued inside `DB::transaction`, or one whose
/// caller already opened a transaction for it - takes the first branch:
/// the advance joins that same transaction, so a caller rollback undoes it
/// along with the row write. A write issued with no ambient transaction at
/// all (a bare `model.save()`) still needs its advance to land as one
/// atomic unit across every identity it touches, so the second branch
/// opens a transaction for that alone - and because nothing else rides on
/// that throwaway transaction, it is the one case where a missing-table
/// failure is safe to swallow; see [`super::ledger::advance_in_dedicated_transaction`].
///
/// The second branch requires a primary connection: `DB::transaction`
/// always opens against it, the same as the generation ledger's own reads
/// (`SqlGenerationLedger::current` / `epoch`) are pinned to it rather than
/// any per-model or named connection a write itself might route through.
/// An app that registers only named connections and never calls `DB::init`
/// has no primary pool at all - a supported, tested configuration (see
/// `eloquent_eager_named_connection.rs`) - and a model write on one of
/// those named connections must keep working exactly as it always has.
///
/// `pub(crate)` for exactly one caller outside this module:
/// [`super::RenderCache::bump_permission_version`], which advances the
/// reserved permission-version identity through this same path so a bump is
/// persisted and logged the way every ORM write's advance is.
pub(crate) async fn advance(identities: Vec<DependencyIdentity>) -> Result<(), FrameworkError> {
    let already_in_transaction = in_transaction();
    if !super::write_side_open(already_in_transaction).await? {
        return Ok(());
    }
    if already_in_transaction {
        #[cfg(any(test, feature = "testing"))]
        seams::hold_point(&identities).await;
        return super::ledger::advance_in_current_transaction(&identities).await;
    }
    if !DB::is_connected() {
        return Ok(());
    }
    // DATA-029: carries every identity an earlier failed advance left
    // behind, so the first advance that can land repairs those missed
    // invalidations too, instead of serving resuming over them. `mark`
    // says which failures those are, so a failure recorded while this
    // advance runs is not resolved by it.
    let (missed, mark) = super::write_side::unresolved();
    let mut carried = identities;
    for missed in missed {
        if !carried.contains(&missed) {
            carried.push(missed);
        }
    }
    let attempted = carried.clone();
    // DATA-039: the row this advance describes has already committed, so a
    // drop of this future before the advance lands - a client disconnect, a
    // timeout, a `select!` - would leave it ahead of its generations with
    // nothing to say so. The guard suspends serving for them instead.
    let unfinished = UnfinishedAdvance(Some(attempted.clone()));
    #[cfg(any(test, feature = "testing"))]
    seams::hold_point(&attempted).await;
    let outcome = DB::transaction(move |_tx| {
        Box::pin(async move { super::ledger::advance_in_dedicated_transaction(&carried).await })
    })
    .await;
    unfinished.finish();
    // CACHE-009: this branch is the one where the advance could not share
    // the row write's transaction, so a failure here leaves a committed row
    // whose dependents may still be served. Serving stops until every
    // identity it missed has been advanced; see
    // `super::write_side::suspend_serving`.
    match &outcome {
        Ok(()) => super::write_side::resolve(&attempted, mark),
        Err(_) => super::write_side::suspend_serving(&attempted),
    }
    outcome
}

/// A dedicated advance that has not finished yet. Dropped unfinished, it
/// records its identities as unresolved and suspends serving, exactly as a
/// failed advance does; [`Self::finish`] disarms it once the advance has an
/// outcome to act on.
struct UnfinishedAdvance(Option<Vec<DependencyIdentity>>);

impl UnfinishedAdvance {
    fn finish(mut self) {
        self.0 = None;
    }
}

impl Drop for UnfinishedAdvance {
    fn drop(&mut self) {
        if let Some(identities) = self.0.take() {
            super::write_side::suspend_serving(&identities);
        }
    }
}

/// Advances, in a transaction of its own, every identity an earlier failed
/// advancement left behind (DATA-029).
///
/// Called after a write committed together with its own advancement. That
/// write succeeded and must not report an older write's failure, so a
/// failure here is logged and leaves serving suspended, which is the
/// protection the missed identities still need.
///
/// `pub(crate)` for the payments webhook hydration, which commits its own
/// raw transaction with its advance inside it (DATA-039) and repairs after
/// that commit the way [`atomic`] does after its own.
pub(crate) async fn repair_unresolved() {
    let (missed, _) = super::write_side::unresolved();
    if missed.is_empty() {
        return;
    }
    if let Err(error) = advance(missed).await {
        tracing::warn!(
            target: "suprnova::render_cache",
            error = %error,
            "render cache could not advance the generations an earlier write missed; \
             stored entries stay unserved until it can",
        );
    }
}

/// CACHE-009: runs a row write together with the generation advancement it
/// triggers as one atomic unit.
///
/// With an ambient transaction the write and its advance already share it,
/// so `write` runs as is. With none, and the write bound for the primary
/// connection with the write side open, one transaction is opened around
/// `write`: the row write inside it routes through `CURRENT_TX`, the
/// advance joins that same transaction through [`advance`]'s first branch,
/// and both commit or roll back together. A write bound for a named
/// connection cannot share a transaction with the ledger, which lives on
/// the primary; it runs as is, and a failed advance then suspends serving
/// (see [`advance`]). The audit of 2026-09-13 (finding ASTRA-10) showed the
/// split commit this closes: a row durable, its advance rolled back, and
/// the old representation still current.
///
/// `write` is a closure returning a future rather than a future, because
/// whether to open a transaction is decided here, before the future is
/// built inside it.
pub(crate) async fn atomic<T, F, Fut>(
    connection: Option<&str>,
    write: F,
) -> Result<T, FrameworkError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, FrameworkError>>,
    T: Send,
{
    let on_primary = connection.is_none_or(|name| name == crate::database::PRIMARY_CONNECTION_NAME);
    let shareable = !in_transaction()
        && on_primary
        && DB::is_connected()
        && super::write_side_open(false).await?;
    if !shareable {
        return write().await;
    }
    let outcome = DB::transaction_ambient(write).await;
    if outcome.is_ok() {
        // The advancement committed with the row it describes. That says
        // nothing about what an earlier, failed advancement missed, so those
        // identities are advanced now rather than forgotten (DATA-029).
        repair_unresolved().await;
    }
    outcome
}

/// The `Table` and `Record` identities a model write advances: every row
/// of the model's table, and the specific row by primary key.
///
/// The record key is built through
/// [`crate::render_cache::collector::record_identity`] - the exact
/// function `observe_record_read_json` uses on the read side - so a
/// write's identity can never drift from what a read observed for the
/// same row. An earlier draft of this function encoded the key by
/// trimming the JSON value's quote characters; that agrees with the read
/// side only for integer keys (no quotes to trim) and silently breaks
/// record-level invalidation for string or UUID keys, whose read-side
/// encoding keeps the quotes. See ruling R45.
fn model_identities<M>(model: &M) -> Result<Vec<DependencyIdentity>, FrameworkError>
where
    M: Model,
    M: From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    row_identities(M::TABLE, &model.primary_key_value_json())
}

/// After a model row was written - `create`, `save`, `update`, `delete`,
/// `force_delete`, and the soft-delete `restore` override: advances the
/// row's table and record generations.
///
/// Checks `super::is_installed()` before `model_identities` runs, not just
/// inside `advance`, so an uninstalled app pays neither SQL nor the primary
/// key's JSON serialization on this path - `advance`'s own check stays as
/// the gate for its other callers, but this is the hottest entry point
/// (every `create`/`save`/`update`/`delete` funnels through it), so the
/// check is duplicated one level up rather than left to run after the work
/// it is meant to skip.
pub async fn after_model_write<M>(model: &M) -> Result<(), FrameworkError>
where
    M: Model,
    M: From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    if !super::write_side_open(in_transaction()).await? {
        return Ok(());
    }
    advance(model_identities(model)?).await
}

/// Explicit-transaction form of [`after_model_write`] for the
/// `Model::*_with_tx` shims (`save_with_tx`, `update_with_tx`,
/// `create_with_tx`, `delete_with_tx`, `force_delete_with_tx`).
///
/// Those shims route their row write through `ExecutorChoice::from_tx(tx)`
/// and bypass the ambient `CURRENT_TX` task-local by design - the explicit
/// handle is authoritative, the same reasoning `touch_owners_with_tx`
/// documents. Calling [`after_model_write`] from inside one of them would
/// find no ambient transaction: `advance` would open a transaction of its
/// own, separate from the caller's `tx`, and a caller that rolls back `tx`
/// would undo the row write while that separately-committed advance
/// stood. Routing through `tx` explicitly instead keeps both in the one
/// transaction the caller controls. See ruling R47.
pub async fn after_model_write_with_tx<M>(tx: &Transaction, model: &M) -> Result<(), FrameworkError>
where
    M: Model,
    M: From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    // Same reasoning as `after_model_write`: check before `model_identities`
    // runs rather than after, since `advance_via_tx` / `advance_via_handle`
    // check `is_installed` only once they are called. `tx` is an explicit
    // transaction handle, so this call already holds the pool connection
    // that handle was granted - pass `true` so `write_side_open` never
    // waits on a second one.
    if !super::write_side_open(true).await? {
        return Ok(());
    }
    super::ledger::advance_via_tx(tx, &model_identities(model)?).await
}

/// The `Table` and `UnkeyedWrite` identities a write that named no rows
/// advances: every row of the table, and the per-table identity every point
/// read observes beside the record it returned.
///
/// The pair is what keeps a point-read entry both narrow and safe: a
/// row-level write advances `Table` and `Record` only, so it cannot reach a
/// point-read entry for another row, while a bulk update, a table-builder
/// write, or a raw statement on the table advances this and reaches every
/// point-read entry the table has.
fn unkeyed_identities(table: &str) -> Result<Vec<DependencyIdentity>, FrameworkError> {
    Ok(vec![
        DependencyIdentity::try_table(table)
            .map_err(|_| FrameworkError::internal("table name out of bounds"))?,
        DependencyIdentity::try_unkeyed_write(table)
            .map_err(|_| FrameworkError::internal("table name out of bounds"))?,
    ])
}

/// After a bulk update or delete (`Builder::update_all` / `delete_all`):
/// the table generation and the table's unkeyed-write identity, because the
/// statement cannot name the rows it changed.
pub async fn after_bulk_write(table: &str) -> Result<(), FrameworkError> {
    advance(unkeyed_identities(table)?).await
}

/// Explicit-transaction-override form of [`after_bulk_write`] for
/// `Builder::with_tx(&tx).update_all(..)` / `.delete_all(..)`.
///
/// `Builder::resolve_write` honours the builder's `tx_override` without
/// installing the ambient `CURRENT_TX` task-local, so `in_transaction()`
/// cannot see it and [`after_bulk_write`] would open a transaction of its
/// own, separate from the caller's `tx` - the identical defect ruling R47
/// fixed for the model `_with_tx` shims. Routing through the explicit
/// handle instead keeps the advance in the same transaction as the bulk
/// row write. See fix1 item 3.
pub async fn after_bulk_write_with_handle(
    handle: &crate::database::transaction::TxHandle,
    table: &str,
) -> Result<(), FrameworkError> {
    super::ledger::advance_via_handle(handle, &unkeyed_identities(table)?).await
}

/// The identities [`after_table_writes_in`] advances: each table and its
/// unkeyed-write identity, once each.
fn table_writes_identities(tables: &[&str]) -> Result<Vec<DependencyIdentity>, FrameworkError> {
    let mut identities = Vec::with_capacity(tables.len() * 2);
    for table in tables {
        for identity in unkeyed_identities(table)? {
            if !identities.contains(&identity) {
                identities.push(identity);
            }
        }
    }
    Ok(identities)
}

/// Whether this process advances generations, decided before a write opens
/// a raw SeaORM transaction for [`after_table_writes_in`] (DATA-039).
///
/// Deciding may probe the schema, and the probe takes a pooled connection
/// of its own. Asked from inside the transaction, it could wait on the
/// connection the transaction already holds, so it is asked first, while
/// the write holds nothing.
pub(crate) async fn advances_generations() -> Result<bool, FrameworkError> {
    super::write_side_open(false).await
}

/// After one operation wrote rows of several tables it cannot name rows
/// in, through `txn`, the raw SeaORM transaction that wrote them: the
/// tables and their unkeyed-write identities, advanced together in one
/// advancement, so the rows and their advance commit or roll back as one
/// unit (DATA-039).
///
/// For a writer that owns a transaction the framework did not open, which
/// `in_transaction()` cannot see: the payments webhook hydration. Advancing
/// after its COMMIT left a window in which a cancellation kept the rows and
/// lost the advance. The caller asks [`advances_generations`] before it
/// opens `txn`, and commits it after this returns, then calls
/// [`repair_unresolved`] the way [`atomic`] does after its own commit.
pub(crate) async fn after_table_writes_in(
    txn: &std::sync::Arc<sea_orm::DatabaseTransaction>,
    tables: &[&str],
) -> Result<(), FrameworkError> {
    super::ledger::advance_in_raw_transaction(txn, &table_writes_identities(tables)?).await
}

/// After a query-builder write on a known table (`DB::table(...).insert` /
/// `.update` / `.delete`), and after a raw statement whose single table the
/// caller named (`DB::affecting_statement_on_table`). Same effect as
/// [`after_bulk_write`] - the table and its unkeyed-write identity - kept as
/// a separate name so each call site reads with its own intent.
pub async fn after_table_write(table: &str) -> Result<(), FrameworkError> {
    after_bulk_write(table).await
}

/// After a feature flag's stored rules changed - `set_flag`, or a name a
/// `reload` found changed: the flag's own generation.
///
/// `pub(crate)`: the only callers are the framework's own feature
/// evaluators, whose reads are the only ones that observe a `Feature`
/// identity, and a generation advanced for a flag nothing observes is a
/// ledger row written for nobody.
pub(crate) async fn after_feature_write(feature: &str) -> Result<(), FrameworkError> {
    advance(vec![DependencyIdentity::try_feature(feature).map_err(
        |_| FrameworkError::internal("feature name out of bounds"),
    )?])
    .await
}

/// After a raw statement whose tables are not known (`DB::statement`):
/// the broad authority every representation observes.
pub async fn after_unknown_write() -> Result<(), FrameworkError> {
    advance(vec![DependencyIdentity::broad()]).await
}

/// The `Table` and `Record` identities for a write to one row of `table`
/// identified by `key`, encoded through
/// [`crate::render_cache::collector::record_identity`] like every other
/// record identity in this module. Shared by [`after_row_write`] and
/// [`after_row_write_with_handle`].
fn row_identities(
    table: &str,
    key: &serde_json::Value,
) -> Result<Vec<DependencyIdentity>, FrameworkError> {
    let mut identities = vec![
        DependencyIdentity::try_table(table)
            .map_err(|_| FrameworkError::internal("table name out of bounds"))?,
    ];
    if let Some(record) = super::collector::record_identity(table, key) {
        identities.push(record);
    }
    Ok(identities)
}

/// After a write to a specific row of a table that is not `Self` - the
/// `#[model(touches = [...])]` parent-touch cascade
/// (`Model::__touch_owners_via`), which `UPDATE`s a named parent table's
/// timestamp column using the child's foreign-key value as the parent's
/// primary key. Unlike [`after_model_write`], which is generic over a
/// `Model`-bound type to reach `M::TABLE` and the row's own primary key,
/// the touch cascade's target is type-erased (reached only through its
/// `RelationEntry`, never hydrated - see `__touch_owners_via`'s own
/// documentation), so there is no `Model` type to be generic over here:
/// the table name and key arrive as plain arguments instead. Advances the
/// parent's table and record generations.
pub async fn after_row_write(table: &str, key: &serde_json::Value) -> Result<(), FrameworkError> {
    advance(row_identities(table, key)?).await
}

/// Explicit-transaction form of [`after_row_write`] for
/// `Model::touch_owners_with_tx`, the only `*_with_tx` function in the
/// framework that executes SQL and, before this fix, advanced nothing -
/// same defect shape as ruling R47, same fix: route through the explicit
/// handle instead of the ambient task-local `touch_owners_with_tx`
/// bypasses by design.
pub async fn after_row_write_with_handle(
    handle: &crate::database::transaction::TxHandle,
    table: &str,
    key: &serde_json::Value,
) -> Result<(), FrameworkError> {
    super::ledger::advance_via_handle(handle, &row_identities(table, key)?).await
}

/// Test-only seam that parks one advancement, so a test can cancel the
/// write that started it at the instant between its row write and its
/// generation advance (DATA-039).
#[cfg(any(test, feature = "testing"))]
pub(crate) mod seams {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use suprnova_live::render_cache::generation::DependencyIdentity;

    /// The table whose next advancement parks, if one is armed.
    static HOLD_NEXT: Mutex<Option<DependencyIdentity>> = Mutex::new(None);
    static HELD: AtomicU64 = AtomicU64::new(0);
    static HELD_NOTIFY: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();

    fn held_notify() -> &'static tokio::sync::Notify {
        HELD_NOTIFY.get_or_init(tokio::sync::Notify::new)
    }

    /// Parks the next advancement that names `table`'s table identity.
    pub(crate) fn hold_next(table: &str) {
        *HOLD_NEXT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(DependencyIdentity::table(table));
    }

    /// How many advancements have parked so far.
    pub(crate) fn held() -> u64 {
        HELD.load(Ordering::SeqCst)
    }

    /// Waits until more than `count` advancements have parked.
    pub(crate) async fn wait_until_held_past(count: u64) {
        loop {
            let notified = held_notify().notified();
            if HELD.load(Ordering::SeqCst) > count {
                return;
            }
            notified.await;
        }
    }

    /// Parks forever when armed for one of `identities`; the test that
    /// armed it cancels the write around it.
    pub(crate) async fn hold_point(identities: &[DependencyIdentity]) {
        let armed = {
            let mut slot = HOLD_NEXT
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let matches = slot
                .as_ref()
                .is_some_and(|table| identities.contains(table));
            if matches {
                slot.take();
            }
            matches
        };
        if armed {
            HELD.fetch_add(1, Ordering::SeqCst);
            held_notify().notify_waiters();
            std::future::pending::<()>().await;
        }
    }
}
