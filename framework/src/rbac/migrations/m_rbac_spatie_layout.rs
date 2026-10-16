//! Upgrade migration that moves the RBAC tables an earlier version of
//! [`CreateRbacTables`](super::CreateRbacTables) created into
//! spatie/laravel-permission's layout, with their rows.
//!
//! The earlier `roles` and `permissions` carried a `display_name`, and the
//! assignments lived in `model_roles`, `model_permissions` and
//! `role_permissions`. Every role and permission keeps its id, name, guard
//! and times; its display name moves to `suprnova_role_details` or
//! `suprnova_permission_details`. Every assignment moves to
//! `model_has_roles`, `model_has_permissions` or `role_has_permissions`,
//! and the earlier assignment tables are dropped once empty. No role,
//! permission or assignment is lost. A model id that `RBAC_MODEL_KEY`'s
//! form cannot hold (text under `int`) stops the migration with the row
//! named, before any table changes; set `RBAC_MODEL_KEY` to the form the
//! application's keys take and run it again.
//!
//! Tables spatie created, missing tables, and tables already in spatie's
//! layout are left alone.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{QueryResult, Statement, Value};

use super::m_create_rbac_tables::{
    ModelKey, create_details, create_model_has, create_named, create_role_has_permissions,
    model_key,
};
use crate::database::migration_guard::{
    MovedRow, UpgradeState, advance_id_sequence, first_misfit, move_earlier_rows, move_rows, quote,
    resume_set_aside, set_aside, upgrade_state,
};
use crate::database::placeholder::placeholder_list;
use crate::database::stored_datetime::StoredDateTime;

/// Moves the earlier Suprnova RBAC tables into spatie's layout.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20261005_000007_rbac_spatie_layout"
    }
}

fn earlier_named(columns: &[crate::database::catalog::CatalogColumn]) -> bool {
    crate::database::catalog::column(columns, "display_name").is_some()
}

fn id_of(row: &QueryResult, column: &str) -> Result<i64, DbErr> {
    if let Ok(id) = row.try_get::<i64>("", column) {
        return Ok(id);
    }
    if let Ok(id) = row.try_get::<i32>("", column) {
        return Ok(i64::from(id));
    }
    let id = row.try_get::<u64>("", column)?;
    <i64 as TryFrom<u64>>::try_from(id)
        .map_err(|_| DbErr::Migration(format!("{column} {id} is out of range")))
}

fn statement(
    backend: sea_orm_migration::sea_orm::DbBackend,
    table: &str,
    columns: &[&str],
    values: Vec<Value>,
) -> Result<Statement, DbErr> {
    let placeholders =
        placeholder_list(backend, 1, columns.len()).map_err(|e| DbErr::Migration(e.to_string()))?;
    Ok(Statement::from_sql_and_values(
        backend,
        format!(
            "INSERT INTO {} ({}) VALUES ({placeholders})",
            quote(backend, table),
            columns.join(", ")
        ),
        values,
    ))
}

/// Move the earlier `roles` or `permissions` into spatie's layout, with
/// its ids, and its display names into `details`.
async fn move_named(
    manager: &SchemaManager<'_>,
    table: &str,
    details: &str,
    details_key: &str,
) -> Result<(), DbErr> {
    match upgrade_state(manager, table, earlier_named).await? {
        UpgradeState::Untouched | UpgradeState::Fresh => return Ok(()),
        UpgradeState::Earlier => {
            set_aside(manager, table).await?;
            create_named(manager, table).await?;
        }
        UpgradeState::Resume => {
            resume_set_aside(manager, table, earlier_named).await?;
            create_named(manager, table).await?;
        }
    }
    create_details(manager, details, details_key).await?;
    let backend = manager.get_database_backend();
    move_earlier_rows(
        manager,
        table,
        "id, name, guard_name, display_name, created_at, updated_at",
        "id",
        |row| {
            let id = id_of(row, "id")?;
            let time = |column: &str| -> Result<Option<chrono::NaiveDateTime>, DbErr> {
                Ok(row
                    .try_get::<Option<StoredDateTime>>("", column)?
                    .map(StoredDateTime::naive_utc))
            };
            let mut writes = vec![statement(
                backend,
                table,
                &["id", "name", "guard_name", "created_at", "updated_at"],
                vec![
                    id.into(),
                    row.try_get::<String>("", "name")?.into(),
                    row.try_get::<String>("", "guard_name")?.into(),
                    time("created_at")?.into(),
                    time("updated_at")?.into(),
                ],
            )?];
            let display_name: Option<String> = row.try_get("", "display_name")?;
            if display_name.is_some() {
                writes.push(statement(
                    backend,
                    details,
                    &[details_key, "display_name"],
                    vec![id.into(), display_name.into()],
                )?);
            }
            Ok(MovedRow {
                key: id.into(),
                writes,
            })
        },
    )
    .await?;
    advance_id_sequence(manager, table).await
}

/// `model_id` as the chosen form takes it, or an error naming the row.
fn model_id_value(
    backend: sea_orm_migration::sea_orm::DbBackend,
    key: ModelKey,
    table: &str,
    row_id: i64,
    model_id: &str,
) -> Result<Value, DbErr> {
    match key {
        ModelKey::Int => model_id
            .parse::<i64>()
            .map(Into::into)
            .map_err(|_| int_refusal(table, &row_id.to_string(), model_id)),
        ModelKey::Uuid | ModelKey::Ulid => {
            Ok(crate::database::morph_key::uuid_value(backend, model_id))
        }
    }
}

/// The refusal for a model id `RBAC_MODEL_KEY=int` cannot hold, naming the
/// row and the setting that would.
fn int_refusal(table: &str, row_id: &str, model_id: &str) -> DbErr {
    DbErr::Migration(format!(
        "{table} row {row_id} assigns to model_id {model_id:?}, which RBAC_MODEL_KEY=int \
         cannot hold; set RBAC_MODEL_KEY to uuid or ulid and run migrate again"
    ))
}

/// The earlier assignment tables and, for each, spatie's table its rows
/// move to and the column naming the role or permission.
const ASSIGNMENTS: [(&str, &str, &str, &str); 2] = [
    (
        "model_permissions",
        "model_has_permissions",
        "permission_id",
        "permissions",
    ),
    ("model_roles", "model_has_roles", "role_id", "roles"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let key = model_key()?;
        // Every model id the earlier assignments hold has to fit the chosen
        // form before any table changes: a refusal part way would leave
        // spatie's tables created in a form the corrected setting does not
        // make.
        if key == ModelKey::Int {
            for (source, ..) in ASSIGNMENTS {
                if manager.has_table(source).await?
                    && let Some((row_id, model_id)) =
                        first_misfit(manager, source, "id", true, "model_id", |value| {
                            value.parse::<i64>().is_ok()
                        })
                        .await?
                {
                    return Err(int_refusal(source, &row_id, &model_id));
                }
            }
        }
        move_named(
            manager,
            "permissions",
            "suprnova_permission_details",
            "permission_id",
        )
        .await?;
        move_named(manager, "roles", "suprnova_role_details", "role_id").await?;
        // While the earlier assignments remain, spatie's tables are the ones
        // this upgrade creates, and a stopped run may have left one without
        // its index; otherwise one that exists is spatie's own.
        for (source, target, owner, owner_table) in ASSIGNMENTS {
            if manager.has_table(source).await? || !manager.has_table(target).await? {
                create_model_has(manager, target, owner, owner_table, key).await?;
            }
        }
        create_role_has_permissions(manager).await?;

        let backend = manager.get_database_backend();
        for (source, target, owner, _) in ASSIGNMENTS {
            if !manager.has_table(source).await? {
                continue;
            }
            move_rows(
                manager,
                source,
                &format!("id, model_type, model_id, {owner}"),
                "id",
                |row| {
                    let row_id = id_of(row, "id")?;
                    let model_id: String = row.try_get("", "model_id")?;
                    Ok(MovedRow {
                        key: row_id.into(),
                        writes: vec![statement(
                            backend,
                            target,
                            &[owner, "model_type", "model_id"],
                            vec![
                                id_of(row, owner)?.into(),
                                row.try_get::<String>("", "model_type")?.into(),
                                model_id_value(backend, key, source, row_id, &model_id)?,
                            ],
                        )?],
                    })
                },
            )
            .await?;
        }
        if manager.has_table("role_permissions").await? {
            move_rows(
                manager,
                "role_permissions",
                "id, role_id, permission_id",
                "id",
                |row| {
                    Ok(MovedRow {
                        key: id_of(row, "id")?.into(),
                        writes: vec![statement(
                            backend,
                            "role_has_permissions",
                            &["permission_id", "role_id"],
                            vec![
                                id_of(row, "permission_id")?.into(),
                                id_of(row, "role_id")?.into(),
                            ],
                        )?],
                    })
                },
            )
            .await?;
        }
        Ok(())
    }

    /// Leaves the tables in spatie's layout: the earlier one cannot hold
    /// what spatie may have written since.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
