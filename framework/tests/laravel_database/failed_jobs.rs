//! LDB-002: `failed_jobs` in Laravel 13's layout, on a table Laravel
//! created and on one the shipped migration created.

use std::sync::Arc;

use chrono::Utc;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::queue::migrations::CreateFailedJobsTable;
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION};
use suprnova::{
    DatabaseFailedJobStore, Envelope, FailedJobStore, MemoryQueueDriver, Queue, QueueDriver,
};
use uuid::Uuid;

use crate::on_every_engine;
use crate::support::{self, Db, Engine};

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

/// `queue:failed` lists the row, `queue:retry <uuid>` pushes its envelope
/// and deletes it, `queue:forget <uuid>` deletes it. The commands are thin
/// wrappers over these calls (`framework/src/queue/failed_console.rs`).
async fn list_retry_and_forget(engine: Engine) {
    for origin in ORIGINS {
        let db = failed_jobs_table(engine, origin).await;
        let store = Arc::new(store(&db));
        let memory = Arc::new(MemoryQueueDriver::new());
        Queue::register_connection("database", memory.clone());
        Queue::set_failed_store(store.clone());

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

        let listed: Vec<Uuid> = store
            .all()
            .await
            .expect("list")
            .iter()
            .map(|r| r.id)
            .collect();
        assert!(
            listed.contains(&retried) && listed.contains(&forgotten),
            "{origin:?}"
        );

        assert!(
            Queue::retry_failed(retried).await.expect("retry"),
            "{origin:?}"
        );
        assert_eq!(
            memory.size().await.unwrap(),
            1,
            "{origin:?}: nothing was pushed"
        );
        let pushed = memory
            .pop(std::time::Duration::from_secs(30))
            .await
            .unwrap()
            .expect("the retried job");
        assert_eq!(
            pushed.envelope.id, env.id,
            "{origin:?}: another envelope was pushed"
        );
        assert!(
            store.find(retried).await.unwrap().is_none(),
            "{origin:?}: row kept"
        );

        assert!(store.forget(forgotten).await.expect("forget"), "{origin:?}");
        assert!(
            store.find(forgotten).await.unwrap().is_none(),
            "{origin:?}: row kept"
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
        let db = failed_jobs_table(engine, origin).await;
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
        let cutoff = Utc::now() - chrono::Duration::hours(2);
        store.flush(Some(cutoff)).await.expect("flush");
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
/// it with an error naming its uuid, pushing nothing and keeping the row.
async fn a_laravel_row_lists_and_retry_refuses_it(engine: Engine) {
    let db = failed_jobs_table(engine, Origin::Laravel).await;
    let store = Arc::new(store(&db));
    let laravel_row = support::rows(&db.conn, "SELECT uuid FROM failed_jobs").await;
    assert_eq!(laravel_row.len(), 1, "the fixture holds one failed job");
    let uuid = Uuid::parse_str(&support::text(&laravel_row[0], "uuid")).unwrap();

    let listed = store.all().await.expect("Laravel's row lists");
    let record = listed.iter().find(|r| r.id == uuid).expect("Laravel's row");
    assert_eq!(record.job_name, "App\\Jobs\\FailingJob");
    assert_eq!(record.queue, "failing");

    let memory = Arc::new(MemoryQueueDriver::new());
    Queue::register_connection("database", memory.clone());
    Queue::set_failed_store(store.clone());
    let refused = Queue::retry_failed(uuid)
        .await
        .expect_err("queue:retry must refuse a row Laravel wrote");
    assert!(
        refused.to_string().contains(&uuid.to_string()),
        "the error names the uuid: {refused}"
    );
    assert_eq!(
        memory.size().await.unwrap(),
        0,
        "a Laravel payload was pushed"
    );
    assert!(
        store.find(uuid).await.unwrap().is_some(),
        "the row was deleted"
    );

    // `queue:retry all` leaves it too.
    Queue::set_driver(memory.clone());
    Queue::retry_all_failed(None).await.expect("retry all");
    assert!(
        store.find(uuid).await.unwrap().is_some(),
        "retry all deleted the row"
    );
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
