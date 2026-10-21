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
//! Expiry across nodes is decided by one clock: the backend's. Every SQL
//! adapter reads milliseconds since the Unix epoch from the database itself
//! ([`sql_now_ms`], read through [`store_now_ms`] or compared inside the
//! statement that guards the row), and every Redis adapter reads
//! `redis.call('TIME')` inside the script that guards the key. Never from the
//! node's own clock, so a node whose clock runs fast can neither extend a
//! lease nor keep an expired entry alive for everyone else.

pub mod redis;
pub mod redis_instances;
pub mod redis_lease;
pub mod redis_store;
pub mod sql_instances;
pub mod sql_lease;
pub mod sql_store;

pub use redis::RedisProviderConfig;
pub use redis_instances::RedisInstanceRecordStore;
pub use redis_lease::RedisLeaseStore;
pub use redis_store::RedisRenderStore;
pub use sql_instances::SqlInstanceRecordStore;
pub use sql_lease::SqlLeaseStore;
pub use sql_store::SqlRenderStore;

use sea_orm::{DbBackend, DbErr};
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
/// snapshot, as the row it writes. The dialect comes from that executor
/// rather than from a parameter, so the clock an adapter reads can never be
/// the wrong dialect's for the connection it reads it on.
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
pub async fn store_now_ms(exec: &ExecutorChoice, offset_ms: u64) -> Result<u64, RenderCacheError> {
    let backend = exec.backend();
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
        .map_err(|error| provider_db_error(&error))?
        .ok_or_else(|| {
            provider_error(FrameworkError::database(
                "store time query returned no row".to_owned(),
            ))
        })?;
    let now: i64 = row
        .try_get_by_index(0)
        .map_err(|error| provider_db_error(&error))?;
    Ok(u64::try_from(now).unwrap_or(0).saturating_add(offset_ms))
}

/// Collapses a backend failure into the one closed provider kind the
/// RenderCache contract exposes, logging the cause's closed-set kind first.
///
/// The cause never reaches the caller: [`RenderCacheError`] carries a kind
/// and nothing else, exactly so a driver message - which can echo bound
/// values - can never travel back into a response. It does not reach the
/// log either. What is logged is the failure's own variant name, which is
/// enough to keep "no primary connection registered"
/// (`kind="service_not_found"`) distinguishable from "the database is down"
/// (`kind="database"`) in whatever collects these logs, and carries no
/// render key, hex identity, or encoded record with it.
/// [`ledger`](super::ledger)'s own `provider_error` makes the same collapse
/// for the generation ledger's contract.
pub(crate) fn provider_error(error: FrameworkError) -> RenderCacheError {
    provider_error_kind(framework_error_kind(&error))
}

/// [`provider_error`] for a SeaORM failure, which is where a bound value
/// would otherwise travel: [`db_error_kind`] names the variant and drops
/// the driver's message.
pub(crate) fn provider_db_error(error: &DbErr) -> RenderCacheError {
    provider_error_kind(db_error_kind(error))
}

/// The one `warn` site behind [`provider_error`] and [`provider_db_error`].
///
/// `kind` is `&'static str` by signature, and that signature is the whole
/// guarantee: nothing a backend, a driver, or a caller composed at runtime
/// can be handed to it.
pub(crate) fn provider_error_kind(kind: &'static str) -> RenderCacheError {
    tracing::warn!(
        target: "suprnova::render_cache",
        kind,
        "render cache tier provider failure",
    );
    RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable)
}

/// The closed-set name of a SeaORM failure, for logging.
///
/// `DbErr`'s `Display` repeats the driver's own message, and the statements
/// these adapters run bind render keys, hex identities, and encoded
/// records, so that message is exactly the thing that must not be logged.
/// The variant name says which layer failed and carries none of it.
///
/// Every variant the pinned SeaORM defines is named here, including the
/// three an application's own RBAC and locking can raise
/// (`RbacError`, `AccessDenied`, `MutexPoisonError`), because a name is
/// what makes one failure distinguishable from another in a log that
/// carries nothing else. `DbErr` is `#[non_exhaustive]`, so the default arm
/// stays: an unrecognised future variant answers `"other"` rather than
/// falling back to the message.
pub(crate) fn db_error_kind(error: &DbErr) -> &'static str {
    match error {
        DbErr::ConnectionAcquire(_) => "connection_acquire",
        DbErr::TryIntoErr { .. } => "try_into",
        DbErr::Conn(_) => "conn",
        DbErr::Exec(_) => "exec",
        DbErr::Query(_) => "query",
        DbErr::ConvertFromU64(_) => "convert_from_u64",
        DbErr::UnpackInsertId => "unpack_insert_id",
        DbErr::UpdateGetPrimaryKey => "update_get_primary_key",
        DbErr::RecordNotFound(_) => "record_not_found",
        DbErr::AttrNotSet(_) => "attr_not_set",
        DbErr::Custom(_) => "custom",
        DbErr::Type(_) => "type",
        DbErr::Json(_) => "json",
        DbErr::Migration(_) => "migration",
        DbErr::RecordNotInserted => "record_not_inserted",
        DbErr::RecordNotUpdated => "record_not_updated",
        DbErr::BackendNotSupported { .. } => "backend_not_supported",
        DbErr::KeyArityMismatch { .. } => "key_arity_mismatch",
        DbErr::PrimaryKeyNotSet { .. } => "primary_key_not_set",
        DbErr::RbacError(_) => "rbac",
        DbErr::AccessDenied { .. } => "access_denied",
        DbErr::MutexPoisonError => "mutex_poison",
        _ => "other",
    }
}

/// The closed-set name of a framework failure, for logging.
///
/// The same rule as [`db_error_kind`], one layer up: a
/// `FrameworkError::Database` message is a driver string, so the variant is
/// what is logged and the message is dropped. The three variants these
/// adapters can raise or receive are named individually, because the
/// distinction between them is the one worth keeping - a service that was
/// never registered is a deployment mistake, a database error is an
/// outage - and everything else answers `"other"`.
pub(crate) fn framework_error_kind(error: &FrameworkError) -> &'static str {
    match error {
        FrameworkError::Database(_) => "database",
        FrameworkError::ServiceNotFound { .. } => "service_not_found",
        FrameworkError::Internal { .. } => "internal",
        _ => "other",
    }
}

/// Milliseconds, counters, and identities as a `BIGINT` column stores them.
///
/// Values above `i64::MAX` are unreachable - epochs count emergency
/// invalidations, tokens count publications, versions count replacements,
/// and an instant is milliseconds since 1970 - so clamping keeps the
/// conversion total without a panic and without wrapping a saturated
/// `u64::MAX` retention into the distant past.
pub(crate) fn as_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The inverse of [`as_i64`]. A negative column value cannot occur: every
/// writer in these adapters is [`as_i64`].
pub(crate) fn as_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Whether a backend's failure message is "that key of `table` is already
/// taken".
///
/// Two adapters here decide an outcome by letting a unique key raise rather
/// than by overwriting: a lease and an instance record are each created by a
/// plain `INSERT`, because a conflict means a peer got there first and the
/// peer's row is the one that stands. PostgreSQL's `SELECT ... FOR UPDATE`
/// locks nothing when the row is absent, so on that backend the unique key
/// is the only thing standing between two nodes creating the same key.
///
/// Classifying by message is the technique this module's neighbours already
/// use for the same class of signal (see
/// [`ledger`](super::ledger)'s missing-table check and the transaction
/// layer's deadlock detection); a driver's typed error kinds are not uniform
/// across the three backends this framework supports. The table name is
/// required alongside the phrase for the same reason that check requires
/// one: a message naming some *other* table's constraint is not this
/// statement's collision, and reading it as one would make a creation answer
/// "someone else has it" with no row written anywhere.
///
/// Anything this does not recognise stays a provider failure, which is the
/// safe direction: a caller told "the store failed" retries or reports,
/// while a caller told "a peer holds it" stops looking. One backend lands
/// there in practice - a MySQL or MariaDB build old enough to report
/// `for key 'PRIMARY'` without the table prefix 8.0.19 added - and a genuine
/// collision on it degrades to a provider failure rather than to a peer's
/// win. Nothing is granted twice either way.
pub(crate) fn is_unique_violation(message: &str, table: &str) -> bool {
    // SQLite: "UNIQUE constraint failed: suprnova_render_leases.render_key".
    // PostgreSQL (SQLSTATE 23505): "duplicate key value violates unique
    // constraint \"suprnova_live_instances_pkey\"". MySQL and MariaDB (1062):
    // "Duplicate entry '...' for key 'suprnova_live_promotions.PRIMARY'".
    message.contains(table)
        && (message.contains("UNIQUE constraint failed")
            || message.contains("duplicate key value")
            || message.contains("Duplicate entry"))
}

/// One `?` (MySQL and SQLite) or `$N` (PostgreSQL) placeholder for the bound
/// value at `index`, counted from one.
///
/// Every statement in these adapters is written once and asks for its
/// placeholders here, so the one thing all three dialects genuinely disagree
/// about is expressed in a single place rather than in a `match` arm per
/// statement. `DbBackend` is `#[non_exhaustive]`, so an unrecognised future
/// variant is refused explicitly rather than silently guessing a syntax it
/// was never proven against.
///
/// # Errors
///
/// Returns an error for any backend other than PostgreSQL, MySQL, or SQLite.
pub(crate) fn bind(backend: DbBackend, index: usize) -> Result<String, FrameworkError> {
    match backend {
        DbBackend::Postgres => Ok(format!("${index}")),
        DbBackend::MySql | DbBackend::Sqlite => Ok("?".to_owned()),
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

/// The clause that locks the rows a `SELECT` reads until the transaction
/// ends, where the dialect has one.
///
/// SQLite is the empty string, and needs to be: it has no `FOR UPDATE`
/// syntax and no use for one, because it serialises writers outright. Every
/// adapter here follows the same pattern - a locked read that decides, then
/// a guarded write, then a re-read that confirms - so the suffix is written
/// once rather than in a `match` arm per statement.
pub(crate) fn row_lock(backend: DbBackend) -> &'static str {
    match backend {
        DbBackend::Postgres | DbBackend::MySql => " FOR UPDATE",
        _ => "",
    }
}

/// [`bind`] for `count` consecutive values starting at `first`, joined with
/// `", "` - the shape a `VALUES (...)` list needs.
///
/// # Errors
///
/// Returns an error for any backend other than PostgreSQL, MySQL, or SQLite.
pub(crate) fn binds(
    backend: DbBackend,
    first: usize,
    count: usize,
) -> Result<String, FrameworkError> {
    (first..first + count)
        .map(|index| bind(backend, index))
        .collect::<Result<Vec<_>, _>>()
        .map(|rendered| rendered.join(", "))
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

    #[test]
    fn a_taken_key_is_recognised_in_every_backend_and_nothing_else_is() {
        // The real phrasings, one per backend this framework supports.
        assert!(is_unique_violation(
            "error returned from database: (code: 2067) UNIQUE constraint failed: suprnova_render_leases.render_key",
            "suprnova_render_leases"
        ));
        assert!(is_unique_violation(
            "error returned from database: duplicate key value violates unique constraint \"suprnova_live_instances_pkey\"",
            "suprnova_live_instances"
        ));
        assert!(is_unique_violation(
            "error returned from database: 1062 (23000): Duplicate entry 'abc-def' for key 'suprnova_live_promotions.PRIMARY'",
            "suprnova_live_promotions"
        ));

        // A store that is simply broken must never be read as "a peer got
        // there first": that would turn an outage into a silent bypass.
        for other in [
            "error returned from database: no such table: suprnova_render_leases",
            "pool timed out while waiting for an open connection",
            "error returned from database: deadlock detected",
        ] {
            assert!(
                !is_unique_violation(other, "suprnova_render_leases"),
                "{other}"
            );
        }

        // Another table's collision is not this statement's collision, and
        // reading it as one would answer "a peer holds it" with no row
        // written anywhere.
        assert!(!is_unique_violation(
            "error returned from database: (code: 2067) UNIQUE constraint failed: suprnova_live_promotions.idempotency",
            "suprnova_live_instances"
        ));
    }

    #[test]
    fn a_seaorm_failure_is_logged_as_a_variant_name_and_never_as_its_message() {
        // What a failing statement actually carries back: the driver
        // repeats the statement and its bound values, which for these
        // adapters are render keys, hex identities, and encoded records.
        let error = sea_orm::DbErr::Query(sea_orm::RuntimeErr::Internal(
            "error returned from database: INSERT INTO suprnova_live_instances \
             VALUES ('0123456789abcdef', x'0a5245434f5244')"
                .to_owned(),
        ));
        // The message is the leak this guards against, so prove it is
        // really in the error before proving it is not in what is logged.
        let message = error.to_string();
        assert!(message.contains("0123456789abcdef"), "{message}");
        assert!(message.contains("0a5245434f5244"), "{message}");

        let logged = db_error_kind(&error);
        assert_eq!(logged, "query");
        assert!(!logged.contains("0123456789abcdef"), "{logged}");
        assert!(!logged.contains("0a5245434f5244"), "{logged}");
        assert!(!logged.contains("suprnova_live_instances"), "{logged}");

        // Every arm answers a name of its own, and nothing composed at
        // runtime can reach the log through any of them.
        assert_eq!(
            db_error_kind(&sea_orm::DbErr::Conn(sea_orm::RuntimeErr::Internal(
                "0123456789abcdef".to_owned()
            ))),
            "conn"
        );
        assert_eq!(
            db_error_kind(&sea_orm::DbErr::Custom("0123456789abcdef".to_owned())),
            "custom"
        );
        assert_eq!(
            db_error_kind(&sea_orm::DbErr::RecordNotInserted),
            "record_not_inserted"
        );
        // The three an application's own RBAC and locking raise. Their
        // payloads are a caller's own strings, so they are exactly the kind
        // of value the name exists to leave behind.
        assert_eq!(
            db_error_kind(&sea_orm::DbErr::RbacError("0123456789abcdef".to_owned())),
            "rbac"
        );
        assert_eq!(
            db_error_kind(&sea_orm::DbErr::AccessDenied {
                permission: "0123456789abcdef".to_owned(),
                resource: "0123456789abcdef".to_owned(),
            }),
            "access_denied"
        );
        assert_eq!(
            db_error_kind(&sea_orm::DbErr::MutexPoisonError),
            "mutex_poison"
        );
    }

    #[test]
    fn a_framework_failure_is_logged_as_a_variant_name_and_never_as_its_message() {
        // A `Database` message is a driver string one layer down, so the
        // same rule applies to it.
        let error = FrameworkError::database(
            "error returned from database: UPDATE suprnova_render_leases SET \
             render_key = 'rk1.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'"
                .to_owned(),
        );
        let message = error.to_string();
        assert!(message.contains("rk1."), "{message}");

        let logged = framework_error_kind(&error);
        assert_eq!(logged, "database");
        assert!(!logged.contains("rk1."), "{logged}");
        assert!(!logged.contains("suprnova_render_leases"), "{logged}");

        // The distinction the log exists for: a connection that was never
        // registered is a deployment mistake, not an outage.
        assert_eq!(
            framework_error_kind(&FrameworkError::ServiceNotFound {
                type_name: "suprnova::database::DbConnection"
            }),
            "service_not_found"
        );
        assert_eq!(
            framework_error_kind(&FrameworkError::internal("rk1.secret".to_owned())),
            "internal"
        );
        assert_eq!(framework_error_kind(&FrameworkError::Unauthorized), "other");
    }

    #[test]
    fn placeholders_are_numbered_on_postgres_and_positional_elsewhere() {
        assert_eq!(bind(DbBackend::Postgres, 3).expect("postgres"), "$3");
        assert_eq!(
            binds(DbBackend::Postgres, 1, 4).expect("postgres"),
            "$1, $2, $3, $4"
        );
        for backend in [DbBackend::MySql, DbBackend::Sqlite] {
            assert_eq!(bind(backend, 3).expect("a dialect"), "?");
            assert_eq!(binds(backend, 1, 4).expect("a dialect"), "?, ?, ?, ?");
        }
    }
}
