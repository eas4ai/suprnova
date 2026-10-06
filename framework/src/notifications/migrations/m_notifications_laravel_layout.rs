//! Upgrade migration that moves a `notifications` table an earlier version
//! of [`CreateNotificationsTable`](super::CreateNotificationsTable)
//! created into Laravel 13's layout, with its rows.
//!
//! The earlier table held `notifiable_id` as 64-character text and its
//! timestamps as `NOT NULL`. The reshaped one is the table
//! `CreateNotificationsTable` creates now, with `notifiable_id` in the form
//! `NOTIFICATIONS_MORPH_KEY` names. Every notification keeps its id, its
//! recipient, its data and its read state. A recipient id that the chosen
//! form cannot hold (text under `int`) stops the migration with the row
//! named, rather than losing the notification; set
//! `NOTIFICATIONS_MORPH_KEY` to the form the application's keys take and
//! run it again.
//!
//! A table Laravel created, a missing table, and a table already in the
//! new layout are left alone.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

use super::m_create_notifications_table::{MorphKey, create_table, morph_key};
use crate::database::migration_guard::{
    MovedRow, UpgradeState, move_earlier_rows, quote, resume_set_aside, set_aside, upgrade_state,
};
use crate::database::stored_datetime::StoredDateTime;

/// Moves an earlier Suprnova `notifications` table into Laravel's layout.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20261005_000005_notifications_laravel_layout"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let key = morph_key()?;
        match upgrade_state(manager, "notifications", super::is_earlier_layout).await? {
            UpgradeState::Untouched | UpgradeState::Fresh => return Ok(()),
            UpgradeState::Earlier => {
                set_aside(manager, "notifications").await?;
                create_table(manager, key).await?;
            }
            UpgradeState::Resume => {
                resume_set_aside(manager, "notifications", super::is_earlier_layout).await?;
                if !manager.has_table("notifications").await? {
                    create_table(manager, key).await?;
                }
            }
        }
        let backend = manager.get_database_backend();
        move_earlier_rows(
            manager,
            "notifications",
            "id, type, notifiable_type, notifiable_id, data, read_at, created_at, updated_at",
            "id",
            |row| {
                let id: String = row.try_get("", "id")?;
                let notifiable_id: String = row.try_get("", "notifiable_id")?;
                let notifiable = match key {
                    MorphKey::Int => {
                        notifiable_id.parse::<i64>().map(Into::into).map_err(|_| {
                            DbErr::Migration(format!(
                                "notification {id} is for notifiable_id {notifiable_id:?}, which \
                             NOTIFICATIONS_MORPH_KEY=int cannot hold; set \
                             NOTIFICATIONS_MORPH_KEY to uuid or ulid and run migrate again"
                            ))
                        })?
                    }
                    MorphKey::Uuid | MorphKey::Ulid => {
                        crate::notifications::uuid_value(backend, &notifiable_id)
                    }
                };
                let time = |column: &str| -> Result<Option<chrono::NaiveDateTime>, DbErr> {
                    Ok(row
                        .try_get::<Option<StoredDateTime>>("", column)?
                        .map(StoredDateTime::naive_utc))
                };
                let placeholders = crate::database::placeholder::placeholder_list(backend, 1, 8)
                    .map_err(|e| DbErr::Migration(e.to_string()))?;
                Ok(MovedRow {
                    key: id.clone().into(),
                    writes: vec![Statement::from_sql_and_values(
                        backend,
                        format!(
                            "INSERT INTO {} (id, type, notifiable_type, notifiable_id, data, \
                             read_at, created_at, updated_at) VALUES ({placeholders})",
                            quote(backend, "notifications")
                        ),
                        vec![
                            crate::notifications::uuid_value(backend, &id),
                            row.try_get::<String>("", "type")?.into(),
                            row.try_get::<String>("", "notifiable_type")?.into(),
                            notifiable,
                            row.try_get::<String>("", "data")?.into(),
                            time("read_at")?.into(),
                            time("created_at")?.into(),
                            time("updated_at")?.into(),
                        ],
                    )],
                })
            },
        )
        .await?;
        Ok(())
    }

    /// Leaves the table in Laravel's layout: the earlier one cannot hold
    /// what Laravel may have written since.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
