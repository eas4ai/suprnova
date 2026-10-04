//! The brute-force counter behind every [`super::TwoFactor`] proof path.
//!
//! Each attempt is one row in `two_factor_attempts`. An attempt is reserved
//! (a pending row) before its code is read, then settled: a wrong code turns
//! the row into a failure, a right code deletes the user's failures, and a
//! proof that could not be evaluated deletes only its own row. Admission
//! counts pending and failed rows together, so parallel guesses cannot all
//! pass one status read, and a success never deletes another request's
//! pending row. Rows older than [`attempt_window`] no longer count, which
//! also retires the reservation of a request that died before it settled.
//!
//! The counter belongs to the second factor alone. A successful password
//! check does not touch it, so wrong codes cannot be washed out by signing
//! in with the password again, and it works with no Magnetar engine
//! installed.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, TransactionTrait,
};

use super::entity as credentials;
use crate::database::DB;
use crate::error::FrameworkError;

/// Failed second-factor attempts inside [`attempt_window`] that lock every
/// proof path of the facade. The same default as Magnetar's password
/// lockout.
pub(crate) const MAX_FAILED_ATTEMPTS: u64 = 5;

/// How long an attempt counts. A lock therefore lifts once the failure that
/// completed it is this old.
pub(crate) fn attempt_window() -> chrono::Duration {
    chrono::Duration::minutes(15)
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
fn unavailable(error: impl std::fmt::Display) -> FrameworkError {
    tracing::error!(%error, "two-factor attempt store failed");
    FrameworkError::domain("two-factor attempt store unavailable", 503)
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
pub(crate) async fn admit(user_id: &str) -> Result<Admission, FrameworkError> {
    let owner = user_id.to_owned();
    locked(user_id, move |transaction| {
        Box::pin(async move {
            let now = crate::clock::now();
            attempt::Entity::delete_many()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .filter(attempt::Column::AttemptedAt.lte(now - attempt_window()))
                .exec(transaction)
                .await?;
            let counted = attempt::Entity::find()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .count(transaction)
                .await?;
            if counted >= MAX_FAILED_ATTEMPTS {
                return Ok(Admission::Locked);
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
            Ok(Admission::Admitted(Reservation { id, user_id: owner }))
        })
    })
    .await
}

/// The reserved attempt's code was wrong: count it as a failure.
pub(crate) async fn record_failure(reservation: &Reservation) -> Result<Failure, FrameworkError> {
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
                .filter(attempt::Column::AttemptedAt.gt(crate::clock::now() - attempt_window()))
                .count(transaction)
                .await?;
            // Admission never lets pending and failed rows exceed the
            // maximum, so the settle that reaches it is the one that locks.
            Ok(Failure {
                failed_attempts,
                locked_now: settled.rows_affected > 0 && failed_attempts >= MAX_FAILED_ATTEMPTS,
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
            attempt::Entity::delete_many()
                .filter(attempt::Column::Id.eq(id.as_str()))
                .exec(transaction)
                .await?;
            attempt::Entity::delete_many()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .filter(attempt::Column::Failed.eq(true))
                .exec(transaction)
                .await?;
            Ok(())
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
    let owner = user_id.to_owned();
    locked(user_id, move |transaction| {
        Box::pin(async move {
            let failed_attempts = attempt::Entity::find()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .filter(attempt::Column::Failed.eq(true))
                .filter(attempt::Column::AttemptedAt.gt(crate::clock::now() - attempt_window()))
                .count(transaction)
                .await?;
            attempt::Entity::delete_many()
                .filter(attempt::Column::UserId.eq(owner.as_str()))
                .exec(transaction)
                .await?;
            Ok(failed_attempts >= MAX_FAILED_ATTEMPTS)
        })
    })
    .await
}
