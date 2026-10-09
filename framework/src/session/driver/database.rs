//! Database-backed session storage driver

use async_trait::async_trait;
use chrono::Datelike;
use sea_orm::sea_query::{
    Alias, DeleteStatement, Expr, ExprTrait, InsertStatement, OnConflict, Query,
};
use sea_orm::{ConnectionTrait, DbErr, DeriveIden, TransactionTrait};
use std::collections::HashMap;
use std::time::Duration;

use crate::database::DB;
use crate::error::FrameworkError;
use crate::session::store::{
    DestroyedSessions, SessionData, SessionMigrationError, SessionStore, guard_identity_in,
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

/// Database session driver over Laravel 13's `sessions` table.
///
/// Stores sessions in the `sessions` table, or in the table given to
/// [`Self::with_table`] (Laravel's `session.table`), in the layout
/// Laravel's migration creates, so a Laravel application reads the rows
/// this driver writes:
///
/// - `id`: the session id, the primary key.
/// - `user_id`: the default guard's user, nullable: a big integer, or a
///   UUID or ULID column for a model whose key is one. The driver reads the
///   column's type once and writes the user id in it; an id that does not
///   fit the column (an opaque id in an integer column) is written as NULL,
///   and the user is still found in the payload.
/// - `ip_address`, `user_agent`: written as NULL.
/// - `payload`: base64 of a JSON object, as the Laravel 13 skeleton's
///   `'serialization' => 'json'` stores it. The object holds the session
///   data, the CSRF token under Laravel's key `_token`, and, first,
///   `"_suprnova":1`, which marks the rows this driver wrote.
/// - `last_activity`: epoch seconds.
///
/// [`gc`](SessionStore::gc) deletes only expired rows that carry the
/// marker, so a session a Laravel application on the same database wrote
/// is left to Laravel's own lifetime.
///
/// The queries are sea-query statements over the table name held at run
/// time, so a configured table name works like the default one.
/// [`CreateSessionsTable`](crate::session::migrations::CreateSessionsTable)
/// creates the table.
pub struct DatabaseSessionDriver {
    lifetime: Duration,
    table: String,
    user_id_column: tokio::sync::OnceCell<UserIdColumn>,
}

/// Column names shared by every session table. Unqualified, so a
/// statement names only the table it targets.
#[derive(DeriveIden)]
enum SessionColumn {
    Id,
    UserId,
    IpAddress,
    UserAgent,
    Payload,
    LastActivity,
}

/// What the `user_id` column holds, read from the catalog once per driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserIdColumn {
    /// `foreignId`: a big integer.
    Integer,
    /// `foreignUuid` on Postgres: the native `uuid` type.
    Uuid,
    /// Text: `foreignUuid` and `foreignUlid` elsewhere, `foreignUlid` on
    /// Postgres, or anything else.
    Text,
}

/// The key the driver writes first in every payload, so `gc` can tell its
/// rows from a Laravel application's.
const PAYLOAD_MARKER: &str = "_suprnova";

/// The JSON every payload starts with. It is 15 bytes, a multiple of 3, so
/// its base64 is a fixed 20-character prefix of the stored payload.
const PAYLOAD_HEAD: &str = "{\"_suprnova\":1,";

/// Laravel's key for the CSRF token in the session's attributes.
const CSRF_KEY: &str = "_token";

/// The key the default guard's user id is kept under in the payload as
/// well as in `user_id`, so an id the column cannot hold (an opaque id in
/// Laravel's integer column) still reads back and still revokes.
const USER_KEY: &str = "_suprnova_user";

/// The session data and CSRF token as one payload: base64 of a JSON object
/// that starts with [`PAYLOAD_HEAD`].
pub(crate) fn encode_payload(
    data: &HashMap<String, serde_json::Value>,
    csrf_token: &str,
    user_id: Option<&str>,
) -> Result<String, FrameworkError> {
    use base64::Engine as _;
    fn encode<T: serde::Serialize + ?Sized>(value: &T) -> Result<String, FrameworkError> {
        serde_json::to_string(value)
            .map_err(|e| FrameworkError::internal(format!("Session serialize error: {e}")))
    }
    let mut json = String::from(PAYLOAD_HEAD);
    json.push_str(&encode(&CSRF_KEY)?);
    json.push(':');
    json.push_str(&encode(&csrf_token)?);
    if let Some(user_id) = user_id {
        json.push(',');
        json.push_str(&encode(USER_KEY)?);
        json.push(':');
        json.push_str(&encode(user_id)?);
    }
    let mut keys: Vec<&String> = data.keys().collect();
    keys.sort();
    for key in keys {
        if key == CSRF_KEY || key == PAYLOAD_MARKER || key == USER_KEY {
            continue;
        }
        json.push(',');
        json.push_str(&encode(key)?);
        json.push(':');
        json.push_str(&encode(&data[key])?);
    }
    json.push('}');
    Ok(base64::engine::general_purpose::STANDARD.encode(json))
}

/// A stored payload, decoded.
struct Payload {
    data: HashMap<String, serde_json::Value>,
    csrf_token: Option<String>,
    user_id: Option<String>,
}

/// A stored payload's data, CSRF token and user. `None` when it is neither
/// base64 of a JSON object nor a JSON object.
fn decode_payload(stored: &str) -> Option<Payload> {
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(stored.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let text = decoded.as_deref().unwrap_or(stored);
    let mut data: HashMap<String, serde_json::Value> = serde_json::from_str(text).ok()?;
    data.remove(PAYLOAD_MARKER);
    let mut text_of = |key: &str| match data.remove(key) {
        Some(serde_json::Value::String(value)) => Some(value),
        _ => None,
    };
    let csrf_token = text_of(CSRF_KEY);
    let user_id = text_of(USER_KEY);
    Some(Payload {
        data,
        csrf_token,
        user_id,
    })
}

/// The base64 prefix of every payload this driver writes.
fn own_payload_prefix() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(PAYLOAD_HEAD)
}

/// One stored session row, decoded column by column: the user id and the
/// time take whatever type the table gave them.
struct SessionRow {
    id: String,
    user_id: Option<String>,
    payload: String,
    last_activity: i64,
}

impl SessionRow {
    fn decode(row: &sea_orm::QueryResult) -> Result<Self, DbErr> {
        Ok(Self {
            id: row.try_get("", "id")?,
            user_id: read_user_id(row)?,
            payload: row.try_get("", "payload")?,
            last_activity: read_epoch(row, "last_activity")?,
        })
    }
}

/// `user_id` as text, whatever the column holds.
fn read_user_id(row: &sea_orm::QueryResult) -> Result<Option<String>, DbErr> {
    if let Ok(id) = row.try_get::<Option<i64>>("", "user_id") {
        return Ok(id.map(|id| id.to_string()));
    }
    if let Ok(id) = row.try_get::<Option<u64>>("", "user_id") {
        return Ok(id.map(|id| id.to_string()));
    }
    if let Ok(id) = row.try_get::<Option<uuid::Uuid>>("", "user_id") {
        return Ok(id.map(|id| id.to_string()));
    }
    row.try_get::<Option<String>>("", "user_id")
}

/// An epoch-seconds column, 32 or 64 bits wide.
fn read_epoch(row: &sea_orm::QueryResult, column: &str) -> Result<i64, DbErr> {
    if let Ok(value) = row.try_get::<i64>("", column) {
        return Ok(value);
    }
    row.try_get::<i32>("", column).map(i64::from)
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
            user_id_column: tokio::sync::OnceCell::new(),
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

    /// The type of the table's `user_id`, read once.
    async fn user_id_column(&self) -> Result<UserIdColumn, FrameworkError> {
        self.user_id_column
            .get_or_try_init(|| async {
                let db = DB::connection()?;
                let columns = crate::database::catalog::table_columns(db.inner(), &self.table)
                    .await
                    .map_err(database_error)?;
                Ok::<_, FrameworkError>(
                    match crate::database::catalog::column(&columns, "user_id") {
                        Some(column) if column.is_integer() => UserIdColumn::Integer,
                        Some(column) if column.data_type == "uuid" => UserIdColumn::Uuid,
                        _ => UserIdColumn::Text,
                    },
                )
            })
            .await
            .copied()
    }

    /// `user_id` as the column takes it: NULL when there is no user, or
    /// when the id does not fit the column. The NULL carries the column's
    /// type, since Postgres refuses a text NULL for a `bigint` or `uuid`
    /// column.
    async fn user_id_value(&self, user_id: Option<&str>) -> Result<sea_orm::Value, FrameworkError> {
        Ok(match self.user_id_column().await? {
            // MySQL's `foreignId` is `BIGINT UNSIGNED`, which holds the ids
            // above `i64::MAX` a `u64` key reaches.
            UserIdColumn::Integer => match user_id {
                Some(id) => match id.parse::<i64>() {
                    Ok(id) => sea_orm::Value::BigInt(Some(id)),
                    Err(_) => match id.parse::<u64>() {
                        Ok(id)
                            if DB::connection()?.inner().get_database_backend()
                                == sea_orm::DatabaseBackend::MySql =>
                        {
                            sea_orm::Value::BigUnsigned(Some(id))
                        }
                        _ => sea_orm::Value::BigInt(None),
                    },
                },
                None => sea_orm::Value::BigInt(None),
            },
            UserIdColumn::Uuid => {
                sea_orm::Value::Uuid(user_id.and_then(|id| uuid::Uuid::parse_str(id).ok()))
            }
            UserIdColumn::Text => sea_orm::Value::String(user_id.map(str::to_owned)),
        })
    }

    /// `INSERT` of one full session row, shared by the upsert in `write`
    /// and the plain insert in `migrate_two_factor_session`.
    async fn insert_row(
        &self,
        session: &SessionData,
        payload: String,
        last_activity: i64,
    ) -> Result<InsertStatement, FrameworkError> {
        let user_id = self.user_id_value(session.user_id.as_deref()).await?;
        let mut insert = Query::insert();
        insert
            .into_table(self.table())
            .columns([
                SessionColumn::Id,
                SessionColumn::UserId,
                SessionColumn::IpAddress,
                SessionColumn::UserAgent,
                SessionColumn::Payload,
                SessionColumn::LastActivity,
            ])
            .values([
                Expr::value(session.id.clone()),
                Expr::value(user_id),
                Expr::value(Option::<String>::None),
                Expr::value(Option::<String>::None),
                Expr::value(payload),
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
            .map(|row| SessionRow::decode(&row))
            .transpose()
            .map_err(database_error)?;

        let Some(session) = result else {
            return Ok(None);
        };
        // Check if expired. The lifetime is capped so the deadline
        // addition stays in range; `saturating_add` is belt-and-suspenders
        // against a far-future stored time, which reads as still active
        // (fail closed) rather than wrapping.
        let now = crate::clock::now().timestamp();
        let expiry = session
            .last_activity
            .saturating_add(self.lifetime_secs_capped());

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
        // empty session, which signs the visitor out; the log says so,
        // without the session id, which is a bearer credential, and
        // without the payload, which is the session's data.
        let Payload {
            data,
            csrf_token,
            user_id,
        } = decode_payload(&session.payload).unwrap_or_else(|| {
            tracing::warn!("stored session payload failed to parse; treating the session as empty");
            Payload {
                data: HashMap::default(),
                csrf_token: None,
                user_id: None,
            }
        });

        Ok(Some(SessionData {
            id: session.id,
            data,
            user_id: session.user_id.or(user_id),
            // A row without a token (one a Laravel application wrote with
            // another serialization) gets a fresh one; the next write
            // stores it.
            csrf_token: csrf_token.unwrap_or_else(crate::session::middleware::generate_csrf_token),
            dirty: false,
            // This row was read from storage under its own id -
            // see `SessionData::loaded_from_store` (SEC-02(c)).
            loaded_from_store: true,
        }))
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        let db = DB::connection()?;

        let payload = encode_payload(
            &session.data,
            &session.csrf_token,
            session.user_id.as_deref(),
        )?;

        let now = crate::clock::now().timestamp();

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
            let user_id = self.user_id_value(session.user_id.as_deref()).await?;
            let update = Query::update()
                .table(self.table())
                .values([
                    (SessionColumn::UserId, Expr::value(user_id)),
                    (SessionColumn::Payload, Expr::value(payload)),
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
        let mut upsert = self.insert_row(session, payload, now).await?;
        upsert.on_conflict(
            OnConflict::column(SessionColumn::Id)
                .update_columns([
                    SessionColumn::UserId,
                    SessionColumn::Payload,
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
        let payload = encode_payload(
            &session.data,
            &session.csrf_token,
            session.user_id.as_deref(),
        )
        .map_err(SessionMigrationError::RolledBack)?;
        let insert = self
            .insert_row(session, payload, crate::clock::now().timestamp())
            .await
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
        let guard = crate::auth::Auth::default_guard_name();
        Ok(self.destroy_guard_sessions(&guard, user_id).await?.count)
    }

    async fn destroy_guard_sessions(
        &self,
        guard: &str,
        user_id: &str,
    ) -> Result<DestroyedSessions, FrameworkError> {
        let db = DB::connection()?;
        let mut destroyed = DestroyedSessions::default();

        // Indexed path: the `user_id` column holds the default guard's user,
        // and no other guard's. The ids are read first so Live can end what
        // these sessions opened; the delete itself stays one statement, so a
        // row rotated in between is still removed. An id the column cannot
        // hold was written as NULL, so the payload scan below finds it.
        let column_value = self.user_id_value(Some(user_id)).await?;
        let fits_column = !matches!(
            column_value,
            sea_orm::Value::BigInt(None) | sea_orm::Value::Uuid(None)
        );
        if guard == crate::auth::Auth::default_guard_name() && fits_column {
            let by_user_column = Query::select()
                .column(SessionColumn::Id)
                .from(self.table())
                .and_where(Expr::col(SessionColumn::UserId).eq(column_value.clone()))
                .to_owned();
            for row in db
                .inner()
                .query_all(&by_user_column)
                .await
                .map_err(database_error)?
            {
                destroyed
                    .ids
                    .push(row.try_get("", "id").map_err(database_error)?);
            }
            let delete = Query::delete()
                .from_table(self.table())
                .and_where(Expr::col(SessionColumn::UserId).eq(column_value))
                .to_owned();
            destroyed.count += db
                .inner()
                .execute(&delete)
                .await
                .map_err(database_error)?
                .rows_affected();
        }

        // The guard's own entry lives only inside the payload
        // (`_auth_guards`), so the indexed column cannot see it: a
        // named-only session has `user_id = NULL`. Compare the surviving
        // rows' entry for this guard exactly in Rust, and no other guard's:
        // admin 7 is not web user 7. Revocation is rare, so correctness
        // outranks index use here; the in-Rust comparison also keeps
        // backend JSON-dialect differences out of the query.
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
            let Some(payload) = decode_payload(&payload) else {
                continue;
            };
            let default_guard_user = guard == crate::auth::Auth::default_guard_name()
                && payload.user_id.as_deref() == Some(user_id);
            if default_guard_user || guard_identity_in(&payload.data, guard) == Some(user_id) {
                let removed = db
                    .inner()
                    .execute(&self.delete_by_id(&id))
                    .await
                    .map_err(database_error)?
                    .rows_affected();
                if removed > 0 {
                    destroyed.count += removed;
                    destroyed.ids.push(id);
                }
            }
        }

        Ok(destroyed)
    }

    async fn destroy_other_guard_sessions(
        &self,
        guard: &str,
        user_id: &str,
        current_id: &str,
    ) -> Result<DestroyedSessions, FrameworkError> {
        let db = DB::connection()?;
        let query = Query::select()
            .columns([SessionColumn::Id, SessionColumn::Payload])
            .from(self.table())
            .and_where(Expr::col(SessionColumn::Id).ne(current_id))
            .to_owned();
        let rows = db.inner().query_all(&query).await.map_err(database_error)?;
        let mut destroyed = DestroyedSessions::default();
        for row in rows {
            let id: String = row.try_get("", "id").map_err(database_error)?;
            let payload: String = row.try_get("", "payload").map_err(database_error)?;
            let Some(payload) = decode_payload(&payload) else {
                continue;
            };
            let default_user = guard == crate::auth::Auth::default_guard_name()
                && payload.user_id.as_deref() == Some(user_id);
            if default_user || guard_identity_in(&payload.data, guard) == Some(user_id) {
                let count = db
                    .inner()
                    .execute(&self.delete_by_id(&id))
                    .await
                    .map_err(database_error)?
                    .rows_affected();
                if count > 0 {
                    destroyed.count += count;
                    destroyed.ids.push(id);
                }
            }
        }
        Ok(destroyed)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        let db = DB::connection()?;

        // A cutoff outside the database's date range must not be bound into
        // SQL: chrono accepts negative years that no session time holds.
        let Some(threshold) = crate::clock::now()
            .naive_utc()
            .checked_sub_signed(chrono::Duration::seconds(self.lifetime_secs_capped()))
        else {
            return Ok(0);
        };
        // Year 1000 is a conservative floor. Sessions written by this
        // driver have modern activity times, so an earlier cutoff has
        // nothing to collect. Skip rather than moving the cutoff forward
        // and risking premature expiry.
        if threshold.year() < 1000 {
            return Ok(0);
        }

        // Only rows this driver wrote: a Laravel application on the same
        // database collects its own sessions on its own lifetime.
        let expired = Query::delete()
            .from_table(self.table())
            .and_where(Expr::col(SessionColumn::LastActivity).lt(threshold.and_utc().timestamp()))
            .and_where(Expr::col(SessionColumn::Payload).like(format!("{}%", own_payload_prefix())))
            .to_owned();
        let result = db.inner().execute(&expired).await.map_err(database_error)?;

        Ok(result.rows_affected())
    }
}
