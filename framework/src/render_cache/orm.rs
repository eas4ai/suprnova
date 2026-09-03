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
//! on the soft-delete `restore` override.

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
/// The common case - a write issued inside `DB::transaction`, or one whose
/// caller already opened a transaction for it - takes the first branch:
/// the advance joins that same transaction, so a caller rollback undoes it
/// along with the row write. A write issued with no ambient transaction at
/// all (a bare `model.save()`) still needs its advance to land as one
/// atomic unit across every identity it touches, so the second branch
/// opens a transaction for that alone.
///
/// The second branch requires a primary connection: `DB::transaction`
/// always opens against it, the same as the generation ledger's own reads
/// (`SqlGenerationLedger::current` / `epoch`) are pinned to it rather than
/// any per-model or named connection a write itself might route through.
/// An app that registers only named connections and never calls `DB::init`
/// has no primary pool at all - a supported, tested configuration (see
/// `eloquent_eager_named_connection.rs`) - and a model write on one of
/// those named connections must keep working exactly as it always has.
/// RenderCache being pinned to a primary connection that does not exist
/// here is no different from its schema not being migrated (see the
/// parallel skip in `ledger::advance_through`): both mean there is nothing
/// to advance against, not that the write itself should start failing.
async fn advance(identities: Vec<DependencyIdentity>) -> Result<(), FrameworkError> {
    if in_transaction() {
        return super::ledger::advance_in_current_transaction(&identities).await;
    }
    if !DB::is_connected() {
        return Ok(());
    }
    DB::transaction(move |_tx| {
        Box::pin(async move { super::ledger::advance_in_current_transaction(&identities).await })
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
    let mut identities = vec![
        DependencyIdentity::try_table(M::TABLE)
            .map_err(|_| FrameworkError::internal("table name out of bounds"))?,
    ];
    if let Some(record) =
        super::collector::record_identity(M::TABLE, &model.primary_key_value_json())
    {
        identities.push(record);
    }
    Ok(identities)
}

/// After a model row was written - `create`, `save`, `update`, `delete`,
/// `force_delete`, and the soft-delete `restore` override: advances the
/// row's table and record generations.
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
    super::ledger::advance_via_tx(tx, &model_identities(model)?).await
}

/// After a bulk update or delete (`Builder::update_all` / `delete_all`):
/// the table generation.
pub async fn after_bulk_write(table: &str) -> Result<(), FrameworkError> {
    advance(vec![DependencyIdentity::try_table(table).map_err(
        |_| FrameworkError::internal("table name out of bounds"),
    )?])
    .await
}

/// After a query-builder write on a known table (`DB::table(...).insert` /
/// `.update` / `.delete`). Same effect as [`after_bulk_write`]; kept as a
/// separate name so each call site reads with its own intent.
pub async fn after_table_write(table: &str) -> Result<(), FrameworkError> {
    after_bulk_write(table).await
}

/// After a raw statement whose tables are not known (`DB::statement`):
/// the broad authority every representation observes.
pub async fn after_unknown_write() -> Result<(), FrameworkError> {
    advance(vec![DependencyIdentity::broad()]).await
}
