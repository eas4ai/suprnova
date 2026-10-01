//! PAR-007: `DB::after_commit` runs a callback once the outermost
//! transaction commits.
//!
//! Inside a [`DB::transaction`] the callback waits for the commit and is
//! dropped when the transaction rolls back. A [`Transaction::rollback_to`]
//! drops the callbacks registered since its savepoint, while the ones
//! registered before it, after it, or inside a savepoint that is kept still
//! run at the commit: Laravel's `DatabaseTransactionsManager` discards the
//! callbacks of a nested transaction that rolls back and runs the rest when
//! the root commits. Outside any transaction the callback runs at once.
//!
//! The `postgres_` and `mysql_` variants run the commit, rollback and
//! savepoint scenario against a disposable server and are ignored by
//! default.

use std::sync::{Arc, Mutex};

use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::{TestContainer, TestContainerGuard, TestDatabase};
use suprnova::{DB, FrameworkError, Model, attrs, model};

#[model(table = "ac_notes", timestamps = false, fillable = ["body"])]
pub struct AcNote {
    pub id: i64,
    pub body: String,
}

type Log = Arc<Mutex<Vec<String>>>;

fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn entries(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// Register a callback that appends `entry` to `log`.
async fn record_after_commit(log: &Log, entry: &str) -> Result<(), FrameworkError> {
    let log = log.clone();
    let entry = entry.to_string();
    DB::after_commit(move || async move {
        log.lock().unwrap().push(entry);
        Ok(())
    })
    .await
}

async fn sqlite() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE ac_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL)",
    )
    .await
    .unwrap();
    db
}

async fn note_count() -> usize {
    AcNote::all().await.unwrap().len()
}

// ---- No transaction -----------------------------------------------------

#[tokio::test]
async fn without_a_transaction_the_callback_runs_at_once() {
    let _db = sqlite().await;
    let log = new_log();

    record_after_commit(&log, "ran").await.unwrap();

    assert_eq!(entries(&log), vec!["ran"]);
}

#[tokio::test]
async fn without_a_transaction_a_failing_callback_returns_its_error() {
    let _db = sqlite().await;
    let err = DB::after_commit(|| async { Err(FrameworkError::internal("callback failed")) })
        .await
        .expect_err("the callback ran at once and failed");
    assert!(err.to_string().contains("callback failed"), "{err}");
}

// ---- Inside DB::transaction ---------------------------------------------

#[tokio::test]
async fn inside_a_transaction_the_callback_waits_for_the_commit() {
    let _db = sqlite().await;
    let log = new_log();
    let seen_rows = Arc::new(Mutex::new(None));

    let in_tx = log.clone();
    let rows = seen_rows.clone();
    DB::transaction(|_tx| {
        Box::pin(async move {
            AcNote::create(attrs! { body: "written in the transaction" }).await?;
            DB::after_commit(move || async move {
                // A read outside the transaction: the row is there only if
                // the commit already happened.
                let count = note_count().await;
                *rows.lock().unwrap() = Some(count);
                Ok(())
            })
            .await?;
            record_after_commit(&in_tx, "ran").await?;
            assert!(
                entries(&in_tx).is_empty(),
                "nothing runs before the transaction commits"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();

    assert_eq!(
        entries(&log),
        vec!["ran"],
        "the callback ran once, after the commit"
    );
    assert_eq!(
        *seen_rows.lock().unwrap(),
        Some(1),
        "the callback saw the committed row"
    );
}

#[tokio::test]
async fn callbacks_run_in_registration_order() {
    let _db = sqlite().await;
    let log = new_log();

    let in_tx = log.clone();
    DB::transaction(|_tx| {
        Box::pin(async move {
            for entry in ["first", "second", "third"] {
                record_after_commit(&in_tx, entry).await?;
            }
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();

    assert_eq!(entries(&log), vec!["first", "second", "third"]);
}

#[tokio::test]
async fn a_transaction_that_rolls_back_never_runs_the_callback() {
    let _db = sqlite().await;
    let log = new_log();

    let in_tx = log.clone();
    let result: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async move {
            AcNote::create(attrs! { body: "rolled back" }).await?;
            record_after_commit(&in_tx, "ran").await?;
            Err(FrameworkError::internal("roll back"))
        })
    })
    .await;

    assert!(result.is_err());
    assert!(
        entries(&log).is_empty(),
        "a rolled-back transaction runs no callback"
    );
    assert_eq!(note_count().await, 0);
}

#[tokio::test]
async fn a_rolled_back_savepoint_drops_only_the_callbacks_registered_inside_it() {
    let _db = sqlite().await;
    let log = new_log();

    let in_tx = log.clone();
    DB::transaction(|tx| {
        Box::pin(async move {
            record_after_commit(&in_tx, "before the savepoint").await?;
            tx.savepoint("inner").await?;
            record_after_commit(&in_tx, "inside the rolled-back savepoint").await?;
            tx.rollback_to("inner").await?;
            record_after_commit(&in_tx, "after the rollback").await?;
            tx.savepoint("kept").await?;
            record_after_commit(&in_tx, "inside a kept savepoint").await?;
            assert!(entries(&in_tx).is_empty());
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();

    assert_eq!(
        entries(&log),
        vec![
            "before the savepoint",
            "after the rollback",
            "inside a kept savepoint",
        ]
    );
}

#[tokio::test]
async fn a_failing_callback_reports_an_error_but_the_commit_stands() {
    let _db = sqlite().await;
    let log = new_log();

    let in_tx = log.clone();
    let err = DB::transaction(|_tx| {
        Box::pin(async move {
            AcNote::create(attrs! { body: "committed" }).await?;
            DB::after_commit(|| async { Err(FrameworkError::internal("callback failed")) }).await?;
            record_after_commit(&in_tx, "the next callback").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect_err("the failing callback surfaces");

    let message = err.to_string();
    assert!(
        message.contains("the transaction itself committed"),
        "{message}"
    );
    assert!(message.contains("callback failed"), "{message}");
    assert_eq!(note_count().await, 1, "the commit is durable");
    assert_eq!(
        entries(&log),
        vec!["the next callback"],
        "one failing callback does not stop the rest"
    );
}

#[tokio::test]
async fn inside_a_manual_transaction_the_callback_runs_at_once() {
    let _db = sqlite().await;
    let log = new_log();

    // A manual transaction installs no ambient transaction, so there is no
    // commit for the callback to wait for.
    let tx = DB::begin_transaction().await.unwrap();
    record_after_commit(&log, "ran").await.unwrap();
    assert_eq!(entries(&log), vec!["ran"]);
    tx.rollback().await.unwrap();
}

// ---- Live engines -------------------------------------------------------

async fn connect_live(env: &str) -> (TestContainerGuard, DbConnection) {
    let url = std::env::var(env).expect("explicit disposable database URL required");
    let guard = TestContainer::fake();
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let database = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    TestContainer::singleton(database.clone());
    (guard, database)
}

async fn live_after_commit(env: &str) {
    use sea_orm::ConnectionTrait;

    let (guard, database) = connect_live(env).await;
    let id_column = match database.inner().get_database_backend() {
        sea_orm::DatabaseBackend::Postgres => "id BIGSERIAL PRIMARY KEY",
        _ => "id BIGINT AUTO_INCREMENT PRIMARY KEY",
    };
    database
        .inner()
        .execute_unprepared(&format!(
            "CREATE TEMPORARY TABLE ac_notes ({id_column}, body VARCHAR(255) NOT NULL)"
        ))
        .await
        .expect("create isolated temporary table");

    // Rollback: nothing runs.
    let log = new_log();
    let in_tx = log.clone();
    let rolled_back: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async move {
            AcNote::create(attrs! { body: "rolled back" }).await?;
            record_after_commit(&in_tx, "rolled back").await?;
            Err(FrameworkError::internal("roll back"))
        })
    })
    .await;
    assert!(rolled_back.is_err());
    assert!(entries(&log).is_empty());

    // Commit, with a savepoint rolled back inside it.
    let seen_rows = Arc::new(Mutex::new(None));
    let in_tx = log.clone();
    let rows = seen_rows.clone();
    DB::transaction(|tx| {
        Box::pin(async move {
            AcNote::create(attrs! { body: "committed" }).await?;
            record_after_commit(&in_tx, "before the savepoint").await?;
            tx.savepoint("inner").await?;
            record_after_commit(&in_tx, "inside the rolled-back savepoint").await?;
            tx.rollback_to("inner").await?;
            DB::after_commit(move || async move {
                let count = note_count().await;
                *rows.lock().unwrap() = Some(count);
                Ok(())
            })
            .await?;
            assert!(entries(&in_tx).is_empty());
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();

    assert_eq!(entries(&log), vec!["before the savepoint"]);
    assert_eq!(*seen_rows.lock().unwrap(), Some(1));

    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_after_commit_waits_for_the_outermost_commit() {
    live_after_commit("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_after_commit_waits_for_the_outermost_commit() {
    live_after_commit("MYSQL_TEST_URL").await;
}
