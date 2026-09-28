//! Migration that normalizes MySQL workflow date columns to `DATETIME`.
//!
//! Early workflow migrations declared these columns as `TIMESTAMP`, but the
//! public entities use `chrono::NaiveDateTime`, whose MySQL storage type is
//! `DATETIME`. The migration is additive for existing installations and a
//! no-op on PostgreSQL and SQLite.
//!
//! MySQL may rebuild and lock both tables while applying these alterations;
//! drain workflow workers before running this migration.

use sea_orm::{ConnectionTrait, DbBackend};
use sea_orm_migration::prelude::*;

const ALTER_WORKFLOWS: &str = r#"
ALTER TABLE `workflows`
    MODIFY COLUMN `next_run_at` DATETIME NULL DEFAULT NULL,
    MODIFY COLUMN `locked_until` DATETIME NULL DEFAULT NULL,
    MODIFY COLUMN `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    MODIFY COLUMN `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    MODIFY COLUMN `started_at` DATETIME NULL DEFAULT NULL,
    MODIFY COLUMN `completed_at` DATETIME NULL DEFAULT NULL
"#;

const ALTER_WORKFLOW_STEPS: &str = r#"
ALTER TABLE `workflow_steps`
    MODIFY COLUMN `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    MODIFY COLUMN `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    MODIFY COLUMN `started_at` DATETIME NULL DEFAULT NULL,
    MODIFY COLUMN `completed_at` DATETIME NULL DEFAULT NULL
"#;

/// Converts legacy MySQL workflow date columns to `DATETIME`.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260901_000001_normalize_workflow_datetime_columns"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() != DbBackend::MySql {
            return Ok(());
        }

        manager
            .get_connection()
            .execute_unprepared(ALTER_WORKFLOWS)
            .await
            .map_err(|error| {
                DbErr::Migration(format!("normalize MySQL workflows date columns: {error}"))
            })?;
        manager
            .get_connection()
            .execute_unprepared(ALTER_WORKFLOW_STEPS)
            .await
            .map_err(|error| {
                DbErr::Migration(format!(
                    "normalize MySQL workflow_steps date columns: {error}"
                ))
            })?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Monotonic by design: converting back would recreate the decode bug
        // and can lose valid DATETIME values outside TIMESTAMP's range.
        Ok(())
    }
}
