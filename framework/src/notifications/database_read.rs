//! Read-side helpers for the `notifications` table.
//!
//! The write half lives in
//! [`crate::notifications::channels::database::DatabaseChannel`]. This
//! module provides the Laravel-equivalent read surface - fetch
//! all/unread/read rows for a notifiable, mark as read/unread, and the
//! mass-update + delete helpers - without forcing every consumer to write
//! the SQL themselves.
//!
//! Laravel exposes these through the `Notifiable` + `HasDatabaseNotifications`
//! traits returning Eloquent relationships. Suprnova's `Notifiable` trait is
//! intentionally minimal (just `route_for`), so the read surface ships as
//! free functions on the framework side that take an explicit
//! `(notifiable_type, notifiable_id)` pair - the same polymorphic pair
//! [`DatabaseChannel`](crate::notifications::channels::database::DatabaseChannel)
//! writes.

use crate::database::placeholder::placeholder;
use crate::database::stored_datetime::StoredDateTime;
use crate::error::FrameworkError;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, FromQueryResult, QueryResult, Statement, Value,
};

/// One persisted notification row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredNotification {
    /// Row primary key (string-typed to carry UUID/ULID notification ids verbatim).
    pub id: String,
    /// Notification type - the `Notification::notification_name()` of the
    /// originating notification.
    pub type_name: String,
    /// Recipient model name (e.g. `"users"`).
    pub notifiable_type: String,
    /// Recipient id, stringified.
    pub notifiable_id: String,
    /// JSON-decoded data column.
    pub data: serde_json::Value,
    /// `Some(t)` iff the recipient has marked the notification read.
    pub read_at: Option<chrono::NaiveDateTime>,
    /// Timestamp at which the row was inserted. Laravel's table allows
    /// `NULL` here.
    pub created_at: Option<chrono::NaiveDateTime>,
    /// Timestamp at which the row was last mutated. Laravel's table allows
    /// `NULL` here.
    pub updated_at: Option<chrono::NaiveDateTime>,
}

/// A key column as text, whatever type the table gave it: `uuid` or text
/// for `id`, an integer, `uuid` or text for `notifiable_id`.
fn key_text(res: &QueryResult, column: &str) -> Result<String, sea_orm::DbErr> {
    if let Ok(text) = res.try_get::<String>("", column) {
        return Ok(text);
    }
    if let Ok(number) = res.try_get::<i64>("", column) {
        return Ok(number.to_string());
    }
    if let Ok(number) = res.try_get::<u64>("", column) {
        return Ok(number.to_string());
    }
    res.try_get::<uuid::Uuid>("", column)
        .map(|uuid| uuid.to_string())
}

impl FromQueryResult for StoredNotification {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, sea_orm::DbErr> {
        let data_text: String = res.try_get("", "data")?;
        let data: serde_json::Value =
            serde_json::from_str(&data_text).map_err(|e| sea_orm::DbErr::Custom(e.to_string()))?;
        Ok(Self {
            id: key_text(res, "id")?,
            type_name: res.try_get("", "type")?,
            notifiable_type: res.try_get("", "notifiable_type")?,
            notifiable_id: key_text(res, "notifiable_id")?,
            data,
            // Laravel's migration creates these as `TIMESTAMP` on MySQL and
            // MariaDB, which a plain `NaiveDateTime` cannot be decoded from.
            // A read time that fails to decode is an error, not an unread
            // notification.
            read_at: res
                .try_get::<Option<StoredDateTime>>("", "read_at")?
                .map(|read_at| read_at.0),
            created_at: res
                .try_get::<Option<StoredDateTime>>("", "created_at")?
                .map(|at| at.0),
            updated_at: res
                .try_get::<Option<StoredDateTime>>("", "updated_at")?
                .map(|at| at.0),
        })
    }
}

const COLS: &str =
    "id, type, notifiable_type, notifiable_id, data, read_at, created_at, updated_at";

/// `notifiable_id` bound as its column takes it.
fn key(db: &DatabaseConnection, notifiable_id: &str) -> Value {
    crate::notifications::morph_key_value(db.get_database_backend(), notifiable_id)
}

/// A notification id bound as its column takes it.
fn id_value(db: &DatabaseConnection, id: &str) -> Value {
    crate::notifications::uuid_value(db.get_database_backend(), id)
}

async fn run(
    db: &DatabaseConnection,
    sql: &str,
    values: Vec<Value>,
) -> Result<Vec<StoredNotification>, FrameworkError> {
    let stmt = Statement::from_sql_and_values(db.get_database_backend(), sql, values);
    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| FrameworkError::internal(format!("notifications read: {e}")))?;
    rows.into_iter()
        .map(|r| {
            StoredNotification::from_query_result(&r, "")
                .map_err(|e| FrameworkError::internal(format!("notifications decode: {e}")))
        })
        .collect()
}

/// The `(notifiable_type, notifiable_id)` predicate every recipient-scoped
/// statement shares, with placeholders starting at ordinal `first`.
///
/// `first` is a parameter rather than a constant because `mark_all_as_read`
/// binds two timestamps ahead of the recipient pair - on Postgres a clause
/// that restarts its numbering silently reads the wrong bind.
fn recipient_predicate(db: &DatabaseConnection, first: usize) -> Result<String, FrameworkError> {
    let backend = db.get_database_backend();
    Ok(format!(
        "notifiable_type = {} AND notifiable_id = {}",
        placeholder(backend, first)?,
        placeholder(backend, first + 1)?
    ))
}

/// All notifications for a recipient, newest first. Laravel's
/// `$user->notifications` equivalent.
pub async fn all_for(
    db: &DatabaseConnection,
    notifiable_type: &str,
    notifiable_id: &str,
) -> Result<Vec<StoredNotification>, FrameworkError> {
    let sql = format!(
        "SELECT {COLS} FROM notifications \
         WHERE {} \
         ORDER BY created_at DESC",
        recipient_predicate(db, 1)?
    );
    run(
        db,
        &sql,
        vec![notifiable_type.into(), key(db, notifiable_id)],
    )
    .await
}

/// Unread notifications (`read_at IS NULL`) for a recipient, newest first.
/// Laravel's `$user->unreadNotifications`.
pub async fn unread_for(
    db: &DatabaseConnection,
    notifiable_type: &str,
    notifiable_id: &str,
) -> Result<Vec<StoredNotification>, FrameworkError> {
    let sql = format!(
        "SELECT {COLS} FROM notifications \
         WHERE {} AND read_at IS NULL \
         ORDER BY created_at DESC",
        recipient_predicate(db, 1)?
    );
    run(
        db,
        &sql,
        vec![notifiable_type.into(), key(db, notifiable_id)],
    )
    .await
}

/// Read notifications (`read_at IS NOT NULL`) for a recipient, newest first.
/// Laravel's `$user->readNotifications`.
pub async fn read_for(
    db: &DatabaseConnection,
    notifiable_type: &str,
    notifiable_id: &str,
) -> Result<Vec<StoredNotification>, FrameworkError> {
    let sql = format!(
        "SELECT {COLS} FROM notifications \
         WHERE {} AND read_at IS NOT NULL \
         ORDER BY created_at DESC",
        recipient_predicate(db, 1)?
    );
    run(
        db,
        &sql,
        vec![notifiable_type.into(), key(db, notifiable_id)],
    )
    .await
}

/// Mark a single notification row as read. No-op if `read_at` is already set
/// (matches Laravel's `markAsRead` idempotence).
pub async fn mark_as_read(db: &DatabaseConnection, id: &str) -> Result<(), FrameworkError> {
    let now = whole_seconds_now();
    let backend = db.get_database_backend();
    let stmt = Statement::from_sql_and_values(
        backend,
        format!(
            "UPDATE notifications SET read_at = {}, updated_at = {} \
             WHERE id = {} AND read_at IS NULL",
            placeholder(backend, 1)?,
            placeholder(backend, 2)?,
            placeholder(backend, 3)?
        ),
        vec![now.into(), now.into(), id_value(db, id)],
    );
    db.execute_raw(stmt)
        .await
        .map_err(|e| FrameworkError::internal(format!("mark_as_read: {e}")))?;
    Ok(())
}

/// Mark a single notification row as unread. No-op if `read_at` is already
/// NULL (matches Laravel's `markAsUnread` idempotence).
pub async fn mark_as_unread(db: &DatabaseConnection, id: &str) -> Result<(), FrameworkError> {
    let now = whole_seconds_now();
    let backend = db.get_database_backend();
    let stmt = Statement::from_sql_and_values(
        backend,
        format!(
            "UPDATE notifications SET read_at = NULL, updated_at = {} \
             WHERE id = {} AND read_at IS NOT NULL",
            placeholder(backend, 1)?,
            placeholder(backend, 2)?
        ),
        vec![now.into(), id_value(db, id)],
    );
    db.execute_raw(stmt)
        .await
        .map_err(|e| FrameworkError::internal(format!("mark_as_unread: {e}")))?;
    Ok(())
}

/// Mass-mark every unread notification for the recipient as read. Laravel's
/// `$user->unreadNotifications->markAsRead()` equivalent. Returns the
/// number of rows updated.
pub async fn mark_all_as_read(
    db: &DatabaseConnection,
    notifiable_type: &str,
    notifiable_id: &str,
) -> Result<u64, FrameworkError> {
    let now = whole_seconds_now();
    let backend = db.get_database_backend();
    let stmt = Statement::from_sql_and_values(
        backend,
        format!(
            "UPDATE notifications SET read_at = {}, updated_at = {} \
             WHERE {} AND read_at IS NULL",
            placeholder(backend, 1)?,
            placeholder(backend, 2)?,
            recipient_predicate(db, 3)?
        ),
        vec![
            now.into(),
            now.into(),
            notifiable_type.into(),
            key(db, notifiable_id),
        ],
    );
    let res = db
        .execute_raw(stmt)
        .await
        .map_err(|e| FrameworkError::internal(format!("mark_all_as_read: {e}")))?;
    Ok(res.rows_affected())
}

/// Delete every notification row for the recipient. Returns the number of
/// rows deleted. Laravel's `$user->notifications()->delete()`.
pub async fn delete_for(
    db: &DatabaseConnection,
    notifiable_type: &str,
    notifiable_id: &str,
) -> Result<u64, FrameworkError> {
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        format!(
            "DELETE FROM notifications WHERE {}",
            recipient_predicate(db, 1)?
        ),
        vec![notifiable_type.into(), key(db, notifiable_id)],
    );
    let res = db
        .execute_raw(stmt)
        .await
        .map_err(|e| FrameworkError::internal(format!("delete_for: {e}")))?;
    Ok(res.rows_affected())
}

/// The current time at whole seconds, the precision of Laravel's
/// timestamp columns.
fn whole_seconds_now() -> chrono::NaiveDateTime {
    let now = crate::clock::now();
    chrono::DateTime::<chrono::Utc>::from_timestamp(now.timestamp(), 0)
        .unwrap_or(now)
        .naive_utc()
}
