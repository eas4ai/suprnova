//! LDB-003: a worker whose failed-jobs store is the database store checks
//! the table before its first pop, and refuses to start on one it cannot
//! write.

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use sea_orm::ConnectionTrait;
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::queue::migrations::CreateJobsTable;
use suprnova::queue::worker::{WorkerConfig, run_worker, run_worker_on};
use suprnova::{
    DatabaseFailedJobStore, DatabaseQueueDriver, MemoryFailedJobStore, MemoryQueueDriver, Queue,
    QueueDriver,
};
use tokio_util::sync::CancellationToken;

use crate::failed_jobs::{ORIGINS, envelope, failed_jobs_table};
use crate::on_every_engine;
use crate::support::{self, Db, Engine};

fn one_job_config() -> WorkerConfig {
    WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(10),
        max_jobs: Some(1),
        queues: Vec::new(),
    }
}

/// A memory queue holding one job whose handler is not registered, so a
/// worker that pops it dead-letters it into the failed-jobs store.
async fn queue_with_one_job() -> Arc<MemoryQueueDriver> {
    let memory = Arc::new(MemoryQueueDriver::new());
    memory
        .push(envelope("Ldb.Unregistered", serde_json::json!({})))
        .await
        .expect("push");
    memory
}

fn install_database_store(db: &Db) {
    Queue::set_failed_store(Arc::new(
        DatabaseFailedJobStore::new(db.conn.clone(), "failed_jobs".to_owned()).unwrap(),
    ));
}

/// Both entry points refuse, with an error naming the table and `column`,
/// and neither pops the job.
async fn assert_refused(column: &str) {
    let memory = queue_with_one_job().await;
    let refused = run_worker(memory.clone(), one_job_config(), CancellationToken::new())
        .await
        .expect_err("run_worker started on a table its store cannot write");
    let text = refused.to_string();
    assert!(
        text.contains("failed_jobs"),
        "the error names the table: {text}"
    );
    assert!(text.contains(column), "the error names `{column}`: {text}");
    assert_eq!(memory.size().await.unwrap(), 1, "run_worker popped the job");

    Queue::register_connection("ldb-check", memory.clone());
    let refused = run_worker_on("ldb-check", one_job_config(), CancellationToken::new())
        .await
        .expect_err("run_worker_on started on a table its store cannot write");
    assert!(refused.to_string().contains(column), "{refused}");
    assert_eq!(
        memory.size().await.unwrap(),
        1,
        "run_worker_on popped the job"
    );
}

async fn a_missing_table_is_refused(engine: Engine) {
    let db = support::empty(engine).await;
    install_database_store(&db);
    let memory = queue_with_one_job().await;
    let refused = run_worker(memory.clone(), one_job_config(), CancellationToken::new())
        .await
        .expect_err("a worker started without a failed_jobs table");
    assert!(refused.to_string().contains("failed_jobs"), "{refused}");
    assert!(refused.to_string().contains("does not exist"), "{refused}");
    assert_eq!(memory.size().await.unwrap(), 1, "the worker popped the job");
}

on_every_engine!(a_missing_table_is_refused =>
    ldb_003_a_missing_failed_jobs_table_stops_the_worker_sqlite,
    ldb_003_a_missing_failed_jobs_table_stops_the_worker_postgres,
    ldb_003_a_missing_failed_jobs_table_stops_the_worker_mysql);

async fn a_missing_column_is_refused(engine: Engine) {
    let db = support::empty(engine).await;
    let text = if engine == Engine::Mysql {
        "LONGTEXT"
    } else {
        "TEXT"
    };
    db.conn
        .execute_unprepared(&format!(
            "CREATE TABLE failed_jobs (id BIGINT PRIMARY KEY, uuid VARCHAR(255) NOT NULL, \
             connection VARCHAR(255) NOT NULL, queue VARCHAR(255) NOT NULL, \
             payload {text} NOT NULL, failed_at TIMESTAMP NULL)"
        ))
        .await
        .unwrap();
    install_database_store(&db);
    assert_refused("exception").await;
}

on_every_engine!(a_missing_column_is_refused =>
    ldb_003_a_table_lacking_a_written_column_stops_the_worker_sqlite,
    ldb_003_a_table_lacking_a_written_column_stops_the_worker_postgres,
    ldb_003_a_table_lacking_a_written_column_stops_the_worker_mysql);

async fn the_earlier_layout_is_refused(engine: Engine) {
    let db = support::empty(engine).await;
    db.conn
        .execute_unprepared(
            "CREATE TABLE failed_jobs (id VARCHAR(64) PRIMARY KEY, connection TEXT NOT NULL, \
             queue TEXT NOT NULL, job_name TEXT NOT NULL, envelope_json TEXT NOT NULL, \
             exception TEXT NOT NULL, failed_at BIGINT NOT NULL)",
        )
        .await
        .unwrap();
    install_database_store(&db);
    assert_refused("envelope_json").await;
}

on_every_engine!(the_earlier_layout_is_refused =>
    ldb_003_the_earlier_suprnova_layout_stops_the_worker_sqlite,
    ldb_003_the_earlier_suprnova_layout_stops_the_worker_postgres,
    ldb_003_the_earlier_suprnova_layout_stops_the_worker_mysql);

/// On MySQL a `TEXT` payload or exception refuses anything over 64 KiB,
/// which would start the same loop.
#[tokio::test]
#[serial_test::serial]
#[ignore = "requires a throwaway MySQL at MYSQL_TEST_URL"]
async fn ldb_003_a_text_payload_or_exception_stops_the_worker_mysql() {
    for (narrow, payload, exception) in [
        ("payload", "TEXT", "LONGTEXT"),
        ("exception", "LONGTEXT", "MEDIUMTEXT"),
    ] {
        let db = support::empty(Engine::Mysql).await;
        db.conn
            .execute_unprepared(&format!(
                "CREATE TABLE failed_jobs (id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY, \
                 uuid VARCHAR(255) NOT NULL UNIQUE, connection VARCHAR(255) NOT NULL, \
                 queue VARCHAR(255) NOT NULL, payload {payload} NOT NULL, \
                 exception {exception} NOT NULL, \
                 failed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP)"
            ))
            .await
            .unwrap();
        install_database_store(&db);
        assert_refused(narrow).await;
    }
}

/// A table Laravel 13 created and one the shipped migration created are
/// both accepted: the worker runs its job, which dead-letters into the
/// table.
async fn laravel_and_migration_tables_are_accepted(engine: Engine) {
    for origin in ORIGINS {
        let db = failed_jobs_table(engine, origin).await;
        install_database_store(&db);
        let before = support::count(&db.conn, "failed_jobs", "").await;
        let memory = queue_with_one_job().await;
        run_worker(memory.clone(), one_job_config(), CancellationToken::new())
            .await
            .unwrap_or_else(|e| panic!("{origin:?}: the worker refused a good table: {e}"));
        assert_eq!(
            memory.size().await.unwrap(),
            0,
            "{origin:?}: the job did not run"
        );
        assert_eq!(
            support::count(&db.conn, "failed_jobs", "").await,
            before + 1,
            "{origin:?}: the dead letter was not recorded"
        );
    }
}

on_every_engine!(laravel_and_migration_tables_are_accepted =>
    ldb_003_laravel_and_migration_tables_are_accepted_sqlite,
    ldb_003_laravel_and_migration_tables_are_accepted_postgres,
    ldb_003_laravel_and_migration_tables_are_accepted_mysql);

/// A worker whose store is not the database store does not look at the
/// table at all.
async fn other_stores_do_not_refuse(engine: Engine) {
    let db = support::empty(engine).await;
    db.conn
        .execute_unprepared(
            "CREATE TABLE failed_jobs (id VARCHAR(64) PRIMARY KEY, envelope_json TEXT NOT NULL)",
        )
        .await
        .unwrap();
    Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));
    let memory = queue_with_one_job().await;
    run_worker(memory.clone(), one_job_config(), CancellationToken::new())
        .await
        .expect("a worker with the memory store refused to start");
    assert_eq!(memory.size().await.unwrap(), 0);
}

on_every_engine!(other_stores_do_not_refuse =>
    ldb_003_a_worker_without_the_database_store_does_not_refuse_sqlite,
    ldb_003_a_worker_without_the_database_store_does_not_refuse_postgres,
    ldb_003_a_worker_without_the_database_store_does_not_refuse_mysql);

/// The URL a child process connects to for this engine's test database.
fn child_url(engine: Engine, db: &Db) -> String {
    match engine {
        Engine::Sqlite => format!(
            "sqlite://{}",
            db.sqlite_path().expect("a SQLite file").display()
        ),
        Engine::Postgres => std::env::var("PG_TEST_URL").unwrap(),
        Engine::Mysql => std::env::var("MYSQL_TEST_URL").unwrap(),
    }
}

/// Run `queue:work --max-jobs 1` in a child process of this test binary
/// against `url`, with `QUEUE_DRIVER=database`, which binds the database
/// failed-jobs store.
fn queue_work(url: &str) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "worker_check::queue_work_child", "--nocapture"])
        .env("LDB_QUEUE_WORK_CHILD", "1")
        .env("DATABASE_URL", url)
        .env("QUEUE_DRIVER", "database")
        .env("APP_ENV", "testing")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the child")
}

/// `queue:work` exits non-zero when it refuses, naming the table and the
/// column, and leaves the queued job in place; on a table Laravel created
/// it runs the job and exits zero.
async fn queue_work_exits_non_zero_when_it_refuses(engine: Engine) {
    let db = support::empty(engine).await;
    CreateJobsTable
        .up(&SchemaManager::new(&db.conn))
        .await
        .expect("the jobs table");
    db.conn
        .execute_unprepared(
            "CREATE TABLE failed_jobs (id VARCHAR(64) PRIMARY KEY, connection TEXT NOT NULL, \
             queue TEXT NOT NULL, job_name TEXT NOT NULL, envelope_json TEXT NOT NULL, \
             exception TEXT NOT NULL, failed_at BIGINT NOT NULL)",
        )
        .await
        .unwrap();
    let jobs = DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).unwrap();
    jobs.push(envelope("Ldb.Unregistered", serde_json::json!({})))
        .await
        .unwrap();

    let output = queue_work(&child_url(engine, &db));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "queue:work did not exit 1 after refusing: {stderr}"
    );
    assert!(stderr.contains("failed_jobs"), "{stderr}");
    assert!(stderr.contains("envelope_json"), "{stderr}");
    assert!(
        stderr.contains(RETURNED),
        "run_with_args did not return the refusal to its caller: {stderr}"
    );
    assert_eq!(jobs.size().await.unwrap(), 1, "queue:work popped the job");

    // Laravel's own table: the worker starts, runs the job and exits 0.
    db.conn
        .execute_unprepared("DROP TABLE failed_jobs")
        .await
        .unwrap();
    for statement in support::fixture(engine).schema {
        if statement.to_lowercase().contains("failed_jobs") {
            db.conn.execute_unprepared(&statement).await.unwrap();
        }
    }
    let output = queue_work(&child_url(engine, &db));
    assert_eq!(
        output.status.code(),
        Some(0),
        "queue:work refused Laravel's table: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        jobs.size().await.unwrap(),
        0,
        "queue:work did not run the job"
    );
    assert_eq!(support::count(&db.conn, "failed_jobs", "").await, 1);
}

on_every_engine!(queue_work_exits_non_zero_when_it_refuses =>
    ldb_003_queue_work_exits_non_zero_when_it_refuses_sqlite,
    ldb_003_queue_work_exits_non_zero_when_it_refuses_postgres,
    ldb_003_queue_work_exits_non_zero_when_it_refuses_mysql);

/// The child half of [`queue_work_exits_non_zero_when_it_refuses`]: a
/// real `queue:work` through `Application`. It does nothing unless the
/// parent started it.
#[test]
fn queue_work_child() {
    if std::env::var("LDB_QUEUE_WORK_CHILD").is_err() {
        return;
    }
    suprnova::boot::load_env().expect("load the configuration");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = runtime.block_on(async {
        suprnova::Application::new()
            .bootstrap(|| async {
                suprnova::Config::register(suprnova::DatabaseConfig::from_env());
                suprnova::DB::init().await.expect("connect");
            })
            .run_with_args(["app", "queue:work", "--max-jobs", "1", "--poll", "10"])
            .await
    });
    // The executable boundary: the failure came back to this caller, which
    // prints it and exits non-zero, as `Application::run` does.
    if let Err(e) = outcome {
        eprintln!("{RETURNED}{}", e.message());
        std::process::exit(1);
    }
}

/// What [`queue_work_child`] prints before a failure `run_with_args`
/// returned to it.
const RETURNED: &str = "run_with_args returned: ";

/// A schema-qualified queue table works on Postgres as an unqualified one
/// does: the shipped migrations upgrade an earlier-layout `failed_jobs` in
/// that schema in place and create `jobs` there, a second `migrate` leaves
/// both alone, the worker's check accepts the table and the worker writes
/// its dead letter into it, and the driver queues and pops through the
/// qualified `jobs`.
#[tokio::test]
#[serial_test::serial]
#[ignore = "requires a throwaway Postgres at PG_TEST_URL"]
async fn ldb_003_schema_qualified_queue_tables_work_postgres() {
    let db = support::empty(Engine::Postgres).await;
    db.conn
        .execute_unprepared("DROP SCHEMA IF EXISTS ldb_queue CASCADE")
        .await
        .unwrap();
    db.conn
        .execute_unprepared("CREATE SCHEMA ldb_queue")
        .await
        .unwrap();
    let jobs = "ldb_queue.jobs";
    let failed = "ldb_queue.failed_jobs";
    db.conn
        .execute_unprepared(&format!(
            "CREATE TABLE {failed} (id VARCHAR(64) PRIMARY KEY, connection TEXT NOT NULL, \
             queue TEXT NOT NULL, job_name TEXT NOT NULL, envelope_json TEXT NOT NULL, \
             exception TEXT NOT NULL, failed_at BIGINT NOT NULL)"
        ))
        .await
        .unwrap();
    let earlier = uuid::Uuid::new_v4();
    let mut env = envelope("Ldb.Earlier", serde_json::json!({}));
    env.id = earlier;
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO {failed} (id, connection, queue, job_name, envelope_json, exception, \
             failed_at) VALUES ('{earlier}', 'database', 'default', 'Ldb.Earlier', '{}', \
             'boom', {})",
            env.to_json().unwrap(),
            chrono::Utc::now().timestamp()
        ))
        .await
        .unwrap();

    crate::failed_jobs::migrate_queue_tables(&db, jobs, failed).await;
    crate::failed_jobs::migrate_queue_tables(&db, jobs, failed).await;
    let moved = support::rows(&db.conn, &format!("SELECT uuid FROM {failed}")).await;
    assert_eq!(moved.len(), 1, "the earlier failed job");
    assert_eq!(support::text(&moved[0], "uuid"), earlier.to_string());
    let public = support::rows(
        &db.conn,
        "SELECT COUNT(*) AS n FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .await;
    assert_eq!(support::int(&public[0], "n"), 0, "a table landed in public");

    run_on_qualified_tables(&db, jobs, failed).await;
    assert_eq!(support::count(&db.conn, failed, "").await, 2);
}

/// On MySQL, where a schema is a database, the stores and the worker's
/// check work on queue tables named with their database. The schema
/// builder refuses to create such a table, naming it, rather than failing
/// inside SeaQuery.
#[tokio::test]
#[serial_test::serial]
#[ignore = "requires a throwaway MySQL at MYSQL_TEST_URL"]
async fn ldb_003_schema_qualified_queue_tables_work_mysql() {
    let db = support::empty(Engine::Mysql).await;
    let rows = support::rows(&db.conn, "SELECT DATABASE() AS name").await;
    let schema = support::text(&rows[0], "name");
    let jobs = format!("{schema}.jobs");
    let failed = format!("{schema}.failed_jobs");
    for statement in [
        format!(
            "CREATE TABLE {failed} (id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY, \
             uuid VARCHAR(255) NOT NULL UNIQUE, connection TEXT NOT NULL, \
             queue TEXT NOT NULL, payload LONGTEXT NOT NULL, exception LONGTEXT NOT NULL, \
             failed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP)"
        ),
        format!(
            "CREATE TABLE {jobs} (id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY, \
             queue VARCHAR(255) NOT NULL, payload LONGTEXT NOT NULL, \
             attempts SMALLINT UNSIGNED NOT NULL, reserved_at INT UNSIGNED NULL, \
             available_at INT UNSIGNED NOT NULL, created_at INT UNSIGNED NOT NULL)"
        ),
        format!(
            "CREATE TABLE {schema}.suprnova_jobs_reservations (job_id BIGINT UNSIGNED PRIMARY KEY, \
             token CHAR(36) NOT NULL UNIQUE, reserved_until BIGINT NOT NULL)"
        ),
    ] {
        db.conn.execute_unprepared(&statement).await.unwrap();
    }
    run_on_qualified_tables(&db, &jobs, &failed).await;
    assert_eq!(support::count(&db.conn, &failed, "").await, 1);

    // SAFETY: the test is serial within its process.
    unsafe { std::env::set_var("QUEUE_DB_TABLE", &jobs) };
    let refused = CreateJobsTable.up(&SchemaManager::new(&db.conn)).await;
    // SAFETY: as above.
    unsafe { std::env::remove_var("QUEUE_DB_TABLE") };
    let refused = refused.expect_err("the builder took a qualified table on MySQL");
    assert!(
        refused
            .to_string()
            .contains(&format!("{schema}.suprnova_jobs_reservations")),
        "{refused}"
    );
}

/// The worker's check accepts the qualified `failed`, the worker writes its
/// dead letter into it, and the driver queues and pops through `jobs`.
async fn run_on_qualified_tables(db: &Db, jobs: &str, failed: &str) {
    let store = DatabaseFailedJobStore::new(db.conn.clone(), failed.to_owned()).unwrap();
    Queue::set_failed_store(Arc::new(store));
    let memory = queue_with_one_job().await;
    run_worker(memory.clone(), one_job_config(), CancellationToken::new())
        .await
        .unwrap_or_else(|e| panic!("the worker refused {failed}: {e}"));
    assert_eq!(memory.size().await.unwrap(), 0, "the job ran");

    let driver = DatabaseQueueDriver::new(db.conn.clone(), jobs.to_owned()).unwrap();
    driver
        .push(envelope("Ldb.Qualified", serde_json::json!({})))
        .await
        .unwrap_or_else(|e| panic!("push onto {jobs}: {e}"));
    let popped = driver
        .pop(Duration::from_secs(30))
        .await
        .unwrap_or_else(|e| panic!("pop from {jobs}: {e}"))
        .expect("the job just pushed");
    driver.ack(&popped.token).await.unwrap();
    assert_eq!(driver.size().await.unwrap(), 0);
}

/// `Application::run_with_args` hands its failures to its caller instead
/// of ending the process: an argv the CLI cannot parse, and a command
/// whose boot precondition fails (this process loaded no configuration
/// before its runtime started). The test reaching its end is the proof.
#[tokio::test]
#[serial_test::serial]
async fn ldb_003_run_with_args_returns_its_failures_to_the_caller() {
    let unknown = suprnova::Application::new()
        .run_with_args(["app", "no-such-command"])
        .await
        .expect_err("an argv the CLI cannot parse");
    assert!(
        unknown.message().contains("no-such-command"),
        "the error carries clap's message: {}",
        unknown.message()
    );
    let refused = suprnova::Application::new()
        .run_with_args(["app", "queue:work", "--max-jobs", "1"])
        .await
        .expect_err("this process loaded no configuration before its runtime");
    assert!(
        refused.message().contains("#[suprnova::main]"),
        "the error carries the boot precondition's advice: {}",
        refused.message()
    );
    suprnova::Application::new()
        .run_with_args(["app", "--help"])
        .await
        .expect("--help prints and returns");
}
