//! Queue inspection parity (#60966): pending/delayed/reserved listings return
//! InspectedJob DTOs on the memory and database drivers and on the fake.

use chrono::Utc;
use sea_orm::{ConnectionTrait, Database};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::database::DatabaseQueueDriver;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope, InspectedJob};
use suprnova::queue::{MemoryQueueDriver, Queue};
use suprnova::{FrameworkError, Job};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InspectMe {
    n: u32,
}

#[async_trait::async_trait]
impl Job for InspectMe {
    fn job_name() -> &'static str {
        "wave5-inspect"
    }
    fn queue() -> Option<&'static str> {
        Some("reports")
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial_test::serial]
async fn memory_driver_lists_pending_delayed_and_reserved() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Queue::push(InspectMe { n: 1 }).await.expect("push");
    Queue::later(Duration::from_secs(3600), InspectMe { n: 2 })
        .await
        .expect("later");

    let pending = Queue::pending_jobs(None).await.expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "wave5-inspect");
    assert_eq!(pending[0].queue.as_deref(), Some("reports"));
    assert_eq!(pending[0].attempts, 0);
    assert!(
        pending[0].id.is_some(),
        "real drivers carry the envelope id"
    );
    assert!(pending[0].created_at.is_some());

    let delayed = Queue::delayed_jobs(None).await.expect("delayed");
    assert_eq!(delayed.len(), 1);
    assert_eq!(delayed[0].payload["n"], 2);

    // Reserve the pending job; it moves lists.
    let _res = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop")
        .expect("reservation");
    let reserved = Queue::reserved_jobs(None).await.expect("reserved");
    assert_eq!(reserved.len(), 1);
    assert_eq!(reserved[0].payload["n"], 1);
    assert!(
        Queue::pending_jobs(None)
            .await
            .expect("pending again")
            .is_empty()
    );
}

#[tokio::test]
#[serial_test::serial]
async fn queue_filter_matches_pop_semantics() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver);
    Queue::push(InspectMe { n: 1 }).await.expect("push"); // queue "reports"

    assert_eq!(
        Queue::pending_jobs(Some("reports"))
            .await
            .expect("filtered")
            .len(),
        1
    );
    assert!(
        Queue::pending_jobs(Some("other"))
            .await
            .expect("other")
            .is_empty()
    );
}

#[tokio::test]
#[serial_test::serial]
async fn fake_lists_recorded_pushes() {
    let _guard = suprnova::queue::testing::install_fake();
    Queue::push(InspectMe { n: 7 })
        .await
        .expect("push under fake");

    let pending: Vec<InspectedJob> = suprnova::queue::testing::pending_jobs();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "wave5-inspect");
    assert_eq!(
        pending[0].attempts, 0,
        "the fake mirrors Laravel: attempts is always 0"
    );
    assert!(
        pending[0].id.is_some(),
        "the fake stamps ids (catalog Q5's S-item)"
    );
}

// ---- database driver -------------------------------------------------

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

fn db_env(name: &str) -> Envelope {
    let now = Utc::now();
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: name.into(),
        queue: None,
        payload: serde_json::json!({ "marker": name }),
        dispatched_at: now,
        available_at: now,
        attempts: 0,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        unique_lock_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    }
}

#[tokio::test]
async fn database_driver_lists_pending_delayed_and_reserved() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    // One immediate envelope.
    d.push(db_env("immediate")).await.unwrap();

    // One delayed envelope.
    let mut delayed = db_env("later");
    delayed.available_at = Utc::now() + chrono::Duration::seconds(3600);
    d.push(delayed).await.unwrap();

    let pending = d.pending_jobs(None).await.unwrap();
    assert_eq!(pending.len(), 1, "only the immediate envelope is pending");
    assert_eq!(pending[0].name, "immediate");
    assert!(pending[0].id.is_some());
    assert!(pending[0].created_at.is_some());

    let delayed_list = d.delayed_jobs(None).await.unwrap();
    assert_eq!(delayed_list.len(), 1);
    assert_eq!(delayed_list[0].name, "later");

    // Reserve the pending job.
    let r1 = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("reservation");
    assert_eq!(r1.envelope.job_name, "immediate");

    let reserved = d.reserved_jobs(None).await.unwrap();
    assert_eq!(reserved.len(), 1);
    assert_eq!(reserved[0].name, "immediate");

    let pending_after = d.pending_jobs(None).await.unwrap();
    assert!(
        pending_after.is_empty(),
        "the reserved envelope must not also show as pending"
    );
}

#[tokio::test]
async fn database_driver_pending_jobs_survives_an_unparseable_row() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db.clone(), "jobs".to_string()).unwrap();

    d.push(db_env("good")).await.unwrap();

    // Insert a poison row directly: garbage envelope_json that will fail to
    // decode when the listing tries to parse it.
    let now = Utc::now().timestamp();
    db.execute_unprepared(&format!(
        "INSERT INTO jobs (id, job_name, queue, envelope_json, available_at, attempts, created_at) \
         VALUES ('{}', 'poison', NULL, 'not valid json', {now}, 0, {now})",
        Uuid::new_v4()
    ))
    .await
    .unwrap();

    let pending = d.pending_jobs(None).await.unwrap();
    assert_eq!(
        pending.len(),
        2,
        "the poison row must still appear, not blind the listing to the good row"
    );

    let poisoned = pending
        .iter()
        .find(|j| j.name == "poison")
        .expect("poison row present");
    assert!(
        poisoned.id.is_none(),
        "unparseable row has no recoverable id"
    );
    assert_eq!(poisoned.payload["unparseable"], true);

    let good = pending
        .iter()
        .find(|j| j.name == "good")
        .expect("good row present");
    assert!(good.id.is_some());
}

/// The queue filter's `queue = ? OR queue IS NULL` arm is Postgres-ordinal
/// sensitive (`queue_filter_clause` in `database.rs`) and only this SQLite
/// run exercises the SQL text; a placeholder-ordinal mistake would still
/// pass on SQLite (its `?` is purely positional) but break on Postgres,
/// which the gate's Postgres suite would then catch - this test pins the
/// filter's *semantics* so a regression there is caught everywhere.
#[tokio::test]
async fn database_driver_queue_filter_matches_default_and_named_queues() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    let mut unrouted = db_env("unrouted");
    unrouted.queue = None;
    d.push(unrouted).await.unwrap();

    let mut billing = db_env("billing-job");
    billing.queue = Some("billing".to_string());
    d.push(billing).await.unwrap();

    let default_only = d
        .pending_jobs(Some(suprnova::queue::envelope::DEFAULT_QUEUE))
        .await
        .unwrap();
    assert_eq!(default_only.len(), 1);
    assert_eq!(default_only[0].name, "unrouted");

    let billing_only = d.pending_jobs(Some("billing")).await.unwrap();
    assert_eq!(billing_only.len(), 1);
    assert_eq!(billing_only[0].name, "billing-job");

    let other = d.pending_jobs(Some("other")).await.unwrap();
    assert!(other.is_empty());
}
