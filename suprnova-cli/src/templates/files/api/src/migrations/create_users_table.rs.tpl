//! The application's own view of a user.
//!
//! Deliberately NOT named `users`: Torii owns that table and creates it
//! with `.if_not_exists()`, a string primary key, and its own credential
//! columns. Both run against the same connection, so sharing the name
//! meant whichever migration ran first silently suppressed the other and
//! `POST /api/auth/register` failed on a missing Torii column. These are
//! two different tables with irreconcilable schemas — this one holds
//! profile data you join against, Torii's holds credentials.

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
                    .col(
                        ColumnDef::new(AppUsers::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
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
    CreatedAt,
}
