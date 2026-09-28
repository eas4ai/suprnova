use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::error::FrameworkError;
use suprnova::events::{EventFacade, dispatched};
use suprnova::queue::events::JobReleasedAfterException;
use suprnova::queue::retry::{RETRY_HINT_CEILING, delay_after_failure, next_delay};
use suprnova::queue::{
    BackoffSchedule, Job, MemoryQueueDriver, Queue,
    worker::{WorkerConfig, register_job, run_worker},
};
use tokio_util::sync::CancellationToken;

#[test]
fn fixed_backoff_returns_constant_delay() {
    let sched = BackoffSchedule::Fixed { secs: 5 };
    assert_eq!(next_delay(&sched, 1, Some(0.0)), Duration::from_secs(5));
    assert_eq!(next_delay(&sched, 7, Some(0.0)), Duration::from_secs(5));
}

#[test]
fn exponential_backoff_doubles_until_cap() {
    let sched = BackoffSchedule::Exponential {
        base_secs: 2,
        cap_secs: 60,
        jitter_ratio: 0.0,
    };
    assert_eq!(next_delay(&sched, 1, Some(0.0)), Duration::from_secs(2));
    assert_eq!(next_delay(&sched, 2, Some(0.0)), Duration::from_secs(4));
    assert_eq!(next_delay(&sched, 3, Some(0.0)), Duration::from_secs(8));
    assert_eq!(next_delay(&sched, 10, Some(0.0)), Duration::from_secs(60));
}

#[test]
fn exponential_jitter_stays_in_band() {
    let sched = BackoffSchedule::Exponential {
        base_secs: 10,
        cap_secs: 1000,
        jitter_ratio: 0.25,
    };
    // attempts=2 -> base_delay = 20. With jitter=±25%, range is [15, 25].
    // deterministic_jitter=Some(1.0) means max (+25%), Some(-1.0) means min (-25%).
    assert_eq!(next_delay(&sched, 2, Some(1.0)), Duration::from_secs(25));
    assert_eq!(next_delay(&sched, 2, Some(-1.0)), Duration::from_secs(15));
}

#[test]
fn exponential_jitter_at_one_does_not_exceed_cap() {
    // Pre-fix, jitter_ratio=1.0 with deterministic_jitter=+1.0
    // produced 2 × cap_secs because the post-jitter delay wasn't
    // re-capped. `cap_secs` is supposed to be a strict ceiling.
    let sched = BackoffSchedule::Exponential {
        base_secs: 10,
        cap_secs: 100,
        jitter_ratio: 1.0,
    };
    // attempts=10 → exponential schedule has saturated at cap (100s).
    // (1 + 1.0) * 100 = 200, but the final clamp pins it to 100.
    assert_eq!(next_delay(&sched, 10, Some(1.0)), Duration::from_secs(100));
}

#[test]
fn exponential_out_of_range_jitter_is_pinned_safely() {
    // jitter_ratio > 1.0 used to scale delays unbounded; NaN crashed
    // when round() was called on `nan`. Clamp + final cap pin both.
    let sched = BackoffSchedule::Exponential {
        base_secs: 10,
        cap_secs: 100,
        jitter_ratio: 5.0,
    };
    assert_eq!(next_delay(&sched, 10, Some(1.0)), Duration::from_secs(100));
    let nan_sched = BackoffSchedule::Exponential {
        base_secs: 10,
        cap_secs: 100,
        jitter_ratio: f32::NAN,
    };
    // NaN collapses to 0 - no jitter, plain capped delay.
    assert_eq!(
        next_delay(&nan_sched, 10, Some(1.0)),
        Duration::from_secs(100)
    );
}

#[test]
fn sequence_backoff_follows_explicit_steps() {
    let sched = BackoffSchedule::Sequence {
        secs: vec![1, 3, 9],
    };
    assert_eq!(next_delay(&sched, 1, Some(0.0)), Duration::from_secs(1));
    assert_eq!(next_delay(&sched, 2, Some(0.0)), Duration::from_secs(3));
    assert_eq!(next_delay(&sched, 3, Some(0.0)), Duration::from_secs(9));
    // beyond the sequence -> last entry sticks
    assert_eq!(next_delay(&sched, 99, Some(0.0)), Duration::from_secs(9));
}

#[test]
fn a_retry_hint_replaces_the_schedule_in_both_directions() {
    let schedule = BackoffSchedule::Fixed { secs: 30 };
    let longer = FrameworkError::rate_limited(Some(Duration::from_secs(90)), "push service");
    let shorter = FrameworkError::rate_limited(Some(Duration::from_secs(2)), "push service");

    assert_eq!(
        delay_after_failure(&schedule, 1, &longer),
        Duration::from_secs(90),
        "retrying sooner would hit a service that already said no"
    );
    assert_eq!(
        delay_after_failure(&schedule, 1, &shorter),
        Duration::from_secs(2),
        "waiting longer than the service asked helps nobody"
    );
}

#[test]
fn a_failure_without_a_hint_keeps_the_schedule() {
    let schedule = BackoffSchedule::Fixed { secs: 30 };
    let no_hint = FrameworkError::rate_limited(None, "push service");
    let other = FrameworkError::internal("boom");

    assert_eq!(
        delay_after_failure(&schedule, 1, &no_hint),
        Duration::from_secs(30)
    );
    assert_eq!(
        delay_after_failure(&schedule, 4, &other),
        Duration::from_secs(30)
    );
}

#[test]
fn a_retry_hint_is_capped() {
    let schedule = BackoffSchedule::Fixed { secs: 30 };
    let absurd = FrameworkError::rate_limited(Some(Duration::from_secs(u64::MAX)), "push service");

    assert_eq!(
        delay_after_failure(&schedule, 1, &absurd),
        RETRY_HINT_CEILING
    );
    assert_eq!(RETRY_HINT_CEILING, Duration::from_secs(86_400));
}

#[derive(Serialize, Deserialize, Clone)]
struct ThrottledJob;

#[async_trait]
impl Job for ThrottledJob {
    fn job_name() -> &'static str {
        "queue_retry::ThrottledJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Err(FrameworkError::rate_limited(
            Some(Duration::from_secs(90)),
            "push service rejected (status 429)",
        ))
    }
    fn max_tries() -> u32 {
        3
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 5 }
    }
}

#[tokio::test]
#[serial]
async fn the_worker_releases_a_throttled_job_for_as_long_as_the_service_asked() {
    register_job::<ThrottledJob>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _events = EventFacade::fake();
    Queue::push(ThrottledJob).await.unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    run_worker(driver, cfg, CancellationToken::new()).await;

    let released = dispatched::<JobReleasedAfterException>(|_| true);
    assert_eq!(released.len(), 1, "one failed attempt, one release");
    assert_eq!(
        released[0].delay_secs, 90,
        "the 90 second hint, not the job's 5 second backoff"
    );
    assert_eq!(
        Queue::delayed_size().await.unwrap(),
        1,
        "the job waits on the queue for its next attempt"
    );
}
