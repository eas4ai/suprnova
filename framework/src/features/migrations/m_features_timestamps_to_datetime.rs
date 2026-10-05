//! Upgrade migration for a `features` table that MySQL or MariaDB created
//! with `TIMESTAMP` time columns.
//!
//! Older versions of [`CreateFeaturesTable`](super::CreateFeaturesTable)
//! created `created_at` and `updated_at` with `timestamp_with_time_zone()`,
//! which is `TIMESTAMP` on MySQL and MariaDB. MySQL's `TIMESTAMP` refuses any
//! time after 2038-01-19 03:14:07 UTC, so from then on every flag write fails.
//! The create migration makes `DATETIME` columns now, but it never alters a
//! table that already exists.
//!
//! This migration converts each of the two columns that is still
//! `TIMESTAMP` to `DATETIME`, keeping its nullability, its
//! `DEFAULT CURRENT_TIMESTAMP` and every stored time in UTC. A column that is
//! already `DATETIME`, a missing table, and every backend but MySQL and
//! MariaDB are left alone, so it is safe to list for every app and to run
//! again. Postgres stores these columns as `timestamp with time zone`, which
//! has no 2038 limit, and SQLite stores text.

use sea_orm_migration::prelude::*;

use crate::database::migration_guard::convert_mysql_timestamps_to_datetime;

/// Converts the `TIMESTAMP` time columns of `features` to `DATETIME` on
/// MySQL and MariaDB.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20261004_000002_features_timestamps_to_datetime"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        convert_mysql_timestamps_to_datetime(manager, "features", &["created_at", "updated_at"])
            .await
    }

    /// Leaves the columns `DATETIME`. Converting them back would restore
    /// the 2038 limit, and a row written after it would make the
    /// conversion fail.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
