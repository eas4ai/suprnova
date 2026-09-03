//! `GenerationLedger` over the application database. Advancement runs on
//! the caller's current transaction so rollback advances nothing.

use std::collections::HashMap;

use async_trait::async_trait;
use sea_orm::{DbBackend, Value};
use suprnova_live::render_cache::generation::{
    DependencyIdentity, GenerationLedger, GenerationSet,
};
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};

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
    let backend = tx.backend();
    let epoch: i64 = DB::scalar(
        "SELECT epoch FROM suprnova_render_epochs WHERE singleton = 1",
        vec![],
    )
    .await?;

    let mut ordered: Vec<&DependencyIdentity> = identities.iter().collect();
    ordered.sort_by_key(|identity| identity.digest());

    for identity in ordered {
        let digest = identity_column(identity);
        DB::statement(
            upsert_sql(backend)?,
            vec![Value::from(digest.clone()), Value::from(epoch)],
        )
        .await?;

        let generation: i64 = DB::scalar(
            &format!(
                "SELECT generation FROM suprnova_render_generations WHERE identity = {}",
                placeholders(backend, 1)?
            ),
            vec![Value::from(digest.clone())],
        )
        .await?;

        DB::statement(
            &format!(
                "INSERT INTO suprnova_render_generation_log \
                 (identity, generation, epoch, committed_at) VALUES ({}, CURRENT_TIMESTAMP)",
                placeholders(backend, 3)?
            ),
            vec![
                Value::from(digest),
                Value::from(generation),
                Value::from(epoch),
            ],
        )
        .await?;
    }
    Ok(())
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
