//! After-commit dispatch parity (#60996): a push inside `DB::transaction` is
//! invisible until the transaction commits, a rollback discards it entirely,
//! and outside a transaction an `after_commit` job dispatches immediately.
//!
//! Laravel evidence: `Database/DatabaseTransactionsManager.php:213-233`
//! (`addCallback` runs immediately with no open transaction; `addCallbackForRollback`
//! is a silent no-op there) and `Queue/Queue.php:366-450` (`enqueueUsing` defers the
//! *entire* push, and the rollback callback releases a unique job's lock).

use chrono::Utc;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::database::events::{TransactionCommitted, TransactionRolledBack};
use suprnova::queue::driver::{QueueDriver, Reservation, ReservationToken};
use suprnova::queue::testing::{install_fake, pushed};
use suprnova::queue::worker::register_job;
use suprnova::queue::{Envelope, FailoverQueueDriver, SyncQueueDriver};
use suprnova::testing::{TestContainer, TestDatabase};
use suprnova::{
    DB, DatabaseConfig, DbConnection, EnvelopeOverrides, FrameworkError, Job, Queue, TxHandle,
    async_trait,
};
use suprnova::{EventFacade, Listener};
use tokio::sync::Notify;
use tokio::time::timeout;

#[derive(Serialize, Deserialize)]
struct SavepointRowJob {
    id: i32,
}

#[async_trait]
impl Job for SavepointRowJob {
    fn job_name() -> &'static str {
        "savepoint-row"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

async fn savepoint_rows_match_jobs(
    first: String,
    second: Option<String>,
    rollback: String,
    expected: &[i32],
) {
    let driver = install_driver();
    let connection = DB::connection().expect("test connection");
    let backend = connection.inner().get_database_backend();
    let table = format!("savepoint_rows_{}", uuid::Uuid::new_v4().simple());
    connection
        .inner()
        .execute_unprepared(&format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY)"))
        .await
        .expect("create owned table");
    let tx_table = table.clone();
    DB::transaction(move |tx| {
        Box::pin(async move {
            for id in 0..3 {
                if id == 1 {
                    tx.savepoint(&first).await?;
                }
                if id == 2 {
                    if let Some(second) = &second {
                        tx.savepoint(second).await?;
                    } else {
                        break;
                    }
                }
                tx.query_all(Statement::from_string(
                    backend,
                    format!("INSERT INTO {tx_table} (id) VALUES ({id})"),
                ))
                .await?;
                Queue::push(SavepointRowJob { id }).await?;
            }
            tx.rollback_to(&rollback).await?;
            // ROLLBACK TO keeps its mark usable. Repeat through another spelling.
            tx.query_all(Statement::from_string(
                backend,
                format!("INSERT INTO {tx_table} (id) VALUES (3)"),
            ))
            .await?;
            Queue::push(SavepointRowJob { id: 3 }).await?;
            tx.rollback_to(&rollback.to_ascii_uppercase()).await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("outer commit");
    let rows: Vec<i32> = connection
        .inner()
        .query_all_raw(Statement::from_string(
            backend,
            format!("SELECT id FROM {table} ORDER BY id"),
        ))
        .await
        .expect("committed rows")
        .into_iter()
        .map(|row| row.try_get("", "id").unwrap())
        .collect();
    let jobs: Vec<i32> = driver
        .envelopes()
        .into_iter()
        .map(|env| {
            serde_json::from_value::<SavepointRowJob>(env.payload)
                .unwrap()
                .id
        })
        .collect();
    connection
        .inner()
        .execute_unprepared(&format!("DROP TABLE {table}"))
        .await
        .expect("drop owned table");
    assert_eq!(rows, expected, "physical rollback boundary");
    assert_eq!(jobs, expected, "deferred effects must match committed rows");
}

async fn savepoint_alias_suite() {
    savepoint_rows_match_jobs("checkpoint".into(), None, "CHECKPOINT".into(), &[0]).await;
    savepoint_rows_match_jobs(
        "checkpoint".into(),
        Some("CHECKPOINT".into()),
        "checkpoint".into(),
        &[0, 1],
    )
    .await;
    let prefix = "a".repeat(63);
    let postgres = DB::connection().unwrap().inner().get_database_backend() == DbBackend::Postgres;
    savepoint_rows_match_jobs(
        format!("{prefix}x"),
        Some(format!("{prefix}y")),
        format!("{prefix}x"),
        if postgres { &[0, 1] } else { &[0] },
    )
    .await;
    if postgres {
        savepoint_rows_match_jobs(format!("{prefix}x"), None, prefix, &[0]).await;
    }
}

#[tokio::test]
#[serial]
async fn savepoint_aliases_sqlite_rows_and_jobs_agree() {
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    savepoint_alias_suite().await;
}

async fn live_savepoint_aliases(variable: &str, expected_backend: DbBackend) {
    let url = std::env::var(variable).expect("explicit disposable database URL required");
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let connection = DbConnection::connect(&config)
        .await
        .expect("live test database");
    assert_eq!(
        connection.inner().get_database_backend(),
        expected_backend,
        "the live dialect test must use its intended backend"
    );
    TestContainer::scope(async move {
        TestContainer::singleton(connection);
        savepoint_alias_suite().await;
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable PostgreSQL in PG_TEST_URL"]
async fn savepoint_aliases_postgres_rows_and_jobs_agree() {
    live_savepoint_aliases("PG_TEST_URL", DbBackend::Postgres).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL/MariaDB in MYSQL_TEST_URL"]
async fn savepoint_aliases_mysql_rows_and_jobs_agree() {
    live_savepoint_aliases("MYSQL_TEST_URL", DbBackend::MySql).await;
}

#[tokio::test]
#[serial]
async fn savepoint_case_alias_releases_unique_job_lock() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    DB::transaction(|tx| {
        Box::pin(async move {
            tx.savepoint("checkpoint").await?;
            assert!(
                Queue::push_unique(UniqueAfterCommitJob {
                    key: "case-alias".into()
                })
                .await?
            );
            tx.rollback_to("CHECKPOINT").await?;
            assert!(
                Queue::push_unique(UniqueAfterCommitJob {
                    key: "case-alias".into()
                })
                .await?
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");
    assert_eq!(driver.count(), 1);
}

#[derive(Default)]
struct CompletionEventGate {
    entered: Notify,
    release: Notify,
}

#[async_trait]
impl Listener<TransactionCommitted> for CompletionEventGate {
    async fn handle(&self, _: &TransactionCommitted) -> Result<(), FrameworkError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(())
    }
}

#[async_trait]
impl Listener<TransactionRolledBack> for CompletionEventGate {
    async fn handle(&self, _: &TransactionRolledBack) -> Result<(), FrameworkError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn cancellation_during_commit_event_preserves_deferred_dispatch() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    let gate = Arc::new(CompletionEventGate::default());
    EventFacade::listen::<TransactionCommitted, _>(gate.clone()).await;
    let caller = tokio::spawn(async {
        DB::transaction(|_| {
            Box::pin(async {
                Queue::push(AfterCommitJob).await?;
                Ok::<(), FrameworkError>(())
            })
        })
        .await
    });
    timeout(Duration::from_secs(10), gate.entered.notified())
        .await
        .expect("commit listener entered");
    caller.abort();
    assert!(caller.await.expect_err("caller aborted").is_cancelled());
    gate.release.notify_one();
    EventFacade::forget::<TransactionCommitted>();
    poll_until(
        || driver.count() == 1,
        Duration::from_secs(2),
        "dispatch after committed caller cancellation",
    )
    .await;
    assert_eq!(driver.only().job_name, AfterCommitJob::job_name());
}

#[tokio::test]
#[serial]
async fn cancellation_during_rollback_event_releases_unique_lock() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    let gate = Arc::new(CompletionEventGate::default());
    EventFacade::listen::<TransactionRolledBack, _>(gate.clone()).await;
    let caller = tokio::spawn(async {
        DB::transaction(|_| {
            Box::pin(async {
                assert!(
                    Queue::push_unique(UniqueAfterCommitJob {
                        key: "rollback-event-abort".into()
                    })
                    .await?
                );
                Err::<(), FrameworkError>(FrameworkError::internal("rollback"))
            })
        })
        .await
    });
    timeout(Duration::from_secs(10), gate.entered.notified())
        .await
        .expect("rollback listener entered");
    caller.abort();
    assert!(caller.await.expect_err("caller aborted").is_cancelled());
    gate.release.notify_one();
    EventFacade::forget::<TransactionRolledBack>();
    timeout(Duration::from_secs(2), async {
        loop {
            if Queue::push_unique(UniqueAfterCommitJob {
                key: "rollback-event-abort".into(),
            })
            .await
            .expect("retry dispatch")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("rollback compensation must release lock");
    assert_eq!(driver.count(), 1, "only the retry dispatch publishes");
}

#[tokio::test]
#[serial]
async fn cancellation_preserves_callback_registration_order() {
    let driver = Arc::new(GatedDriver::new());
    Queue::set_driver(driver.clone());
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    let caller = tokio::spawn(async {
        DB::transaction(|_| {
            Box::pin(async {
                Queue::push(BlockingAfterCommitJob).await?;
                Queue::push(OtherAfterCommitJob).await?;
                Ok::<(), FrameworkError>(())
            })
        })
        .await
    });
    poll_until(
        || driver.entered.load(Ordering::SeqCst),
        Duration::from_secs(10),
        "first callback entered",
    )
    .await;
    caller.abort();
    assert!(caller.await.expect_err("caller aborted").is_cancelled());
    let second_started = timeout(Duration::from_millis(100), async {
        loop {
            if !driver.pushed.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    driver.release.notify_one();
    poll_until(
        || driver.pushed.lock().unwrap().len() == 2,
        Duration::from_secs(2),
        "ordered callbacks complete",
    )
    .await;
    assert!(
        !second_started,
        "second callback must wait for the first callback after caller cancellation"
    );
    let names: Vec<_> = driver
        .pushed
        .lock()
        .unwrap()
        .iter()
        .map(|env| env.job_name.clone())
        .collect();
    assert_eq!(
        names,
        [
            BlockingAfterCommitJob::job_name(),
            OtherAfterCommitJob::job_name()
        ]
    );
}

// --- Driver -----------------------------------------------------------------

/// Records every envelope the framework hands it, so a test can assert on the
/// exact `available_at` a deferred push resolved - something a `pop`-based
/// assertion cannot do for a job that is delayed by ten minutes.
#[derive(Default)]
struct RecordingDriver {
    pushed: Mutex<Vec<Envelope>>,
    fail: AtomicBool,
}

impl RecordingDriver {
    fn count(&self) -> usize {
        self.pushed.lock().unwrap().len()
    }

    fn only(&self) -> Envelope {
        let g = self.pushed.lock().unwrap();
        assert_eq!(g.len(), 1, "expected exactly one pushed envelope");
        g[0].clone()
    }

    fn envelopes(&self) -> Vec<Envelope> {
        self.pushed.lock().unwrap().clone()
    }
}

#[async_trait]
impl QueueDriver for RecordingDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(FrameworkError::internal(
                "recording driver refused the push",
            ));
        }
        self.pushed.lock().unwrap().push(env);
        Ok(())
    }

    async fn pop(&self, _vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }

    async fn ack(&self, _t: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn nack(&self, _t: &ReservationToken, _delay: Duration) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        Ok(self.count() as u64)
    }

    fn name(&self) -> &'static str {
        "recording"
    }
}

fn install_driver() -> Arc<RecordingDriver> {
    let driver = Arc::new(RecordingDriver::default());
    Queue::set_driver(driver.clone());
    driver
}

// --- Jobs -------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AfterCommitJob;

#[async_trait]
impl Job for AfterCommitJob {
    fn job_name() -> &'static str {
        "wave5-after-commit"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Opts out of after-commit dispatch (the framework default) so
/// `Queue::push_after_commit` has something to opt in per push.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlainJob;

#[async_trait]
impl Job for PlainJob {
    fn job_name() -> &'static str {
        "wave5-after-commit-plain"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

const LONG_DELAY_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DelayedAfterCommitJob;

#[async_trait]
impl Job for DelayedAfterCommitJob {
    fn job_name() -> &'static str {
        "wave5-after-commit-delayed"
    }
    fn after_commit() -> bool {
        true
    }
    fn delay() -> Option<Duration> {
        Some(Duration::from_secs(LONG_DELAY_SECS as u64))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UniqueAfterCommitJob {
    key: String,
}

#[async_trait]
impl Job for UniqueAfterCommitJob {
    fn job_name() -> &'static str {
        "wave5-after-commit-unique"
    }
    fn after_commit() -> bool {
        true
    }
    fn unique_id(&self) -> Option<String> {
        Some(self.key.clone())
    }
    fn unique_for() -> Duration {
        Duration::from_secs(300)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DelayedUniqueAfterCommitJob {
    key: String,
}

#[async_trait]
impl Job for DelayedUniqueAfterCommitJob {
    fn job_name() -> &'static str {
        "wave5-after-commit-unique-delayed"
    }
    fn after_commit() -> bool {
        true
    }
    fn delay() -> Option<Duration> {
        Some(Duration::from_secs(LONG_DELAY_SECS as u64))
    }
    fn unique_id(&self) -> Option<String> {
        Some(self.key.clone())
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// A second opted-in job type, so a test can tell two concurrent transactions'
/// deferred pushes apart by `job_name` alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OtherAfterCommitJob;

#[async_trait]
impl Job for OtherAfterCommitJob {
    fn job_name() -> &'static str {
        "wave5-after-commit-other"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

static NESTED_TX_RESULT: Mutex<Option<Result<(), String>>> = Mutex::new(None);

/// Runs under the `sync` driver, so its handler executes inline on the push -
/// which for an after-commit job means it executes from inside the commit
/// drain. Opening its own transaction there proves the drain runs *outside*
/// the `CURRENT_TX` scope; if it did not, `DB::transaction` would reject the
/// call as nested.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpensItsOwnTransactionJob;

#[async_trait]
impl Job for OpensItsOwnTransactionJob {
    fn job_name() -> &'static str {
        "wave5-after-commit-nested-tx"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        let outcome = DB::transaction(|_tx| Box::pin(async { Ok::<(), FrameworkError>(()) }))
            .await
            .map_err(|e| e.to_string());
        *NESTED_TX_RESULT.lock().unwrap() = Some(outcome);
        Ok(())
    }
}

// --- Tests ------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn push_inside_a_transaction_is_invisible_until_commit() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push(AfterCommitJob).await?;
            assert_eq!(
                Queue::size().await?,
                0,
                "an after_commit push must not reach the driver before the commit"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("transaction commits");

    assert_eq!(
        driver.count(),
        1,
        "the job must reach the driver once the transaction commits"
    );
}

#[tokio::test]
#[serial]
async fn a_rollback_discards_the_deferred_push() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let result: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push(AfterCommitJob).await?;
            Err(FrameworkError::internal("force rollback"))
        })
    })
    .await;

    assert!(result.is_err(), "the transaction rolled back");
    assert_eq!(
        driver.count(),
        0,
        "a rolled-back transaction must discard the deferred push"
    );
}

#[tokio::test]
#[serial]
async fn outside_a_transaction_after_commit_dispatches_immediately() {
    let driver = install_driver();
    Queue::push(AfterCommitJob).await.expect("push");
    assert_eq!(
        driver.count(),
        1,
        "with no open transaction the push happens now (Laravel's addCallback rule)"
    );
}

#[tokio::test]
#[serial]
async fn a_per_push_override_of_false_pushes_immediately() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|_tx| {
        Box::pin(async {
            // Laravel's `beforeCommit()`: the per-push override outranks the
            // job's own `after_commit()`.
            Queue::push_with(
                AfterCommitJob,
                EnvelopeOverrides {
                    after_commit: Some(false),
                    ..Default::default()
                },
            )
            .await?;
            assert_eq!(
                Queue::size().await?,
                1,
                "an explicit after_commit: Some(false) must push immediately"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(driver.count(), 1);
}

#[tokio::test]
#[serial]
async fn push_after_commit_defers_a_job_that_did_not_opt_in() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push_after_commit(PlainJob).await?;
            assert_eq!(
                Queue::size().await?,
                0,
                "push_after_commit must defer even a job whose after_commit() is false"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(driver.count(), 1);
}

#[tokio::test]
#[serial]
async fn the_job_delay_is_recomputed_at_commit_time() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let before = Utc::now();
    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push(DelayedAfterCommitJob).await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let env = driver.only();
    let floor =
        before + chrono::Duration::seconds(LONG_DELAY_SECS) + chrono::Duration::milliseconds(400);
    assert!(
        env.available_at >= floor,
        "Job::delay() must be measured from the commit, not from the push: \
         available_at {} is earlier than {floor}",
        env.available_at
    );
}

#[tokio::test]
#[serial]
async fn an_explicit_available_at_survives_the_deferral() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let before = Utc::now();
    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::later(Duration::from_secs(LONG_DELAY_SECS as u64), AfterCommitJob).await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let env = driver.only();
    let ceiling =
        before + chrono::Duration::seconds(LONG_DELAY_SECS) + chrono::Duration::milliseconds(400);
    let floor =
        before + chrono::Duration::seconds(LONG_DELAY_SECS) - chrono::Duration::milliseconds(100);
    assert!(
        env.available_at <= ceiling && env.available_at >= floor,
        "an explicit `later` timestamp is the caller's intent: it must survive the \
         deferral unchanged, neither recomputed at commit nor dropped for `now`. \
         available_at {} is outside {floor}..={ceiling}",
        env.available_at
    );
}

#[tokio::test]
#[serial]
async fn bulk_defers_the_whole_batch() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::bulk(vec![AfterCommitJob, AfterCommitJob, AfterCommitJob]).await?;
            assert_eq!(Queue::size().await?, 0, "bulk must defer the whole batch");
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(driver.count(), 3, "every job in the batch lands at commit");
}

#[tokio::test]
#[serial]
async fn a_rollback_discards_the_whole_deferred_batch() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let result: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async {
            Queue::bulk(vec![AfterCommitJob, AfterCommitJob]).await?;
            Err(FrameworkError::internal("force rollback"))
        })
    })
    .await;

    assert!(result.is_err());
    assert_eq!(driver.count(), 0, "a rollback discards the whole batch");
}

#[tokio::test]
#[serial]
async fn push_unique_locks_now_but_defers_the_envelope() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    DB::transaction(|_tx| {
        Box::pin(async {
            let first = Queue::push_unique(UniqueAfterCommitJob {
                key: "defer-1".into(),
            })
            .await?;
            assert!(
                first,
                "the lock winner reports true even though the push is pending"
            );
            assert_eq!(Queue::size().await?, 0, "the envelope waits for the commit");

            let second = Queue::push_unique(UniqueAfterCommitJob {
                key: "defer-1".into(),
            })
            .await?;
            assert!(
                !second,
                "dedupe must still work inside the transaction - the lock is taken at push time"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let env = driver.only();
    assert_eq!(
        env.idempotency_key.as_deref(),
        Some("defer-1"),
        "the deferred envelope carries the dedupe id the lock was taken under"
    );
    assert!(
        env.unique_lock_owner.is_some(),
        "the owner token of the lock taken at push time has to reach the envelope: \
         for a unique_until_processing job the worker is what releases it, and an \
         owner-scoped release is the only kind the framework has"
    );
}

#[tokio::test]
#[serial]
async fn a_rollback_releases_the_unique_lock_it_took() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    let result: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async {
            let taken = Queue::push_unique(UniqueAfterCommitJob {
                key: "defer-2".into(),
            })
            .await?;
            assert!(taken);
            Err(FrameworkError::internal("force rollback"))
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(driver.count(), 0, "nothing was queued");

    let retried = Queue::push_unique(UniqueAfterCommitJob {
        key: "defer-2".into(),
    })
    .await
    .expect("re-push");
    assert!(
        retried,
        "the rollback must release the lock it took, or the whole unique_for window \
         is blocked by a dispatch that never happened"
    );
    assert_eq!(driver.count(), 1);
}

#[tokio::test]
#[serial]
async fn the_queue_fake_records_a_push_immediately_inside_a_transaction() {
    let _guard = install_fake();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push(AfterCommitJob).await?;
            assert_eq!(
                pushed::<AfterCommitJob>().len(),
                1,
                "the fake records immediately so assertions do not need a commit"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");
}

#[tokio::test]
#[serial]
async fn a_manual_transaction_never_defers() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let tx = DB::begin_transaction().await.expect("begin");
    Queue::push(AfterCommitJob).await.expect("push");
    assert_eq!(
        driver.count(),
        1,
        "DB::begin_transaction installs no CURRENT_TX, so there is no drain point \
         and the push must not be deferred into a registry nothing drains"
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
#[serial]
async fn the_deferred_push_runs_outside_the_transaction_scope() {
    *NESTED_TX_RESULT.lock().unwrap() = None;
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    register_job::<OpensItsOwnTransactionJob>();
    Queue::set_driver(Arc::new(SyncQueueDriver::new()));

    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push(OpensItsOwnTransactionJob).await?;
            assert!(
                NESTED_TX_RESULT.lock().unwrap().is_none(),
                "the sync driver runs the handler inline on push, so a deferred push \
                 must not have run it yet"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let outcome = NESTED_TX_RESULT.lock().unwrap().clone();
    assert_eq!(
        outcome,
        Some(Ok(())),
        "the commit drain must run outside the CURRENT_TX scope, or a job dispatched \
         after commit cannot open a transaction of its own"
    );
}

#[tokio::test]
#[serial]
async fn a_failing_deferred_push_surfaces_from_the_transaction() {
    let driver = install_driver();
    driver.fail.store(true, Ordering::SeqCst);
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let result = DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push(AfterCommitJob).await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await;

    let err = result.expect_err("a failing after-commit callback must not be swallowed");
    assert!(
        err.to_string().contains("after-commit callback failed"),
        "the error must say the transaction itself committed: {err}"
    );
}

// --- Paths where the closure said Ok but the commit never landed ------------

#[tokio::test]
#[serial]
async fn a_leaked_tx_handle_still_releases_a_deferred_unique_lock() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    // Leaking a TxHandle past an `Ok` return blocks the commit: `Arc::try_unwrap`
    // cannot reach the transaction, so nothing is ever made durable. The
    // deferred push must be compensated exactly as a rollback would be.
    let leaked: Arc<Mutex<Option<TxHandle>>> = Arc::new(Mutex::new(None));
    let leaked_for_closure = leaked.clone();

    let result: Result<(), FrameworkError> = DB::transaction(move |tx| {
        let slot = leaked_for_closure.clone();
        let handle = tx.handle();
        Box::pin(async move {
            *slot.lock().unwrap() = Some(handle);
            let taken = Queue::push_unique(UniqueAfterCommitJob {
                key: "leaked-handle".into(),
            })
            .await?;
            assert!(taken, "the lock is taken at push time");
            Ok::<(), FrameworkError>(())
        })
    })
    .await;

    assert!(
        result.is_err(),
        "a leaked TxHandle blocks the commit, so the call must fail"
    );
    assert_eq!(
        driver.count(),
        0,
        "the commit never landed, so nothing queued"
    );

    let retried = Queue::push_unique(UniqueAfterCommitJob {
        key: "leaked-handle".into(),
    })
    .await
    .expect("re-push");
    assert!(
        retried,
        "a transaction that never committed must release the lock its deferred \
         push took, exactly as a rollback does"
    );
}

#[tokio::test]
#[serial]
async fn a_refused_commit_still_releases_a_deferred_unique_lock() {
    let driver = install_driver();
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    // A deferred foreign key is checked at COMMIT, not at INSERT, so this is a
    // transaction whose closure succeeds and whose COMMIT the database refuses.
    db.execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .expect("pragma");
    db.execute_unprepared("CREATE TABLE t17_parent (id INTEGER PRIMARY KEY)")
        .await
        .expect("parent table");
    db.execute_unprepared(
        "CREATE TABLE t17_child (\
            id INTEGER PRIMARY KEY, \
            parent_id INTEGER NOT NULL REFERENCES t17_parent(id) DEFERRABLE INITIALLY DEFERRED\
         )",
    )
    .await
    .expect("child table");

    let result: Result<(), FrameworkError> = DB::transaction(|tx| {
        Box::pin(async move {
            let backend = tx.backend();
            tx.query_all(sea_orm::Statement::from_string(
                backend,
                "INSERT INTO t17_child (id, parent_id) VALUES (1, 999)".to_owned(),
            ))
            .await?;
            let taken = Queue::push_unique(UniqueAfterCommitJob {
                key: "refused-commit".into(),
            })
            .await?;
            assert!(taken, "the lock is taken at push time");
            Ok::<(), FrameworkError>(())
        })
    })
    .await;

    let err = result.expect_err("the deferred foreign key must fail the COMMIT");
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected the COMMIT-time constraint failure, got: {err}"
    );
    assert_eq!(
        driver.count(),
        0,
        "the commit was refused, so nothing queued"
    );

    let retried = Queue::push_unique(UniqueAfterCommitJob {
        key: "refused-commit".into(),
    })
    .await
    .expect("re-push");
    assert!(
        retried,
        "a refused COMMIT must release the lock the deferred push took"
    );
}

// --- The two deliberate deviations -----------------------------------------

#[tokio::test]
#[serial]
async fn a_failing_deferred_unique_push_releases_its_own_lock() {
    let driver = install_driver();
    driver.fail.store(true, Ordering::SeqCst);
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    let result = DB::transaction(|_tx| {
        Box::pin(async {
            let taken = Queue::push_unique(UniqueAfterCommitJob {
                key: "push-fails".into(),
            })
            .await?;
            assert!(taken);
            Ok::<(), FrameworkError>(())
        })
    })
    .await;

    let err = result.expect_err("the driver refused the deferred push");
    assert!(
        err.to_string().contains("after-commit callback failed"),
        "the transaction committed; only the deferred push failed: {err}"
    );

    // The commit DID land, so the rollback callback never ran. The dedupe key
    // gates re-submission of a dispatch that happened, and this one did not.
    driver.fail.store(false, Ordering::SeqCst);
    let retried = Queue::push_unique(UniqueAfterCommitJob {
        key: "push-fails".into(),
    })
    .await
    .expect("re-push");
    assert!(
        retried,
        "a deferred push that failed must release its own dedupe lock, the same \
         way commit_on_success releases when its body fails"
    );
    assert_eq!(driver.count(), 1);
}

#[tokio::test]
#[serial]
async fn a_deferred_unique_push_recomputes_the_delay_at_commit_time() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    let before = Utc::now();
    DB::transaction(|_tx| {
        Box::pin(async {
            Queue::push_unique(DelayedUniqueAfterCommitJob {
                key: "delayed-unique".into(),
            })
            .await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let env = driver.only();
    let floor =
        before + chrono::Duration::seconds(LONG_DELAY_SECS) + chrono::Duration::milliseconds(400);
    assert!(
        env.available_at >= floor,
        "push_unique must resolve Job::delay() the same way push does - against \
         the commit, not the push: available_at {} is earlier than {floor}",
        env.available_at
    );
}

// --- Registry isolation -----------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn two_concurrent_transactions_keep_separate_registries() {
    let driver = install_driver();

    // A pool of its own, so two transactions can be open at once. `sqlite::memory:`
    // gives each pooled connection an independent database, which is exactly
    // right here: the test asserts on registry isolation and issues no SQL.
    let config = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .max_connections(4)
        .min_connections(2)
        .logging(false)
        .build();
    let conn = DbConnection::connect(&config).await.expect("pool");

    let (a_registered_tx, a_registered_rx) = tokio::sync::oneshot::channel::<()>();
    let (b_registered_tx, b_registered_rx) = tokio::sync::oneshot::channel::<()>();
    let (a_committed_tx, a_committed_rx) = tokio::sync::oneshot::channel::<()>();

    let observed_after_a = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed_for_b = observed_after_a.clone();
    let driver_for_b = driver.clone();

    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        TestContainer::scope(async move {
            TestContainer::singleton(conn);

            let a = TestContainer::spawn(async move {
                DB::transaction(move |_tx| {
                    Box::pin(async move {
                        Queue::push(AfterCommitJob).await?;
                        a_registered_tx.send(()).expect("signal A");
                        // Hold the transaction open until B has registered too,
                        // so both scopes are live at the same instant.
                        b_registered_rx.await.expect("await B");
                        Ok::<(), FrameworkError>(())
                    })
                })
                .await
                .expect("A commits");
                a_committed_tx.send(()).expect("signal A committed");
            });

            let b = TestContainer::spawn(async move {
                DB::transaction(move |_tx| {
                    Box::pin(async move {
                        a_registered_rx.await.expect("await A");
                        Queue::push(OtherAfterCommitJob).await?;
                        b_registered_tx.send(()).expect("signal B");
                        a_committed_rx.await.expect("await A's commit");
                        // A's drain has finished. If the registry were shared,
                        // B's still-pending job would already be on the driver.
                        *observed_for_b.lock().unwrap() = driver_for_b
                            .envelopes()
                            .into_iter()
                            .map(|e| e.job_name)
                            .collect();
                        Ok::<(), FrameworkError>(())
                    })
                })
                .await
                .expect("B commits");
            });

            a.await.expect("A joins");
            b.await.expect("B joins");
        }),
    )
    .await;
    outcome.expect("neither transaction may hang");

    assert_eq!(
        *observed_after_a.lock().unwrap(),
        vec!["wave5-after-commit".to_string()],
        "A's commit must publish A's job and only A's - a shared registry would \
         have drained B's pending push too"
    );
    let names: Vec<String> = driver.envelopes().into_iter().map(|e| e.job_name).collect();
    assert_eq!(
        names,
        vec![
            "wave5-after-commit".to_string(),
            "wave5-after-commit-other".to_string()
        ],
        "each transaction publishes its own job at its own commit"
    );
}

// --- Savepoints -------------------------------------------------------------
//
// Laravel evidence: `Database/DatabaseTransactionsManager.php` `rollback($connection,
// $level)` discards every callback staged above the level being rolled back, so a
// dispatch registered inside a nested block that was undone never fires. Suprnova's
// nested block is a savepoint, so `Transaction::rollback_to` is the same event.

#[tokio::test]
#[serial]
async fn a_savepoint_rollback_discards_a_push_registered_above_it() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|tx| {
        Box::pin(async move {
            tx.savepoint("sp_discard").await?;
            Queue::push(AfterCommitJob).await?;
            tx.rollback_to("sp_discard").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(
        driver.count(),
        0,
        "the rows the push described were rolled back with the savepoint, so the \
         push must never reach the driver"
    );
}

#[tokio::test]
#[serial]
async fn a_savepoint_rollback_releases_the_unique_lock_taken_above_it() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    DB::transaction(|tx| {
        Box::pin(async move {
            tx.savepoint("sp_unique").await?;
            let taken = Queue::push_unique(UniqueAfterCommitJob {
                key: "savepoint-unique".into(),
            })
            .await?;
            assert!(taken, "the lock is taken at push time");
            tx.rollback_to("sp_unique").await?;

            // The compensation runs at `rollback_to`, not at the commit, so the
            // key is free again inside the very transaction that rolled it back.
            let again = Queue::push_unique(UniqueAfterCommitJob {
                key: "savepoint-unique".into(),
            })
            .await?;
            assert!(
                again,
                "a savepoint rollback must hand the dedupe lock back immediately, or a \
                 retry inside the same transaction is blocked for the whole unique_for window"
            );
            tx.rollback_to("sp_unique").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(
        driver.count(),
        0,
        "both attempts were rolled back with the savepoint"
    );

    let after = Queue::push_unique(UniqueAfterCommitJob {
        key: "savepoint-unique".into(),
    })
    .await
    .expect("re-push");
    assert!(
        after,
        "nothing was ever dispatched, so the lock must not outlive the transaction"
    );
}

#[tokio::test]
#[serial]
async fn a_savepoint_that_is_never_rolled_back_keeps_its_push() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|tx| {
        Box::pin(async move {
            tx.savepoint("sp_kept").await?;
            Queue::push(AfterCommitJob).await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(
        driver.count(),
        1,
        "the savepoint's rows committed with the transaction, so its deferred push \
         must dispatch exactly as an unmarked one does"
    );
}

#[tokio::test]
#[serial]
async fn a_push_registered_before_the_savepoint_survives_a_rollback_to() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|tx| {
        Box::pin(async move {
            Queue::push(AfterCommitJob).await?;
            tx.savepoint("sp_below").await?;
            Queue::push(OtherAfterCommitJob).await?;
            tx.rollback_to("sp_below").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let names: Vec<String> = driver.envelopes().into_iter().map(|e| e.job_name).collect();
    assert_eq!(
        names,
        vec!["wave5-after-commit".to_string()],
        "only what was registered above the mark is discarded; the rows below it \
         still committed"
    );
}

#[tokio::test]
#[serial]
async fn nested_savepoints_roll_back_only_the_inner_one() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|tx| {
        Box::pin(async move {
            tx.savepoint("sp_outer").await?;
            Queue::push(AfterCommitJob).await?;
            tx.savepoint("sp_inner").await?;
            Queue::push(OtherAfterCommitJob).await?;
            tx.rollback_to("sp_inner").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let names: Vec<String> = driver.envelopes().into_iter().map(|e| e.job_name).collect();
    assert_eq!(
        names,
        vec!["wave5-after-commit".to_string()],
        "rolling back the inner savepoint leaves the outer one's push alone"
    );
}

#[tokio::test]
#[serial]
async fn rolling_back_to_the_outer_savepoint_discards_the_inner_one_too() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    DB::transaction(|tx| {
        Box::pin(async move {
            tx.savepoint("sp_outer").await?;
            Queue::push(AfterCommitJob).await?;
            tx.savepoint("sp_inner").await?;
            Queue::push(OtherAfterCommitJob).await?;
            // Every backend destroys `sp_inner` here, so the marks above
            // `sp_outer` have to go with it.
            tx.rollback_to("sp_outer").await?;
            Queue::push(AfterCommitJob).await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let names: Vec<String> = driver.envelopes().into_iter().map(|e| e.job_name).collect();
    assert_eq!(
        names,
        vec!["wave5-after-commit".to_string()],
        "both inner pushes are discarded, and the one registered after the rollback \
         still dispatches"
    );
}

#[tokio::test]
#[serial]
async fn a_repeated_savepoint_name_unwinds_to_the_innermost_one() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    // Every backend resolves `ROLLBACK TO SAVEPOINT x` to the most recent `x`
    // and leaves that savepoint usable afterwards, so the mark stack has to do
    // the same or the registry and the rows stop describing the same savepoint.
    DB::transaction(|tx| {
        Box::pin(async move {
            tx.savepoint("dup").await?;
            Queue::push(AfterCommitJob).await?;
            // Rolls the first `dup` back but does not release it.
            tx.rollback_to("dup").await?;

            Queue::push(OtherAfterCommitJob).await?;

            // A second `dup` shadows the first from here on.
            tx.savepoint("dup").await?;
            Queue::push(DelayedAfterCommitJob).await?;
            tx.rollback_to("dup").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let env = driver.only();
    assert_eq!(
        env.job_name, "wave5-after-commit-other",
        "the second rollback must unwind to the *second* `dup`, keeping the push \
         made between the two; unwinding to the first would discard it, and \
         treating the name as already spent would keep the third"
    );
}

#[tokio::test]
#[serial]
async fn a_savepoint_issued_as_raw_sql_is_not_unwound() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    // `SAVEPOINT` issued out of band leaves no mark, so `rollback_to` has no
    // recorded length to unwind to. It rolls the rows back, warns, and keeps the
    // callbacks: discarding a deferred dispatch on a guess is the worse failure,
    // and a push that happens when it should not is at least visible.
    DB::transaction(|tx| {
        Box::pin(async move {
            let backend = tx.backend();
            tx.query_all(sea_orm::Statement::from_string(
                backend,
                "SAVEPOINT raw_sp".to_owned(),
            ))
            .await?;
            Queue::push(AfterCommitJob).await?;
            tx.rollback_to("raw_sp").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    assert_eq!(
        driver.count(),
        1,
        "an unmarked savepoint leaves the registry intact; use Transaction::savepoint \
         if the deferred dispatches are meant to unwind with the rows"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_manual_transaction_savepoint_leaves_the_enclosing_registry_alone() {
    let driver = install_driver();

    // Two transactions open at once needs two connections, so this borrows the
    // pool `two_concurrent_transactions_keep_separate_registries` uses. Each
    // `sqlite::memory:` connection is an independent database, which costs
    // nothing here: the test issues no SQL beyond the savepoint statements.
    let config = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .max_connections(4)
        .min_connections(2)
        .logging(false)
        .build();
    let conn = DbConnection::connect(&config).await.expect("pool");

    // A manual transaction opened inside the closure finds `CURRENT_TX` set. Its
    // savepoints must not reach that registry: they belong to a different
    // physical transaction, and unwinding the closure's deferred pushes on its
    // `rollback_to` would discard dispatches whose rows are still there.
    //
    // The push sits deliberately *between* the manual savepoint and its
    // rollback, which is what makes this test discriminate. Read the registry
    // from the ambient `CURRENT_TX` instead of from the handle and the mark
    // lands at length 0, the push takes the list to 1, and `rollback_to`
    // truncates it back to 0 - the enclosing transaction's job silently never
    // dispatches. With the registry on the handle the manual transaction has
    // none, marks nothing, unwinds nothing, and the push survives.
    TestContainer::scope(async move {
        TestContainer::singleton(conn);
        DB::transaction(|_tx| {
            Box::pin(async move {
                let manual = DB::begin_transaction().await?;
                manual.savepoint("sp_manual").await?;
                // Registered against the enclosing closure - the only registry
                // in play - while the manual savepoint is open.
                Queue::push(AfterCommitJob).await?;
                manual.rollback_to("sp_manual").await?;
                manual.rollback().await?;
                Ok::<(), FrameworkError>(())
            })
        })
        .await
        .expect("commit");
    })
    .await;

    assert_eq!(
        driver.count(),
        1,
        "the enclosing transaction's deferred push is not a manual transaction's to \
         unwind"
    );
}

// --- Deferral through the failover decorator --------------------------------
//
// The two halves compose or they do not: the deferral has to survive a
// fall-through to a second connection, and the compensation has to survive it
// too. A rollback that only knew how to undo a push the primary accepted would
// strand every lock taken while the primary was down.

/// A driver whose `push` always fails, standing in for a primary connection
/// that is down.
struct DownDriver;

#[async_trait]
impl QueueDriver for DownDriver {
    async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("primary connection is down"))
    }

    async fn pop(&self, _vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }

    async fn ack(&self, _t: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn nack(&self, _t: &ReservationToken, _delay: Duration) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    fn name(&self) -> &'static str {
        "down"
    }
}

/// Bind a failover connection whose primary refuses every push, so a successful
/// push can only be one that fell through to `fallback`.
fn install_failover_onto(fallback: Arc<RecordingDriver>) {
    let failover = FailoverQueueDriver::new(vec![
        (
            "primary".to_string(),
            Arc::new(DownDriver) as Arc<dyn QueueDriver>,
        ),
        ("fallback".to_string(), fallback as Arc<dyn QueueDriver>),
    ])
    .expect("two drivers");
    Queue::set_driver(Arc::new(failover));
}

#[tokio::test]
#[serial]
async fn a_deferred_push_falls_over_with_its_available_at_and_overrides_intact() {
    let fallback = Arc::new(RecordingDriver::default());
    install_failover_onto(fallback.clone());
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let before = Utc::now();
    let seen_mid_transaction = fallback.clone();
    DB::transaction(move |_tx| {
        let fallback = seen_mid_transaction.clone();
        Box::pin(async move {
            Queue::later_with(
                Duration::from_secs(LONG_DELAY_SECS as u64),
                PlainJob,
                EnvelopeOverrides {
                    after_commit: Some(true),
                    queue: Some("failover-lane".into()),
                    max_tries: Some(7),
                    ..Default::default()
                },
            )
            .await?;
            assert_eq!(
                fallback.count(),
                0,
                "the deferral runs before the fall-through, so nothing reaches any \
                 connection until the commit"
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .expect("commit");

    let env = fallback.only();
    assert_eq!(
        env.queue.as_deref(),
        Some("failover-lane"),
        "the per-push queue override has to survive both the deferral and the \
         fall-through"
    );
    assert_eq!(env.max_tries, 7, "so does max_tries");
    let floor =
        before + chrono::Duration::seconds(LONG_DELAY_SECS) - chrono::Duration::milliseconds(100);
    assert!(
        env.available_at >= floor,
        "the caller's explicit timestamp is not the fallback's to recompute: \
         available_at {} is earlier than {floor}",
        env.available_at
    );
}

#[tokio::test]
#[serial]
async fn a_rolled_back_deferred_push_releases_its_lock_even_when_the_primary_is_down() {
    let fallback = Arc::new(RecordingDriver::default());
    install_failover_onto(fallback.clone());
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    let result: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async {
            let taken = Queue::push_unique(UniqueAfterCommitJob {
                key: "failover-rollback".into(),
            })
            .await?;
            assert!(
                taken,
                "the lock is taken at push time, before any driver runs"
            );
            Err(FrameworkError::internal("force rollback"))
        })
    })
    .await;

    assert!(result.is_err(), "the transaction rolled back");
    assert_eq!(fallback.count(), 0, "nothing reached any connection");

    let retried = Queue::push_unique(UniqueAfterCommitJob {
        key: "failover-rollback".into(),
    })
    .await
    .expect("re-push");
    assert!(
        retried,
        "the compensation is registered against the transaction, not against a \
         connection, so a down primary must not strand the lock"
    );
    assert_eq!(
        fallback.count(),
        1,
        "and the re-dispatch itself still falls over to the fallback"
    );
}

// ── Cancellation barriers (P4-04) ───────────────────────────────────────────
// Aborting the task running `DB::transaction` must not strand deferred
// work: the database rolls back on drop, but queue publication and lock
// ownership live outside the database. These tests abort before the
// commit and while post-commit callbacks are pending, then prove the
// effects still land exactly once.

/// Poll `cond` until it holds or `timeout` elapses. The sleeps yield,
/// which is what lets detached finalizer tasks run on the
/// `current_thread` test runtime.
async fn poll_until(cond: impl Fn() -> bool, timeout: Duration, what: &str) {
    tokio::time::timeout(timeout, async {
        while !cond() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test]
#[serial]
async fn an_aborted_transaction_releases_its_deferred_unique_lock() {
    let driver = install_driver();
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(tokio::sync::Notify::new());
    let (entered_in, release_in) = (entered.clone(), release.clone());
    let handle = tokio::spawn(async move {
        DB::transaction(|_tx| {
            Box::pin(async move {
                let taken = Queue::push_unique(UniqueAfterCommitJob {
                    key: "abort-1".into(),
                })
                .await?;
                assert!(taken, "the lock is taken at push time");
                entered_in.store(true, Ordering::SeqCst);
                release_in.notified().await;
                Ok::<(), FrameworkError>(())
            })
        })
        .await
    });

    poll_until(
        || entered.load(Ordering::SeqCst),
        Duration::from_secs(10),
        "the transaction to take its lock and park",
    )
    .await;
    handle.abort();
    let _ = handle.await;

    // The diverted compensation must release the lock: the same unique
    // key becomes pushable again immediately, not after `unique_for`.
    // A bespoke loop here (rather than `poll_until`) because the
    // condition itself awaits.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let retried = Queue::push_unique(UniqueAfterCommitJob {
                key: "abort-1".into(),
            })
            .await
            .unwrap_or(false);
            if retried {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the aborted transaction's unique lock must be released");
    assert_eq!(
        driver.count(),
        1,
        "only the re-push envelope exists; the aborted dispatch never published"
    );
}

/// Driver that parks its first push behind a gate, so a test can abort
/// the transaction task while a post-commit callback is in flight.
struct GatedDriver {
    pushed: Mutex<Vec<Envelope>>,
    gate_armed: AtomicBool,
    entered: AtomicBool,
    release: tokio::sync::Notify,
}

impl GatedDriver {
    fn new() -> Self {
        Self {
            pushed: Mutex::new(Vec::new()),
            gate_armed: AtomicBool::new(true),
            entered: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait]
impl QueueDriver for GatedDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        if self.gate_armed.swap(false, Ordering::SeqCst) {
            self.entered.store(true, Ordering::SeqCst);
            self.release.notified().await;
        }
        self.pushed.lock().unwrap().push(env);
        Ok(())
    }

    async fn pop(&self, _vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }

    async fn ack(&self, _t: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn nack(&self, _t: &ReservationToken, _delay: Duration) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        Ok(self.pushed.lock().unwrap().len() as u64)
    }

    fn name(&self) -> &'static str {
        "gated"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlockingAfterCommitJob;

#[async_trait]
impl Job for BlockingAfterCommitJob {
    fn job_name() -> &'static str {
        "wave5-after-commit-blocking"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn an_abort_after_commit_still_completes_every_callback_exactly_once() {
    let driver = Arc::new(GatedDriver::new());
    Queue::set_driver(driver.clone());
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let handle = tokio::spawn(async {
        DB::transaction(|_tx| {
            Box::pin(async {
                // Registration order is execution order: the blocking push
                // runs first and parks inside the driver gate.
                Queue::push(BlockingAfterCommitJob).await?;
                Queue::push(OtherAfterCommitJob).await?;
                Ok::<(), FrameworkError>(())
            })
        })
        .await
    });

    poll_until(
        || driver.entered.load(Ordering::SeqCst),
        Duration::from_secs(10),
        "the first post-commit callback to go in flight",
    )
    .await;
    handle.abort();
    let _ = handle.await;
    // Let the in-flight callback finish; the owned runner then starts
    // the remaining callbacks in order. `notify_one` (not
    // `notify_waiters`) so the permit survives if the child has not
    // parked yet.
    driver.release.notify_one();

    poll_until(
        || driver.pushed.lock().unwrap().len() == 2,
        Duration::from_secs(10),
        "both post-commit callbacks to complete after the abort",
    )
    .await;
    let names: Vec<String> = driver
        .pushed
        .lock()
        .unwrap()
        .iter()
        .map(|env| env.job_name.clone())
        .collect();
    assert!(
        names.contains(&BlockingAfterCommitJob::job_name().to_string()),
        "the in-flight callback must survive the abort, got: {names:?}"
    );
    assert!(
        names.contains(&OtherAfterCommitJob::job_name().to_string()),
        "the unstarted remainder must divert, got: {names:?}"
    );
}
