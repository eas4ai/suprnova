//! Live-Postgres coverage for the workflow claim lease (P4-05).
//!
//! The claim computes its initial expiry from the database clock
//! (`NOW() + lease`), never from a client timestamp taken before the
//! round trip: a slow claim must not return an already-reclaimable row.
//! At the accepted minimum lease a second worker must not reclaim the
//! row immediately, while a genuinely expired lease must be reclaimable.
//!
//! Run with a disposable Postgres:
//!
//! ```text
//! docker run -d --rm --name suprnova-pg -e POSTGRES_PASSWORD=pw \
//!     -e POSTGRES_DB=suprnova_test -p 55998:5432 postgres:17-alpine
//! PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55998/suprnova_test \
//!     cargo test -p suprnova --test workflow_claim_postgres -- --ignored
//! ```

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::prelude::*;
use serial_test::serial;
use std::time::Duration;
use suprnova::workflow::WorkflowConfig;
use suprnova::workflow::migrations::CreateWorkflowsTable;
use suprnova::workflow::store::{claim_next_workflow, get_workflow_record, insert_workflow};
use suprnova::{DB, DatabaseConfig};

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

struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateWorkflowsTable)]
    }
}

/// Which way a server clock read is cut to whole seconds.
#[derive(Clone, Copy)]
enum Bound {
    /// Down: the read before the claim, which the expiry must cover in full.
    Floor,
    /// Up: the read after the claim, which the expiry must not pass.
    Ceil,
}

/// `expr`, a Postgres timestamp, as whole Unix seconds cut toward `bound`.
///
/// `FLOOR` and `CEIL` before the cast: a cast alone rounds to the nearest
/// second, which moves a lower bound up or an upper bound down.
async fn epoch_secs(db: &DatabaseConnection, expr: &str, bound: Bound) -> i64 {
    let cut = match bound {
        Bound::Floor => "FLOOR",
        Bound::Ceil => "CEIL",
    };
    let stmt = Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!("SELECT {cut}(EXTRACT(EPOCH FROM {expr}))::BIGINT AS secs"),
    );
    let row = db
        .query_one_raw(stmt)
        .await
        .expect("server clock read")
        .expect("server clock row");
    row.try_get("", "secs").expect("secs column")
}

/// Current database-server time as whole seconds, so lease bounds are
/// measured against the same clock the claim and reclaim predicates use.
async fn server_now_secs(db: &DatabaseConnection, bound: Bound) -> i64 {
    epoch_secs(db, "NOW()", bound).await
}

fn min_lease_config() -> WorkflowConfig {
    WorkflowConfig {
        poll_interval_ms: 50,
        concurrency: 1,
        lock_timeout_secs: suprnova::workflow::config::MIN_LOCK_TIMEOUT_SECS,
        max_attempts: 3,
        retry_backoff_secs: 0,
    }
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn claim_at_minimum_lease_is_server_anchored_and_not_instantly_reclaimable() {
    let raw = connect_postgres().await;
    raw.execute_unprepared("DROP TABLE IF EXISTS workflows")
        .await
        .expect("drop workflows fixture");
    Migrator::up(&raw, None)
        .await
        .expect("migrate workflows fixture");

    DB::init_with(DatabaseConfig::builder().url(pg_url()).build())
        .await
        .expect("DB::init_with");

    let config = min_lease_config();
    let lease = config.lock_timeout_secs as i64;

    insert_workflow("lease-probe", "{}", 3)
        .await
        .expect("insert workflow");

    let before = server_now_secs(&raw, Bound::Floor).await;
    let claimed = claim_next_workflow("worker-a", &config)
        .await
        .expect("claim")
        .expect("a pending row must be claimable");
    let after = server_now_secs(&raw, Bound::Ceil).await;

    // The expiry is measured from the server clock at claim time: no
    // less than the full lease after the read that preceded the claim,
    // no more than the full lease after the read that followed it.
    // A client-side deadline would sit below `before + lease` by the
    // whole claim latency (and by any worker/database clock skew).
    let record = get_workflow_record(claimed.id)
        .await
        .expect("read claimed row");
    let locked_until = record.locked_until.expect("claim sets locked_until");
    let min_expiry = chrono::DateTime::from_timestamp(before + lease, 0)
        .expect("valid test bound")
        .naive_utc();
    let max_expiry = chrono::DateTime::from_timestamp(after + lease, 0)
        .expect("valid test bound")
        .naive_utc();
    assert!(
        locked_until >= min_expiry,
        "expiry {locked_until} must cover the full {lease}s lease from the \
         pre-claim server read: the claim latency must not eat it"
    );
    assert!(
        locked_until <= max_expiry,
        "expiry {locked_until} must not exceed the full {lease}s lease past \
         the post-claim server read"
    );

    // A second worker at the minimum lease must not reclaim the row
    // immediately: the expiry above has to hold against live competition.
    let rival = claim_next_workflow("worker-b", &config)
        .await
        .expect("rival claim");
    assert!(
        rival.is_none(),
        "a just-claimed row at the minimum lease must not be instantly reclaimable"
    );

    // ...while a genuinely expired lease must still be reclaimable.
    raw.execute_unprepared(
        "UPDATE workflows SET locked_until = NOW() - INTERVAL '1 second' WHERE status = 'running'",
    )
    .await
    .expect("expire the lease");
    let reclaimed = claim_next_workflow("worker-b", &config)
        .await
        .expect("reclaim")
        .expect("an expired lease must be reclaimable");
    assert_eq!(reclaimed.id, claimed.id);
    assert_eq!(reclaimed.attempts, claimed.attempts + 1);
}

/// The clock reads that bound the lease cut their fraction the right way:
/// the read before the claim down, the read after it up. A plain
/// `::BIGINT` cast rounds, so a read at half a second or later came out a
/// second high, the lower bound passed the real expiry, and the lease test
/// above failed whenever the claim landed in the second half of a second.
/// A fixed timestamp at 52.6 seconds makes that case happen every run.
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_clock_reads_floor_before_and_ceil_after() {
    let raw = connect_postgres().await;
    let at = "TIMESTAMPTZ '1970-01-01 00:00:52.6+00'";
    assert_eq!(epoch_secs(&raw, at, Bound::Floor).await, 52);
    assert_eq!(epoch_secs(&raw, at, Bound::Ceil).await, 53);
}

/// A cancelled worker drains and returns `Ok` promptly. It runs on
/// Postgres, the only database the worker claims from: on any other the
/// worker now refuses to start, so the SQLite version of this test could
/// no longer reach the drain.
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_worker_run_with_cancel_returns_cleanly() {
    let raw = connect_postgres().await;
    raw.execute_unprepared("DROP TABLE IF EXISTS workflows")
        .await
        .expect("drop workflows fixture");
    Migrator::up(&raw, None)
        .await
        .expect("migrate workflows fixture");
    DB::init_with(DatabaseConfig::builder().url(pg_url()).build())
        .await
        .expect("DB::init_with");

    let worker = suprnova::workflow::WorkflowWorker::with_config(WorkflowConfig {
        poll_interval_ms: 20,
        ..min_lease_config()
    });
    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_for_worker = cancel.clone();
    let handle = tokio::spawn(async move { worker.run_with_cancel(cancel_for_worker).await });

    // Let the worker reach its idle poll, with no row to claim.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!handle.is_finished(), "the worker idles on Postgres");
    cancel.cancel();

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("the worker exits within 1s of cancellation")
        .expect("the worker task does not panic")
        .expect("run_with_cancel returns Ok on a graceful drain");
}
