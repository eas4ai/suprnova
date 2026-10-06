//! Upgrade migration that moves a `features` table an earlier version of
//! [`CreateFeaturesTable`](super::CreateFeaturesTable) created into
//! laravel/pennant's layout, with its rows.
//!
//! The earlier table keyed a flag by `scope_key` (`""`, `user:42`,
//! `team:staff`) and held `enabled`, `description` and `updated_by`. Each
//! flag moves to Pennant's row for the same flag: the scope Pennant
//! serializes (`__laravel_null`, `{FEATURES_USER_SCOPE}|42`), and the value
//! `true` or `false`. The description and the actor move to
//! `suprnova_feature_details`. No flag changes its answer.
//!
//! A table Pennant created, a missing table, and a table already in
//! Pennant's layout are left alone.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

use super::m_create_features_table::{create_details, create_features};
use crate::database::migration_guard::{
    MovedRow, UpgradeState, move_earlier_rows, quote, resume_set_aside, set_aside, upgrade_state,
};
use crate::database::placeholder::placeholder_list;
use crate::database::stored_datetime::StoredDateTime;
use crate::features::store::{DETAILS_TABLE, FEATURES_TABLE, scope_to_stored, stored_value};

/// Moves an earlier Suprnova `features` table into Pennant's layout.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20261005_000006_features_pennant_layout"
    }
}

/// `enabled` as the earlier table stored it: a boolean, or an integer on
/// engines without one.
fn enabled(row: &sea_orm_migration::sea_orm::QueryResult) -> Result<bool, DbErr> {
    if let Ok(enabled) = row.try_get::<bool>("", "enabled") {
        return Ok(enabled);
    }
    Ok(row.try_get::<i64>("", "enabled")? != 0)
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        match upgrade_state(manager, FEATURES_TABLE, super::is_earlier_layout).await? {
            UpgradeState::Untouched | UpgradeState::Fresh => return create_details(manager).await,
            UpgradeState::Earlier => {
                set_aside(manager, FEATURES_TABLE).await?;
                create_features(manager).await?;
            }
            UpgradeState::Resume => {
                resume_set_aside(manager, FEATURES_TABLE, super::is_earlier_layout).await?;
                create_features(manager).await?;
            }
        }
        create_details(manager).await?;
        let backend = manager.get_database_backend();
        let placeholders = |count| {
            placeholder_list(backend, 1, count).map_err(|e| DbErr::Migration(e.to_string()))
        };
        move_earlier_rows(
            manager,
            FEATURES_TABLE,
            "id, name, scope_key, enabled, description, updated_by, created_at, updated_at",
            "id",
            |row| {
                let name: String = row.try_get("", "name")?;
                let scope = scope_to_stored(&row.try_get::<String>("", "scope_key")?);
                let description: Option<String> = row.try_get("", "description")?;
                let updated_by: Option<String> = row.try_get("", "updated_by")?;
                let time = |column: &str| -> Result<Option<chrono::NaiveDateTime>, DbErr> {
                    Ok(row
                        .try_get::<Option<StoredDateTime>>("", column)?
                        .map(StoredDateTime::naive_utc))
                };
                let mut writes = vec![Statement::from_sql_and_values(
                    backend,
                    format!(
                        "INSERT INTO {} (name, scope, value, created_at, updated_at) VALUES ({})",
                        quote(backend, FEATURES_TABLE),
                        placeholders(5)?
                    ),
                    vec![
                        name.clone().into(),
                        scope.clone().into(),
                        stored_value(enabled(row)?).into(),
                        time("created_at")?.into(),
                        time("updated_at")?.into(),
                    ],
                )];
                if description.is_some() || updated_by.is_some() {
                    writes.push(Statement::from_sql_and_values(
                        backend,
                        format!(
                            "INSERT INTO {} (name, scope, description, updated_by) VALUES ({})",
                            quote(backend, DETAILS_TABLE),
                            placeholders(4)?
                        ),
                        vec![
                            name.into(),
                            scope.into(),
                            description.into(),
                            updated_by.into(),
                        ],
                    ));
                }
                let id = match row.try_get::<i64>("", "id") {
                    Ok(id) => id,
                    Err(_) => i64::from(row.try_get::<i32>("", "id")?),
                };
                Ok(MovedRow {
                    key: id.into(),
                    writes,
                })
            },
        )
        .await?;
        Ok(())
    }

    /// Leaves the table in Pennant's layout: the earlier one cannot hold
    /// the rich values Pennant may have written since.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
