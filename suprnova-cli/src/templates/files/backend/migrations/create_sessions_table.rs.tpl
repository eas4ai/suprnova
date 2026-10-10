use sea_orm_migration::prelude::*;
use suprnova::session::migrations::{create_sessions_table, SessionUserKey};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The sessions table of the Laravel 13 skeleton, keyed to the
        // `User` model's integer id. A `sessions` table that already exists,
        // Laravel's included, is left as it is; one an earlier scaffold
        // created is moved into this layout by `CreateSessionsTable`, which
        // the Migrator lists after this migration.
        if manager.has_table("sessions").await? {
            return Ok(());
        }
        create_sessions_table(manager, "sessions", SessionUserKey::Integer).await
    }

    /// Leaves the table. `up` skips a `sessions` table that already exists,
    /// so the table may be Laravel's, with its signed-in sessions, and
    /// rolling back must not drop them.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
