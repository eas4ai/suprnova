//! Add ranking invalidation without rewriting migrations an application already ran.
use sea_orm_migration::prelude::*;

/// Empty digests on existing runs force regeneration after an upgrade, while the
/// starter reader enforces the current peer minimum before that regeneration runs.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("recommendation_runs"))
                    .add_column(
                        ColumnDef::new(Alias::new("ranking_digest"))
                            .string_len(64)
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("recommendation_runs"))
                    .drop_column(Alias::new("ranking_digest"))
                    .to_owned(),
            )
            .await
    }
}
