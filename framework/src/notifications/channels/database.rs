//! Database notification channel - writes one row per notification.
//!
//! Persists notifications to the `notifications` table in Laravel 13's
//! layout, which
//! [`CreateNotificationsTable`](crate::notifications::migrations::CreateNotificationsTable)
//! creates and Laravel's own `make:notifications-table` migration creates
//! the same way. Each delivery is one `INSERT`: a fresh UUID id, the
//! notification name as `type`, the `notifiable_type` registered at
//! construction, the recipient route as `notifiable_id`, and the
//! JSON-encoded payload. `read_at` starts `NULL`; consumers flip it when
//! the recipient acks the notification.
//!
//! Laravel stores class names in `type` and `notifiable_type`. An
//! application sharing its database with Laravel passes the recipient
//! model's `morph_type` (`App\Models\User`, or the alias Laravel's morph
//! map gives it) so both applications find the same inbox.

use crate::database::placeholder::placeholder_list;
use crate::error::FrameworkError;
use crate::notifications::{Channel, DynNotification};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use uuid::Uuid;

/// Notification channel that persists each delivery as one row in the
/// `notifications` table.
///
/// `notifiable_type` is the table / class name of the recipient model
/// (e.g. `"users"`); the channel pairs it with the route returned by
/// `Notifiable::route_for("database")` (typically the recipient's id
/// stringified) to form the polymorphic recipient reference.
pub struct DatabaseChannel {
    db: DatabaseConnection,
    notifiable_type: String,
}

impl DatabaseChannel {
    /// Build a `DatabaseChannel` writing rows into `notifications` against
    /// `db`, tagging each row with `notifiable_type` (the polymorphic recipient class).
    pub fn new(db: DatabaseConnection, notifiable_type: impl Into<String>) -> Self {
        Self {
            db,
            notifiable_type: notifiable_type.into(),
        }
    }
}

#[async_trait]
impl Channel for DatabaseChannel {
    fn name(&self) -> &'static str {
        "database"
    }

    async fn deliver(
        &self,
        route: &str,
        notification: &dyn DynNotification,
    ) -> Result<(), FrameworkError> {
        let id = Uuid::new_v4();
        let now = whole_seconds(crate::clock::now()).naive_utc();
        let data_json = serde_json::to_string(&notification.data())
            .map_err(|e| FrameworkError::internal(format!("DatabaseChannel encode: {e}")))?;

        let backend = self.db.get_database_backend();
        // `read_at` is the literal NULL in the middle of the list, so the
        // seven binds are not the eight columns - the placeholder run has to
        // be rendered around it rather than one-per-column.
        let stmt = Statement::from_sql_and_values(
            backend,
            format!(
                "INSERT INTO notifications (id, type, notifiable_type, notifiable_id, data, read_at, created_at, updated_at) \
                 VALUES ({}, NULL, {})",
                placeholder_list(backend, 1, 5)?,
                placeholder_list(backend, 6, 2)?
            ),
            [
                crate::notifications::uuid_value(backend, &id.to_string()),
                notification.name().to_string().into(),
                self.notifiable_type.clone().into(),
                crate::notifications::morph_key_value(backend, route),
                data_json.into(),
                now.into(),
                now.into(),
            ],
        );
        self.db
            .execute_raw(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("DatabaseChannel insert: {e}")))?;
        Ok(())
    }
}

/// `at` without its fraction: Laravel's timestamp columns keep whole
/// seconds, and Postgres would round a fraction rather than cut it.
fn whole_seconds(at: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(at.timestamp(), 0).unwrap_or(at)
}
