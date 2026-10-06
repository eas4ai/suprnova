//! The framework-owned migration for the database session driver's table.
//!
//! [`CreateSessionsTable`] creates `sessions` (or the table `SESSION_TABLE`
//! names) in the layout Laravel 13's skeleton creates, so a Suprnova
//! application runs on a database Laravel created and both read each
//! other's rows:
//!
//! ```rust,no_run
//! use sea_orm_migration::MigratorTrait;
//! use suprnova::session::migrations::{CreateSessionsTable, SessionUserKey};
//!
//! pub struct Migrator;
//!
//! impl MigratorTrait for Migrator {
//!     fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
//!         vec![
//!             // ... the users table first ...
//!             Box::new(CreateSessionsTable::new(SessionUserKey::Integer)),
//!         ]
//!     }
//! }
//! ```
//!
//! A table Laravel created is left exactly as it is. A table in the layout
//! earlier Suprnova scaffolds created (`csrf_token`, a date-time
//! `last_activity`, JSON `payload`) is reshaped with its rows: every
//! session keeps its id, its user, its data and its CSRF token, so nobody
//! is signed out by the upgrade.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{QueryResult, Statement};

use crate::database::catalog::{CatalogColumn, column};
use crate::database::migration_guard::{
    MovedRow, UpgradeState, move_earlier_rows, resume_set_aside, set_aside, upgrade_state,
};
use crate::database::stored_datetime::StoredDateTime;
use crate::schema::Schema;

/// The key type of the default guard's user model, which decides the type
/// of `sessions.user_id`, as Laravel's skeleton decides it with
/// `foreignId`, `foreignUuid` or `foreignUlid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionUserKey {
    /// An auto-incrementing integer key: `foreignId`, a big integer
    /// (unsigned on MySQL). Laravel's default.
    #[default]
    Integer,
    /// A UUID key (`unique_id = "uuid"`): `foreignUuid`, `uuid` on
    /// Postgres and `CHAR(36)` elsewhere.
    Uuid,
    /// A ULID key (`unique_id = "ulid"`): `foreignUlid`, `CHAR(26)`.
    Ulid,
}

/// Creates the sessions table in Laravel 13's layout, or reshapes one in
/// the earlier Suprnova layout with its rows. See the [module docs](self).
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateSessionsTable {
    user_key: SessionUserKey,
}

impl CreateSessionsTable {
    /// The migration for a user model whose key is `user_key`.
    pub const fn new(user_key: SessionUserKey) -> Self {
        Self { user_key }
    }
}

impl MigrationName for CreateSessionsTable {
    fn name(&self) -> &str {
        "m20261005_000004_create_sessions_table"
    }
}

/// The table the session driver reads: `SESSION_TABLE`, or `sessions`.
fn sessions_table() -> Result<String, DbErr> {
    let table = std::env::var("SESSION_TABLE").unwrap_or_else(|_| "sessions".to_owned());
    if !crate::session::driver::database::valid_session_table(&table) {
        return Err(DbErr::Migration(format!(
            "SESSION_TABLE={table:?} is not a valid table name; {}",
            crate::session::driver::database::SESSION_TABLE_RULE
        )));
    }
    Ok(table)
}

/// Earlier Suprnova scaffolds kept the CSRF token in a column of its own;
/// Laravel keeps it in the payload.
fn is_earlier(columns: &[CatalogColumn]) -> bool {
    column(columns, "csrf_token").is_some()
}

/// Create `table` in the layout of Laravel 13's skeleton for `user_key`.
///
/// # Errors
///
/// Returns [`DbErr`] when a statement fails.
pub async fn create_sessions_table(
    manager: &SchemaManager<'_>,
    table: &str,
    user_key: SessionUserKey,
) -> Result<(), DbErr> {
    Schema::create(manager, table, |t| {
        t.string("id").primary();
        match user_key {
            SessionUserKey::Integer => t.unsigned_big_integer("user_id").nullable().index(),
            SessionUserKey::Uuid => t.uuid("user_id").nullable().index(),
            SessionUserKey::Ulid => t.ulid("user_id").nullable().index(),
        };
        t.string("ip_address").length(45).nullable();
        t.text("user_agent").nullable();
        t.long_text("payload");
        t.integer("last_activity").index();
    })
    .await
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSessionsTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = sessions_table()?;
        match upgrade_state(manager, &table, is_earlier).await? {
            UpgradeState::Untouched => return Ok(()),
            UpgradeState::Fresh => {
                return create_sessions_table(manager, &table, self.user_key).await;
            }
            UpgradeState::Earlier => {
                set_aside(manager, &table).await?;
                create_sessions_table(manager, &table, self.user_key).await?;
            }
            UpgradeState::Resume => {
                resume_set_aside(manager, &table, is_earlier).await?;
                if !manager.has_table(&table).await? {
                    create_sessions_table(manager, &table, self.user_key).await?;
                }
            }
        }
        let backend = manager.get_database_backend();
        let user_key = self.user_key;
        let target = table.clone();
        move_earlier_rows(
            manager,
            &table,
            "id, user_id, payload, csrf_token, last_activity",
            "id",
            |row| move_session(row, backend, &target, user_key),
        )
        .await?;
        Ok(())
    }

    /// Leaves the table: it holds the signed-in sessions, and Laravel may
    /// read it.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

fn move_session(
    row: &QueryResult,
    backend: sea_orm_migration::sea_orm::DbBackend,
    table: &str,
    user_key: SessionUserKey,
) -> Result<MovedRow, DbErr> {
    let id: String = row.try_get("", "id")?;
    let user_id: Option<String> = row.try_get("", "user_id")?;
    let payload: String = row.try_get("", "payload")?;
    let csrf_token: String = row.try_get("", "csrf_token")?;
    let last_activity = row
        .try_get::<StoredDateTime>("", "last_activity")?
        .and_utc()
        .timestamp();
    let data: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_str(&payload).unwrap_or_default();
    let payload =
        crate::session::driver::database::encode_payload(&data, &csrf_token, user_id.as_deref())
            .map_err(|e| DbErr::Migration(format!("session payload: {e}")))?;
    // A NULL carries the column's type too: Postgres refuses a text NULL
    // for a `bigint` or `uuid` column.
    let user_id = match user_key {
        SessionUserKey::Integer => sea_orm_migration::sea_orm::Value::BigInt(
            user_id.as_deref().and_then(|id| id.parse::<i64>().ok()),
        ),
        SessionUserKey::Uuid if backend == sea_orm_migration::sea_orm::DbBackend::Postgres => {
            sea_orm_migration::sea_orm::Value::Uuid(
                user_id
                    .as_deref()
                    .and_then(|id| uuid::Uuid::parse_str(id).ok()),
            )
        }
        _ => sea_orm_migration::sea_orm::Value::String(user_id),
    };
    let placeholders = crate::database::placeholder::placeholder_list(backend, 1, 6)
        .map_err(|e| DbErr::Migration(e.to_string()))?;
    Ok(MovedRow {
        key: id.clone().into(),
        writes: vec![Statement::from_sql_and_values(
            backend,
            format!(
                "INSERT INTO {} (id, user_id, ip_address, user_agent, payload, last_activity) \
                 VALUES ({placeholders})",
                crate::database::migration_guard::quote(backend, table)
            ),
            vec![
                id.into(),
                user_id,
                Option::<String>::None.into(),
                Option::<String>::None.into(),
                payload.into(),
                last_activity.into(),
            ],
        )],
    })
}
