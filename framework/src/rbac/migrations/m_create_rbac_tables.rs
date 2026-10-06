//! Migration creating the RBAC tables in spatie/laravel-permission's
//! layout.
//!
//! `roles` and `permissions` hold `id`, `name`, `guard_name` and nullable
//! timestamps, unique on `(name, guard_name)`. The assignments live in
//! `model_has_roles` and `model_has_permissions` (`model_type`, `model_id`
//! and the role or permission, all three the primary key) and
//! `role_has_permissions`, with spatie's indexes and cascading foreign
//! keys. `model_id` follows `RBAC_MODEL_KEY` (`int`, the default, for
//! spatie's big integer; `uuid` or `ulid` for those keys). The framework's
//! own `suprnova_role_details` and `suprnova_permission_details` hold the
//! display names an earlier release kept.
//!
//! The time columns are `DATETIME` on MySQL, where spatie's migration says
//! `TIMESTAMP`: `TIMESTAMP` refuses any time after 2038-01-19.
//!
//! A table that already exists is left exactly as it is: one spatie
//! created stays as spatie left it, and one an earlier version of this
//! migration created is reshaped by
//! [`RbacToSpatieLayout`](super::RbacToSpatieLayout).

use sea_orm_migration::prelude::*;

use crate::schema::Schema;

/// Migration that creates roles, permissions, and polymorphic assignments.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260610_000001_create_rbac_tables"
    }
}

/// The key type of the models roles and permissions are assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelKey {
    /// `int`: spatie's `unsignedBigInteger`.
    Int,
    /// `uuid`: a UUID column.
    Uuid,
    /// `ulid`: a ULID column.
    Ulid,
}

/// Read `RBAC_MODEL_KEY`: `int` when unset.
pub(crate) fn model_key() -> Result<ModelKey, DbErr> {
    match std::env::var("RBAC_MODEL_KEY") {
        Err(_) => Ok(ModelKey::Int),
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "" | "int" => Ok(ModelKey::Int),
            "uuid" => Ok(ModelKey::Uuid),
            "ulid" => Ok(ModelKey::Ulid),
            _ => Err(DbErr::Migration(format!(
                "RBAC_MODEL_KEY={raw:?} is not one of int, uuid or ulid"
            ))),
        },
    }
}

/// Create `roles` or `permissions` in spatie's layout.
pub(crate) async fn create_named(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    if manager.has_table(table).await? {
        return Ok(());
    }
    Schema::create(manager, table, |t| {
        t.unsigned_id();
        t.string("name");
        t.string("guard_name");
        t.date_time("created_at").precision(0).nullable();
        t.date_time("updated_at").precision(0).nullable();
        t.unique(&["name", "guard_name"]);
    })
    .await
}

/// Create `model_has_roles` or `model_has_permissions` in spatie's layout:
/// `owner` is `role_id` or `permission_id`, referencing `owner_table`.
pub(crate) async fn create_model_has(
    manager: &SchemaManager<'_>,
    table: &str,
    owner: &str,
    owner_table: &str,
    key: ModelKey,
) -> Result<(), DbErr> {
    if manager.has_table(table).await? {
        return Ok(());
    }
    Schema::create(manager, table, |t| {
        t.unsigned_big_integer(owner);
        t.string("model_type");
        match key {
            ModelKey::Int => t.unsigned_big_integer("model_id"),
            ModelKey::Uuid => t.uuid("model_id"),
            ModelKey::Ulid => t.ulid("model_id"),
        };
        t.index(&["model_id", "model_type"]);
        t.foreign(owner)
            .constrained(owner_table)
            .cascade_on_delete();
        t.primary(&[owner, "model_id", "model_type"]);
    })
    .await
}

/// Create `role_has_permissions` in spatie's layout.
pub(crate) async fn create_role_has_permissions(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    if manager.has_table("role_has_permissions").await? {
        return Ok(());
    }
    Schema::create(manager, "role_has_permissions", |t| {
        t.unsigned_big_integer("permission_id");
        t.unsigned_big_integer("role_id");
        t.foreign("permission_id")
            .constrained("permissions")
            .cascade_on_delete();
        t.foreign("role_id")
            .constrained("roles")
            .cascade_on_delete();
        t.primary(&["permission_id", "role_id"]);
    })
    .await
}

/// Create a details table (`suprnova_role_details` keyed by `role_id`, or
/// `suprnova_permission_details` keyed by `permission_id`).
pub(crate) async fn create_details(
    manager: &SchemaManager<'_>,
    table: &str,
    key: &str,
) -> Result<(), DbErr> {
    if manager.has_table(table).await? {
        return Ok(());
    }
    Schema::create(manager, table, |t| {
        t.unsigned_big_integer(key).primary();
        t.string("display_name").nullable();
    })
    .await
}

/// Every table this migration describes, spatie's created only where
/// missing. A table in the earlier layout counts as present and is left to
/// [`RbacToSpatieLayout`](super::RbacToSpatieLayout).
pub(crate) async fn create_all(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let key = model_key()?;
    create_named(manager, "permissions").await?;
    create_named(manager, "roles").await?;
    // An earlier release's tables keep their assignments in tables of
    // other names; spatie's are created only once that layout is gone.
    if !crate::rbac::migrations::is_earlier_layout(manager).await? {
        create_model_has(
            manager,
            "model_has_permissions",
            "permission_id",
            "permissions",
            key,
        )
        .await?;
        create_model_has(manager, "model_has_roles", "role_id", "roles", key).await?;
        create_role_has_permissions(manager).await?;
    }
    create_details(manager, "suprnova_role_details", "role_id").await?;
    create_details(manager, "suprnova_permission_details", "permission_id").await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_all(manager).await
    }

    /// Drops the framework's details tables and leaves spatie's: a table
    /// spatie created looks the same as the one this migration creates, and
    /// rolling back must not drop its roles and assignments with it.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["suprnova_permission_details", "suprnova_role_details"] {
            manager
                .drop_table(
                    Table::drop()
                        .table(Alias::new(table))
                        .if_exists()
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
