//! Pending rotations of a confirmed second factor.
//!
//! [`super::TwoFactor::re_enroll`] proves the confirmed secret and mints a
//! new one. The new secret waits in `two_factor_rotations` until
//! [`super::TwoFactor::confirm`] proves it. Meanwhile the confirmed secret
//! in `two_factor_credentials` keeps gating sign-in, so a rotation nobody
//! finishes never leaves the account without a second factor, and
//! [`super::TwoFactor::enroll`], which takes no proof, finds the row
//! confirmed and cannot replace anything.

use sea_orm::sea_query::Expr;
use sea_orm::{ActiveValue::Set, ColumnTrait, DbErr, EntityTrait, QueryFilter, TransactionTrait};

use super::entity as credentials;
use crate::crypto::Crypt;
use crate::database::DB;
use crate::error::FrameworkError;

mod pending {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "two_factor_rotations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        #[sea_orm(column_type = "Text")]
        pub secret: String,
        #[sea_orm(column_type = "Text")]
        pub recovery_codes: String,
        pub created_at: chrono::DateTime<chrono::Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A rotation waiting for its new secret to be confirmed.
pub(super) struct PendingRotation {
    /// The stored ciphertext, which names this rotation exactly.
    pub(super) ciphertext: String,
    /// The decrypted base32 secret.
    pub(super) secret_b32: String,
}

/// Whether a database error reports the rotation table as missing.
fn names_missing_table(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("two_factor_rotations")
        && (message.contains("no such table")
            || message.contains("does not exist")
            || message.contains("doesn't exist"))
}

/// A store failure answers 503. A missing table gets a log line naming the
/// migration that creates it.
fn unavailable(error: impl std::fmt::Display) -> FrameworkError {
    let message = error.to_string();
    if names_missing_table(&message) {
        tracing::error!(
            error = %message,
            "the two_factor_rotations table is missing: add \
             suprnova::auth_flows::two_factor::migration_rotation::Migration to the \
             application's migrator and run the migrations; until then re_enroll \
             answers 503"
        );
    } else {
        tracing::error!(error = %message, "two-factor rotation store failed");
    }
    FrameworkError::domain("two-factor rotation store unavailable", 503)
}

/// Store a proven rotation, replacing any earlier one of the same user.
pub(super) async fn store(
    user_id: &str,
    encrypted_secret: String,
    encrypted_recovery: String,
) -> Result<(), FrameworkError> {
    let db = DB::connection().map_err(unavailable)?;
    let transaction = db.inner().begin().await.map_err(unavailable)?;
    let outcome = async {
        pending::Entity::delete_by_id(user_id.to_owned())
            .exec(&transaction)
            .await?;
        pending::Entity::insert(pending::ActiveModel {
            user_id: Set(user_id.to_owned()),
            secret: Set(encrypted_secret),
            recovery_codes: Set(encrypted_recovery),
            created_at: Set(crate::clock::now()),
        })
        .exec_without_returning(&transaction)
        .await?;
        Ok::<_, DbErr>(())
    }
    .await;
    match outcome {
        Ok(()) => transaction.commit().await.map_err(unavailable),
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(unavailable(error))
        }
    }
}

/// The user's pending rotation, if one waits. Without the table no rotation
/// can have been stored, so a missing table reads as none.
pub(super) async fn find(user_id: &str) -> Result<Option<PendingRotation>, FrameworkError> {
    let db = DB::connection()?;
    let row = match pending::Entity::find_by_id(user_id.to_owned())
        .one(db.inner())
        .await
    {
        Ok(row) => row,
        Err(error) if names_missing_table(&error.to_string()) => return Ok(None),
        Err(error) => return Err(unavailable(error)),
    };
    let Some(row) = row else {
        return Ok(None);
    };
    let secret_b32 =
        Crypt::decrypt_string(crate::crypto::CryptPurpose::TwoFactorSecret, &row.secret)?;
    Ok(Some(PendingRotation {
        ciphertext: row.secret,
        secret_b32,
    }))
}

/// Forget the user's pending rotation. A missing table holds none.
pub(super) async fn discard(user_id: &str) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    match pending::Entity::delete_by_id(user_id.to_owned())
        .exec(db.inner())
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if names_missing_table(&error.to_string()) => Ok(()),
        Err(error) => Err(unavailable(error)),
    }
}

/// Finish the rotation whose secret a code was just checked against: the
/// new secret and recovery codes replace the confirmed ones, confirmed now,
/// and the code's window is claimed. Returns `false`, changing nothing,
/// when the rotation or the confirmed secret it replaces changed since they
/// were read.
pub(super) async fn promote(
    user_id: &str,
    confirmed_ciphertext: &str,
    rotation_ciphertext: &str,
    current_timestep: i64,
    when: chrono::DateTime<chrono::Utc>,
) -> Result<bool, FrameworkError> {
    let db = DB::connection().map_err(unavailable)?;
    let transaction = db.inner().begin().await.map_err(unavailable)?;
    let outcome = async {
        let Some(rotation) = pending::Entity::find_by_id(user_id.to_owned())
            .filter(pending::Column::Secret.eq(rotation_ciphertext))
            .one(&transaction)
            .await?
        else {
            return Ok(false);
        };
        let replaced = credentials::Entity::update_many()
            .col_expr(credentials::Column::Secret, Expr::value(rotation.secret))
            .col_expr(
                credentials::Column::RecoveryCodes,
                Expr::value(Some(rotation.recovery_codes)),
            )
            .col_expr(credentials::Column::ConfirmedAt, Expr::value(Some(when)))
            .col_expr(
                credentials::Column::LastUsedTimestep,
                Expr::value(current_timestep + super::TOTP_SKEW_STEPS),
            )
            .col_expr(credentials::Column::UpdatedAt, Expr::value(when))
            .filter(credentials::Column::UserId.eq(user_id))
            .filter(credentials::Column::Secret.eq(confirmed_ciphertext))
            .filter(credentials::Column::ConfirmedAt.is_not_null())
            .exec(&transaction)
            .await?;
        if replaced.rows_affected != 1 {
            return Ok(false);
        }
        let removed = pending::Entity::delete_many()
            .filter(pending::Column::UserId.eq(user_id))
            .filter(pending::Column::Secret.eq(rotation_ciphertext))
            .exec(&transaction)
            .await?;
        Ok::<_, DbErr>(removed.rows_affected == 1)
    }
    .await;
    match outcome {
        Ok(true) => {
            transaction.commit().await.map_err(unavailable)?;
            Ok(true)
        }
        Ok(false) => {
            let _ = transaction.rollback().await;
            Ok(false)
        }
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(unavailable(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::names_missing_table;

    #[test]
    fn missing_table_wordings_of_each_engine_are_recognized() {
        for message in [
            "error returned from database: (code: 1) no such table: two_factor_rotations",
            "error returned from database: relation \"two_factor_rotations\" does not exist",
            "error returned from database: 1146 (42S02): Table 'app.two_factor_rotations' doesn't exist",
        ] {
            assert!(names_missing_table(message), "{message}");
        }
        assert!(!names_missing_table("no such table: two_factor_attempts"));
    }
}
