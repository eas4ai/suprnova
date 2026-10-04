//! The brute-force counter behind every [`super::TwoFactor`] proof path.
//!
//! Each attempt is one row in `two_factor_attempts`. An attempt is reserved
//! (a pending row) before its code is read, then settled: a wrong code turns
//! the row into a failure, a right code deletes the user's failures, and a
//! proof that could not be evaluated deletes only its own row. Admission
//! counts pending and failed rows together, so parallel guesses cannot all
//! pass one status read, and a success never deletes another request's
//! pending row. Rows older than the configured window
//! ([`super::TwoFactorLockout`]) no longer count, which also retires the
//! reservation of a request that died before it settled.
//!
//! Inside a user's lock every statement either reads, inserts, or names
//! rows by primary key. On MySQL and MariaDB, under REPEATABLE READ, a
//! delete or update that selects rows by a range of the `user_id` index
//! takes gap locks that reach into neighbouring users' keys, and
//! concurrent admissions for different users then deadlock each other's
//! inserts. Expired rows are therefore deleted by id after the admission
//! commits, outside the lock.
//!
//! The counter belongs to the second factor alone. A successful password
//! check does not touch it, so wrong codes cannot be washed out by signing
//! in with the password again, and it works with no Magnetar engine
//! installed.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, QuerySelect, TransactionTrait,
};

use super::TwoFactorLockout;
use super::entity as credentials;
use crate::database::DB;
use crate::error::FrameworkError;

/// The oldest attempt time that still counts under `lockout`.
fn window_floor(
    now: chrono::DateTime<chrono::Utc>,
    lockout: TwoFactorLockout,
) -> chrono::DateTime<chrono::Utc> {
    now.checked_sub_signed(lockout.window())
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
}

mod attempt {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "two_factor_attempts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub user_id: String,
        pub attempted_at: chrono::DateTime<chrono::Utc>,
        /// `false` while the attempt is pending, `true` once it failed.
        pub failed: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// One reserved attempt, settled by exactly one of [`record_failure`],
/// [`record_success`] or [`release`].
pub(crate) struct Reservation {
    id: String,
    user_id: String,
}

/// The answer to [`admit`].
pub(crate) enum Admission {
    /// The attempt is reserved; its code may be read.
    Admitted(Reservation),
    /// The user has no attempt left in the window.
    Locked,
}

/// What [`record_failure`] did.
pub(crate) struct Failure {
    /// Failures now counting against the user.
    pub(crate) failed_attempts: u64,
    /// This failure is the one that locked the user.
    pub(crate) locked_now: bool,
}

/// Every store failure answers 503: the attempt could not be counted, so
/// the proof must not be evaluated or reported as settled.
///
/// A missing `two_factor_attempts` table gets its own log line naming the
/// migration that creates it, so an operator who upgraded without adding it
/// sees why every proof path answers 503.
fn unavailable(error: impl std::fmt::Display) -> FrameworkError {
    let message = error.to_string();
    if names_missing_attempt_table(&message) {
        tracing::error!(
            error = %message,
            "the two_factor_attempts table is missing: add \
             suprnova::auth_flows::two_factor::migration_attempts::Migration to the \
             application's migrator and run the migrations; until then every \
             two-factor proof answers 503"
        );
    } else {
        tracing::error!(error = %message, "two-factor attempt store failed");
    }
    FrameworkError::domain("two-factor attempt store unavailable", 503)
}

/// Whether a database error reports the attempt table as missing, in the
/// wording SQLite ("no such table"), PostgreSQL ("does not exist") and
/// MySQL ("doesn't exist") use.
fn names_missing_attempt_table(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("two_factor_attempts")
        && (message.contains("no such table")
            || message.contains("does not exist")
            || message.contains("doesn't exist"))
}

/// Run `work` in one transaction that holds the user's write lock.
///
/// The lock is a no-op update of the user's credentials row: a row lock on
/// PostgreSQL and MySQL, the database write lock on SQLite. Counting after
/// it, in a separate statement, sees every attempt the previous holder
/// committed.
async fn locked<T, F>(user_id: &str, work: F) -> Result<T, FrameworkError>
where
    F: for<'t> FnOnce(
        &'t DatabaseTransaction,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, DbErr>> + Send + 't>,
    >,
{
    let db = DB::connection().map_err(unavailable)?;
    let transaction = db.inner().begin().await.map_err(unavailable)?;
    let outcome = async {
        credentials::Entity::update_many()
            .col_expr(
                credentials::Column::UpdatedAt,
                Expr::col(credentials::Column::UpdatedAt),
            )
            .filter(credentials::Column::UserId.eq(user_id))
            .exec(&transaction)
            .await?;
        work(&transaction).await
    }
    .await;
    match outcome {
        Ok(value) => {
            transaction.commit().await.map_err(unavailable)?;
            Ok(value)
        }
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(unavailable(error))
        }
    }
}

/// Reserve one attempt for `user_id`, or report it locked.
///
/// Only attempts inside the window count. The expired ones are deleted
/// after the reservation commits, by id and outside the lock; a failure to
/// delete them leaves rows that no longer count.
pub(crate) async fn admit(user_id: &str) -> Result<Admission, FrameworkError> {
    let lockout = TwoFactorLockout::resolve()?;
    let owner = user_id.to_owned();
    let (admission, expired) = locked(user_id, move |transaction| {
        Box::pin(async move {
            let now = crate::clock::now();
            let floor = window_floor(now, lockout);
            let counted = attempt::Entity::find()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .filter(attempt::Column::AttemptedAt.gt(floor))
                .count(transaction)
                .await?;
            let expired = attempt_ids(
                transaction,
                attempt::Entity::find()
                    .filter(attempt::Column::UserId.eq(owner.as_str()))
                    .filter(attempt::Column::AttemptedAt.lte(floor)),
            )
            .await?;
            if counted >= u64::from(lockout.max_attempts()) {
                return Ok((Admission::Locked, expired));
            }
            let id = uuid::Uuid::new_v4().to_string();
            attempt::Entity::insert(attempt::ActiveModel {
                id: Set(id.clone()),
                user_id: Set(owner.clone()),
                attempted_at: Set(now),
                failed: Set(false),
            })
            .exec_without_returning(transaction)
            .await?;
            Ok((
                Admission::Admitted(Reservation { id, user_id: owner }),
                expired,
            ))
        })
    })
    .await?;
    purge_expired(expired).await;
    Ok(admission)
}

/// The ids of the attempts `query` selects, read without locking anything.
async fn attempt_ids(
    connection: &impl ConnectionTrait,
    query: sea_orm::Select<attempt::Entity>,
) -> Result<Vec<String>, DbErr> {
    query
        .select_only()
        .column(attempt::Column::Id)
        .into_tuple::<String>()
        .all(connection)
        .await
}

/// Delete attempt rows by primary key. A key lookup locks only the rows it
/// names, never a gap another user's insert needs.
async fn delete_attempts(connection: &impl ConnectionTrait, ids: Vec<String>) -> Result<(), DbErr> {
    if ids.is_empty() {
        return Ok(());
    }
    attempt::Entity::delete_many()
        .filter(attempt::Column::Id.is_in(ids))
        .exec(connection)
        .await?;
    Ok(())
}

/// Best-effort removal of attempts that fell out of the window.
async fn purge_expired(ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    let outcome = match DB::connection() {
        Ok(db) => delete_attempts(db.inner(), ids)
            .await
            .map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = outcome {
        tracing::warn!(
            error = %error,
            "could not delete expired two-factor attempts; they no longer count and \
             the next admission retries"
        );
    }
}

/// The reserved attempt's code was wrong: count it as a failure.
pub(crate) async fn record_failure(reservation: &Reservation) -> Result<Failure, FrameworkError> {
    let lockout = TwoFactorLockout::resolve()?;
    let id = reservation.id.clone();
    let owner = reservation.user_id.clone();
    locked(&reservation.user_id, move |transaction| {
        Box::pin(async move {
            let settled = attempt::Entity::update_many()
                .col_expr(attempt::Column::Failed, Expr::value(true))
                .filter(attempt::Column::Id.eq(id.as_str()))
                .filter(attempt::Column::Failed.eq(false))
                .exec(transaction)
                .await?;
            let failed_attempts = attempt::Entity::find()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .filter(attempt::Column::Failed.eq(true))
                .filter(attempt::Column::AttemptedAt.gt(window_floor(crate::clock::now(), lockout)))
                .count(transaction)
                .await?;
            // Admission never lets pending and failed rows exceed the
            // maximum, so the settle that reaches it is the one that locks.
            Ok(Failure {
                failed_attempts,
                locked_now: settled.rows_affected > 0
                    && failed_attempts >= u64::from(lockout.max_attempts()),
            })
        })
    })
    .await
}

/// The reserved attempt's code was right: clear the user's failures. Other
/// requests' pending attempts stay counted until they settle.
pub(crate) async fn record_success(reservation: &Reservation) -> Result<(), FrameworkError> {
    let id = reservation.id.clone();
    let owner = reservation.user_id.clone();
    locked(&reservation.user_id, move |transaction| {
        Box::pin(async move {
            let mut ids = attempt_ids(
                transaction,
                attempt::Entity::find()
                    .filter(attempt::Column::UserId.eq(owner.as_str()))
                    .filter(attempt::Column::Failed.eq(true)),
            )
            .await?;
            ids.push(id);
            delete_attempts(transaction, ids).await
        })
    })
    .await
}

/// The reserved attempt's code could not be evaluated: drop the reservation
/// without counting anything.
pub(crate) async fn release(reservation: &Reservation) -> Result<(), FrameworkError> {
    let db = DB::connection().map_err(unavailable)?;
    attempt::Entity::delete_many()
        .filter(attempt::Column::Id.eq(reservation.id.as_str()))
        .exec(db.inner())
        .await
        .map_err(unavailable)?;
    Ok(())
}

/// Forget every attempt of `user_id`. Returns whether the user was locked.
pub(crate) async fn clear(user_id: &str) -> Result<bool, FrameworkError> {
    let lockout = TwoFactorLockout::resolve()?;
    let owner = user_id.to_owned();
    locked(user_id, move |transaction| {
        Box::pin(async move {
            let failed_attempts = attempt::Entity::find()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .filter(attempt::Column::Failed.eq(true))
                .filter(attempt::Column::AttemptedAt.gt(window_floor(crate::clock::now(), lockout)))
                .count(transaction)
                .await?;
            let ids = attempt_ids(
                transaction,
                attempt::Entity::find().filter(attempt::Column::UserId.eq(owner.as_str())),
            )
            .await?;
            delete_attempts(transaction, ids).await?;
            Ok(failed_attempts >= u64::from(lockout.max_attempts()))
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::names_missing_attempt_table;

    #[test]
    fn missing_table_wordings_of_each_engine_are_recognized() {
        for message in [
            "error returned from database: (code: 1) no such table: two_factor_attempts",
            "error returned from database: relation \"two_factor_attempts\" does not exist",
            "error returned from database: 1146 (42S02): Table 'app.two_factor_attempts' doesn't exist",
        ] {
            assert!(names_missing_attempt_table(message), "{message}");
        }
        assert!(!names_missing_attempt_table(
            "error returned from database: database is locked"
        ));
        assert!(!names_missing_attempt_table(
            "no such table: two_factor_credentials"
        ));
    }
}
