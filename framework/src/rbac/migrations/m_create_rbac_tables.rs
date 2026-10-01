//! Migration creating the framework-owned RBAC tables.

use sea_orm_migration::prelude::*;

use crate::database::migration_guard::create_index_if_missing;

/// Migration that creates roles, permissions, and polymorphic assignments.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260610_000001_create_rbac_tables"
    }
}

#[derive(DeriveIden)]
enum Roles {
    Table,
    Id,
    Name,
    DisplayName,
    GuardName,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Permissions {
    Table,
    Id,
    Name,
    DisplayName,
    GuardName,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum RolePermissions {
    Table,
    Id,
    RoleId,
    PermissionId,
}

#[derive(DeriveIden)]
enum ModelRoles {
    Table,
    Id,
    ModelType,
    ModelId,
    RoleId,
}

#[derive(DeriveIden)]
enum ModelPermissions {
    Table,
    Id,
    ModelType,
    ModelId,
    PermissionId,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Roles::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Roles::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Roles::Name).string_len(255).not_null())
                    .col(ColumnDef::new(Roles::DisplayName).string_len(255).null())
                    .col(
                        ColumnDef::new(Roles::GuardName)
                            .string_len(255)
                            .not_null()
                            .default("web"),
                    )
                    .col(
                        ColumnDef::new(Roles::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Roles::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;
        create_index_if_missing(
            manager,
            "roles",
            Index::create()
                .name("idx_roles_name_guard_name")
                .col(Roles::Name)
                .col(Roles::GuardName)
                .unique()
                .to_owned(),
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(Permissions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Permissions::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Permissions::Name).string_len(255).not_null())
                    .col(
                        ColumnDef::new(Permissions::DisplayName)
                            .string_len(255)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Permissions::GuardName)
                            .string_len(255)
                            .not_null()
                            .default("web"),
                    )
                    .col(
                        ColumnDef::new(Permissions::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Permissions::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;
        create_index_if_missing(
            manager,
            "permissions",
            Index::create()
                .name("idx_permissions_name_guard_name")
                .col(Permissions::Name)
                .col(Permissions::GuardName)
                .unique()
                .to_owned(),
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(RolePermissions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RolePermissions::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(RolePermissions::RoleId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RolePermissions::PermissionId)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        create_index_if_missing(
            manager,
            "role_permissions",
            Index::create()
                .name("idx_role_permissions_role_permission")
                .col(RolePermissions::RoleId)
                .col(RolePermissions::PermissionId)
                .unique()
                .to_owned(),
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(ModelRoles::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ModelRoles::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ModelRoles::ModelType)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ModelRoles::ModelId)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(ColumnDef::new(ModelRoles::RoleId).big_integer().not_null())
                    .to_owned(),
            )
            .await?;
        create_index_if_missing(
            manager,
            "model_roles",
            Index::create()
                .name("idx_model_roles_model_role")
                .col(ModelRoles::ModelType)
                .col(ModelRoles::ModelId)
                .col(ModelRoles::RoleId)
                .unique()
                .to_owned(),
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(ModelPermissions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ModelPermissions::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ModelPermissions::ModelType)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ModelPermissions::ModelId)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ModelPermissions::PermissionId)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        create_index_if_missing(
            manager,
            "model_permissions",
            Index::create()
                .name("idx_model_permissions_model_permission")
                .col(ModelPermissions::ModelType)
                .col(ModelPermissions::ModelId)
                .col(ModelPermissions::PermissionId)
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ModelPermissions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(ModelRoles::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(RolePermissions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Permissions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Roles::Table).to_owned())
            .await
    }
}
