//! Migration that creates the `features` table in laravel/pennant's
//! layout, consumed by [`crate::features::DatabaseEvaluator`] and the
//! admin facade, and the framework's `suprnova_feature_details` beside it.
//!
//! Schema:
//!
//! ```text
//! features (
//!   id          BIGINT       PRIMARY KEY AUTO_INCREMENT (unsigned on MySQL)
//!   name        VARCHAR(255) NOT NULL
//!   scope       VARCHAR(255) NOT NULL   -- '__laravel_null' = global
//!   value       TEXT         NOT NULL   -- JSON: true, false, "blue", ...
//!   created_at  DATETIME     NULL       -- timestamp(0) on Postgres
//!   updated_at  DATETIME     NULL
//!   UNIQUE INDEX features_name_scope_unique (name, scope)
//! )
//!
//! suprnova_feature_details (
//!   name        VARCHAR(255) NOT NULL
//!   scope       VARCHAR(255) NOT NULL
//!   description TEXT         NULL
//!   updated_by  VARCHAR(255) NULL
//!   PRIMARY KEY (name, scope)
//! )
//! ```
//!
//! The time columns are `DATETIME` on MySQL and MariaDB, where Pennant's
//! migration says `TIMESTAMP`: `TIMESTAMP` refuses any time after
//! 2038-01-19.
//!
//! A `features` table that already exists is left exactly as it is: one
//! Pennant created stays as Pennant left it, and one an earlier version of
//! this migration created is reshaped by
//! [`FeaturesToPennantLayout`](super::FeaturesToPennantLayout). The details
//! table is created whenever it is missing.
//!
//! Consumer apps include this migration in their `Migrator`'s
//! `migrations()` list - the framework owns the schema, the app owns
//! when to apply it.

use sea_orm_migration::prelude::*;

use crate::features::store::{DETAILS_TABLE, FEATURES_TABLE};
use crate::schema::Schema;

/// Migration that creates the `features` table.
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

/// Create `features` in Pennant's layout.
pub(crate) async fn create_features(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    Schema::create(manager, FEATURES_TABLE, |t| {
        t.unsigned_id();
        t.string("name");
        t.string("scope");
        t.text("value");
        t.date_time("created_at").precision(0).nullable();
        t.date_time("updated_at").precision(0).nullable();
        t.unique(&["name", "scope"]);
    })
    .await
}

/// Create `suprnova_feature_details` unless it exists.
pub(crate) async fn create_details(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    if manager.has_table(DETAILS_TABLE).await? {
        return Ok(());
    }
    Schema::create(manager, DETAILS_TABLE, |t| {
        t.string("name");
        t.string("scope");
        t.text("description").nullable();
        t.string("updated_by").nullable();
        t.primary(&["name", "scope"]);
    })
    .await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_table(FEATURES_TABLE).await? {
            create_features(manager).await?;
        }
        create_details(manager).await
    }

    /// Drops the framework's details table and leaves `features`: a table
    /// Pennant created looks the same as the one this migration creates,
    /// and rolling back must not drop Pennant's flags with it.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new(DETAILS_TABLE))
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}
