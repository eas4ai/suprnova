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

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new("sessions")).if_exists().to_owned())
            .await
    }
}
