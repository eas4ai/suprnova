//! The binary's own `crate::migrations::Migrator`, laid out the way an
//! application lays out `src/migrations/mod.rs`, so `test_database!()` with
//! no argument has the migrator its documentation names.

use sea_orm_migration::MigrationName;
use sea_orm_migration::prelude::*;

/// The application-convention migrator: one table, `macro_probes`.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateMacroProbes)]
    }
}

struct CreateMacroProbes;

impl MigrationName for CreateMacroProbes {
    fn name(&self) -> &str {
        "m20261004_000001_create_macro_probes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateMacroProbes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MacroProbes::Table)
                    .col(
                        ColumnDef::new(MacroProbes::Id)
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MacroProbes::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum MacroProbes {
    Table,
    Id,
}
