//! Migration creating `two_factor_rotations`, where a proven rotation of a
//! confirmed second factor waits for its new secret to be confirmed.
//!
//! [`crate::auth_flows::TwoFactor::re_enroll`] used to overwrite the
//! confirmed secret and clear `confirmed_at`, so the second factor stopped
//! gating sign-in until the new secret was confirmed, and a proof-less
//! [`crate::auth_flows::TwoFactor::enroll`] could replace the pending
//! secret. The new secret now waits in this table while the confirmed one
//! keeps gating.
//!
//! Lands separately from [`super::migration::Migration`] so a deployment
//! that already ran the earlier migrations rolls forward without data loss.

use sea_orm_migration::prelude::*;

/// Migration that creates `two_factor_rotations`.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260101_000004_create_two_factor_rotations"
    }
}

#[derive(DeriveIden)]
enum TwoFactorRotations {
    Table,
    UserId,
    Secret,
    RecoveryCodes,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(TwoFactorRotations::Table)
                    .if_not_exists()
                    .col(
                        // Bounded, so MySQL and MariaDB can index it; the
                        // facade refuses longer user ids on every engine.
                        ColumnDef::new(TwoFactorRotations::UserId)
                            .string_len(255)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(TwoFactorRotations::Secret).text().not_null())
                    .col(
                        ColumnDef::new(TwoFactorRotations::RecoveryCodes)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorRotations::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TwoFactorRotations::Table).to_owned())
            .await
    }
}
