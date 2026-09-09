//! `GenerationLedger` over the application database. Advancement runs on
//! the caller's current transaction so rollback advances nothing.

use std::collections::HashMap;

use async_trait::async_trait;
use sea_orm::{DbBackend, Value};
use suprnova_live::render_cache::generation::{
    DependencyIdentity, GenerationLedger, GenerationSet,
};
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};

use super::providers::framework_error_kind;
use crate::database::transaction::{ExecutorChoice, TxHandle};
use crate::{DB, FrameworkError, PRIMARY_CONNECTION_NAME, Transaction};

/// The executor every read in this module goes through: the ambient
/// `CURRENT_TX` when one is open (a render's own transaction, so the
/// window-close read shares its snapshot; a caller's write transaction, so a
/// probe lands on its connection), and the primary pool otherwise, never a
/// read replica.
///
/// Resolved directly rather than through `DB::select_on` (final review, F2 /
/// ruling R118): the raw `DB::select` family now marks an active collector
/// report incomplete, because a raw statement's tables cannot be recorded,
/// and the ledger's own reads run inside every render's collector scope.
/// Routed through the facade, every render would decline itself. The same
/// primary pin as before: `resolve_read` still yields to the ambient
/// transaction first and then honours the explicit primary override without
/// ever consulting `__read_replica__`, which a database-authoritative ledger
/// must never read (a lagging follower would report an entry fresh that the
/// primary already moved past).
async fn primary_executor() -> Result<ExecutorChoice, FrameworkError> {
    ExecutorChoice::resolve_read(None, Some(PRIMARY_CONNECTION_NAME), None).await
}

/// Wraps a driver error into the framework's database error, the way every
/// `exec` call in this module reports one.
fn database_error(error: sea_orm::DbErr) -> FrameworkError {
    FrameworkError::database(error.to_string())
}

/// Hex-encodes an identity's digest for the `identity` column. Every value
/// that crosses the database boundary as an identity goes through this, so
/// the encoding never drifts between the write path (which knows
/// identities) and a raw digest read (which uses `hex::encode` directly on
/// the digest bytes - the same encoding, since `identity.digest()` is what
/// this hashes).
fn identity_column(identity: &DependencyIdentity) -> String {
    hex::encode(identity.digest())
}

/// True when `error` looks like "the `suprnova_render_epochs` table does
/// not exist" rather than any other database failure.
///
/// SeaORM does not expose a typed "table missing" variant - every driver
/// surfaces it as an opaque `DbErr::Query` / `DbErr::Exec` wrapping a
/// backend-specific message. Requires BOTH the table's own name and one of
/// the phrasings each backend is known to use for a missing table:
/// SQLite's `no such table: suprnova_render_epochs`, Postgres's `relation
/// "suprnova_render_epochs" does not exist`, and MySQL's `Table
/// '...suprnova_render_epochs' doesn't exist`.
///
/// The table name is required, not optional: `does not exist` alone is far
/// broader than "this table is missing". It also matches a half-applied
/// migration (`column "epoch" does not exist`, where invalidation should
/// fail loudly, not skip silently), PgBouncer's transaction-pooling
/// failure (`prepared statement "sqlx_s_3" does not exist`, a transient
/// infrastructure fault, not a permanent "not installed"), a wrong
/// `search_path` in a schema-per-tenant deployment naming some other
/// relation, and MySQL's 1305 `SAVEPOINT ... does not exist`. None of
/// those name `suprnova_render_epochs`, so requiring it excludes all of
/// them while still matching every backend's real missing-table message.
/// The same string-matching technique (minus this table-name requirement)
/// is used elsewhere in this codebase (`vector/qdrant.rs`) for the same
/// class of "the resource I expect isn't there" signal.
///
/// Takes the already-stringified message rather than a `&sea_orm::DbErr`
/// so [`migration_present`] can reuse the exact same check against a
/// [`FrameworkError`](crate::FrameworkError)'s text (its `Display` passes
/// the underlying database message through verbatim - see
/// [`FrameworkError::database`](crate::FrameworkError::database)) without a
/// second, drifting copy of these three phrasings.
fn is_missing_table_error(message: &str) -> bool {
    is_missing_table_error_for(message, "suprnova_render_epochs")
}

/// The same check for any one of this module's tables, so
/// [`tier_migration_present`] can probe its own table without a second copy
/// of the three backend phrasings. The table name stays required for
/// exactly the reasons [`is_missing_table_error`] documents.
fn is_missing_table_error_for(message: &str, table: &str) -> bool {
    message.contains(table)
        && (message.contains("no such table")
            || message.contains("does not exist")
            || message.contains("doesn't exist"))
}

/// Once-per-process warning that a write path skipped advancing a
/// generation because `suprnova_render_epochs` is missing, even though a
/// RenderCache runtime is installed for this process. See ruling R65: a
/// missing table on a process that never called `RenderCache::install` is
/// silent by design (`MissingTablePolicy::Skip`'s ordinary case, matching
/// every uninstalled application and test database); a missing table
/// after `RenderCache::install` succeeded is a schema regression - a bad
/// deploy, a dropped table - that would otherwise stop advancing
/// generations, and therefore stop invalidating anything, silently
/// forever.
static WARNED_MISSING_TABLE_AFTER_INSTALL: std::sync::Once = std::sync::Once::new();

/// Collapses a database failure into the one closed provider kind
/// [`RenderCacheError`] exposes for this contract. The underlying message
/// is dropped from the returned error deliberately: `RenderCacheError`'s
/// messages never carry keys, bodies, or identity material, and a raw
/// `DbErr` string could echo bound values back into a response.
///
/// It does not reach the log either. What is logged is the failure's own
/// variant name through [`framework_error_kind`], so "no owning
/// transaction" (`kind="internal"`, a programming error at the call site)
/// stays distinguishable from "the database is down" (`kind="database"`) in
/// whatever collects these logs, and no bound value travels with it: the
/// statements this module runs bind hex dependency identities and epoch
/// numbers, and `FrameworkError::Database`'s message is the driver's own
/// string, which repeats them. The providers module makes the same collapse
/// for the tier adapters.
fn provider_error(error: FrameworkError) -> RenderCacheError {
    tracing::warn!(
        target: "suprnova::render_cache",
        kind = framework_error_kind(&error),
        "render cache generation ledger provider failure",
    );
    RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable)
}

/// Whether the RenderCache migration's tables are present on the primary
/// connection.
///
/// `RenderCache::install` calls this at boot (ruling R58): the migration
/// is deliberately not part of the CLI scaffold template (the same
/// opt-in shape as `two_factor` and `features`), so an application that
/// enables RenderCache without adding it would otherwise install
/// successfully and fail every request against a missing table - a much
/// worse first experience than one sentence at boot naming the fix.
///
/// Any database error other than a missing `suprnova_render_epochs`
/// propagates unchanged (for example, no primary connection registered at
/// all): `install` should fail loudly on that too, not report it as "the
/// migration is missing".
pub(crate) async fn migration_present() -> Result<bool, FrameworkError> {
    let exec = primary_executor().await?;
    let statement = sea_orm::Statement::from_sql_and_values(
        exec.backend(),
        "SELECT epoch FROM suprnova_render_epochs WHERE singleton = 1",
        vec![],
    );
    match exec.query_one(statement).await {
        Ok(_) => Ok(true),
        Err(e) if is_missing_table_error(&e.to_string()) => Ok(false),
        Err(e) => Err(database_error(e)),
    }
}

/// [`migration_present`] read on a pooled primary connection rather than
/// through the ambient executor.
///
/// The write side's probe must never run inside a caller's transaction. On
/// PostgreSQL a failed statement poisons the enclosing transaction, and
/// `COMMIT` on a poisoned transaction returns the ROLLBACK tag without
/// raising, so probing a missing table from inside a caller's transaction
/// would discard that caller's own write while reporting success.
/// `primary_executor` honours an ambient transaction by design, which is
/// right for a generation read and wrong here, so this takes the primary
/// pool directly - a connection no caller transaction owns.
///
/// # Errors
///
/// Returns the database error when the probe fails for any reason other
/// than the table being absent, and when no primary connection exists.
pub(crate) async fn migration_present_off_transaction() -> Result<bool, FrameworkError> {
    use sea_orm::ConnectionTrait as _;

    let connection = DB::get()?;
    let backend = connection.inner().get_database_backend();
    let statement = sea_orm::Statement::from_sql_and_values(
        backend,
        "SELECT epoch FROM suprnova_render_epochs WHERE singleton = 1",
        vec![],
    );
    match connection.inner().query_one_raw(statement).await {
        Ok(_) => Ok(true),
        Err(e) if is_missing_table_error(&e.to_string()) => Ok(false),
        Err(e) => Err(database_error(e)),
    }
}

/// Whether the tier migration's tables are present on the primary
/// connection.
///
/// The Tier 1 and Tier 2 profiles need the four tables
/// [`migration::TierMigration`](super::migration::TierMigration) creates,
/// and that migration is opt-in for the same reason
/// [`migration::Migration`](super::migration::Migration) is: an application
/// on the embedded profile should carry neither. Install probes this before
/// building a database-backed provider, so a profile whose schema is not
/// there fails at boot with one actionable sentence rather than on every
/// request.
///
/// Probes `suprnova_render_entries`, the first table that migration
/// creates. Any database error other than that table being missing
/// propagates unchanged, exactly as in this module's own
/// `migration_present` (crate-private, so this is a plain code span rather
/// than a link): "the migration is missing" is a specific answer, not a
/// catch-all for a database that cannot be reached at all.
///
/// # Errors
///
/// Returns the database error when the probe fails for any reason other
/// than the table being absent.
pub async fn tier_migration_present() -> Result<bool, FrameworkError> {
    let exec = primary_executor().await?;
    let statement = sea_orm::Statement::from_sql_and_values(
        exec.backend(),
        "SELECT render_key FROM suprnova_render_entries WHERE render_key IS NULL",
        vec![],
    );
    match exec.query_one(statement).await {
        Ok(_) => Ok(true),
        Err(e) if is_missing_table_error_for(&e.to_string(), "suprnova_render_entries") => {
            Ok(false)
        }
        Err(e) => Err(database_error(e)),
    }
}

/// One `?` (MySQL/SQLite) or `$N` (Postgres) placeholder per position,
/// joined with `, ` in order, matching the active backend's bind syntax.
///
/// `DbBackend` is `#[non_exhaustive]`, so an unrecognised future variant is
/// refused explicitly rather than silently guessing a bind syntax it was
/// never proven against.
fn placeholders(backend: DbBackend, count: usize) -> Result<String, FrameworkError> {
    match backend {
        DbBackend::Postgres => Ok((1..=count)
            .map(|index| format!("${index}"))
            .collect::<Vec<_>>()
            .join(", ")),
        DbBackend::MySql | DbBackend::Sqlite => Ok(std::iter::repeat_n("?", count)
            .collect::<Vec<_>>()
            .join(", ")),
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

/// Turns `(identity, generation)` rows into the zero-filled
/// [`GenerationSet`] [`GenerationLedger::current`] promises, plus the epoch
/// carried by the one row whose identity is empty.
///
/// Both reads in this module fetch the same two columns and fill the same
/// set from them; they differ only in whether the statement also selects the
/// authority epoch. `current`'s statement never can, so it always gets
/// `None` back and ignores it, while `current_with_epoch`'s union always
/// does. An empty identity can only be that extra row: every real value in
/// the column is the 64 hex characters `hex::encode` produces from a 32-byte
/// digest.
///
/// Every requested digest gets an entry, present in the table or not: an
/// unobserved digest is 0 by contract (see [`GenerationLedger::current`]'s
/// doc), and
/// [`CoherenceCheck::compare`](suprnova_live::render_cache::CoherenceCheck::compare)
/// reads a decoded entry's observations back through
/// `GenerationSet::get_digest`, which returns `None` for a digest this set
/// never recorded. Only inserting what the query returned would leave an
/// untouched dependency absent instead of `Some(0)`; the observed side
/// (built the same way, at write time) would still hold `Some(0)` for it,
/// the two would compare unequal, and every entry that ever observed an
/// untouched dependency would be reported moved on every request, forever.
fn zero_filled_set(
    dependencies: &[[u8; 32]],
    rows: Vec<sea_orm::QueryResult>,
) -> Result<(GenerationSet, Option<u64>), RenderCacheError> {
    let mut found: HashMap<String, u64> = HashMap::new();
    let mut epoch: Option<u64> = None;
    for row in rows {
        let identity: String = row
            .try_get_by_index(0)
            .map_err(|e| provider_error(database_error(e)))?;
        let generation: i64 = row
            .try_get_by_index(1)
            .map_err(|e| provider_error(database_error(e)))?;
        if identity.is_empty() {
            epoch = Some(generation as u64);
        } else {
            found.insert(identity, generation as u64);
        }
    }
    let mut set = GenerationSet::default();
    for dependency in dependencies {
        let generation = found.get(&hex::encode(dependency)).copied().unwrap_or(0);
        set.insert_digest(*dependency, generation)?;
    }
    Ok((set, epoch))
}

/// The per-backend upsert that creates a dependency's row at generation 1
/// or advances an existing row by one.
///
/// `epoch` is written only on the initial insert, not on the conflict
/// branch: it is per-row provenance ("which authority epoch first observed
/// this dependency"), and nothing in this crate reads it back for a
/// freshness decision - that authority is `suprnova_render_epochs` alone,
/// consulted through [`GenerationLedger::epoch`] and folded into
/// [`suprnova_live::render_cache::CoherenceCheck::compare`] as the digest of
/// [`DependencyIdentity::Broad`], never through this column.
fn upsert_sql(backend: DbBackend) -> Result<&'static str, FrameworkError> {
    match backend {
        DbBackend::MySql => Ok(
            "INSERT INTO suprnova_render_generations (identity, generation, epoch, updated_at) \
             VALUES (?, 1, ?, CURRENT_TIMESTAMP) \
             ON DUPLICATE KEY UPDATE generation = generation + 1, updated_at = CURRENT_TIMESTAMP",
        ),
        DbBackend::Postgres => Ok(
            "INSERT INTO suprnova_render_generations (identity, generation, epoch, updated_at) \
             VALUES ($1, 1, $2, CURRENT_TIMESTAMP) \
             ON CONFLICT (identity) DO UPDATE SET \
             generation = suprnova_render_generations.generation + 1, \
             updated_at = CURRENT_TIMESTAMP",
        ),
        DbBackend::Sqlite => Ok(
            "INSERT INTO suprnova_render_generations (identity, generation, epoch, updated_at) \
             VALUES (?, 1, ?, CURRENT_TIMESTAMP) \
             ON CONFLICT (identity) DO UPDATE SET generation = generation + 1, \
             updated_at = CURRENT_TIMESTAMP",
        ),
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

/// Advances each identity by one inside the caller's `DB::transaction`
/// scope and appends one append-only row per identity to the change log.
///
/// An empty slice is a no-op that touches neither the transaction
/// requirement nor the database, matching [`GenerationLedger::current`]'s
/// own empty-input short circuit: a caller with nothing to advance should
/// not be forced to hold an open transaction just to ask for one.
///
/// Otherwise fails outright when no transaction is active. A generation
/// advance that is not part of the same transaction as the data change it
/// represents could commit after a rollback undoes that change, or be lost
/// after the change survives - either way the cached entries it is meant to
/// fence would disagree with the database they claim to track. Every read
/// this function issues (the current epoch, and the post-upsert generation)
/// goes through the `DB` facade rather than the transaction handle
/// directly, but still lands on the same connection: the facade's
/// read/write resolution consults the ambient `CURRENT_TX` task-local
/// before falling back to the pool, and this function has already
/// confirmed that task-local is set.
///
/// Identities are locked in ascending digest order, never in the order the
/// caller supplied. Two overlapping transactions advancing the same two
/// identities in opposite orders - which happens routinely, since the
/// collector reports identities in first-seen order and two requests can
/// observe them in either order - would otherwise each hold one row and
/// wait on the other's, and the database's deadlock detector kills one of
/// them (`DB::transaction` does not retry). Sorting first makes every
/// concurrent caller acquire locks in the same global order, so that
/// circular wait can never form. Proven live against Postgres: see
/// `live_postgres_concurrent_advances_in_opposite_order_do_not_deadlock` in
/// `framework/tests/render_cache_ledger.rs`.
///
/// Returns `Ok(())` immediately, issuing no SQL at all, when no RenderCache
/// runtime has been installed for this process (`super::is_installed`) -
/// see the module-level flag's own documentation for why probing is
/// unconditionally unsafe rather than merely unnecessary.
///
/// A missing-table failure on the probe below always propagates here
/// rather than being swallowed: `Transaction::current()` is the ambient
/// transaction the *caller* opened with `DB::transaction`, which may hold
/// other writes. On Postgres a failed statement poisons the whole
/// transaction, and `COMMIT` on a poisoned transaction returns the
/// ROLLBACK tag without raising - so swallowing here would let the
/// caller's later `commit()` report success while silently discarding
/// everything it wrote. See fix1 item 1. The no-ambient-transaction
/// fallback (`orm::advance` opening a transaction solely to hold this
/// advance, with nothing else riding on it) uses this crate's own
/// `advance_in_dedicated_transaction` instead, which is safe to swallow
/// in.
pub async fn advance_in_current_transaction(
    identities: &[DependencyIdentity],
) -> Result<(), FrameworkError> {
    if !super::write_side_open().await? || identities.is_empty() {
        return Ok(());
    }
    let tx = Transaction::current().ok_or_else(|| {
        FrameworkError::internal("RenderCache generation advance requires the owning transaction")
    })?;
    advance_through(
        &ExecutorChoice::from_tx(&tx),
        identities,
        MissingTablePolicy::Propagate,
    )
    .await
}

/// Advances `identities` inside a transaction that `orm::advance`'s
/// no-ambient-transaction fallback opened solely to hold this advance -
/// nothing else rides on it, so a missing-table failure is safe to treat
/// as "RenderCache is not installed against this specific database" and
/// skip, the same reasoning [`advance_in_current_transaction`] documents
/// for why it must NOT do the same. `pub(crate)`: this is `orm::advance`'s
/// own implementation detail, not a second public entry point.
pub(crate) async fn advance_in_dedicated_transaction(
    identities: &[DependencyIdentity],
) -> Result<(), FrameworkError> {
    if identities.is_empty() {
        return Ok(());
    }
    let tx = Transaction::current().ok_or_else(|| {
        FrameworkError::internal("RenderCache generation advance requires the owning transaction")
    })?;
    advance_through(
        &ExecutorChoice::from_tx(&tx),
        identities,
        MissingTablePolicy::Skip,
    )
    .await
}

/// Whether [`advance_through`] propagates or swallows a missing-table
/// failure on its epoch probe. See [`advance_in_current_transaction`] and
/// [`advance_in_dedicated_transaction`] for which case is which and why.
#[derive(Clone, Copy)]
enum MissingTablePolicy {
    /// The ambient transaction is the caller's own; swallowing would risk
    /// converting a poisoned transaction into a silent rollback reported
    /// as success.
    Propagate,
    /// The transaction holds nothing but this advance; a missing table
    /// means RenderCache is not installed here, and there is nothing else
    /// in the transaction that swallowing the error could put at risk.
    Skip,
}

/// The upsert-and-log body shared by [`advance_in_current_transaction`],
/// [`advance_in_dedicated_transaction`], and [`advance_via_handle`] (the
/// explicit-transaction form the `Model::*_with_tx` shims and
/// `Builder::with_tx` bulk writes need - see rulings R47 and fix1 item 3):
/// each identity's generation row upserted and its change-log row
/// appended, in ascending digest order.
///
/// Issues every statement directly through `exec` - never through the
/// `DB` facade's `DB::statement` / `DB::scalar` - because `DB::statement`
/// itself calls [`super::orm::after_unknown_write`] for any non-`SELECT`
/// statement, and this function's own upsert and log-insert statements
/// are exactly that shape. Routing them back through `DB::statement`
/// would recurse: this advance would trigger another broad-authority
/// advance, which would trigger another, without ever returning.
async fn advance_through(
    exec: &ExecutorChoice,
    identities: &[DependencyIdentity],
    on_missing_table: MissingTablePolicy,
) -> Result<(), FrameworkError> {
    if identities.is_empty() {
        return Ok(());
    }
    let backend = exec.backend();
    let epoch_row = match exec
        .query_one(sea_orm::Statement::from_sql_and_values(
            backend,
            "SELECT epoch FROM suprnova_render_epochs WHERE singleton = 1",
            vec![],
        ))
        .await
    {
        Ok(row) => row,
        Err(e)
            if matches!(on_missing_table, MissingTablePolicy::Skip)
                && is_missing_table_error(&e.to_string()) =>
        {
            // A table that disappeared after the write side said it was
            // present is the same schema regression in a worker as in the
            // server (ruling R65, re-keyed for iteration 006): both stop
            // advancing generations, and every entry that depended on the
            // tables the write touched is served without invalidation until
            // the migration is applied.
            if super::write_side::decision() == super::write_side::WriteSideDecision::Open {
                WARNED_MISSING_TABLE_AFTER_INSTALL.call_once(|| {
                    tracing::warn!(
                        target: "suprnova::render_cache",
                        "a write skipped advancing a RenderCache generation because \
                         suprnova_render_epochs is missing, even though a RenderCache \
                         runtime is installed for this process; every entry that \
                         depends on the tables this write touched will keep being \
                         served without invalidation until the RenderCache migration \
                         is applied",
                    );
                });
            }
            return Ok(());
        }
        Err(e) => return Err(FrameworkError::database(e.to_string())),
    }
    .ok_or_else(|| {
        FrameworkError::internal(
            "RenderCache generation advance: suprnova_render_epochs has no singleton row",
        )
    })?;
    let epoch: i64 = epoch_row
        .try_get_by_index(0)
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    let mut ordered: Vec<&DependencyIdentity> = identities.iter().collect();
    ordered.sort_by_key(|identity| identity.digest());

    for identity in ordered {
        let digest = identity_column(identity);
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            upsert_sql(backend)?,
            vec![Value::from(digest.clone()), Value::from(epoch)],
        ))
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

        let generation_row = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                format!(
                    "SELECT generation FROM suprnova_render_generations WHERE identity = {}",
                    placeholders(backend, 1)?
                ),
                vec![Value::from(digest.clone())],
            ))
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?
            .ok_or_else(|| {
                FrameworkError::internal(
                    "RenderCache generation advance: row disappeared immediately after upsert",
                )
            })?;
        let generation: i64 = generation_row
            .try_get_by_index(0)
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            format!(
                "INSERT INTO suprnova_render_generation_log \
                 (identity, generation, epoch, committed_at) VALUES ({}, CURRENT_TIMESTAMP)",
                placeholders(backend, 3)?
            ),
            vec![
                Value::from(digest),
                Value::from(generation),
                Value::from(epoch),
            ],
        ))
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;
    }
    Ok(())
}

/// Advances `identities` through an explicit transaction instead of the
/// ambient `CURRENT_TX` task-local `advance_in_current_transaction`
/// consults.
///
/// The `Model::*_with_tx` shims (`save_with_tx`, `update_with_tx`,
/// `create_with_tx`, `delete_with_tx`, `force_delete_with_tx`) route their
/// row write through `ExecutorChoice::from_tx(tx)` and bypass `CURRENT_TX`
/// by design - the explicit handle is authoritative, not the task-local.
/// Calling [`advance_in_current_transaction`] from inside one of them
/// would find no ambient transaction: it would either fail outright, or -
/// worse - open a transaction of its own that commits independently of
/// the caller's `tx`, so a caller that rolls back `tx` would undo the row
/// write while that separately-committed advance stood. This function
/// closes that gap by taking the transaction explicitly. See ruling R47.
///
/// Delegates to [`advance_via_handle`], which is what actually issues SQL;
/// see it for the install gate and the missing-table propagation rule
/// (same as [`advance_in_current_transaction`]: this is always the
/// caller's own transaction, never a throwaway one, so a missing-table
/// failure always propagates).
pub async fn advance_via_tx(
    tx: &Transaction,
    identities: &[DependencyIdentity],
) -> Result<(), FrameworkError> {
    advance_via_handle(&tx.handle(), identities).await
}

/// Advances `identities` through an explicit query-builder transaction
/// override (`Builder::with_tx(&tx)`).
///
/// `Builder::resolve_write` honours `Builder::tx_override` (set by
/// `with_tx`) without installing the ambient `CURRENT_TX` task-local -
/// `in_transaction()` cannot see it, so `M::query().with_tx(&tx).update_all(..)`
/// would otherwise land its row write on the caller's explicit transaction
/// while the advance opened a separate one: the advance could then commit
/// independently of a bulk write that later rolls back, and it takes a
/// second pooled connection while the caller holds the only one on a
/// single-connection test database. The identical defect ruling R47 fixed
/// for the model `_with_tx` shims; see fix1 item 3.
///
/// Also the function [`advance_via_tx`] delegates to, since a
/// [`Transaction`] converts cheaply to a [`TxHandle`] via
/// [`Transaction::handle`] and both cases need identical treatment: gate
/// on this crate's `render_cache::is_installed`, then never swallow a
/// missing-table failure (this is always the caller's own transaction).
pub async fn advance_via_handle(
    handle: &TxHandle,
    identities: &[DependencyIdentity],
) -> Result<(), FrameworkError> {
    if !super::write_side_open().await? {
        return Ok(());
    }
    advance_through(
        &ExecutorChoice::from_handle(handle),
        identities,
        MissingTablePolicy::Propagate,
    )
    .await
}

/// The application-database generation authority: a [`GenerationLedger`]
/// over the tables that [`super::migration::Migration`] creates.
#[derive(Clone, Copy, Debug, Default)]
pub struct SqlGenerationLedger;

impl SqlGenerationLedger {
    /// A ledger over the default connection.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Emergency authority epoch advance. Every entry observed at the prior
    /// epoch becomes unreachable at its next freshness check, since
    /// [`suprnova_live::render_cache::CoherenceCheck::compare`] treats any
    /// epoch change as every observed dependency having moved.
    ///
    /// Errors when no row was updated. `DB::statement` alone would report
    /// only that the driver accepted the statement, which is `true` for an
    /// `UPDATE` that matched zero rows just as much as one that matched the
    /// singleton - an operator invoking this as the emergency lever would
    /// see `Ok(())` and believe the cache had been invalidated when nothing
    /// had changed. `DB::affecting_statement` reports the row count, so a
    /// missing singleton (an unapplied migration) fails loudly instead.
    pub async fn advance_epoch(&self) -> Result<(), RenderCacheError> {
        let rows_affected = DB::affecting_statement(
            "UPDATE suprnova_render_epochs SET epoch = epoch + 1 WHERE singleton = 1",
            vec![],
        )
        .await
        .map_err(provider_error)?;
        if rows_affected == 0 {
            tracing::warn!(
                target: "suprnova::render_cache",
                "advance_epoch updated no row; suprnova_render_epochs is missing its \
                 singleton, most likely because the render cache migration has not run",
            );
            return Err(RenderCacheError::new(
                RenderCacheErrorKind::ProviderUnavailable,
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl GenerationLedger for SqlGenerationLedger {
    async fn current(&self, dependencies: &[[u8; 32]]) -> Result<GenerationSet, RenderCacheError> {
        if dependencies.is_empty() {
            return Ok(GenerationSet::default());
        }

        // Pinned to the primary through `primary_executor` (see its doc):
        // inside a transaction the transaction's own connection wins,
        // otherwise the primary pool, never a replica.
        let exec = primary_executor().await.map_err(provider_error)?;
        let backend = exec.backend();
        let digests: Vec<String> = dependencies.iter().map(hex::encode).collect();
        let sql = format!(
            "SELECT identity, generation FROM suprnova_render_generations WHERE identity IN ({})",
            placeholders(backend, digests.len()).map_err(provider_error)?
        );
        let values: Vec<Value> = digests.into_iter().map(Value::from).collect();
        let rows = exec
            .query_all(sea_orm::Statement::from_sql_and_values(
                backend, &sql, values,
            ))
            .await
            .map_err(|e| provider_error(database_error(e)))?;

        // This statement selects no epoch row, so the second half is always
        // `None` here; see `zero_filled_set`.
        let (found, _) = zero_filled_set(dependencies, rows)?;
        Ok(found)
    }

    async fn advance(&self, identities: &[DependencyIdentity]) -> Result<(), RenderCacheError> {
        advance_in_current_transaction(identities)
            .await
            .map_err(provider_error)
    }

    async fn current_with_epoch(
        &self,
        dependencies: &[[u8; 32]],
    ) -> Result<(GenerationSet, u64), RenderCacheError> {
        // No `IN` list to bind and nothing to zero-fill: the epoch is the
        // whole answer, so it is read the way `epoch` reads it rather than
        // through a one-branch union.
        if dependencies.is_empty() {
            return Ok((GenerationSet::default(), self.epoch().await?));
        }

        // The same primary pin `current` and `epoch` take, for the same two
        // reasons: a render's own transaction wins so the window-close read
        // shares its snapshot, and a lagging replica must never answer for
        // the authority `advance_epoch` writes to.
        let exec = primary_executor().await.map_err(provider_error)?;
        let backend = exec.backend();
        let digests: Vec<String> = dependencies.iter().map(hex::encode).collect();
        // One statement on all three dialects. The epoch arrives as an extra
        // row under the empty identity, which no dependency can collide with:
        // every real identity column value is the 64 hex characters
        // `hex::encode` produces from a 32-byte digest.
        let sql = format!(
            "SELECT identity, generation FROM suprnova_render_generations \
             WHERE identity IN ({}) \
             UNION ALL \
             SELECT '' AS identity, epoch AS generation FROM suprnova_render_epochs \
             WHERE singleton = 1",
            placeholders(backend, digests.len()).map_err(provider_error)?
        );
        let values: Vec<Value> = digests.into_iter().map(Value::from).collect();
        let rows = exec
            .query_all(sea_orm::Statement::from_sql_and_values(
                backend, &sql, values,
            ))
            .await
            .map_err(|e| provider_error(database_error(e)))?;

        let (set, epoch) = zero_filled_set(dependencies, rows)?;

        // A missing epoch row is the same failure `epoch` raises for it, for
        // the same reason: without the singleton there is no authority to
        // compare an entry against, and guessing one would report every
        // stored entry coherent.
        let Some(epoch) = epoch else {
            tracing::warn!(
                target: "suprnova::render_cache",
                "current_with_epoch: suprnova_render_epochs has no singleton row",
            );
            return Err(RenderCacheError::new(
                RenderCacheErrorKind::ProviderUnavailable,
            ));
        };
        Ok((set, epoch))
    }

    async fn epoch(&self) -> Result<u64, RenderCacheError> {
        // Same primary pin as `current`, and it matters even more here:
        // `advance_epoch` is the emergency invalidation lever, its `UPDATE`
        // always lands on the primary, and if this read came from a lagging
        // replica the lever would appear to do nothing for the length of
        // replication lag.
        let exec = primary_executor().await.map_err(provider_error)?;
        let row = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                exec.backend(),
                "SELECT epoch FROM suprnova_render_epochs WHERE singleton = 1",
                vec![],
            ))
            .await
            .map_err(|e| provider_error(database_error(e)))?
            .ok_or_else(|| {
                tracing::warn!(
                    target: "suprnova::render_cache",
                    "epoch: suprnova_render_epochs has no singleton row",
                );
                RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable)
            })?;
        let epoch: i64 = row
            .try_get_by_index(0)
            .map_err(|e| provider_error(database_error(e)))?;
        Ok(epoch as u64)
    }

    async fn lift_epoch_above(&self, stamped: u64) -> Result<u64, RenderCacheError> {
        let bound = i64::try_from(stamped)
            .map_err(|_| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        let lifted = bound
            .checked_add(1)
            .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        // The same primary pin `epoch` and `advance_epoch` take: the lift is
        // the recovery lever, and a lagging replica must never answer for it.
        let exec = primary_executor().await.map_err(provider_error)?;
        let backend = exec.backend();
        let binds = placeholders(backend, 2).map_err(provider_error)?;
        let (set, guard) = binds
            .split_once(", ")
            .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
        let sql = format!(
            "UPDATE suprnova_render_epochs SET epoch = {set} \
             WHERE singleton = 1 AND epoch <= {guard}"
        );
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            &sql,
            vec![Value::from(lifted), Value::from(bound)],
        ))
        .await
        .map_err(|e| provider_error(database_error(e)))?;
        // Read back the way `epoch` reads it, so a missing singleton fails
        // the way `advance_epoch` fails rather than reporting a lift that
        // never landed. A zero row count is not a failure: it is the
        // ordinary answer when another node already lifted past `stamped`.
        self.epoch().await
    }
}

#[cfg(test)]
mod tests {
    //! What a generation ledger failure is allowed to say.
    //!
    //! The behaviour of this module is proven against a real database in
    //! `framework/tests/render_cache/ledger.rs`; what a live test cannot
    //! show is what reaches the log on the way past, which is where a bound
    //! value would otherwise escape.
    use super::*;

    #[tracing_test::traced_test]
    #[test]
    fn a_ledger_provider_failure_logs_its_kind_and_never_the_driver_message() {
        // What a failing statement in this module actually carries back: the
        // driver repeats the statement and its bound values, which here are
        // hex dependency identities.
        let error = database_error(sea_orm::DbErr::Query(sea_orm::RuntimeErr::Internal(
            "error returned from database: INSERT INTO suprnova_render_generations \
             VALUES ('0123456789abcdef')"
                .to_owned(),
        )));
        // The message is the leak this guards against, so prove it is really
        // in the error before proving it is not in what is logged.
        let message = error.to_string();
        assert!(message.contains("0123456789abcdef"), "{message}");

        assert_eq!(
            provider_error(error).kind(),
            RenderCacheErrorKind::ProviderUnavailable,
            "every database failure is one closed kind to the caller"
        );

        assert!(
            logs_contain("render cache generation ledger provider failure"),
            "the failure is logged"
        );
        assert!(logs_contain("kind=\"database\""), "as a closed-set kind");
        assert!(
            !logs_contain("0123456789abcdef"),
            "and never as the driver's message"
        );
        assert!(!logs_contain("suprnova_render_generations"));

        // The distinction the log exists for: a call with no owning
        // transaction is a programming error at the call site, not an
        // outage.
        assert_eq!(
            provider_error(FrameworkError::internal("no owning transaction".to_owned())).kind(),
            RenderCacheErrorKind::ProviderUnavailable
        );
        assert!(logs_contain("kind=\"internal\""));
    }
}
