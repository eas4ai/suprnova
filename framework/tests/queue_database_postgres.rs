//! PostgreSQL coverage for the database queue driver, database batch
//! repository, and database failed-jobs store (DATA-01).
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
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
};
use std::{sync::Arc, time::Duration};
use suprnova::queue::database::DatabaseQueueDriver;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::failed::{DatabaseFailedJobStore, FailedJobStore};
use suprnova::queue::{
    BackoffSchedule, Batch, BatchOptions, BatchRepository, CURRENT_SCHEMA_VERSION,
    DatabaseBatchRepository, Envelope,
};
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

async fn fresh_batch_tables(
    db: &DatabaseConnection,
    batches: &str,
    settlements: &str,
    gate_function: &str,
    gate_trigger: &str,
    gate_key: i64,
) {
    for sql in [
        format!("DROP TABLE IF EXISTS {settlements} CASCADE"),
        format!("DROP TABLE IF EXISTS {batches} CASCADE"),
        format!("DROP FUNCTION IF EXISTS {gate_function}() CASCADE"),
        format!(
            "CREATE TABLE {batches} (
                id            TEXT PRIMARY KEY,
                name          TEXT NOT NULL,
                total_jobs    BIGINT NOT NULL,
                options_json  TEXT NOT NULL,
                created_at    BIGINT NOT NULL,
                cancelled_at  BIGINT NULL,
                finished_at   BIGINT NULL
            )"
        ),
        format!(
            "CREATE TABLE {settlements} (
                batch_id   TEXT NOT NULL,
                job_id     TEXT NOT NULL,
                failed     INTEGER NOT NULL,
                settled_at BIGINT NOT NULL,
                PRIMARY KEY (batch_id, job_id)
            )"
        ),
        format!(
            "CREATE FUNCTION {gate_function}() RETURNS trigger AS $$
             BEGIN
                 PERFORM pg_advisory_xact_lock_shared({gate_key});
                 RETURN NEW;
             END;
             $$ LANGUAGE plpgsql"
        ),
        format!(
            "CREATE CONSTRAINT TRIGGER {gate_trigger}
             AFTER INSERT ON {settlements}
             DEFERRABLE INITIALLY DEFERRED
             FOR EACH ROW EXECUTE FUNCTION {gate_function}()"
        ),
    ] {
        db.execute_unprepared(&sql).await.expect("batch fixture");
    }
}

fn batch(name: &str, total_jobs: u64) -> Batch {
    Batch {
        id: Uuid::new_v4().to_string(),
        name: name.into(),
        total_jobs,
        pending_jobs: total_jobs,
        failed_jobs: 0,
        failed_job_ids: Vec::new(),
        options: BatchOptions::default(),
        created_at: Utc::now(),
        cancelled_at: None,
        finished_at: None,
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

/// Hold both transactions after they derive their counts but before either
/// commit. Without a parent-row lock each sees only its own settlement and
/// both report one pending job; no worker observes the terminal zero.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn concurrent_final_batch_settlements_elect_one_terminal_observer() {
    const BATCHES: &str = "pg_batch_terminal_batches";
    const SETTLEMENTS: &str = "pg_batch_terminal_settlements";
    const GATE_FUNCTION: &str = "pg_batch_terminal_gate_fn";
    const GATE_TRIGGER: &str = "pg_batch_terminal_gate";
    const GATE_KEY: i64 = 20_808;

    let db = connect_postgres().await;
    fresh_batch_tables(
        &db,
        BATCHES,
        SETTLEMENTS,
        GATE_FUNCTION,
        GATE_TRIGGER,
        GATE_KEY,
    )
    .await;

    let repo = Arc::new(
        DatabaseBatchRepository::with_tables(
            db.clone(),
            BATCHES.to_string(),
            SETTLEMENTS.to_string(),
        )
        .expect("valid fixture table names"),
    );
    let batch = batch("terminal-election", 2);
    let batch_id = batch.id.clone();
    repo.store(batch).await.expect("store batch");

    // The deferred trigger takes this lock during COMMIT. Holding it lets the
    // test prove whether both transactions reached commit before either one
    // could make its settlement visible.
    let mut gate = db
        .get_postgres_connection_pool()
        .acquire()
        .await
        .expect("acquire gate connection");
    sea_orm::sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(GATE_KEY)
        .execute(&mut *gate)
        .await
        .expect("hold commit gate");

    let start = Arc::new(tokio::sync::Barrier::new(3));
    let settle = |job_id| {
        let repo = Arc::clone(&repo);
        let batch_id = batch_id.clone();
        let start = Arc::clone(&start);
        tokio::spawn(async move {
            start.wait().await;
            repo.record_successful_job(&batch_id, job_id).await
        })
    };
    let first = settle(Uuid::new_v4());
    let second = settle(Uuid::new_v4());
    start.wait().await;

    // Old code reaches the deferred gate twice. Fixed code reaches it once
    // while the other transaction waits on the parent row's FOR UPDATE lock.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    format!(
                        "SELECT
                           (SELECT COUNT(*) FROM pg_locks
                            WHERE locktype = 'advisory'
                              AND classid = 0
                              AND objid = {GATE_KEY}
                              AND objsubid = 1
                              AND NOT granted),
                           EXISTS (
                               SELECT 1 FROM pg_stat_activity
                               WHERE datname = current_database()
                                 AND wait_event_type = 'Lock'
                                 AND query LIKE '%{BATCHES}%'
                                 AND query LIKE '%FOR UPDATE%'
                           )"
                    ),
                ))
                .await
                .expect("inspect settlement waiters")
                .expect("waiter counts row");
            let gate_waiters: i64 = row.try_get_by_index(0).expect("gate waiter count");
            let row_lock_waiter: bool = row.try_get_by_index(1).expect("row-lock waiter flag");
            if gate_waiters >= 2 || (gate_waiters >= 1 && row_lock_waiter) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("settlements must reach either the commit gate or parent-row lock");

    let unlocked: bool = sea_orm::sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(GATE_KEY)
        .fetch_one(&mut *gate)
        .await
        .expect("release commit gate");
    assert!(unlocked, "the test connection must own the commit gate");
    drop(gate);

    let (first, second) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(first, second)
    })
    .await
    .expect("both settlements must finish after the gate opens");
    let mut pending = vec![
        first
            .expect("first settlement task")
            .expect("first settlement")
            .pending_jobs,
        second
            .expect("second settlement task")
            .expect("second settlement")
            .pending_jobs,
    ];
    pending.sort_unstable();

    for sql in [
        format!("DROP TABLE {SETTLEMENTS}"),
        format!("DROP TABLE {BATCHES}"),
        format!("DROP FUNCTION {GATE_FUNCTION}()"),
    ] {
        db.execute_unprepared(&sql)
            .await
            .expect("clean batch fixture");
    }

    assert_eq!(
        pending,
        vec![0, 1],
        "exactly one final settlement must observe pending_jobs == 0"
    );
}

/// A growth request queued behind the final settlement must not reopen the
/// batch after the worker has already received the terminal zero snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn completed_batch_cannot_be_resurrected_by_concurrent_growth() {
    const BATCHES: &str = "pg_batch_growth_batches";
    const SETTLEMENTS: &str = "pg_batch_growth_settlements";
    const GATE_FUNCTION: &str = "pg_batch_growth_gate_fn";
    const GATE_TRIGGER: &str = "pg_batch_growth_gate";
    const GATE_KEY: i64 = 20_811;

    let db = connect_postgres().await;
    fresh_batch_tables(
        &db,
        BATCHES,
        SETTLEMENTS,
        GATE_FUNCTION,
        GATE_TRIGGER,
        GATE_KEY,
    )
    .await;
    let repo = Arc::new(
        DatabaseBatchRepository::with_tables(
            db.clone(),
            BATCHES.to_string(),
            SETTLEMENTS.to_string(),
        )
        .expect("valid fixture table names"),
    );
    let batch = batch("growth-order", 1);
    let batch_id = batch.id.clone();
    repo.store(batch).await.expect("store batch");

    let mut gate = db
        .get_postgres_connection_pool()
        .acquire()
        .await
        .expect("acquire growth-gate connection");
    sea_orm::sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(GATE_KEY)
        .execute(&mut *gate)
        .await
        .expect("hold growth gate");

    let settling = {
        let repo = Arc::clone(&repo);
        let batch_id = batch_id.clone();
        tokio::spawn(async move { repo.record_successful_job(&batch_id, Uuid::new_v4()).await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    format!(
                        "SELECT COUNT(*) FROM pg_locks
                         WHERE locktype = 'advisory'
                           AND classid = 0
                           AND objid = {GATE_KEY}
                           AND objsubid = 1
                           AND NOT granted"
                    ),
                ))
                .await
                .expect("inspect terminal settlement waiter")
                .expect("terminal settlement waiter row");
            let waiters: i64 = row.try_get_by_index(0).expect("terminal waiter count");
            if waiters >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("final settlement must pause at commit");

    let growing = {
        let repo = Arc::clone(&repo);
        let batch_id = batch_id.clone();
        tokio::spawn(async move { repo.increment_total_jobs(&batch_id, 1).await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    format!(
                        "SELECT EXISTS (
                             SELECT 1 FROM pg_stat_activity
                             WHERE datname = current_database()
                               AND wait_event_type = 'Lock'
                               AND query LIKE '%{BATCHES}%'
                         )"
                    ),
                ))
                .await
                .expect("inspect growth lock waiter")
                .expect("growth waiter row");
            let row_lock_waiter: bool = row.try_get_by_index(0).expect("growth waiter flag");
            if row_lock_waiter {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("growth must wait behind the final settlement");

    let unlocked: bool = sea_orm::sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(GATE_KEY)
        .fetch_one(&mut *gate)
        .await
        .expect("release growth gate");
    assert!(unlocked, "the test connection must own the growth gate");
    drop(gate);

    let (settled, grown) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(settling, growing)
    })
    .await
    .expect("settlement and growth must finish after the gate opens");
    assert_eq!(
        settled
            .expect("settlement task")
            .expect("final settlement")
            .pending_jobs,
        0
    );
    assert!(
        grown.expect("growth task").is_err(),
        "growth ordered after terminal settlement must be rejected"
    );
    let snapshot = repo.find(&batch_id).await.unwrap().unwrap();
    assert_eq!(snapshot.total_jobs, 1);
    assert_eq!(snapshot.pending_jobs, 0);

    for sql in [
        format!("DROP TABLE {SETTLEMENTS}"),
        format!("DROP TABLE {BATCHES}"),
        format!("DROP FUNCTION {GATE_FUNCTION}()"),
    ] {
        db.execute_unprepared(&sql)
            .await
            .expect("clean growth fixture");
    }
}

/// Deletion and settlement must acquire the parent batch in the same order.
/// Otherwise deletion can clear the old children, a concurrent settlement can
/// insert a new child, and deletion can then remove only the parent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn concurrent_batch_delete_cannot_leave_an_orphan_settlement() {
    const BATCHES: &str = "pg_batch_delete_batches";
    const SETTLEMENTS: &str = "pg_batch_delete_settlements";
    const INSERT_GATE_FUNCTION: &str = "pg_batch_delete_insert_gate_fn";
    const INSERT_GATE_TRIGGER: &str = "pg_batch_delete_insert_gate";
    const INSERT_GATE_KEY: i64 = 20_809;
    const DELETE_GATE_FUNCTION: &str = "pg_batch_delete_gate_fn";
    const DELETE_GATE_TRIGGER: &str = "pg_batch_delete_gate";
    const DELETE_GATE_KEY: i64 = 20_810;

    let db = connect_postgres().await;
    fresh_batch_tables(
        &db,
        BATCHES,
        SETTLEMENTS,
        INSERT_GATE_FUNCTION,
        INSERT_GATE_TRIGGER,
        INSERT_GATE_KEY,
    )
    .await;
    for sql in [
        format!("DROP FUNCTION IF EXISTS {DELETE_GATE_FUNCTION}() CASCADE"),
        format!(
            "CREATE FUNCTION {DELETE_GATE_FUNCTION}() RETURNS trigger AS $$
             BEGIN
                 PERFORM pg_advisory_xact_lock_shared({DELETE_GATE_KEY});
                 RETURN OLD;
             END;
             $$ LANGUAGE plpgsql"
        ),
        format!(
            "CREATE TRIGGER {DELETE_GATE_TRIGGER}
             AFTER DELETE ON {SETTLEMENTS}
             FOR EACH ROW EXECUTE FUNCTION {DELETE_GATE_FUNCTION}()"
        ),
    ] {
        db.execute_unprepared(&sql)
            .await
            .expect("install delete gate");
    }

    let repo = Arc::new(
        DatabaseBatchRepository::with_tables(
            db.clone(),
            BATCHES.to_string(),
            SETTLEMENTS.to_string(),
        )
        .expect("valid fixture table names"),
    );
    let batch = batch("delete-order", 2);
    let batch_id = batch.id.clone();
    repo.store(batch).await.expect("store batch");
    repo.record_successful_job(&batch_id, Uuid::new_v4())
        .await
        .expect("seed settlement");

    let mut gate = db
        .get_postgres_connection_pool()
        .acquire()
        .await
        .expect("acquire delete-gate connection");
    sea_orm::sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(DELETE_GATE_KEY)
        .execute(&mut *gate)
        .await
        .expect("hold delete gate");

    let deleting = {
        let repo = Arc::clone(&repo);
        let batch_id = batch_id.clone();
        tokio::spawn(async move { repo.delete(&batch_id).await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    format!(
                        "SELECT COUNT(*) FROM pg_locks
                         WHERE locktype = 'advisory'
                           AND classid = 0
                           AND objid = {DELETE_GATE_KEY}
                           AND objsubid = 1
                           AND NOT granted"
                    ),
                ))
                .await
                .expect("inspect delete waiter")
                .expect("delete waiter row");
            let waiters: i64 = row.try_get_by_index(0).expect("delete waiter count");
            if waiters >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("delete must pause after removing existing settlements");

    let settling = {
        let repo = Arc::clone(&repo);
        let batch_id = batch_id.clone();
        tokio::spawn(async move { repo.record_successful_job(&batch_id, Uuid::new_v4()).await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if settling.is_finished() {
                break;
            }
            let row = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    format!(
                        "SELECT EXISTS (
                             SELECT 1 FROM pg_stat_activity
                             WHERE datname = current_database()
                               AND wait_event_type = 'Lock'
                               AND query LIKE '%{BATCHES}%'
                               AND query LIKE '%FOR UPDATE%'
                         )"
                    ),
                ))
                .await
                .expect("inspect settlement lock waiter")
                .expect("settlement waiter row");
            let row_lock_waiter: bool = row.try_get_by_index(0).expect("row-lock waiter flag");
            if row_lock_waiter {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("settlement must either finish or wait behind the deleting parent");

    let unlocked: bool = sea_orm::sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(DELETE_GATE_KEY)
        .fetch_one(&mut *gate)
        .await
        .expect("release delete gate");
    assert!(unlocked, "the test connection must own the delete gate");
    drop(gate);

    let (deleted, settled) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(deleting, settling)
    })
    .await
    .expect("delete and settlement must finish after the gate opens");
    assert!(
        deleted.expect("delete task").expect("delete batch"),
        "the delete operation wins after locking the parent first"
    );
    assert!(
        settled.expect("settlement task").is_err(),
        "a settlement ordered after deletion must report the missing batch"
    );

    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT COUNT(*) FROM {SETTLEMENTS}"),
        ))
        .await
        .expect("count remaining settlements")
        .expect("settlement count row");
    let remaining: i64 = row.try_get_by_index(0).expect("settlement count");
    assert_eq!(remaining, 0, "deletion must not leave an orphan settlement");

    for sql in [
        format!("DROP TABLE {SETTLEMENTS}"),
        format!("DROP TABLE {BATCHES}"),
        format!("DROP FUNCTION {INSERT_GATE_FUNCTION}()"),
        format!("DROP FUNCTION {DELETE_GATE_FUNCTION}()"),
    ] {
        db.execute_unprepared(&sql)
            .await
            .expect("clean delete fixture");
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
