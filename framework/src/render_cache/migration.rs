//! Durable generation truth: current generations, an append-only change log,
//! and the authority epoch.
//!
//! The identity column holds the lowercase hex of a
//! [`DependencyIdentity`](super::DependencyIdentity)'s 32-byte digest, never
//! raw binary: a fixed 64-character hex string
//! compares and indexes identically on Postgres, MySQL, and SQLite, and it
//! is the exact wire form [`GenerationSet`](suprnova_live::render_cache::GenerationSet)
//! already uses. `string_len(64)` is sized explicitly rather than left at
//! SeaORM's default `.string()` (`VARCHAR(255)`): under `utf8mb4` that
//! default is 1020 bytes, which runs into MySQL's index key length limit on
//! a primary key column.
//!
//! Consumer apps include this migration in their `Migrator`'s
//! `migrations()` list - the framework owns the schema, the app owns when
//! to apply it.

use sea_orm_migration::prelude::*;

/// Creates the three `suprnova_render_` tables.
pub struct Migration;

impl MigrationName for Migration {
    // Explicit, file-stable name. `DeriveMigrationName` derives from the
    // file stem (`file!()`), which for this file is just "migration" - not
    // unique enough once a second framework-owned migration lands in
    // another module also named `migration.rs` (the two-factor migration
    // already is one; it explains the same collision risk for the same
    // reason). Matches that convention.
    fn name(&self) -> &str {
        "m20260903_000000_create_render_cache_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Generations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Generations::Identity)
                            .string_len(64)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Generations::Generation)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Generations::Epoch).big_integer().not_null())
                    .col(
                        ColumnDef::new(Generations::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(GenerationLog::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GenerationLog::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::Identity)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::Generation)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::Epoch)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::CommittedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("suprnova_render_generation_log_identity")
                    .table(GenerationLog::Table)
                    .col(GenerationLog::Identity)
                    .col(GenerationLog::Id)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Epochs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Epochs::Singleton)
                            .small_integer()
                            .not_null()
                            .primary_key()
                            .check(Expr::col(Epochs::Singleton).eq(1_i16)),
                    )
                    .col(ColumnDef::new(Epochs::Epoch).big_integer().not_null())
                    .to_owned(),
            )
            .await?;
        let insert = Query::insert()
            .into_table(Epochs::Table)
            .columns([Epochs::Singleton, Epochs::Epoch])
            .values_panic([1_i16.into(), 1_i64.into()])
            .to_owned();
        manager.exec_stmt(insert).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Epochs::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(GenerationLog::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Generations::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Generations {
    #[sea_orm(iden = "suprnova_render_generations")]
    Table,
    Identity,
    Generation,
    Epoch,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum GenerationLog {
    #[sea_orm(iden = "suprnova_render_generation_log")]
    Table,
    Id,
    Identity,
    Generation,
    Epoch,
    CommittedAt,
}

#[derive(DeriveIden)]
enum Epochs {
    #[sea_orm(iden = "suprnova_render_epochs")]
    Table,
    Singleton,
    Epoch,
}
