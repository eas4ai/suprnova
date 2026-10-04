//! End-to-end test for the Queue facade's delayed-dispatch path.
//! Drives Queue::later through the MemoryQueueDriver and asserts the
//! envelope is invisible until tokio's virtual clock advances past the
//! delay deadline.

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::{FrameworkError, Job, Queue, async_trait};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ScheduledNote {
    body: String,
}

#[async_trait]
impl Job for ScheduledNote {
    fn job_name() -> &'static str {
        "ScheduledNote"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
#[serial]
async fn queue_later_dispatches_via_driver_and_honors_delay() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Queue::later(
        Duration::from_secs(60),
        ScheduledNote {
            body: "later".into(),
        },
    )
    .await
    .unwrap();

    // Immediately after dispatch, the driver MUST not surface the message.
    let nothing = driver.pop(Duration::from_millis(10)).await.unwrap();
    assert!(
        nothing.is_none(),
        "delayed job must not be visible before its deadline"
    );

    // Advance Tokio's virtual clock past available_at.
    tokio::time::advance(Duration::from_secs(61)).await;

    // Pop should succeed and the envelope should match the dispatched job.
    let reservation = driver
        .pop(Duration::from_millis(10))
        .await
        .unwrap()
        .expect("delayed job must be visible after available_at");
    assert_eq!(reservation.envelope.job_name, "ScheduledNote");
    assert_eq!(reservation.envelope.payload["body"], "later");

    driver.ack(&reservation.token).await.unwrap();
}

/// A job that asks to wait a million years: a delay that fits a duration
/// but runs past the last date the clock can hold.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct MillionYearNote;

const MILLION_YEARS: Duration = Duration::from_secs(1_000_000 * 365 * 86_400);

#[async_trait]
impl Job for MillionYearNote {
    fn job_name() -> &'static str {
        "MillionYearNote"
    }
    fn delay() -> Option<Duration> {
        Some(MILLION_YEARS)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// A delay too long for any date is an error naming the delay, not a panic
/// in the date arithmetic, for every delayed dispatch.
#[tokio::test]
#[serial]
async fn a_delay_too_long_for_a_date_is_an_error() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let note = || ScheduledNote {
        body: "forever".into(),
    };

    let outcomes = [
        tokio::spawn(Queue::later(MILLION_YEARS, note())).await,
        tokio::spawn(Queue::later_with(
            MILLION_YEARS,
            note(),
            suprnova::EnvelopeOverrides::default(),
        ))
        .await,
        tokio::spawn(async move { Queue::later_unique(MILLION_YEARS, note()).await.map(|_| ()) })
            .await,
        tokio::spawn(Queue::push(MillionYearNote)).await,
    ];
    for (call, outcome) in ["later", "later_with", "later_unique", "Job::delay"]
        .iter()
        .zip(&outcomes)
    {
        assert!(
            matches!(outcome, Ok(Err(error)) if error.to_string().contains("delay")),
            "{call} with a million-year delay must return an error, got {outcome:?}"
        );
    }
    assert_eq!(driver.size().await.unwrap(), 0, "nothing was queued");
}
