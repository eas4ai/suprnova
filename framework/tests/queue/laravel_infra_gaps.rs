//! Laravel infrastructure gaps in the queue: a connection's own after-commit
//! setting and a job's explicit choice (PAR-151).
//!
//! Laravel evidence: `Queue/Queue.php:400-411` (`shouldDispatchAfterCommit`
//! lets a job's own `afterCommit`, `false` included, win over the
//! connection's) and `Queue/Connectors/RedisConnector.php:49` (each
//! connection passes its own `after_commit`).

use crate::env_snapshot::{EnvSnapshot, set_env};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use suprnova::App;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::testing::TestDatabase;
use suprnova::{DB, EnvelopeOverrides, FrameworkError, Job, Queue, async_trait};

/// Every variable these tests read, restored when the test ends.
const ENV_KEYS: &[&str] = &[
    "QUEUE_AFTER_COMMIT",
    "QUEUE_AUDIT_AFTER_COMMIT",
    "QUEUE_AUDIT_LOG_V2_AFTER_COMMIT",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlainJob;

#[async_trait]
impl Job for PlainJob {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-plain"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Forgets the registered connections when the test ends, so a connection
/// registered here does not change what a name means for the next test.
struct ForgetConnections;

impl Drop for ForgetConnections {
    fn drop(&mut self) {
        suprnova::queue::testing::forget_connections();
    }
}

fn to_audit() -> EnvelopeOverrides {
    EnvelopeOverrides {
        connection: Some("audit".into()),
        ..Default::default()
    }
}

/// A default connection and a registered `audit` connection, each over its
/// own in-memory driver.
fn default_and_audit() -> (Arc<MemoryQueueDriver>, Arc<MemoryQueueDriver>) {
    let default = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(default.clone());
    let audit = Arc::new(MemoryQueueDriver::new());
    Queue::register_connection("audit", audit.clone());
    (default, audit)
}

#[tokio::test]
#[serial]
async fn queue_audit_after_commit_defers_a_push_to_audit_and_no_other_connection() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(ENV_KEYS);
    set_env("QUEUE_AFTER_COMMIT", None);
    set_env("QUEUE_AUDIT_AFTER_COMMIT", Some("true"));
    let _forget = ForgetConnections;
    let (default, audit) = default_and_audit();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let (default_in, audit_in) = (default.clone(), audit.clone());
    DB::transaction(move |_tx| {
        Box::pin(async move {
            Queue::push_with(PlainJob, to_audit()).await?;
            Queue::push(PlainJob).await?;
            assert_eq!(
                audit_in.size(None).await?,
                0,
                "QUEUE_AUDIT_AFTER_COMMIT=true holds a push to `audit` until the commit"
            );
            assert_eq!(
                default_in.size(None).await?,
                1,
                "a push to another connection goes out at once"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(audit.size(None).await.unwrap(), 1, "the commit pushes it");
    assert_eq!(default.size(None).await.unwrap(), 1);
}

/// Says it never waits for a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BeforeCommitJob;

#[async_trait]
impl Job for BeforeCommitJob {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-before-commit"
    }
    fn after_commit_choice() -> Option<bool> {
        Some(false)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Says it always waits for a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WaitingJob;

#[async_trait]
impl Job for WaitingJob {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-waiting"
    }
    fn after_commit_choice() -> Option<bool> {
        Some(true)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Opts in the old way, through `after_commit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptedInJob;

#[async_trait]
impl Job for OptedInJob {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-opted-in"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Unique, and says it never waits for a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UniqueBeforeCommitJob;

#[async_trait]
impl Job for UniqueBeforeCommitJob {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-unique-before-commit"
    }
    fn unique_id(&self) -> Option<String> {
        Some("one".into())
    }
    fn after_commit_choice() -> Option<bool> {
        Some(false)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Unique, on the `audit` connection, with no choice of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UniqueAuditJob;

#[async_trait]
impl Job for UniqueAuditJob {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-unique-audit"
    }
    fn connection() -> Option<&'static str> {
        Some("audit")
    }
    fn unique_id(&self) -> Option<String> {
        Some("one".into())
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[test]
fn after_commit_choice_follows_after_commit_by_default() {
    assert_eq!(OptedInJob::after_commit_choice(), Some(true));
    assert_eq!(PlainJob::after_commit_choice(), None);
    assert_eq!(BeforeCommitJob::after_commit_choice(), Some(false));
}

#[tokio::test]
#[serial]
async fn a_job_that_chooses_false_is_pushed_at_once_under_queue_after_commit() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(ENV_KEYS);
    set_env("QUEUE_AFTER_COMMIT", Some("true"));
    set_env("QUEUE_AUDIT_AFTER_COMMIT", None);
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let inside = driver.clone();
    DB::transaction(move |_tx| {
        Box::pin(async move {
            Queue::push(BeforeCommitJob).await?;
            Queue::bulk(vec![BeforeCommitJob]).await?;
            assert!(Queue::push_unique(UniqueBeforeCommitJob).await?);
            assert_eq!(
                inside.size(None).await?,
                3,
                "the job's own Some(false) wins over QUEUE_AFTER_COMMIT, for push, bulk \
                 and push_unique"
            );
            Queue::push(PlainJob).await?;
            assert_eq!(
                inside.size(None).await?,
                3,
                "a job with no choice still waits"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(driver.size(None).await.unwrap(), 4);
}

#[tokio::test]
#[serial]
async fn a_connection_setting_made_in_code_defers_its_pushes_and_is_read_back() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(ENV_KEYS);
    set_env("QUEUE_AFTER_COMMIT", None);
    set_env("QUEUE_AUDIT_AFTER_COMMIT", None);
    let _forget = ForgetConnections;
    let (default, audit) = default_and_audit();
    Queue::set_connection_after_commit("audit", true);
    assert_eq!(Queue::connection_after_commit("audit"), Some(true));
    assert_eq!(Queue::connection_after_commit("reports"), None);
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let (default_in, audit_in) = (default.clone(), audit.clone());
    DB::transaction(move |_tx| {
        Box::pin(async move {
            Queue::push_with(PlainJob, to_audit()).await?;
            Queue::push(PlainJob).await?;
            assert_eq!(audit_in.size(None).await?, 0, "audit waits for the commit");
            assert_eq!(default_in.size(None).await?, 1, "the default does not");
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");
    assert_eq!(audit.size(None).await.unwrap(), 1);
}

#[tokio::test]
#[serial]
async fn the_decision_takes_the_push_then_the_job_then_the_connection_then_the_process() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(ENV_KEYS);
    // The process waits; the audit connection says it does not.
    set_env("QUEUE_AFTER_COMMIT", Some("true"));
    set_env("QUEUE_AUDIT_AFTER_COMMIT", Some("false"));
    let _forget = ForgetConnections;
    let (default, audit) = default_and_audit();
    assert_eq!(Queue::connection_after_commit("audit"), Some(false));
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let (default_in, audit_in) = (default.clone(), audit.clone());
    DB::transaction(move |_tx| {
        Box::pin(async move {
            // The connection's `false` wins over the process.
            Queue::push_with(PlainJob, to_audit()).await?;
            assert_eq!(audit_in.size(None).await?, 1, "audit's false wins");
            // The job's `Some(true)` wins over the connection.
            Queue::push_with(WaitingJob, to_audit()).await?;
            assert_eq!(audit_in.size(None).await?, 1, "the job's choice wins");
            // The push's own override wins over the job.
            Queue::push_with(
                WaitingJob,
                EnvelopeOverrides {
                    after_commit: Some(false),
                    ..to_audit()
                },
            )
            .await?;
            assert_eq!(audit_in.size(None).await?, 2, "the push's override wins");
            // No choice anywhere but the process: it waits.
            Queue::push(PlainJob).await?;
            assert_eq!(
                default_in.size(None).await?,
                0,
                "QUEUE_AFTER_COMMIT decides last"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");
    assert_eq!(audit.size(None).await.unwrap(), 3);
    assert_eq!(default.size(None).await.unwrap(), 1);
}

#[tokio::test]
#[serial]
async fn push_unique_follows_the_connections_setting() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(ENV_KEYS);
    set_env("QUEUE_AFTER_COMMIT", None);
    set_env("QUEUE_AUDIT_AFTER_COMMIT", Some("true"));
    let _forget = ForgetConnections;
    let (_default, audit) = default_and_audit();
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let audit_in = audit.clone();
    DB::transaction(move |_tx| {
        Box::pin(async move {
            assert!(Queue::push_unique(UniqueAuditJob).await?);
            assert_eq!(audit_in.size(None).await?, 0, "the unique push waits");
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");
    assert_eq!(audit.size(None).await.unwrap(), 1);
}

#[tokio::test]
#[serial]
async fn the_variable_name_upper_cases_the_connection_and_replaces_other_characters() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = EnvSnapshot::capture(ENV_KEYS);
    set_env("QUEUE_AUDIT_LOG_V2_AFTER_COMMIT", Some("1"));
    assert_eq!(Queue::connection_after_commit("audit-log.v2"), Some(true));
    set_env("QUEUE_AUDIT_LOG_V2_AFTER_COMMIT", Some("0"));
    assert_eq!(Queue::connection_after_commit("audit-log.v2"), Some(false));
    set_env("QUEUE_AUDIT_LOG_V2_AFTER_COMMIT", Some("sometimes"));
    assert_eq!(
        Queue::connection_after_commit("audit-log.v2"),
        None,
        "a value that is no boolean decides nothing"
    );
}
