//! After-commit dispatch parity (#60996): a push inside `DB::transaction` is
//! invisible until the transaction commits, a rollback discards it entirely,
//! and outside a transaction an `after_commit` job dispatches immediately.
//!
//! Laravel evidence: `Database/DatabaseTransactionsManager.php:213-233`
//! (`addCallback` runs immediately with no open transaction; `addCallbackForRollback`
//! is a silent no-op there) and `Queue/Queue.php:366-450` (`enqueueUsing` defers the
//! *entire* push, and the rollback callback releases a unique job's lock).

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::queue::driver::{QueueDriver, Reservation, ReservationToken};
use suprnova::queue::testing::{install_fake, pushed};
use suprnova::queue::worker::register_job;
use suprnova::queue::{Envelope, SyncQueueDriver};
use suprnova::testing::TestDatabase;
use suprnova::{DB, EnvelopeOverrides, FrameworkError, Job, Queue, async_trait};

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

    assert_eq!(
        driver.count(),
        1,
        "exactly one envelope lands, and only after the commit"
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
