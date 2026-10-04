//! Migration that creates the framework-owned `notifications` table that
//! [`crate::notifications::channels::database::DatabaseChannel`] writes
//! and the read helpers in [`crate::notifications`] query - see
//! [`crate::notifications::migrations`] for the convention.
//!
//! The column types are portable across SQLite, Postgres and MySQL:
//!
//! - `id` is `CHAR(36)`, a UUID, the same width on every engine.
//! - `type` and `notifiable_type` are `VARCHAR(255)`, Laravel's widths, so
//!   rows migrated from a Laravel app fit.
//! - `notifiable_id` is `VARCHAR(64)`, so integer keys (as text) and UUID
//!   keys share one schema without per-driver casting.
//! - `data` is `TEXT` holding the JSON payload. MySQL `TEXT` holds 65,535
//!   bytes, more than any realistic notification.
//! - The timestamps are `DATETIME` (`timestamp` on Postgres) without a
//!   zone; the channel writes UTC. Not MySQL's `TIMESTAMP`, which refuses
//!   any time after 2038-01-19. Tables this migration created before have
//!   `TIMESTAMP` there, and the read helpers read both.

use sea_orm_migration::prelude::*;

use crate::database::migration_guard::create_index_if_missing;

/// Migration that creates the framework-owned `notifications` table.
pub struct Migration;

impl MigrationName for Migration {
    // Explicit, date-prefixed name, as the workflow and features migrations
    // use. `DeriveMigrationName` would derive it from the module path,
    // which a later framework migration could collide with.
    fn name(&self) -> &str {
        "m20260516_000001_create_notifications_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Notifications::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Notifications::Id)
                            .char_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Notifications::Type)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Notifications::NotifiableType)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Notifications::NotifiableId)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Notifications::Data).text().not_null())
                    .col(ColumnDef::new(Notifications::ReadAt).date_time().null())
                    .col(
                        ColumnDef::new(Notifications::CreatedAt)
                            .date_time()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Notifications::UpdatedAt)
                            .date_time()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // The inbox helpers filter on the recipient pair and on `read_at`.
        // An app that created the table by hand from the schema this
        // migration replaced already has both indexes; see
        // `create_index_if_missing` for why each is checked first.
        create_index_if_missing(
            manager,
            "notifications",
            Index::create()
                .name("idx_notifications_notifiable")
                .col(Notifications::NotifiableType)
                .col(Notifications::NotifiableId)
                .to_owned(),
        )
        .await?;

        create_index_if_missing(
            manager,
            "notifications",
            Index::create()
                .name("idx_notifications_read_at")
                .col(Notifications::ReadAt)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Notifications::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Notifications {
    Table,
    Id,
    Type,
    NotifiableType,
    NotifiableId,
    Data,
    ReadAt,
    CreatedAt,
    UpdatedAt,
}
