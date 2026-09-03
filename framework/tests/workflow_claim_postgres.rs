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

/// Current database-server time as whole seconds, so lease bounds are
/// measured against the same clock the claim and reclaim predicates use.
async fn server_now_secs(db: &DatabaseConnection) -> i64 {
    let stmt = Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT EXTRACT(EPOCH FROM NOW())::BIGINT AS now_secs".to_string(),
    );
    let row = db
        .query_one_raw(stmt)
        .await
        .expect("server clock read")
        .expect("server clock row");
    row.try_get("", "now_secs").expect("now_secs column")
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

    let before = server_now_secs(&raw).await;
    let claimed = claim_next_workflow("worker-a", &config)
        .await
        .expect("claim")
        .expect("a pending row must be claimable");
    let after = server_now_secs(&raw).await;

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
