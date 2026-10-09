//! Sync + Null driver smoke tests.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use suprnova::error::FrameworkError;
use suprnova::queue::{Job, NullQueueDriver, Queue, SyncQueueDriver, worker::register_job};

static SYNC_RAN: AtomicU32 = AtomicU32::new(0);

#[derive(Serialize, Deserialize, Clone)]
struct SyncDriverJob;

#[async_trait]
impl Job for SyncDriverJob {
    fn job_name() -> &'static str {
        "queue_drivers_sync_null::SyncDriverJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        SYNC_RAN.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn sync_driver_executes_inline_on_push() {
    SYNC_RAN.store(0, Ordering::SeqCst);
    register_job::<SyncDriverJob>();
    Queue::set_driver(Arc::new(SyncQueueDriver::new()));
    Queue::push(SyncDriverJob).await.unwrap();
    assert_eq!(SYNC_RAN.load(Ordering::SeqCst), 1);
}

#[derive(Serialize, Deserialize, Clone)]
struct NullDriverJob;

#[async_trait]
impl Job for NullDriverJob {
    fn job_name() -> &'static str {
        "queue_drivers_sync_null::NullDriverJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        unreachable!("null driver must not run the handler");
    }
}

#[tokio::test]
#[serial]
async fn null_driver_discards_pushes_without_running() {
    register_job::<NullDriverJob>();
    Queue::set_driver(Arc::new(NullQueueDriver::new()));
    Queue::push(NullDriverJob).await.unwrap();
    Queue::push(NullDriverJob).await.unwrap();
    // Null driver reports size 0 always.
    assert_eq!(Queue::size().await.unwrap(), 0);
}

/// The default `release`, which the null and sync drivers and any custom
/// driver inherit, refuses a delay too long for any date instead of
/// panicking on it, and instead of turning a delay too long for a duration
/// into no delay at all.
#[tokio::test]
async fn the_default_release_refuses_a_delay_too_long_for_a_date() {
    use std::time::Duration;
    use suprnova::queue::driver::{QueueDriver, ReservationToken};

    let now = suprnova::clock::now();
    let env = suprnova::queue::Envelope {
        schema_version: suprnova::queue::CURRENT_SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        job_name: "release-probe".into(),
        queue: None,
        payload: serde_json::json!({}),
        dispatched_at: now,
        available_at: now,
        attempts: 1,
        max_tries: 3,
        backoff: suprnova::queue::BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
        context: None,
    };
    let million_years = Duration::from_secs(1_000_000 * 365 * 86_400);
    for delay in [million_years, Duration::MAX] {
        let env = env.clone();
        let outcome = tokio::spawn(async move {
            NullQueueDriver::new()
                .release(&ReservationToken(uuid::Uuid::new_v4()), &env, delay)
                .await
        })
        .await;
        assert!(
            matches!(&outcome, Ok(Err(error)) if error.to_string().contains("delay")),
            "a release delayed {delay:?} must return an error, got {outcome:?}"
        );
    }
}
