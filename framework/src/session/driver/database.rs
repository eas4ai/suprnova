//! Database-backed session storage driver

use async_trait::async_trait;
use chrono::Datelike;
use sea_orm::sea_query::{
    Alias, DeleteStatement, Expr, ExprTrait, InsertStatement, OnConflict, Query,
};
use sea_orm::{ConnectionTrait, DbErr, DeriveIden, FromQueryResult, TransactionTrait};
use std::collections::HashMap;
use std::time::Duration;

use crate::database::DB;
use crate::database::stored_datetime::StoredDateTime;
use crate::error::FrameworkError;
use crate::session::store::{
    SessionData, SessionMigrationError, SessionStore, guard_principal_ids_in,
};

/// The table [`DatabaseSessionDriver::new`] reads and writes.
const DEFAULT_SESSION_TABLE: &str = "sessions";

/// Longest accepted table name, in bytes. Postgres truncates an
/// identifier past 63 bytes, so a longer name would silently name a
/// different table there than on the other backends.
const MAX_SESSION_TABLE_LEN: usize = 63;

/// The naming rule [`valid_session_table`] enforces, worded for the
/// errors that [`DatabaseSessionDriver::with_table`] and `Config::init`
/// return, so both name the same rule.
pub(crate) const SESSION_TABLE_RULE: &str = "use 1 to 63 ASCII letters, digits or underscores, \
     starting with a letter or underscore";

/// Whether `name` may name the session table: ASCII, a letter or `_`
/// first, then letters, digits or `_`, 1 to 63 bytes.
///
/// Every statement quotes the name, so this rule is not what keeps SQL
/// out. It makes a typo fail when the driver is built, or at boot for
/// `SESSION_TABLE`, instead of on the first request, and it keeps the
/// name portable to every backend the driver supports.
pub(crate) fn valid_session_table(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    bytes.len() <= MAX_SESSION_TABLE_LEN
        && (first.is_ascii_alphabetic() || *first == b'_')
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

/// Database session driver using SeaORM
///
/// Stores sessions in the `sessions` table, or in the table given to
/// [`Self::with_table`] (Laravel's `session.table`), with the following
/// schema:
/// - id: VARCHAR (primary key) - session ID
/// - user_id: VARCHAR (nullable) - authenticated user ID (string, supports both numeric and opaque IDs)
/// - payload: TEXT - JSON serialized session data
/// - csrf_token: VARCHAR - CSRF protection token
/// - last_activity: TIMESTAMP or DATETIME (`timestamp` or `timestamptz` on
///   Postgres) - last access time, in UTC
///
/// The queries are sea-query statements over the table name held at run
/// time. A SeaORM entity fixes its table at compile time, which is why
/// the [`sessions`] entity cannot serve a configured name.
pub struct DatabaseSessionDriver {
    lifetime: Duration,
    table: String,
}

/// Column names shared by every session table. Unqualified, so a
/// statement names only the table it targets.
#[derive(DeriveIden)]
enum SessionColumn {
    Id,
    UserId,
    Payload,
    CsrfToken,
    LastActivity,
}

/// One stored session row, decoded by column name.
///
/// `last_activity` is a [`StoredDateTime`]: older scaffolds created it
/// with `.timestamp()`, which is `TIMESTAMP` on MySQL and MariaDB, and a
/// plain `NaiveDateTime` decodes only from `DATETIME` there.
#[derive(FromQueryResult)]
struct SessionRow {
    id: String,
    user_id: Option<String>,
    payload: String,
    csrf_token: String,
    last_activity: StoredDateTime,
}

fn database_error(error: DbErr) -> FrameworkError {
    FrameworkError::database(error.to_string())
}

impl DatabaseSessionDriver {
    /// Create a new database session driver over the `sessions` table.
    pub fn new(lifetime: Duration) -> Self {
        Self::with_configured_table(lifetime, DEFAULT_SESSION_TABLE)
    }

    /// Create a database session driver over `table` instead of
    /// `sessions`. The app's migration has to create that table with the
    /// same columns.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming `table` when it is not 1 to 63
    /// ASCII letters, digits or underscores starting with a letter or
    /// underscore.
    pub fn with_table(
        lifetime: Duration,
        table: impl Into<String>,
    ) -> Result<Self, FrameworkError> {
        let table = table.into();
        if !valid_session_table(&table) {
            return Err(FrameworkError::internal(format!(
                "session table name {table:?} is not valid: {SESSION_TABLE_RULE}"
            )));
        }
        Ok(Self::with_configured_table(lifetime, table))
    }

    /// Create a driver over `table` without checking the name.
    ///
    /// For callers that cannot return an error, such as the infallible
    /// [`SessionMiddleware::new`](crate::session::SessionMiddleware::new).
    /// `Config::init` already rejects a bad `SESSION_TABLE` at boot, and
    /// every statement quotes the name, so a bad name cannot inject SQL:
    /// it fails as a missing table on the first query.
    pub(crate) fn with_configured_table(lifetime: Duration, table: impl Into<String>) -> Self {
        Self {
            lifetime,
            table: table.into(),
        }
    }

    /// The configured table as a quoted identifier.
    fn table(&self) -> Alias {
        Alias::new(self.table.as_str())
    }

    /// Session lifetime in whole seconds, capped at
    /// [`MAX_SESSION_LIFETIME_SECS`](crate::session::MAX_SESSION_LIFETIME_SECS)
    /// so the `u64`→`i64` conversion is exact and deadline arithmetic
    /// cannot overflow. Env parsing clamps to the same bound, but a
    /// programmatically built config can carry any [`Duration`].
    fn lifetime_secs_capped(&self) -> i64 {
        let secs = i64::try_from(self.lifetime.as_secs()).unwrap_or(i64::MAX);
        // `Ord::min` by path: sea-query's `ExprTrait`, imported for the
        // statements below, is implemented for every value and has a
        // `min` of its own.
        Ord::min(secs, crate::session::MAX_SESSION_LIFETIME_SECS as i64)
    }

    /// `INSERT` of one full session row, shared by the upsert in `write`
    /// and the plain insert in `migrate_two_factor_session`.
    fn insert_row(
        &self,
        session: &SessionData,
        payload: String,
        last_activity: chrono::NaiveDateTime,
    ) -> Result<InsertStatement, FrameworkError> {
        let mut insert = Query::insert();
        insert
            .into_table(self.table())
            .columns([
                SessionColumn::Id,
                SessionColumn::UserId,
                SessionColumn::Payload,
                SessionColumn::CsrfToken,
                SessionColumn::LastActivity,
            ])
            .values([
                Expr::value(session.id.clone()),
                Expr::value(session.user_id.clone()),
                Expr::value(payload),
                Expr::value(session.csrf_token.clone()),
                Expr::value(last_activity),
            ])
            .map_err(|e| FrameworkError::internal(format!("session insert statement: {e}")))?;
        Ok(insert)
    }

    /// `DELETE` of the row whose id is `id`.
    fn delete_by_id(&self, id: &str) -> DeleteStatement {
        Query::delete()
            .from_table(self.table())
            .and_where(Expr::col(SessionColumn::Id).eq(id))
            .to_owned()
    }
}

#[async_trait]
impl SessionStore for DatabaseSessionDriver {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        let db = DB::connection()?;

        let select = Query::select()
            .columns([
                SessionColumn::Id,
                SessionColumn::UserId,
                SessionColumn::Payload,
                SessionColumn::CsrfToken,
                SessionColumn::LastActivity,
            ])
            .from(self.table())
            .and_where(Expr::col(SessionColumn::Id).eq(id))
            .limit(1)
            .to_owned();
        let result = db
            .inner()
            .query_one(&select)
            .await
            .map_err(database_error)?
            .map(|row| SessionRow::from_query_result(&row, ""))
            .transpose()
            .map_err(database_error)?;

        if let Some(session) = result {
            // Check if expired. The lifetime is capped so the `i64`
            // conversion is exact and the deadline addition stays in
            // range; `checked_add` is belt-and-suspenders against a
            // far-future stored timestamp, which reads as still active
            // (fail closed) rather than panicking.
            let now = crate::clock::now().naive_utc();
            let expiry = session
                .last_activity
                .0
                .checked_add_signed(chrono::Duration::seconds(self.lifetime_secs_capped()))
                .unwrap_or(chrono::NaiveDateTime::MAX);

            if now > expiry {
                // Session expired, clean it up. The read already answers
                // "no session"; a failed delete only leaves the row for
                // `gc`, so it is logged and not returned.
                if let Err(error) = self.destroy(id).await {
                    tracing::warn!(
                        error = %error,
                        "expired session row could not be deleted; garbage collection will remove it"
                    );
                }
                return Ok(None);
            }

            // Parse the payload. A payload that does not parse reads as an
            // empty session, which signs the visitor out; the log says why,
            // without the session id, which is a bearer credential. It
            // also leaves out serde_json's message, which quotes the value
            // when the payload is a JSON string instead of a map.
            let data: HashMap<String, serde_json::Value> = serde_json::from_str(&session.payload)
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        category = ?error.classify(),
                        line = error.line(),
                        column = error.column(),
                        "stored session payload failed to parse; treating the session as empty"
                    );
                    HashMap::default()
                });

            Ok(Some(SessionData {
                id: session.id,
                data,
                user_id: session.user_id,
                csrf_token: session.csrf_token,
                dirty: false,
                // This row was read from storage under its own id -
                // see `SessionData::loaded_from_store` (SEC-02(c)).
                loaded_from_store: true,
            }))
        } else {
            Ok(None)
        }
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        let db = DB::connection()?;

        let payload = serde_json::to_string(&session.data)
            .map_err(|e| FrameworkError::internal(format!("Session serialize error: {}", e)))?;

        let now = crate::clock::now().naive_utc();

        // SEC-02(c): a session that was read from an existing row under
        // `session.id` must be written back as an UPDATE-ONLY - no
        // INSERT fallback. Without this branch, the upsert below would
        // silently recreate (resurrect) a row that was deleted between
        // this request's read and its write - e.g. a concurrent
        // `destroy_for_user` password-reset revocation, or `gc` - and
        // hand the resurrected row's session cookie right back to
        // whoever is holding it, including the party the revocation was
        // meant to lock out. `session.id` is guaranteed fresh (never
        // read as existing) whenever it was set via
        // `SessionData::rotate_id` this request, so the id-rotation
        // paths (login, 2FA promotion, remember-me hydration, manual
        // regenerate, `invalidate_session`) still fall through to the
        // upsert arm and create their new row exactly as before.
        if session.loaded_from_store {
            let update = Query::update()
                .table(self.table())
                .values([
                    (SessionColumn::UserId, Expr::value(session.user_id.clone())),
                    (SessionColumn::Payload, Expr::value(payload)),
                    (
                        SessionColumn::CsrfToken,
                        Expr::value(session.csrf_token.clone()),
                    ),
                    (SessionColumn::LastActivity, Expr::value(now)),
                ])
                .and_where(Expr::col(SessionColumn::Id).eq(session.id.as_str()))
                .to_owned();
            let result = db.inner().execute(&update).await.map_err(database_error)?;

            if result.rows_affected() == 0 {
                // The row is gone - most likely a concurrent
                // revocation. Declining to resurrect it is the correct
                // outcome, not a failure: the next read of this
                // session id will correctly find nothing. The log leaves
                // the id out: it is a bearer credential.
                tracing::debug!(
                    authenticated = session.user_id.is_some(),
                    "session write skipped: row no longer exists (revoked or expired concurrently)"
                );
            }

            return Ok(());
        }

        // Fresh session (never read as existing under this id) - atomic
        // upsert: INSERT ... ON CONFLICT(id) DO UPDATE SET ...
        // The previous check-then-insert/update was a read-modify-write
        // race - two parallel writers persisting a fresh-but-shared
        // session id (e.g. a SPA reconnecting after the DB row was
        // gc'd while the cookie was still valid) could both see "no
        // existing row" and both attempt INSERT; one would win, the
        // other would fail the UNIQUE constraint, and the SessionMiddleware
        // fail-closed branch would 500 the loser. ON CONFLICT collapses
        // both branches into a single round-trip + skips the pre-read
        // on the happy path. sea-query renders the OnConflict clause as
        // Postgres `ON CONFLICT DO UPDATE`, MySQL `ON DUPLICATE KEY
        // UPDATE`, and SQLite `ON CONFLICT DO UPDATE`.
        let mut upsert = self.insert_row(session, payload, now)?;
        upsert.on_conflict(
            OnConflict::column(SessionColumn::Id)
                .update_columns([
                    SessionColumn::UserId,
                    SessionColumn::Payload,
                    SessionColumn::CsrfToken,
                    SessionColumn::LastActivity,
                ])
                .to_owned(),
        );

        db.inner().execute(&upsert).await.map_err(database_error)?;

        Ok(())
    }

    async fn migrate_two_factor_session(
        &self,
        old_id: &str,
        session: &SessionData,
    ) -> Result<(), SessionMigrationError> {
        let db = DB::connection().map_err(SessionMigrationError::RolledBack)?;
        let payload = serde_json::to_string(&session.data).map_err(|e| {
            SessionMigrationError::RolledBack(FrameworkError::internal(format!(
                "Session serialize error: {e}"
            )))
        })?;
        let insert = self
            .insert_row(session, payload, crate::clock::now().naive_utc())
            .map_err(SessionMigrationError::RolledBack)?;
        let delete_old = self.delete_by_id(old_id);

        let transaction = db.inner().begin().await.map_err(|e| {
            SessionMigrationError::RolledBack(FrameworkError::database(e.to_string()))
        })?;
        let migration = async {
            let deleted = transaction
                .execute(&delete_old)
                .await
                .map_err(database_error)?;
            if deleted.rows_affected() != 1 {
                return Err(FrameworkError::internal(
                    "atomic 2FA session migration requires an existing old session",
                ));
            }

            transaction.execute(&insert).await.map_err(database_error)?;
            Ok(())
        }
        .await;

        match migration {
            Ok(()) => transaction.commit().await.map_err(|e| {
                SessionMigrationError::OutcomeUnknown(FrameworkError::database(e.to_string()))
            }),
            Err(error) => {
                if let Err(rollback_error) = transaction.rollback().await {
                    tracing::error!(
                        operation = "two_factor_session_migration_rollback",
                        classification = "backend_failure",
                        "atomic session migration rollback failed"
                    );
                    return Err(SessionMigrationError::OutcomeUnknown(
                        FrameworkError::database(format!(
                            "session migration failed and rollback was not confirmed: {rollback_error}"
                        )),
                    ));
                }
                Err(SessionMigrationError::RolledBack(error))
            }
        }
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        let db = DB::connection()?;

        db.inner()
            .execute(&self.delete_by_id(id))
            .await
            .map_err(database_error)?;

        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let db = DB::connection()?;

        // Indexed path: sessions whose default-guard principal is `user_id`.
        let by_user_column = Query::delete()
            .from_table(self.table())
            .and_where(Expr::col(SessionColumn::UserId).eq(user_id))
            .to_owned();
        let mut deleted = db
            .inner()
            .execute(&by_user_column)
            .await
            .map_err(database_error)?
            .rows_affected();

        // Named-guard principals live only inside the payload
        // (`_auth_guards`), so the indexed column cannot see them: a
        // named-only session has `user_id = NULL`, and a multi-principal
        // session carries a different top-level id. Compare the surviving
        // rows' guard identities exactly in Rust. Revocation is rare, so
        // correctness outranks index use here; the in-Rust comparison
        // also keeps backend JSON-dialect differences out of the query.
        let surviving = Query::select()
            .columns([SessionColumn::Id, SessionColumn::Payload])
            .from(self.table())
            .to_owned();
        let rows = db
            .inner()
            .query_all(&surviving)
            .await
            .map_err(database_error)?;
        for row in rows {
            let id: String = row.try_get("", "id").map_err(database_error)?;
            let payload: String = row.try_get("", "payload").map_err(database_error)?;
            let data: HashMap<String, serde_json::Value> =
                serde_json::from_str(&payload).unwrap_or_default();
            if guard_principal_ids_in(&data)
                .iter()
                .any(|principal| principal == user_id)
            {
                deleted += db
                    .inner()
                    .execute(&self.delete_by_id(&id))
                    .await
                    .map_err(database_error)?
                    .rows_affected();
            }
        }

        Ok(deleted)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        let db = DB::connection()?;

        // A cutoff outside the database's date range must not be bound into
        // SQL: chrono accepts negative years that MySQL cannot encode and
        // dates older than PostgreSQL's timestamp range.
        let Some(threshold) = crate::clock::now()
            .naive_utc()
            .checked_sub_signed(chrono::Duration::seconds(self.lifetime_secs_capped()))
        else {
            return Ok(0);
        };
        // Year 1000 is a conservative portable SQL datetime floor. Sessions
        // written by this driver have modern activity timestamps, so an
        // earlier cutoff has nothing to collect. Skip rather than moving
        // the cutoff forward and risking premature expiry.
        if threshold.year() < 1000 {
            return Ok(0);
        }

        let expired = Query::delete()
            .from_table(self.table())
            .and_where(Expr::col(SessionColumn::LastActivity).lt(threshold))
            .to_owned();
        let result = db.inner().execute(&expired).await.map_err(database_error)?;

        Ok(result.rows_affected())
    }
}

/// SeaORM entity for the default `sessions` table.
///
/// It documents the columns every session table needs and stays public
/// so code that names it keeps compiling. [`DatabaseSessionDriver`] no
/// longer queries through it: an entity fixes its table name at compile
/// time, and the driver serves the table the app configures.
pub mod sessions {
    use sea_orm::entity::prelude::*;

    /// SeaORM model for a single row in `sessions`.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "sessions")]
    pub struct Model {
        /// Session id (the cookie value), kept as the primary key.
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        /// Authenticated user id, if any; null for guest sessions.
        pub user_id: Option<String>,
        /// Serialized session payload (encoded by the configured session encoder).
        #[sea_orm(column_type = "Text")]
        pub payload: String,
        /// Per-session CSRF token rotated when the session id rotates.
        pub csrf_token: String,
        /// UTC time of the last activity on this session, used for sliding
        /// TTL. A [`StoredDateTime`](crate::database::StoredDateTime) reads
        /// `DATETIME` and the `TIMESTAMP` older scaffolds created on MySQL
        /// and MariaDB, `timestamp` and `timestamptz` on Postgres, and SQLite
        /// text, so a whole-row read works on every one.
        pub last_activity: crate::database::StoredDateTime,
    }

    /// SeaORM relation enum - `sessions` is a leaf table with no declared
    /// foreign-key relations.
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
