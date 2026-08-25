//! Queue introspection + bulk + clear tests.

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::error::FrameworkError;
use suprnova::queue::testing::{install_fake, pushed_with_available_at};
use suprnova::queue::{Job, MemoryQueueDriver, Queue, QueueDriver};

#[derive(Serialize, Deserialize, Clone)]
struct Marker {
    x: u32,
}

#[async_trait]
impl Job for Marker {
    fn job_name() -> &'static str {
        "queue_introspection::Marker"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn bulk_pushes_every_job() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::bulk(vec![Marker { x: 1 }, Marker { x: 2 }, Marker { x: 3 }])
        .await
        .unwrap();
    assert_eq!(driver.pending_size().await.unwrap(), 3);
    assert_eq!(Queue::pending_size().await.unwrap(), 3);
    assert_eq!(Queue::size().await.unwrap(), 3);
}

#[tokio::test]
#[serial]
async fn clear_removes_every_envelope() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::bulk(vec![Marker { x: 1 }, Marker { x: 2 }])
        .await
        .unwrap();
    let removed = Queue::clear().await.unwrap();
    assert_eq!(removed, 2);
    assert_eq!(Queue::size().await.unwrap(), 0);
}

// ---- Job::delay() (Laravel 13.25 #60916) ---------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct DelayedMarker {
    x: u32,
}

#[async_trait]
impl Job for DelayedMarker {
    fn job_name() -> &'static str {
        "queue_introspection::DelayedMarker"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
    fn delay() -> Option<Duration> {
        Some(Duration::from_secs(120))
    }
}

#[tokio::test]
#[serial]
async fn bulk_records_per_job_delay_in_the_fake() {
    let _guard = install_fake();
    let before = Utc::now();
    Queue::bulk(vec![
        DelayedMarker { x: 1 },
        DelayedMarker { x: 2 },
        DelayedMarker { x: 3 },
    ])
    .await
    .unwrap();
    let after = Utc::now();

    let entries = pushed_with_available_at::<DelayedMarker>();
    assert_eq!(entries.len(), 3, "every job in the bulk must be captured");
    for (job, available_at) in &entries {
        let msg = format!(
            "job x={} must record now + Job::delay() (120s), got {available_at}",
            job.x
        );
        assert!(
            *available_at >= before + ChronoDuration::seconds(120)
                && *available_at <= after + ChronoDuration::seconds(120),
            "{msg}"
        );
    }
}

#[tokio::test]
#[serial]
async fn bulk_delayed_job_is_not_pending_undelayed_sibling_is() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // `bulk<J>` is monomorphic in `J`, so one Vec cannot mix a delayed and
    // an undelayed job - two `bulk` calls, one per job type, land both
    // envelopes on the same driver instead.
    Queue::bulk(vec![DelayedMarker { x: 1 }]).await.unwrap();
    Queue::bulk(vec![Marker { x: 1 }]).await.unwrap();

    assert_eq!(
        driver.pending_size().await.unwrap(),
        1,
        "only the undelayed sibling is pending"
    );
    assert_eq!(
        driver.delayed_size().await.unwrap(),
        1,
        "the declared-delay job is held back"
    );

    let popped = driver
        .pop(Duration::from_millis(10))
        .await
        .unwrap()
        .expect("the undelayed sibling must pop immediately");
    assert_eq!(popped.envelope.job_name, "queue_introspection::Marker");
    driver.ack(&popped.token).await.unwrap();

    let nothing = driver.pop(Duration::from_millis(10)).await.unwrap();
    assert!(
        nothing.is_none(),
        "the delayed sibling must not pop before its deadline"
    );
}
