//! Migration that creates the framework-owned `features` table consumed
//! by [`crate::features::DatabaseEvaluator`].
//!
//! Schema:
//!
//! ```text
//! features (
//!   id          BIGINT      PRIMARY KEY AUTO_INCREMENT
//!   name        VARCHAR(255) NOT NULL
//!   scope_key   VARCHAR(255) NOT NULL DEFAULT ''  -- '' = global; "user:42" / "team:staff" etc.
//!   enabled     BOOLEAN     NOT NULL
//!   description TEXT
//!   updated_by  VARCHAR(255)                      -- audit: which user toggled it (opaque id, string-typed for UUID/ULID support)
//!   created_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP  -- DATETIME on MySQL
//!   updated_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP  -- DATETIME on MySQL
//!   UNIQUE INDEX (name, scope_key)
//! )
//! ```
//!
//! The time columns are `DATETIME` on MySQL and MariaDB, whose `TIMESTAMP`
//! refuses any time after 2038-01-19. A table this migration created before
//! has `TIMESTAMP` there;
//! [`FeatureTimestampsToDatetime`](super::FeatureTimestampsToDatetime)
//! converts it.
//!
//! Consumer apps include this migration in their `Migrator`'s
//! `migrations()` list - the framework owns the schema, the app owns
//! when to apply it. The example app wires this in Task 7.

use sea_orm_migration::prelude::*;

use crate::database::migration_guard::{create_index_if_missing, utc_timestamp_column};

/// Migration that creates the framework-owned `features` table.
///
/// Re-exported as `CreateFeaturesTable` from the parent migrations module
/// so consumer apps can list it in their `Migrator::migrations()`.
pub struct Migration;

impl MigrationName for Migration {
    // Explicit, date-prefixed name. `DeriveMigrationName` would derive
    // from the parent module path (`m_create_features_table`) which is
    // unique by chance today but offers no protection against later
    // framework migrations colliding on path. Matches the convention
    // used by the 2FA migrations (Phase 11).
    fn name(&self) -> &str {
        "m20260101_000003_create_features_table"
    }
}

#[derive(DeriveIden)]
enum Features {
    Table,
    Id,
    Name,
    ScopeKey,
    Enabled,
    Description,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .create_table(
                Table::create()
                    .table(Features::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Features::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Features::Name).string_len(255).not_null())
                    .col(
                        ColumnDef::new(Features::ScopeKey)
                            .string_len(255)
                            .not_null()
                            .default(""),
                    )
                    .col(ColumnDef::new(Features::Enabled).boolean().not_null())
                    .col(ColumnDef::new(Features::Description).text().null())
                    .col(ColumnDef::new(Features::UpdatedBy).string_len(255).null())
                    .col(
                        utc_timestamp_column(Features::CreatedAt, backend)
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        utc_timestamp_column(Features::UpdatedAt, backend)
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        create_index_if_missing(
            manager,
            "features",
            Index::create()
                .name("idx_features_name_scope_key")
                .col(Features::Name)
                .col(Features::ScopeKey)
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Features::Table).to_owned())
            .await
    }
}
