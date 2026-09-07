//! Tier 1 and Tier 2 provider adapters: the concrete stores a database or
//! Redis profile builds, behind the engine's own provider contracts.
//!
//! Everything here is an adapter. The decisions - what supersedes what, how
//! long an entry may live, what a fenced publication means - belong to
//! `suprnova_live::render_cache`; these modules only carry them to a
//! backend and back.
//!
//! # Store time
//!
//! Expiry across nodes is decided by one clock: the database's. Every SQL
//! adapter reads milliseconds since the Unix epoch from the backend itself
//! ([`sql_now_ms`], read through [`store_now_ms`] or compared inside the
//! statement that guards the row), never from the node's own clock, so a
//! node whose clock runs fast can neither extend a lease nor keep an
//! expired entry alive for everyone else.

pub mod sql_store;

pub use sql_store::SqlRenderStore;

use sea_orm::DbBackend;
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};

use crate::FrameworkError;
use crate::database::transaction::ExecutorChoice;

/// The dialect expression for "milliseconds since the Unix epoch, as this
/// database reports it".
///
/// Used two ways, and both are deliberate: inlined into the statement that
/// guards a row (so the comparison happens at the backend, inside the same
/// transaction as the write it guards) and read out through
/// [`store_now_ms`] when an adapter needs the number itself to compute an
/// expiry column.
///
/// Every dialect's expression is integer-typed, and on MySQL that takes an
/// explicit cast: `UNIX_TIMESTAMP(NOW(3))` returns `DECIMAL`, which a driver
/// will not decode into `i64`. The cast lives here rather than at the one
/// call site that decodes a value, so every consumer - the lease takeover
/// and token minting of Task 5, the record expiry guards of Task 6, this
/// store's own expiry comparisons - gets one shape per dialect and none of
/// them has to know which dialect needed help.
///
/// `DbBackend` is `#[non_exhaustive]`, so an unrecognised future variant is
/// refused explicitly rather than silently guessing an expression it was
/// never proven against - the same rule
/// [`ledger`](super::ledger)'s own dialect helpers follow.
///
/// # Errors
///
/// Returns an error for any backend other than Postgres, MySQL, or SQLite.
pub fn sql_now_ms(backend: DbBackend) -> Result<&'static str, FrameworkError> {
    match backend {
        DbBackend::Sqlite => Ok("CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)"),
        DbBackend::Postgres => Ok("(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT"),
        DbBackend::MySql => Ok("CAST(UNIX_TIMESTAMP(NOW(3)) * 1000 AS SIGNED)"),
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

/// Reads [`sql_now_ms`] on `exec` and returns it as milliseconds since the
/// Unix epoch, plus `offset_ms`.
///
/// Run this on the executor that carries the guarded statement - inside a
/// publication's own transaction, not on a second connection - so the
/// expiry an adapter writes is measured on the same clock, and the same
/// snapshot, as the row it writes.
///
/// `offset_ms` is a test seam, and it is per adapter instance rather than
/// process-wide on purpose: several stores share one test binary, and a
/// process-global offset would let one test's expiry move another's.
/// Production always passes zero (see
/// [`SqlRenderStore::set_time_offset_for_test`]).
///
/// # Errors
///
/// Returns [`RenderCacheErrorKind::ProviderUnavailable`] when the backend
/// is unsupported, the read fails, or the backend returns no row - none of
/// which carry a key, a byte, or a SQL value into the message.
pub async fn store_now_ms(
    exec: &ExecutorChoice,
    backend: DbBackend,
    offset_ms: u64,
) -> Result<u64, RenderCacheError> {
    // Selected verbatim: `sql_now_ms` already returns an integer-typed
    // expression on every dialect, so nothing is wrapped here.
    let sql = format!("SELECT {}", sql_now_ms(backend).map_err(provider_error)?);
    let row = exec
        .query_one(sea_orm::Statement::from_sql_and_values(
            backend,
            &sql,
            vec![],
        ))
        .await
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
        .ok_or_else(|| {
            provider_error(FrameworkError::database(
                "store time query returned no row".to_owned(),
            ))
        })?;
    let now: i64 = row
        .try_get_by_index(0)
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
    Ok(u64::try_from(now).unwrap_or(0).saturating_add(offset_ms))
}

/// Collapses a backend failure into the one closed provider kind the
/// RenderCache contract exposes, logging the underlying cause first.
///
/// The cause never reaches the caller: [`RenderCacheError`] carries a kind
/// and nothing else, exactly so a driver message - which can echo bound
/// values - can never travel back into a response. It is logged at `warn`
/// so "no primary connection registered" stays distinguishable from "the
/// database is down" in whatever collects these logs. Mirrors
/// [`ledger`](super::ledger)'s own `provider_error`.
pub(crate) fn provider_error(error: FrameworkError) -> RenderCacheError {
    tracing::warn!(
        target: "suprnova::render_cache",
        %error,
        "render cache tier provider failure",
    );
    RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable)
}

#[cfg(test)]
mod tests {
    //! The store clock's shape per dialect. Every adapter in this module
    //! either inlines [`sql_now_ms`] into a comparison or decodes it through
    //! [`store_now_ms`], and the second only works if the expression is
    //! integer-typed - which on MySQL it is not without the cast asserted
    //! below.
    use super::*;

    #[test]
    fn the_store_clock_is_integer_typed_on_every_supported_dialect() {
        assert_eq!(
            sql_now_ms(DbBackend::Sqlite).expect("sqlite"),
            "CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)"
        );
        assert_eq!(
            sql_now_ms(DbBackend::Postgres).expect("postgres"),
            "(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT"
        );
        // `UNIX_TIMESTAMP(NOW(3))` alone returns DECIMAL, which no driver
        // decodes into `i64`. Without this cast `store_now_ms` fails on
        // MySQL, and every expiry an adapter writes there fails with it.
        assert_eq!(
            sql_now_ms(DbBackend::MySql).expect("mysql"),
            "CAST(UNIX_TIMESTAMP(NOW(3)) * 1000 AS SIGNED)"
        );
    }
}
