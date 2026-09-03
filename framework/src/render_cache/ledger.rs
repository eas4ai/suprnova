//! `GenerationLedger` over the application database. Advancement runs on
//! the caller's current transaction so rollback advances nothing.

use std::collections::HashMap;

use async_trait::async_trait;
use sea_orm::{DbBackend, Value};
use suprnova_live::render_cache::generation::{
    DependencyIdentity, GenerationLedger, GenerationSet,
};
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};

use crate::database::transaction::ExecutorChoice;
use crate::{DB, FrameworkError, PRIMARY_CONNECTION_NAME, Transaction};

/// Hex-encodes an identity's digest for the `identity` column. Every value
/// that crosses the database boundary as an identity goes through this, so
/// the encoding never drifts between the write path (which knows
/// identities) and a raw digest read (which uses `hex::encode` directly on
/// the digest bytes - the same encoding, since `identity.digest()` is what
/// this hashes).
fn identity_column(identity: &DependencyIdentity) -> String {
    hex::encode(identity.digest())
}

/// True when `error` looks like "this table does not exist" rather than a
/// real database failure.
///
/// SeaORM does not expose a typed "table missing" variant - every driver
/// surfaces it as an opaque `DbErr::Query` / `DbErr::Exec` wrapping a
/// backend-specific message, so this matches on the phrasing each backend
/// is known to use: SQLite's `no such table`, Postgres's `relation ... does
/// not exist`, and MySQL's `Table '...' doesn't exist`. The same
/// string-matching technique is already used elsewhere in this codebase
/// (`vector/qdrant.rs`) for the same class of "the resource I expect isn't
/// there" signal. Scoped to callers that already know the failing
/// statement names one of the three `suprnova_render_*` tables, so a false
/// positive here would have to be some other error that happens to share
/// this exact phrasing against the exact same query - not a realistic risk
/// in practice.
fn is_missing_table_error(error: &sea_orm::DbErr) -> bool {
    let message = error.to_string();
    message.contains("no such table")
        || message.contains("does not exist")
        || message.contains("doesn't exist")
}

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
pub async fn advance_in_current_transaction(
    identities: &[DependencyIdentity],
) -> Result<(), FrameworkError> {
    if identities.is_empty() {
        return Ok(());
    }
    let tx = Transaction::current().ok_or_else(|| {
        FrameworkError::internal("RenderCache generation advance requires the owning transaction")
    })?;
    advance_through(&ExecutorChoice::from_tx(&tx), identities).await
}

/// The upsert-and-log body shared by [`advance_in_current_transaction`]
/// (the ambient `CURRENT_TX` form) and [`advance_via_tx`] (the explicit
/// `&Transaction` form the `Model::*_with_tx` shims need - see ruling
/// R47): each identity's generation row upserted and its change-log row
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
        // The three `suprnova_render_*` tables land in one migration, so a
        // missing `suprnova_render_epochs` means none of them exist: this
        // database has never run `render_cache::migration::Migration`, most
        // likely because the application (or, overwhelmingly, a test
        // database built without it) does not use RenderCache at all.
        // "Live is complete without it" (see the module documentation)
        // extends to every write path this module instruments: a model
        // save must keep working exactly as it always has when the
        // generation ledger's own schema was never installed, the same way
        // any other write on an unmigrated table would simply not exist
        // yet rather than becoming mandatory infrastructure everyone pays
        // for. Every other database failure - a real connectivity problem,
        // a permissions error - still propagates.
        Err(e) if is_missing_table_error(&e) => return Ok(()),
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

/// Advances `identities` through an explicit transaction handle instead of
/// the ambient `CURRENT_TX` task-local `advance_in_current_transaction`
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
/// Mirrors [`advance_in_current_transaction`] statement-for-statement
/// (same upsert, same lock ordering, same append-only log row per
/// identity), substituting `tx`'s executor for the ambient one.
pub async fn advance_via_tx(
    tx: &Transaction,
    identities: &[DependencyIdentity],
) -> Result<(), FrameworkError> {
    advance_through(&ExecutorChoice::from_tx(tx), identities).await
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

        // Pinned to the primary, not `DB::select`: `resolve_read` silently
        // routes an ambient-transaction-free read to a registered
        // `__read_replica__`, and a database-authoritative ledger reading a
        // lagging follower would recheck an entry against a generation the
        // primary already moved past, reporting it fresh when it is not.
        // `DB::select_on` still yields to `CURRENT_TX` first (see its own
        // doc), which is the precedence this needs: inside a transaction
        // the transaction's own connection wins, exactly as `DB::select`
        // would have resolved it.
        let backend = DB::connection()
            .map_err(provider_error)?
            .inner()
            .get_database_backend();
        let digests: Vec<String> = dependencies.iter().map(hex::encode).collect();
        let sql = format!(
            "SELECT identity, generation FROM suprnova_render_generations WHERE identity IN ({})",
            placeholders(backend, digests.len()).map_err(provider_error)?
        );
        let values: Vec<Value> = digests.into_iter().map(Value::from).collect();
        let rows = DB::select_on(PRIMARY_CONNECTION_NAME, &sql, values)
            .await
            .map_err(provider_error)?;

        let mut found: HashMap<String, u64> = HashMap::new();
        for row in rows {
            let identity = row.get_string("identity").map_err(provider_error)?;
            let generation = row.get_int("generation").map_err(provider_error)?;
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
        // replication lag. `DB::scalar` has no `_on` variant, so this reads
        // the row through `DB::select_on` and decodes it by column name
        // instead.
        let rows = DB::select_on(
            PRIMARY_CONNECTION_NAME,
            "SELECT epoch FROM suprnova_render_epochs WHERE singleton = 1",
            vec![],
        )
        .await
        .map_err(provider_error)?;
        let row = rows.first().ok_or_else(|| {
            tracing::warn!(
                target: "suprnova::render_cache",
                "epoch: suprnova_render_epochs has no singleton row",
            );
            RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable)
        })?;
        let epoch = row.get_int("epoch").map_err(provider_error)?;
        Ok(epoch as u64)
    }
}
