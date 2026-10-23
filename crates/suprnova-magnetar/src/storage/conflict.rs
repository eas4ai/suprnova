//! Running a transaction again when the database ends it in a conflict
//! with a concurrent transaction.
//!
//! Two requests that race for one row, such as two presentations of one
//! single-use token, can make the database end one of the transactions
//! instead of making it wait: a deadlock, or a serialization failure. That
//! transaction lost a race; it did nothing wrong. Every database names the
//! same remedy, to run the whole transaction again, and the new run reads
//! what the winner committed: a token already spent, for example.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rand::Rng;
use sea_orm::{DatabaseConnection, DbErr, TransactionTrait};

use super::{AuthTransaction, db_error};
use crate::{Error, Result};

/// How many times [`in_retried_transaction`] runs a transaction, the first
/// run included.
pub(crate) const TRANSACTION_ATTEMPTS: u32 = 3;

/// Whether the database ended a statement, and with it the transaction, in
/// a conflict with a concurrent transaction. The codes, each one the
/// database's own advice to run the transaction again:
///
/// - MySQL and MariaDB 1213, a deadlock. InnoDB rolled the transaction back.
/// - MySQL and MariaDB 1205, a lock wait timeout: another transaction held
///   the row past `innodb_lock_wait_timeout`.
/// - MariaDB 1020, "Record has changed since last read". Under
///   `innodb_snapshot_isolation`, on by default since MariaDB 11.8, a write
///   or locking read of a row another transaction changed after this one
///   took its snapshot fails with it. It is MariaDB's serialization
///   failure, and its message asks for a restart.
/// - Postgres 40001 (`serialization_failure`) and 40P01
///   (`deadlock_detected`).
///
/// The code decides, never the text of the message. SQLite serializes its
/// writers and reports none of these.
#[cfg(any(
    feature = "seaorm-sqlite",
    feature = "seaorm-postgres",
    feature = "seaorm-mysql"
))]
pub(crate) fn is_transaction_conflict(error: &DbErr) -> bool {
    use sea_orm::RuntimeErr;

    let (DbErr::Conn(RuntimeErr::SqlxError(error))
    | DbErr::Exec(RuntimeErr::SqlxError(error))
    | DbErr::Query(RuntimeErr::SqlxError(error))) = error
    else {
        return false;
    };
    let sea_orm::sqlx::Error::Database(database) = &**error else {
        return false;
    };
    #[cfg(feature = "seaorm-mysql")]
    if let Some(mysql) = database.try_downcast_ref::<sea_orm::sqlx::mysql::MySqlDatabaseError>() {
        return matches!(mysql.number(), 1020 | 1205 | 1213);
    }
    matches!(database.code().as_deref(), Some("40001" | "40P01"))
}

/// Without a database driver no statement runs, so none conflicts.
#[cfg(not(any(
    feature = "seaorm-sqlite",
    feature = "seaorm-postgres",
    feature = "seaorm-mysql"
)))]
pub(crate) fn is_transaction_conflict(_error: &DbErr) -> bool {
    false
}

/// Run `operation` in a transaction and commit it, as `in_transaction`
/// does, and run it again in a new transaction when the database ended the
/// last one in a conflict with a concurrent transaction
/// ([`is_transaction_conflict`]): up to [`TRANSACTION_ATTEMPTS`] runs in
/// all, with a short random pause before each new run so that the racing
/// transactions do not meet again at the same instant.
///
/// The operation reports its statements' errors through
/// [`AuthTransaction::database_error`], which notes a conflict. A failure
/// mapped with [`db_error`] notes none and is returned at once, as is every
/// other error. A commit that fails in a conflict runs the transaction
/// again too.
///
/// When every run ends in a conflict, the answer is [`Error::Conflict`]:
/// the operation kept losing a race, which is not an internal failure.
pub(crate) async fn in_retried_transaction<T, F>(db: &DatabaseConnection, operation: F) -> Result<T>
where
    F: for<'a> Fn(
            &'a mut AuthTransaction<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>
        + Sync,
{
    let mut attempt = 1;
    loop {
        let conflict = match run_once(db, &operation).await {
            Run::Finished(result) => return result,
            Run::Conflict(error) => error,
        };
        if attempt == TRANSACTION_ATTEMPTS {
            return Err(Error::Conflict {
                resource: "database transaction".to_owned(),
                message: format!(
                    "ended in a conflict with a concurrent transaction \
                     {TRANSACTION_ATTEMPTS} times; the last: {conflict}"
                ),
            });
        }
        tokio::time::sleep(pause_before(attempt + 1)).await;
        attempt += 1;
    }
}

/// How one run of a transaction ended.
enum Run<T> {
    /// Committed, or failed with an error that another run would not mend.
    Finished(Result<T>),
    /// Rolled back after the database ended it in a conflict.
    Conflict(Error),
}

/// One run of `operation` in a transaction of its own.
async fn run_once<T, F>(db: &DatabaseConnection, operation: &F) -> Run<T>
where
    F: for<'a> Fn(
        &'a mut AuthTransaction<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>,
{
    let mut transaction = match db.begin().await {
        Ok(transaction) => transaction,
        Err(error) => return Run::Finished(Err(db_error(error))),
    };
    let conflict = Arc::new(AtomicBool::new(false));
    let outcome = {
        let mut borrowed =
            AuthTransaction::noting_conflicts(&mut transaction, Arc::clone(&conflict));
        operation(&mut borrowed).await
    };
    match outcome {
        Ok(value) => match transaction.commit().await {
            Ok(()) => Run::Finished(Ok(value)),
            Err(error) if is_transaction_conflict(&error) => Run::Conflict(db_error(error)),
            Err(error) => Run::Finished(Err(db_error(error))),
        },
        Err(error) => {
            let _ = transaction.rollback().await;
            if conflict.load(Ordering::Relaxed) {
                Run::Conflict(error)
            } else {
                Run::Finished(Err(error))
            }
        }
    }
}

/// The pause before run `attempt`: 5 to 25 milliseconds, more for each
/// later run.
fn pause_before(attempt: u32) -> Duration {
    let jitter = rand::thread_rng().gen_range(5..=25_u64);
    Duration::from_millis(jitter * u64::from(attempt - 1))
}

#[cfg(all(test, feature = "seaorm-sqlite"))]
mod tests {
    use std::sync::atomic::AtomicU32;

    use sea_orm::Database;

    use super::*;

    async fn database() -> DatabaseConnection {
        Database::connect("sqlite::memory:")
            .await
            .expect("connect SQLite")
    }

    fn failure() -> Error {
        Error::Internal {
            message: "a statement failed".to_owned(),
        }
    }

    #[tokio::test]
    async fn a_conflict_runs_the_transaction_again() {
        let db = database().await;
        let runs = AtomicU32::new(0);
        let answer = in_retried_transaction(&db, |tx| {
            let run = runs.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move {
                if run < TRANSACTION_ATTEMPTS {
                    tx.note_conflict();
                    return Err(failure());
                }
                Ok(run)
            })
        })
        .await;
        assert_eq!(answer, Ok(TRANSACTION_ATTEMPTS));
        assert_eq!(runs.load(Ordering::SeqCst), TRANSACTION_ATTEMPTS);
    }

    #[tokio::test]
    async fn a_conflict_on_every_run_is_a_conflict_not_an_internal_error() {
        let db = database().await;
        let runs = AtomicU32::new(0);
        let answer: Result<()> = in_retried_transaction(&db, |tx| {
            runs.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                tx.note_conflict();
                Err(failure())
            })
        })
        .await;
        assert!(
            matches!(&answer, Err(Error::Conflict { resource, .. }) if resource == "database transaction"),
            "{answer:?}"
        );
        assert_eq!(runs.load(Ordering::SeqCst), TRANSACTION_ATTEMPTS);
    }

    #[tokio::test]
    async fn another_failure_is_not_run_again() {
        let db = database().await;
        let runs = AtomicU32::new(0);
        let answer: Result<()> = in_retried_transaction(&db, |_tx| {
            runs.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Err(failure()) })
        })
        .await;
        assert_eq!(answer, Err(failure()));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_sqlite_error_is_no_conflict() {
        let db = database().await;
        let error =
            sea_orm::ConnectionTrait::execute_unprepared(&db, "SELECT * FROM no_such_table")
                .await
                .expect_err("no such table");
        assert!(!is_transaction_conflict(&error), "{error}");
        assert!(!is_transaction_conflict(&DbErr::Custom(
            "a conflict".to_owned()
        )));
    }

    #[test]
    fn each_new_run_waits_longer_and_briefly() {
        assert_eq!(pause_before(1), Duration::ZERO);
        for _ in 0..50 {
            let second = pause_before(2);
            assert!((Duration::from_millis(5)..=Duration::from_millis(25)).contains(&second));
            let third = pause_before(3);
            assert!((Duration::from_millis(10)..=Duration::from_millis(50)).contains(&third));
        }
    }
}
