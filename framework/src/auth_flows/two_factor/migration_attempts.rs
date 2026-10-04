//! Migration creating `two_factor_attempts`, the brute-force counter of the
//! [`crate::auth_flows::TwoFactor`] facade.
//!
//! Second-factor failures need a counter of their own. Kept on the shared
//! per-email password counter, a successful password check cleared them,
//! so an attacker who knew the password could alternate a few wrong codes
//! with one correct password and guess codes forever. A framework-owned
//! table also works with no Magnetar engine installed.
//!
//! Lands separately from [`super::migration::Migration`] so a deployment
//! that already ran the earlier migrations rolls forward without data loss.

use sea_orm_migration::prelude::*;

/// Migration that creates `two_factor_attempts`.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260101_000003_create_two_factor_attempts"
    }
}

#[derive(DeriveIden)]
enum TwoFactorAttempts {
    Table,
    Id,
    UserId,
    AttemptedAt,
    Failed,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(TwoFactorAttempts::Table)
                    .if_not_exists()
                    // Bounded strings rather than TEXT: MySQL cannot index
                    // TEXT columns without a prefix length.
                    .col(
                        ColumnDef::new(TwoFactorAttempts::Id)
                            .string_len(64)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorAttempts::UserId)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorAttempts::AttemptedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorAttempts::Failed)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_two_factor_attempts_user_id")
                    .table(TwoFactorAttempts::Table)
                    .col(TwoFactorAttempts::UserId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TwoFactorAttempts::Table).to_owned())
            .await
    }
}
