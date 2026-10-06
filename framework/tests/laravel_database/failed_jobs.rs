//! LDB-002: `failed_jobs` in Laravel 13's layout, on a table Laravel
//! created and on one the shipped migration created.
//!
//! `queue:failed`, `queue:retry`, `queue:forget` and `queue:flush` run as
//! console commands of a real `Application`, in a child process of this
//! test binary ([`console`]), so the test reads what each prints and the
//! exit status it ends with.

use std::process::{Command, Output, Stdio};

use chrono::Utc;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::queue::migrations::{CreateFailedJobsTable, CreateJobsTable};
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION};
use suprnova::{
    DatabaseFailedJobStore, DatabaseQueueDriver, Envelope, FailedJobStore, QueueDriver,
};
use uuid::Uuid;

use crate::on_every_engine;
use crate::support::{self, Db, Engine};
use crate::worker_check::child_url;

/// Who created the `failed_jobs` table a case runs against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Laravel 13's migration, from the committed fixture, with the failed
    /// job Laravel's worker logged.
    Laravel,
    /// [`CreateFailedJobsTable`] on an empty database.
    Migration,
}

pub const ORIGINS: [Origin; 2] = [Origin::Laravel, Origin::Migration];

/// The connection names Laravel 13's default `config/queue.php` defines.
const LARAVEL_CONNECTIONS: [&str; 8] = [
    "sync",
    "database",
    "beanstalkd",
    "sqs",
    "redis",
    "deferred",
    "background",
    "failover",
];

pub async fn failed_jobs_table(engine: Engine, origin: Origin) -> Db {
    match origin {
        Origin::Laravel => support::laravel(engine).await.0,
        Origin::Migration => {
            let db = support::empty(engine).await;
            CreateFailedJobsTable
                .up(&SchemaManager::new(&db.conn))
                .await
                .expect("the shipped failed_jobs migration runs on an empty database");
            db
        }
    }
}

pub fn envelope(job_name: &str, payload: serde_json::Value) -> Envelope {
    let now = Utc::now();
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: job_name.to_owned(),
        queue: None,
        payload,
        dispatched_at: now,
        available_at: now,
        attempts: 3,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
        context: None,
    }
}

fn store(db: &Db) -> DatabaseFailedJobStore {
    DatabaseFailedJobStore::new(db.conn.clone(), "failed_jobs".to_owned())
        .expect("failed_jobs is a valid table name")
}

async fn row(db: &Db, uuid: Uuid) -> serde_json::Value {
    let rows = support::rows(
        &db.conn,
        &format!(
            "SELECT uuid, connection, queue, payload, exception FROM failed_jobs \
             WHERE uuid = '{uuid}'"
        ),
    )
    .await;
    assert_eq!(rows.len(), 1, "exactly one row holds uuid {uuid}");
    rows[0].clone()
}

/// Each dead letter gets a fresh uuid, never the envelope id, and a second
/// dead letter of the same envelope (a redelivered one) is accepted.
async fn a_dead_letter_gets_a_fresh_uuid(engine: Engine) {
    for origin in ORIGINS {
        let db = failed_jobs_table(engine, origin).await;
        let store = store(&db);
        let env = envelope("App.SendInvoice", serde_json::json!({ "invoice": 7 }));
        let first = store
            .log("database", "default", &env, "boom")
            .await
            .unwrap_or_else(|e| panic!("{origin:?}: the first dead letter is written: {e}"));
        let second = store
            .log("database", "default", &env, "boom again")
            .await
            .unwrap_or_else(|e| {
                panic!("{origin:?}: a second dead letter of one envelope is written: {e}")
            });
        assert_ne!(first, env.id, "{origin:?}: the row reuses the envelope id");
        assert_ne!(second, env.id, "{origin:?}: the row reuses the envelope id");
        assert_ne!(first, second, "{origin:?}: two rows share a uuid");
        assert_eq!(
            support::text(&row(&db, first).await, "uuid"),
            first.to_string()
        );
        assert_eq!(
            support::text(&row(&db, second).await, "uuid"),
            second.to_string()
        );
    }
}

on_every_engine!(a_dead_letter_gets_a_fresh_uuid =>
    ldb_002_a_dead_letter_gets_a_fresh_uuid_sqlite,
    ldb_002_a_dead_letter_gets_a_fresh_uuid_postgres,
    ldb_002_a_dead_letter_gets_a_fresh_uuid_mysql);

/// `payload` is the envelope with `displayName` and `uuid` added, and
/// `connection` names the Suprnova connection in a form no Laravel 13
/// default connection takes.
async fn the_row_is_what_laravel_reads(engine: Engine) {
    for origin in ORIGINS {
        let db = failed_jobs_table(engine, origin).await;
        let store = store(&db);
        let env = envelope("App.Reports.Build", serde_json::json!({ "report": "q3" }));
        let id = store
            .log("database", "reports", &env, "the report failed")
            .await
            .expect("log");
        let stored = row(&db, id).await;
        let payload: serde_json::Value =
            serde_json::from_str(&support::text(&stored, "payload")).expect("payload is JSON");
        assert_eq!(payload["displayName"], "App.Reports.Build", "{origin:?}");
        assert_eq!(payload["uuid"], id.to_string(), "{origin:?}");
        let decoded = Envelope::from_json(&support::text(&stored, "payload"))
            .expect("the payload decodes back into the envelope");
        assert_eq!(decoded.id, env.id);
        assert_eq!(decoded.job_name, env.job_name);
        assert_eq!(decoded.payload, env.payload);
        let connection = support::text(&stored, "connection");
        assert!(
            !LARAVEL_CONNECTIONS.contains(&connection.as_str()),
            "{origin:?}: connection {connection:?} is a Laravel 13 default connection"
        );
        assert_eq!(support::text(&stored, "queue"), "reports");
        let found = store.find(id).await.expect("find").expect("the record");
        assert_eq!(
            found.connection, "database",
            "the record names its connection"
        );
        assert_eq!(found.job_name, "App.Reports.Build");
    }
}

on_every_engine!(the_row_is_what_laravel_reads =>
    ldb_002_the_payload_and_connection_are_what_laravel_reads_sqlite,
    ldb_002_the_payload_and_connection_are_what_laravel_reads_postgres,
    ldb_002_the_payload_and_connection_are_what_laravel_reads_mysql);

/// `failed_at` is within a second of the failure.
async fn failed_at_is_the_failure_time(engine: Engine) {
    for origin in ORIGINS {
        let db = failed_jobs_table(engine, origin).await;
        let store = store(&db);
        let before = Utc::now();
        let id = store
            .log(
                "database",
                "default",
                &envelope("App.Tick", serde_json::json!({})),
                "x",
            )
            .await
            .expect("log");
        let after = Utc::now();
        let failed_at = store
            .find(id)
            .await
            .expect("find")
            .expect("record")
            .failed_at;
        assert!(
            failed_at.timestamp() >= before.timestamp() - 1
                && failed_at.timestamp() <= after.timestamp() + 1,
            "{origin:?}: failed_at {failed_at} is not within a second of {before}..{after}"
        );
    }
}

on_every_engine!(failed_at_is_the_failure_time =>
    ldb_002_failed_at_is_within_a_second_of_the_failure_sqlite,
    ldb_002_failed_at_is_within_a_second_of_the_failure_postgres,
    ldb_002_failed_at_is_within_a_second_of_the_failure_mysql);

/// A 1 MiB payload and a 1 MiB exception are stored whole.
async fn a_mebibyte_is_stored_whole(engine: Engine) {
    const MIB: usize = 1024 * 1024;
    for origin in ORIGINS {
        let db = failed_jobs_table(engine, origin).await;
        let store = store(&db);
        let blob = "p".repeat(MIB);
        let exception = "e".repeat(MIB);
        let env = envelope("App.Big", serde_json::json!({ "blob": blob }));
        let id = store
            .log("database", "default", &env, &exception)
            .await
            .unwrap_or_else(|e| panic!("{origin:?}: a 1 MiB record is refused: {e}"));
        let found = store.find(id).await.expect("find").expect("record");
        assert_eq!(
            found.exception.len(),
            MIB,
            "{origin:?}: the exception was cut"
        );
        let decoded = Envelope::from_json(&found.envelope_json).expect("decode");
        assert_eq!(
            decoded.payload["blob"].as_str().map(str::len),
            Some(MIB),
            "{origin:?}: the payload was cut"
        );
    }
}

on_every_engine!(a_mebibyte_is_stored_whole =>
    ldb_002_a_mebibyte_payload_and_exception_are_stored_whole_sqlite,
    ldb_002_a_mebibyte_payload_and_exception_are_stored_whole_postgres,
    ldb_002_a_mebibyte_payload_and_exception_are_stored_whole_mysql);

/// What [`console_child`] prints on stderr before a failure
/// `run_with_args` returned to it.
const CONSOLE_FAILED: &str = "the console command failed: ";

/// Run `suprnova <args>` as a console command of a real `Application`, in
/// a child process of this test binary, against the database at `url`.
/// `QUEUE_DRIVER=database` wires the queue on `jobs` and the failed-jobs
/// store on `failed_jobs`, as an application's boot does.
fn console(url: &str, args: &[&str]) -> Output {
    Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", "failed_jobs::console_child", "--nocapture"])
        .env(
            "LDB_CONSOLE_CHILD",
            serde_json::to_string(args).expect("the arguments as JSON"),
        )
        .env("DATABASE_URL", url)
        .env("QUEUE_DRIVER", "database")
        .env("APP_ENV", "testing")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the child")
}

/// The child half of [`console`]: one console command through
/// `Application::run_with_args`. It does nothing unless [`console`]
/// started it.
#[test]
fn console_child() {
    let Ok(args) = std::env::var("LDB_CONSOLE_CHILD") else {
        return;
    };
    let args: Vec<String> = serde_json::from_str(&args).expect("the arguments");
    suprnova::boot::load_env().expect("load the configuration");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let outcome = runtime.block_on(async {
        suprnova::Application::new()
            .bootstrap(|| async {
                suprnova::Config::register(suprnova::DatabaseConfig::from_env());
                suprnova::DB::init().await.expect("connect");
            })
            .run_with_args(std::iter::once("app".to_owned()).chain(args))
            .await
    });
    // The executable boundary: print the failure and exit non-zero, as
    // `Application::run` does.
    if let Err(e) = outcome {
        eprintln!("{CONSOLE_FAILED}{}", e.message());
        std::process::exit(1);
    }
}

/// The child's stdout and stderr, for an assertion message.
fn printed(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// `failed_jobs` from `origin`, with the `jobs` table the queue pushes a
/// retried job onto (Laravel's own, on the Laravel fixture).
async fn console_database(engine: Engine, origin: Origin) -> Db {
    let db = failed_jobs_table(engine, origin).await;
    CreateJobsTable
        .up(&SchemaManager::new(&db.conn))
        .await
        .expect("the jobs table");
    db
}

/// `queue:failed` lists the rows, `queue:retry <uuid>` pushes the row's
/// envelope onto its queue and deletes the row, and `queue:forget <uuid>`
/// deletes the row.
async fn list_retry_and_forget(engine: Engine) {
    for origin in ORIGINS {
        let db = console_database(engine, origin).await;
        let url = child_url(engine, &db);
        let store = store(&db);
        let env = envelope("App.Retry.Me", serde_json::json!({ "n": 1 }));
        let retried = store
            .log("database", "default", &env, "boom")
            .await
            .unwrap();
        let forgotten = store
            .log(
                "database",
                "default",
                &envelope("App.Forget.Me", serde_json::json!({})),
                "boom",
            )
            .await
            .unwrap();

        let listed = console(&url, &["queue:failed"]);
        assert!(listed.status.success(), "{origin:?}: {}", printed(&listed));
        let stdout = String::from_utf8_lossy(&listed.stdout);
        for (id, job) in [(retried, "App.Retry.Me"), (forgotten, "App.Forget.Me")] {
            assert!(
                stdout
                    .lines()
                    .any(|line| line.contains(&id.to_string()) && line.contains(job)),
                "{origin:?}: queue:failed does not list {id} ({job}):\n{stdout}"
            );
        }

        let jobs = DatabaseQueueDriver::new(db.conn.clone(), "jobs".to_owned()).unwrap();
        let retry = console(&url, &["queue:retry", &retried.to_string()]);
        assert!(retry.status.success(), "{origin:?}: {}", printed(&retry));
        assert!(
            String::from_utf8_lossy(&retry.stdout).contains(&format!(
                "The failed job [{retried}] has been pushed back onto the queue."
            )),
            "{origin:?}: {}",
            printed(&retry)
        );
        assert!(
            store.find(retried).await.unwrap().is_none(),
            "{origin:?}: queue:retry kept the row"
        );
        let pushed = jobs
            .pop(std::time::Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{origin:?}: queue:retry pushed nothing"));
        assert_eq!(
            pushed.envelope.id, env.id,
            "{origin:?}: another envelope was pushed"
        );
        jobs.ack(&pushed.token).await.unwrap();

        let forget = console(&url, &["queue:forget", &forgotten.to_string()]);
        assert!(forget.status.success(), "{origin:?}: {}", printed(&forget));
        assert!(
            String::from_utf8_lossy(&forget.stdout)
                .contains(&format!("The failed job [{forgotten}] has been deleted.")),
            "{origin:?}: {}",
            printed(&forget)
        );
        assert!(
            store.find(forgotten).await.unwrap().is_none(),
            "{origin:?}: queue:forget kept the row"
        );
    }
}

on_every_engine!(list_retry_and_forget =>
    ldb_002_failed_lists_retry_pushes_and_forget_deletes_sqlite,
    ldb_002_failed_lists_retry_pushes_and_forget_deletes_postgres,
    ldb_002_failed_lists_retry_pushes_and_forget_deletes_mysql);

/// `queue:flush --hours=2` deletes a row older than two hours and keeps a
/// newer one.
async fn flush_hours_keeps_newer_rows(engine: Engine) {
    for origin in ORIGINS {
        let db = console_database(engine, origin).await;
        let store = store(&db);
        let old = store
            .log(
                "database",
                "default",
                &envelope("Old", serde_json::json!({})),
                "x",
            )
            .await
            .unwrap();
        let recent = store
            .log(
                "database",
                "default",
                &envelope("Recent", serde_json::json!({})),
                "x",
            )
            .await
            .unwrap();
        let backdate = |uuid: Uuid, hours: i64| {
            let at = (Utc::now() - chrono::Duration::hours(hours)).naive_utc();
            let backend = db.conn.get_database_backend();
            let sql = if backend == sea_orm::DbBackend::Postgres {
                "UPDATE failed_jobs SET failed_at = $1 WHERE uuid = $2"
            } else {
                "UPDATE failed_jobs SET failed_at = ? WHERE uuid = ?"
            };
            Statement::from_sql_and_values(backend, sql, [at.into(), uuid.to_string().into()])
        };
        db.conn.execute_raw(backdate(old, 3)).await.unwrap();
        db.conn.execute_raw(backdate(recent, 1)).await.unwrap();
        let flush = console(&child_url(engine, &db), &["queue:flush", "--hours=2"]);
        assert!(flush.status.success(), "{origin:?}: {}", printed(&flush));
        assert!(
            String::from_utf8_lossy(&flush.stdout).contains("older than 2 hour(s) deleted."),
            "{origin:?}: {}",
            printed(&flush)
        );
        assert!(
            store.find(old).await.unwrap().is_none(),
            "{origin:?}: older row kept"
        );
        assert!(
            store.find(recent).await.unwrap().is_some(),
            "{origin:?}: newer row deleted"
        );
    }
}

on_every_engine!(flush_hours_keeps_newer_rows =>
    ldb_002_flush_hours_deletes_older_rows_and_keeps_newer_sqlite,
    ldb_002_flush_hours_deletes_older_rows_and_keeps_newer_postgres,
    ldb_002_flush_hours_deletes_older_rows_and_keeps_newer_mysql);

/// The failed job Laravel's worker logged lists, and `queue:retry` refuses
/// it with an error naming its uuid and a non-zero exit, pushing nothing
/// and keeping the row.
async fn a_laravel_row_lists_and_retry_refuses_it(engine: Engine) {
    let db = console_database(engine, Origin::Laravel).await;
    let url = child_url(engine, &db);
    let store = store(&db);
    let laravel_row = support::rows(&db.conn, "SELECT uuid FROM failed_jobs").await;
    assert_eq!(laravel_row.len(), 1, "the fixture holds one failed job");
    let uuid = Uuid::parse_str(&support::text(&laravel_row[0], "uuid")).unwrap();

    let listed = console(&url, &["queue:failed"]);
    assert!(listed.status.success(), "{}", printed(&listed));
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(
        stdout.lines().any(|line| line.contains(&uuid.to_string())
            && line.contains("App\\Jobs\\FailingJob")
            && line.contains("failing")),
        "queue:failed does not list Laravel's row:\n{stdout}"
    );

    let queued = support::count(&db.conn, "jobs", "").await;
    let refused = console(&url, &["queue:retry", &uuid.to_string()]);
    assert_eq!(
        refused.status.code(),
        Some(1),
        "queue:retry did not fail on a row Laravel wrote: {}",
        printed(&refused)
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains(CONSOLE_FAILED) && stderr.contains(&uuid.to_string()),
        "the error does not name the uuid: {stderr}"
    );
    assert_eq!(
        support::count(&db.conn, "jobs", "").await,
        queued,
        "a Laravel payload was pushed"
    );
    assert!(
        store.find(uuid).await.unwrap().is_some(),
        "the row was deleted"
    );

    // `queue:retry all` leaves it too.
    let all = console(&url, &["queue:retry", "all"]);
    assert!(all.status.success(), "{}", printed(&all));
    assert!(
        store.find(uuid).await.unwrap().is_some(),
        "retry all deleted the row"
    );
    assert_eq!(support::count(&db.conn, "jobs", "").await, queued);
}

on_every_engine!(a_laravel_row_lists_and_retry_refuses_it =>
    ldb_002_a_laravel_row_lists_and_retry_refuses_it_sqlite,
    ldb_002_a_laravel_row_lists_and_retry_refuses_it_postgres,
    ldb_002_a_laravel_row_lists_and_retry_refuses_it_mysql);

/// Run the shipped `jobs` and `failed_jobs` migrations with the tables
/// named `jobs` and `failed`, as `QUEUE_DB_TABLE` and
/// `QUEUE_FAILED_DB_TABLE` name them.
pub(crate) async fn migrate_queue_tables(db: &Db, jobs: &str, failed: &str) {
    // SAFETY: the test is serial within its process, and nothing else reads
    // the environment while the migrations run.
    unsafe {
        std::env::set_var("QUEUE_DB_TABLE", jobs);
        std::env::set_var("QUEUE_FAILED_DB_TABLE", failed);
    }
    let manager = SchemaManager::new(&db.conn);
    let migrated = match suprnova::queue::migrations::CreateJobsTable
        .up(&manager)
        .await
    {
        Ok(()) => CreateFailedJobsTable.up(&manager).await,
        Err(e) => Err(e),
    };
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("QUEUE_DB_TABLE");
        std::env::remove_var("QUEUE_FAILED_DB_TABLE");
    }
    migrated.unwrap_or_else(|e| panic!("the queue migrations for {jobs} and {failed}: {e}"));
}

/// A queue table named with capitals or with a reserved word works from
/// the shipped migration through every operation of the driver and the
/// store. The migrations create the names quoted; the stores have to quote
/// them the same way, or Postgres folds `FailedJobs` to `failedjobs` and
/// every engine reads `order` as a keyword.
async fn configured_names_are_quoted_alike(engine: Engine) {
    for (jobs, failed) in [("QueueJobs", "FailedJobs"), ("order", "select")] {
        let db = support::empty(engine).await;
        migrate_queue_tables(&db, jobs, failed).await;

        let driver = suprnova::DatabaseQueueDriver::new(db.conn.clone(), jobs.to_owned())
            .expect("a valid jobs table name");
        driver
            .push(envelope("Ldb.Quoted", serde_json::json!({ "n": 1 })))
            .await
            .unwrap_or_else(|e| panic!("{engine:?} {jobs}: push: {e}"));
        assert_eq!(driver.size().await.expect("size"), 1, "{engine:?} {jobs}");
        let popped = driver
            .pop(std::time::Duration::from_secs(30))
            .await
            .unwrap_or_else(|e| panic!("{engine:?} {jobs}: pop: {e}"))
            .expect("the job just pushed");
        driver
            .nack(&popped.token, std::time::Duration::ZERO)
            .await
            .unwrap_or_else(|e| panic!("{engine:?} {jobs}: nack: {e}"));
        let popped = driver
            .pop(std::time::Duration::from_secs(30))
            .await
            .expect("pop again")
            .expect("the nacked job");
        driver
            .ack(&popped.token)
            .await
            .unwrap_or_else(|e| panic!("{engine:?} {jobs}: ack: {e}"));
        assert_eq!(driver.size().await.expect("size"), 0, "{engine:?} {jobs}");

        let store = DatabaseFailedJobStore::new(db.conn.clone(), failed.to_owned())
            .expect("a valid failed-jobs table name");
        store.check().await.unwrap_or_else(|e| {
            panic!("{engine:?} {failed}: the worker's check refused the migrated table: {e}")
        });
        let env = envelope("Ldb.Quoted", serde_json::json!({ "n": 2 }));
        let id = store
            .log("database", "default", &env, "boom")
            .await
            .unwrap_or_else(|e| panic!("{engine:?} {failed}: log: {e}"));
        assert_eq!(store.all().await.expect("all").len(), 1, "{engine:?}");
        assert!(store.find(id).await.expect("find").is_some(), "{engine:?}");
        assert_eq!(store.count().await.expect("count"), 1, "{engine:?}");
        assert!(store.forget(id).await.expect("forget"), "{engine:?}");
        store
            .log("database", "default", &env, "boom")
            .await
            .expect("log again");
        assert_eq!(store.flush(None).await.expect("flush"), 1, "{engine:?}");
    }
}

on_every_engine!(configured_names_are_quoted_alike =>
    ldb_002_capitalized_and_reserved_queue_table_names_work_sqlite,
    ldb_002_capitalized_and_reserved_queue_table_names_work_postgres,
    ldb_002_capitalized_and_reserved_queue_table_names_work_mysql);
