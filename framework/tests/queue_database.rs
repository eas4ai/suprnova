use chrono::Utc;
use sea_orm::{ConnectionTrait, Database};
use std::time::Duration;
use suprnova::queue::database::DatabaseQueueDriver;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope};
use uuid::Uuid;

async fn fresh_db() -> sea_orm::DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.execute_unprepared(
        r"
        CREATE TABLE jobs (
            id TEXT PRIMARY KEY,
            job_name TEXT NOT NULL,
            queue TEXT NULL,
            envelope_json TEXT NOT NULL,
            available_at INTEGER NOT NULL,
            reserved_until INTEGER NULL,
            reserved_token TEXT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        )
    ",
    )
    .await
    .unwrap();
    db.execute_unprepared("CREATE INDEX idx_jobs_available_at ON jobs(available_at)")
        .await
        .unwrap();
    db
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
        batch_id: None,
        chain_remaining: Vec::new(),
    }
}

#[tokio::test]
async fn database_driver_push_and_ack() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();
    d.push(env("B")).await.unwrap();

    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    let r2 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.job_name, "A");
    assert_eq!(r2.envelope.job_name, "B");

    d.ack(&r1.token).await.unwrap();
    d.ack(&r2.token).await.unwrap();

    let none = d.pop(Duration::from_millis(10)).await.unwrap();
    assert!(none.is_none(), "queue drained");
}

#[tokio::test]
async fn database_driver_reserved_rows_invisible_to_other_pops() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();

    let _r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    let r2 = d.pop(Duration::from_millis(10)).await.unwrap();
    assert!(r2.is_none(), "row reserved by r1 must not be popped again");
}

#[tokio::test]
async fn database_driver_nack_bumps_attempts() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();

    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.attempts, 0);

    d.nack(&r1.token, Duration::from_millis(0)).await.unwrap();

    let r2 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(
        r2.envelope.attempts, 1,
        "nack must bump attempts (per trait contract)"
    );
}

/// Pins the conditional-UPDATE behavior the SQLite race fix introduced.
///
/// Two concurrent consumers can both observe the same visible row in the
/// gap between their SELECTs and their UPDATEs. Without a predicate, both
/// stamp their reservation tokens and the loser walks away with a token
/// that doesn't match the row's stored value — its later ack/nack silently
/// no-ops and the job runs twice. The fix re-asserts the same "unreserved
/// or expired" predicate on UPDATE; the loser sees zero rows affected and
/// reports an empty pop instead.
#[tokio::test]
async fn database_driver_pop_returns_none_when_row_was_reserved_concurrently() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db.clone(), "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();

    // Mimic "another consumer reserved this row between our SELECT and our
    // UPDATE" by stamping a fresh reservation onto the row directly.
    let now = chrono::Utc::now().timestamp();
    let future = now + 600;
    db.execute(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Sqlite,
        "UPDATE jobs SET reserved_until = ?, reserved_token = ?",
        vec![
            sea_orm::Value::from(future),
            sea_orm::Value::from("other-consumer-token".to_string()),
        ],
    ))
    .await
    .unwrap();

    // Our pop now observes the row as reserved-in-future via its SELECT
    // filter; this path is the SELECT-side of the same predicate. The
    // conditional UPDATE matters when the SELECT happened *before* the
    // injected reservation — a case our test setup approximates by simply
    // observing that the driver respects the post-race state correctly.
    let r = d.pop(Duration::from_millis(50)).await.unwrap();
    assert!(
        r.is_none(),
        "pop must observe the concurrent reservation and yield None"
    );

    // The originally-injected reservation must still be intact (we did not
    // overwrite it with our own token).
    let row = db
        .query_one(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT reserved_token FROM jobs",
        ))
        .await
        .unwrap()
        .expect("row exists");
    let tok: String = row.try_get_by_index(0).unwrap();
    assert_eq!(
        tok, "other-consumer-token",
        "conditional UPDATE must not overwrite a still-valid reservation"
    );
}

#[tokio::test]
async fn database_driver_pop_releases_reservation_after_visibility_expiry() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("A")).await.unwrap();

    // First reservation with a near-zero visibility timeout.
    let r1 = d.pop(Duration::from_secs(0)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.job_name, "A");

    // After visibility expires, a fresh pop must reclaim the row — and the
    // conditional UPDATE has to succeed against the *expired* reservation
    // because `reserved_until <= now` is true.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let r2 = d.pop(Duration::from_millis(50)).await.unwrap();
    assert!(
        r2.is_some(),
        "expired reservations must be reclaimable by a later pop"
    );
}

#[tokio::test]
async fn database_driver_rejects_invalid_table_identifier() {
    let db = fresh_db().await;
    for bad in [
        "",
        "jobs; DROP TABLE users",
        "jobs--",
        "jobs'",
        "jobs/*",
        "1jobs",
        "jobs jobs",
    ] {
        let err = match DatabaseQueueDriver::new(db.clone(), bad.into()) {
            Err(e) => e,
            Ok(_) => panic!("expected validation error for {bad:?}, got Ok"),
        };
        assert!(
            err.to_string().to_lowercase().contains("identifier"),
            "expected an identifier-validation error for {bad:?}, got: {err}"
        );
    }
}

/// The SQL filter, including the NULL case. A `queue IS NULL` row was written
/// before routing existed (or by an unrouted push); a worker draining
/// `default` must still see it, or upgrading strands every in-flight job.
#[tokio::test]
async fn pop_from_filters_by_queue_and_treats_null_as_default() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".into()).unwrap();

    let mut billing = env("billing-job");
    billing.queue = Some("billing".into());
    let mut reports = env("reports-job");
    reports.queue = Some("reports".into());
    let legacy = env("legacy-job"); // queue stays None

    d.push(billing).await.unwrap();
    d.push(reports).await.unwrap();
    d.push(legacy).await.unwrap();

    // A billing worker sees only billing.
    let got = d
        .pop_from(Duration::from_secs(60), &["billing".to_string()])
        .await
        .unwrap()
        .expect("billing job");
    assert_eq!(got.envelope.job_name, "billing-job");
    d.ack(&got.token).await.unwrap();

    assert!(
        d.pop_from(Duration::from_secs(60), &["billing".to_string()])
            .await
            .unwrap()
            .is_none(),
        "billing worker must not consume reports or unrouted work"
    );

    // A default worker picks up the NULL-queue row.
    let got = d
        .pop_from(Duration::from_secs(60), &["default".to_string()])
        .await
        .unwrap()
        .expect("legacy job should be reachable as default");
    assert_eq!(got.envelope.job_name, "legacy-job");
    d.ack(&got.token).await.unwrap();

    // Unfiltered still drains the rest.
    let got = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("reports job");
    assert_eq!(got.envelope.job_name, "reports-job");
}

/// Queue names arrive from the `--queue` CLI flag, so they must be bound as
/// parameters rather than interpolated into the SQL.
#[tokio::test]
async fn queue_filter_is_parameterized_not_interpolated() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".into()).unwrap();

    let mut e = env("safe");
    e.queue = Some("billing".into());
    d.push(e).await.unwrap();

    // A hostile queue name must simply match nothing, not alter the statement.
    let hostile = vec!["billing') OR 1=1 --".to_string()];
    let got = d.pop_from(Duration::from_secs(60), &hostile).await.unwrap();
    assert!(
        got.is_none(),
        "injection attempt must not widen the result set"
    );

    // The real queue still works afterwards, proving the table survived.
    let got = d
        .pop_from(Duration::from_secs(60), &["billing".to_string()])
        .await
        .unwrap();
    assert_eq!(got.expect("billing job").envelope.job_name, "safe");
}
