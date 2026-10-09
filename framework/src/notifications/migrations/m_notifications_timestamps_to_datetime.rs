//! Upgrade migration for a `notifications` table that an earlier version of
//! [`CreateNotificationsTable`](super::CreateNotificationsTable) created on
//! MySQL or MariaDB with `TIMESTAMP` time columns.
//!
//! MySQL's `TIMESTAMP` refuses any time after 2038-01-19 03:14:07 UTC, so
//! from then on every notification write fails. This migration converts
//! each of `read_at`, `created_at` and `updated_at` that is still
//! `TIMESTAMP` to `DATETIME`, keeping its nullability and every stored time
//! in UTC.
//!
//! It touches only a table in the earlier Suprnova layout (a 64-character
//! text `notifiable_id`). A table Laravel created stays exactly as Laravel
//! left it, `TIMESTAMP` columns included; a missing table, a column that is
//! already `DATETIME`, and every backend but MySQL and MariaDB are left
//! alone too, so it is safe to list for every app and to run again.

use sea_orm_migration::prelude::*;

use crate::database::migration_guard::convert_mysql_timestamps_to_datetime;

/// Converts the `TIMESTAMP` time columns of an earlier Suprnova
/// `notifications` table to `DATETIME` on MySQL and MariaDB.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20261004_000001_notifications_timestamps_to_datetime"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let columns =
            crate::database::catalog::table_columns(manager.get_connection(), "notifications")
                .await?;
        if !super::is_earlier_layout(&columns) {
            return Ok(());
        }
        convert_mysql_timestamps_to_datetime(
            manager,
            "notifications",
            &["read_at", "created_at", "updated_at"],
        )
        .await
    }

    /// Leaves the columns `DATETIME`. Converting them back would restore
    /// the 2038 limit, and a row written after it would make the
    /// conversion fail.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
