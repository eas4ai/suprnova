//! Canonical application and Magnetar user table.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
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
                    .col(utc_time(AppUsers::EmailVerifiedAt, backend).null())
                    .col(utc_time(AppUsers::LockedAt, backend).null())
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
                        utc_time(AppUsers::CreatedAt, backend)
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(utc_time(AppUsers::UpdatedAt, backend).null())
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

/// A time column the `User` model and Magnetar both read as
/// `DateTime<Utc>`: `timestamp with time zone` on Postgres, the only type
/// that decodes as `DateTime<Utc>` there; DATETIME on MySQL and MariaDB,
/// which holds dates past 2038-01-19 where TIMESTAMP stops; text on SQLite.
fn utc_time(column: AppUsers, backend: DbBackend) -> ColumnDef {
    let mut def = ColumnDef::new(column);
    match backend {
        DbBackend::MySql => def.date_time(),
        _ => def.timestamp_with_time_zone(),
    };
    def
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
