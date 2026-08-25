//! PostgreSQL coverage for the `queue:work` boot sequence and worker loop
//! (DATA-01).
//!
//! Two things are pinned here:
//!
//! 1. **Boot ordering.** `QUEUE_DRIVER=database` resolves its connection
//!    from `DB`, so the app's bootstrap (which calls `DB::init`) has to run
//!    *before* the env-driven driver bootstrap. The worker subcommands used
//!    to do the reverse and died with "requires DB::init() to run first"
//!    before a single job was popped.
//! 2. **The loop itself against Postgres.** `run_worker` is the same
//!    function `queue:work` spawns; driving it against a Postgres-backed
//!    driver + failed-jobs store exercises push/pop/ack and the dead-letter
//!    write on the backend the SQLite suite never touched.
//!
//! Run with a disposable Postgres:
//!
//! ```text
//! docker run -d --rm --name suprnova-pg -e POSTGRES_PASSWORD=pw \
//!     -e POSTGRES_DB=suprnova_test -p 55999:5432 postgres:17-alpine
//! PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55999/suprnova_test \
//!     cargo test -p suprnova --test queue_worker_postgres -- --ignored
//! ```

use async_trait::async_trait;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::error::FrameworkError;
use suprnova::queue::failed::{DatabaseFailedJobStore, FailedJobStore};
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::queue::{Job, Queue, bootstrap_from_env};
use suprnova::{DB, DatabaseConfig};
use tokio_util::sync::CancellationToken;

fn pg_url() -> String {
    std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres")
}

async fn connect_postgres() -> DatabaseConnection {
    let mut options = ConnectOptions::new(pg_url());
    options
        .max_connections(4)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

async fn fresh_jobs_table(db: &DatabaseConnection, table: &str) {
    for sql in [
        format!("DROP TABLE IF EXISTS {table}"),
        format!(
            "CREATE TABLE {table} (
                id              TEXT PRIMARY KEY,
                job_name        TEXT NOT NULL,
                queue           TEXT NULL,
                envelope_json   TEXT NOT NULL,
                available_at    INTEGER NOT NULL,
                reserved_until  INTEGER NULL,
                reserved_token  TEXT NULL,
                attempts        INTEGER NOT NULL DEFAULT 0,
                created_at      INTEGER NOT NULL
            )"
        ),
    ] {
        db.execute_unprepared(&sql).await.expect("jobs fixture");
    }
}

async fn fresh_failed_jobs_table(db: &DatabaseConnection, table: &str) {
    for sql in [
        format!("DROP TABLE IF EXISTS {table}"),
        format!(
            "CREATE TABLE {table} (
                id              TEXT PRIMARY KEY,
                connection      TEXT NOT NULL,
                queue           TEXT NOT NULL,
                job_name        TEXT NOT NULL,
                envelope_json   TEXT NOT NULL,
                exception       TEXT NOT NULL,
                failed_at       INTEGER NOT NULL
            )"
        ),
    ] {
        db.execute_unprepared(&sql)
            .await
            .expect("failed_jobs fixture");
    }
}

/// Restores the queue env vars this binary mutates. `#[serial]` keeps the
/// mutation from racing another test in the same process.
struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(pairs: &[(&'static str, &str)]) -> Self {
        let saved = pairs
            .iter()
            .map(|(k, _)| (*k, std::env::var(k).ok()))
            .collect();
        for (k, v) in pairs {
            // SAFETY: serial test — no other thread reads or writes these
            // process-global vars concurrently.
            unsafe {
                std::env::set_var(k, v);
            }
        }
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            // SAFETY: same as above.
            unsafe {
                match v {
                    Some(value) => std::env::set_var(k, value),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

static RAN: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize, Deserialize, Clone)]
struct GoodJob;

#[async_trait]
impl Job for GoodJob {
    fn job_name() -> &'static str {
        "queue_worker_postgres::GoodJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct DeadJob;

#[async_trait]
impl Job for DeadJob {
    fn job_name() -> &'static str {
        "queue_worker_postgres::DeadJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("permanent failure"))
    }
    fn max_tries() -> u32 {
        1
    }
}

fn one_job_config() -> WorkerConfig {
    WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(10),
        max_jobs: Some(1),
        queues: Vec::new(),
    }
}

async fn row_count(db: &DatabaseConnection, table: &str) -> i64 {
    let row = db
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT COUNT(*) FROM {table}"),
        ))
        .await
        .expect("count query")
        .expect("count row");
    row.try_get_by_index(0).expect("count column")
}

/// The `queue:work` boot sequence end to end: initialise the database, then
/// let `bootstrap_from_env` build the database driver from it, then drain a
/// job through `run_worker`.
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_queue_worker_boots_after_db_init_and_drains_a_job() {
    let raw = connect_postgres().await;
    fresh_jobs_table(&raw, "pg_worker_jobs").await;

    let _env = EnvGuard::set(&[
        ("QUEUE_DRIVER", "database"),
        ("QUEUE_DB_TABLE", "pg_worker_jobs"),
    ]);

    // Step 1 — the app's bootstrap. `DB::init_with` is what a scaffolded
    // `bootstrap::register()` calls; the driver bootstrap must come after it.
    DB::init_with(DatabaseConfig::builder().url(pg_url()).build())
        .await
        .expect("DB::init_with must succeed before the drivers boot");

    // Step 2 — the env-driven driver bootstrap the worker subcommands run.
    bootstrap_from_env()
        .await
        .expect("QUEUE_DRIVER=database must resolve the initialised connection");
    assert_eq!(Queue::driver_name().unwrap(), "database");

    register_job::<GoodJob>();
    RAN.store(0, Ordering::SeqCst);
    Queue::push(GoodJob).await.expect("push through the facade");
    assert_eq!(row_count(&raw, "pg_worker_jobs").await, 1);

    let driver = Queue::driver().expect("driver registered");
    run_worker(driver, one_job_config(), CancellationToken::new()).await;

    assert_eq!(RAN.load(Ordering::SeqCst), 1, "handler ran exactly once");
    assert_eq!(
        row_count(&raw, "pg_worker_jobs").await,
        0,
        "a successful job must be acked out of the table"
    );
}

/// The dead-letter path: an exhausted job is acked off `jobs` and written to
/// the Postgres-backed failed-jobs store.
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_queue_worker_dead_letters_into_the_database_store() {
    let raw = connect_postgres().await;
    fresh_jobs_table(&raw, "pg_worker_dead_jobs").await;
    fresh_failed_jobs_table(&raw, "pg_worker_failed_jobs").await;

    let _env = EnvGuard::set(&[
        ("QUEUE_DRIVER", "database"),
        ("QUEUE_DB_TABLE", "pg_worker_dead_jobs"),
    ]);

    DB::init_with(DatabaseConfig::builder().url(pg_url()).build())
        .await
        .expect("DB::init_with");
    bootstrap_from_env().await.expect("driver bootstrap");

    let store = Arc::new(
        DatabaseFailedJobStore::new(raw.clone(), "pg_worker_failed_jobs".into())
            .expect("failed store"),
    );
    Queue::set_failed_store(store.clone());

    register_job::<DeadJob>();
    Queue::push(DeadJob).await.expect("push");

    let driver = Queue::driver().expect("driver registered");
    run_worker(driver, one_job_config(), CancellationToken::new()).await;

    let failed = store.all().await.expect("read failed jobs");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].job_name, DeadJob::job_name());
    assert!(failed[0].exception.contains("permanent failure"));
    assert_eq!(
        row_count(&raw, "pg_worker_dead_jobs").await,
        0,
        "the exhausted job must be acked off the queue table"
    );
}
