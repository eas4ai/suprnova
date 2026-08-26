//! PostgreSQL coverage for the database queue driver and the database
//! failed-jobs store (DATA-01).
//!
//! Both stores hand-write their SQL. Before DATA-01 they emitted `?`
//! positional placeholders - SQLite/MySQL syntax that Postgres rejects
//! outright - so every parameterised statement (push, ack, nack, the
//! counters, the queue filter, and the whole failed-jobs surface) failed
//! on Postgres. The rest of the suite is SQLite-only, which is exactly why
//! that survived; these tests exercise the same paths against a real
//! Postgres.
//!
//! Run with a disposable Postgres:
//!
//! ```text
//! docker run -d --rm --name suprnova-pg -e POSTGRES_PASSWORD=pw \
//!     -e POSTGRES_DB=suprnova_test -p 55999:5432 postgres:17-alpine
//! PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55999/suprnova_test \
//!     cargo test -p suprnova --test queue_database_postgres -- --ignored
//! ```

use chrono::Utc;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use std::time::Duration;
use suprnova::queue::database::DatabaseQueueDriver;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::failed::{DatabaseFailedJobStore, FailedJobStore};
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope};
use uuid::Uuid;

async fn connect_postgres() -> DatabaseConnection {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(4)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

/// Create the `jobs` table exactly as `manual/queues.md` documents it, so
/// the placeholder fix is proved against the schema users actually copy.
/// Each test owns its own table name - the binary's tests run in parallel
/// against one database.
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

fn env(name: &str) -> Envelope {
    let now = Utc::now();
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: name.into(),
        queue: None,
        payload: serde_json::json!({}),
        dispatched_at: now,
        available_at: now,
        attempts: 0,
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
    }
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_push_pop_ack_round_trips() {
    let db = connect_postgres().await;
    fresh_jobs_table(&db, "pg_jobs_push_ack").await;
    let d = DatabaseQueueDriver::new(db, "pg_jobs_push_ack".into()).unwrap();

    d.push(env("A")).await.expect("push A");
    d.push(env("B")).await.expect("push B");

    let r1 = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("pop A");
    let r2 = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("pop B");
    assert_eq!(r1.envelope.job_name, "A");
    assert_eq!(r2.envelope.job_name, "B");

    d.ack(&r1.token).await.expect("ack A");
    d.ack(&r2.token).await.expect("ack B");

    assert!(
        d.pop(Duration::from_millis(10)).await.unwrap().is_none(),
        "queue drained"
    );
    assert_eq!(d.size().await.unwrap(), 0);
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_nack_bumps_attempts_and_requeues() {
    let db = connect_postgres().await;
    fresh_jobs_table(&db, "pg_jobs_nack").await;
    let d = DatabaseQueueDriver::new(db, "pg_jobs_nack".into()).unwrap();

    d.push(env("A")).await.expect("push");
    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().expect("pop");
    assert_eq!(r1.envelope.attempts, 0);

    d.nack(&r1.token, Duration::from_millis(0))
        .await
        .expect("nack");

    let r2 = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("requeued");
    assert_eq!(
        r2.envelope.attempts, 1,
        "nack must bump attempts (per trait contract)"
    );
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_counters_report_pending_delayed_and_reserved() {
    let db = connect_postgres().await;
    fresh_jobs_table(&db, "pg_jobs_counters").await;
    let d = DatabaseQueueDriver::new(db, "pg_jobs_counters".into()).unwrap();

    let mut later = env("later");
    later.available_at = Utc::now() + chrono::Duration::seconds(3600);
    d.push(env("now")).await.expect("push now");
    d.push(later).await.expect("push later");

    assert_eq!(d.size().await.unwrap(), 2);
    assert_eq!(d.pending_size().await.unwrap(), 1);
    assert_eq!(d.delayed_size().await.unwrap(), 1);
    assert_eq!(d.reserved_size().await.unwrap(), 0);

    let _r = d
        .pop(Duration::from_secs(600))
        .await
        .unwrap()
        .expect("visible job");
    assert_eq!(d.reserved_size().await.unwrap(), 1);
    assert_eq!(d.pending_size().await.unwrap(), 0);

    assert_eq!(d.clear().await.unwrap(), 2);
    assert_eq!(d.size().await.unwrap(), 0);
}

/// The queue filter binds one placeholder per queue name *after* the two
/// timestamp binds, so it is the site where Postgres' ordinal numbering
/// actually has to be threaded rather than emitted as a constant.
#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_pop_from_filters_by_queue_and_treats_null_as_default() {
    let db = connect_postgres().await;
    fresh_jobs_table(&db, "pg_jobs_routing").await;
    let d = DatabaseQueueDriver::new(db, "pg_jobs_routing".into()).unwrap();

    let mut billing = env("billing-job");
    billing.queue = Some("billing".into());
    let mut reports = env("reports-job");
    reports.queue = Some("reports".into());
    let legacy = env("legacy-job"); // queue stays NULL

    d.push(billing).await.unwrap();
    d.push(reports).await.unwrap();
    d.push(legacy).await.unwrap();

    // Two names in the IN list - $3 and $4.
    let got = d
        .pop_from(
            Duration::from_secs(60),
            &["billing".to_string(), "reports".to_string()],
        )
        .await
        .unwrap()
        .expect("billing or reports job");
    assert!(matches!(
        got.envelope.job_name.as_str(),
        "billing-job" | "reports-job"
    ));
    d.ack(&got.token).await.unwrap();

    // A default worker still sees the NULL-queue row.
    let got = d
        .pop_from(Duration::from_secs(60), &["default".to_string()])
        .await
        .unwrap()
        .expect("legacy job reachable as default");
    assert_eq!(got.envelope.job_name, "legacy-job");
    d.ack(&got.token).await.unwrap();

    // A hostile queue name must match nothing rather than alter the statement.
    let hostile = vec!["billing') OR 1=1 --".to_string()];
    assert!(
        d.pop_from(Duration::from_secs(60), &hostile)
            .await
            .unwrap()
            .is_none(),
        "injection attempt must not widen the result set"
    );
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_failed_store_log_find_forget_and_flush() {
    let db = connect_postgres().await;
    fresh_failed_jobs_table(&db, "pg_failed_jobs").await;
    let store = DatabaseFailedJobStore::new(db, "pg_failed_jobs".into()).unwrap();

    let id = store
        .log("database", "default", &env("Boom"), "it exploded")
        .await
        .expect("log");
    let other = store
        .log("database", "billing", &env("Bang"), "it also exploded")
        .await
        .expect("log");

    assert_eq!(store.count().await.unwrap(), 2);
    assert_eq!(store.ids().await.unwrap().len(), 2);

    let found = store.find(id).await.expect("find").expect("row present");
    assert_eq!(found.job_name, "Boom");
    assert_eq!(found.queue, "default");
    assert!(found.exception.contains("it exploded"));

    assert!(store.forget(id).await.expect("forget"));
    assert!(!store.forget(id).await.expect("forget is idempotent"));
    assert!(store.find(id).await.expect("find").is_none());
    assert_eq!(store.count().await.unwrap(), 1);

    // Cutoff in the future removes the remaining row; the bound cutoff is
    // the only parameter, so this covers the `flush(Some(..))` branch.
    let flushed = store
        .flush(Some(Utc::now() + chrono::Duration::seconds(60)))
        .await
        .expect("flush");
    assert_eq!(flushed, 1);
    assert!(store.find(other).await.expect("find").is_none());
    assert_eq!(store.count().await.unwrap(), 0);
}
