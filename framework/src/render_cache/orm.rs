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
    if !super::write_side_open().await? {
        return Ok(());
    }
    if in_transaction() {
        return super::ledger::advance_in_current_transaction(&identities).await;
    }
    if !DB::is_connected() {
        return Ok(());
    }
    DB::transaction(move |_tx| {
        Box::pin(async move { super::ledger::advance_in_dedicated_transaction(&identities).await })
    })
    .await
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
    if !super::write_side_open().await? {
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
    // check `is_installed` only once they are called.
    if !super::write_side_open().await? {
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
