//! Canonical application and Magnetar user table.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AppUsers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppUsers::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AppUsers::Email)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(AppUsers::Name).string().null())
                    .col(ColumnDef::new(AppUsers::PasswordHash).string().null())
                    .col(ColumnDef::new(AppUsers::RememberToken).string().null())
                    .col(ColumnDef::new(AppUsers::EmailVerifiedAt).timestamp().null())
                    .col(ColumnDef::new(AppUsers::LockedAt).timestamp().null())
                    .col(
                        ColumnDef::new(AppUsers::AuthEpoch)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AppUsers::SessionVersion)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AppUsers::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(ColumnDef::new(AppUsers::UpdatedAt).timestamp().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AppUsers::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum AppUsers {
    Table,
    Id,
    Email,
    Name,
    PasswordHash,
    RememberToken,
    EmailVerifiedAt,
    LockedAt,
    AuthEpoch,
    SessionVersion,
    UpdatedAt,
    CreatedAt,
}
