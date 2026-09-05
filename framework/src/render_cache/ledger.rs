//! `GenerationLedger` over the application database. Advancement runs on
//! the caller's current transaction so rollback advances nothing.

use std::collections::HashMap;

use async_trait::async_trait;
use sea_orm::{DbBackend, Value};
use suprnova_live::render_cache::generation::{
    DependencyIdentity, GenerationLedger, GenerationSet,
};
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};

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
    message.contains("suprnova_render_epochs")
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
/// `DbErr` string could echo bound values back into a response. It is not
/// thrown away entirely, though: logged first at `warn`, so "no owning
/// transaction" (a programming error at the call site) stays distinguishable
/// from "the database is down" in whatever collects these logs, even though
/// both surface identically to the caller.
fn provider_error(error: FrameworkError) -> RenderCacheError {
    tracing::warn!(
        target: "suprnova::render_cache",
        %error,
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
    if !super::is_installed() || identities.is_empty() {
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
            // See ruling R65: silent when no RenderCache runtime is
            // installed (the ordinary case for every application and test
            // database that does not use RenderCache at all), but a
            // process-lifetime-once warning when one is - a schema that
            // regressed underneath an installed runtime stops advancing
            // generations silently otherwise, and every entry it should
            // have invalidated is served stale forever.
            if super::is_installed() {
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
    if !super::is_installed() {
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
        let mut set = GenerationSet::default();
        if dependencies.is_empty() {
            return Ok(set);
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

        let mut found: HashMap<String, u64> = HashMap::new();
        for row in rows {
            let identity: String = row
                .try_get_by_index(0)
                .map_err(|e| provider_error(database_error(e)))?;
            let generation: i64 = row
                .try_get_by_index(1)
                .map_err(|e| provider_error(database_error(e)))?;
            found.insert(identity, generation as u64);
        }

        // Every requested digest gets an entry, present in the table or
        // not: an unobserved digest is 0 by contract (see
        // `GenerationLedger::current`'s doc), and `CoherenceCheck::compare`
        // reads a decoded entry's observations back through
        // `GenerationSet::get_digest`, which returns `None` for a digest
        // this set never recorded. Only inserting what the query returned
        // would leave an untouched dependency absent instead of `Some(0)`;
        // the observed side (built the same way, at write time) would
        // still hold `Some(0)` for it, the two would compare unequal, and
        // every entry that ever observed an untouched dependency would be
        // reported moved on every request, forever.
        for dependency in dependencies {
            let generation = found.get(&hex::encode(dependency)).copied().unwrap_or(0);
            set.insert_digest(*dependency, generation)?;
        }
        Ok(set)
    }

    async fn advance(&self, identities: &[DependencyIdentity]) -> Result<(), RenderCacheError> {
        advance_in_current_transaction(identities)
            .await
            .map_err(provider_error)
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
}
