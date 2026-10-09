//! Laravel infrastructure gaps in queued listeners: the unique lock a
//! listener's job declares, and a delay the registration gives (PAR-150).
//!
//! Laravel evidence: `Events/Dispatcher.php:669-672` (the dispatcher takes
//! the unique lock before it queues a listener), `:767-777` (the unique
//! fields are copied onto the listener's job) and
//! `Events/QueuedClosure.php:131-141` (a delay as a date, an interval or
//! seconds).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::events::testing::assert_dispatched;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::events::UniqueJobSkipped;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::testing::pushed_with_available_at;
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::testing::TestClock;
use suprnova::{
    Event, EventDispatcher, EventFacade, FrameworkError, Job, Listener, Queue, QueuedListener,
    async_trait,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
struct OrderPlaced {
    order_id: u32,
}

/// An event that says when its follow-up is due.
#[derive(Debug, Clone)]
struct ReminderScheduled {
    send_at: DateTime<Utc>,
}

impl Event for ReminderScheduled {
    fn event_name() -> &'static str {
        "laravel_infra_gaps.ReminderScheduled"
    }
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str {
        "laravel_infra_gaps.OrderPlaced"
    }
}

/// A job that is unique per order.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReindexOrder {
    order_id: u32,
}

#[async_trait]
impl Job for ReindexOrder {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-reindex-order"
    }
    fn unique_id(&self) -> Option<String> {
        Some("k".into())
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

fn install_cache() {
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
}

#[tokio::test]
#[serial]
async fn a_second_dispatch_to_a_unique_queued_listener_pushes_nothing_and_reports_the_skip() {
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    // The local dispatcher runs the listener; the fake records the
    // `UniqueJobSkipped` the queue dispatches through the facade.
    let _events = EventFacade::fake();
    let dispatcher = EventDispatcher::new();
    dispatcher
        .listen::<OrderPlaced, _>(Arc::new(QueuedListener::<OrderPlaced, ReindexOrder>::new(
            |e| ReindexOrder {
                order_id: e.order_id,
            },
        )))
        .await;

    dispatcher
        .dispatch(OrderPlaced { order_id: 7 })
        .await
        .unwrap();
    dispatcher
        .dispatch(OrderPlaced { order_id: 7 })
        .await
        .unwrap();

    assert_eq!(
        driver.size(None).await.unwrap(),
        1,
        "the second dispatch finds the lock held and pushes nothing"
    );
    assert_dispatched::<UniqueJobSkipped>(|e| {
        e.unique_id == "k" && e.job_name == "laravel-infra-gaps-reindex-order"
    });
}

static UNTIL_PROCESSING_HANDLED: AtomicUsize = AtomicUsize::new(0);

/// Unique until a worker starts it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RebuildIndex;

#[async_trait]
impl Job for RebuildIndex {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-rebuild-index"
    }
    fn unique_id(&self) -> Option<String> {
        Some("index".into())
    }
    fn unique_until_processing() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        UNTIL_PROCESSING_HANDLED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_unique_until_processing_listener_job_releases_its_lock_when_it_starts() {
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<RebuildIndex>();
    UNTIL_PROCESSING_HANDLED.store(0, Ordering::SeqCst);
    let listener = QueuedListener::<OrderPlaced, RebuildIndex>::new(|_| RebuildIndex);

    listener.handle(&OrderPlaced { order_id: 1 }).await.unwrap();
    listener.handle(&OrderPlaced { order_id: 1 }).await.unwrap();
    assert_eq!(
        driver.size(None).await.unwrap(),
        1,
        "the duplicate is skipped"
    );

    tokio::time::timeout(
        Duration::from_secs(15),
        run_worker(
            driver.clone(),
            WorkerConfig {
                max_jobs: Some(1),
                poll_interval: Duration::from_millis(10),
                ..WorkerConfig::default()
            },
            CancellationToken::new(),
        ),
    )
    .await
    .expect("the worker settles the job within 15s")
    .expect("the worker starts");
    assert_eq!(UNTIL_PROCESSING_HANDLED.load(Ordering::SeqCst), 1);

    listener.handle(&OrderPlaced { order_id: 1 }).await.unwrap();
    assert_eq!(
        driver.size(None).await.unwrap(),
        1,
        "the lock was released when processing began, so the next dispatch is pushed"
    );
}

/// Declares a delay of its own, which the listener's delay outranks.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SendReminder {
    order_id: u32,
}

#[async_trait]
impl Job for SendReminder {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-send-reminder"
    }
    fn delay() -> Option<Duration> {
        Some(Duration::from_secs(600))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_listener_delay_delays_the_job_and_wins_over_the_jobs_own() {
    let clock = TestClock::freeze();
    let _queue = Queue::fake();
    let listener = QueuedListener::<OrderPlaced, SendReminder>::new(|e| SendReminder {
        order_id: e.order_id,
    })
    .delay(Duration::from_secs(30));

    listener.handle(&OrderPlaced { order_id: 3 }).await.unwrap();

    let pushed = pushed_with_available_at::<SendReminder>();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].0.order_id, 3);
    assert_eq!(pushed[0].1, clock.now() + chrono::Duration::seconds(30));
}

#[tokio::test]
#[serial]
async fn delay_until_delays_the_job_to_the_time_the_event_names() {
    let clock = TestClock::freeze();
    let _queue = Queue::fake();
    let send_at = clock.now() + chrono::Duration::hours(2);
    let listener =
        QueuedListener::<ReminderScheduled, SendReminder>::new(|_| SendReminder { order_id: 4 })
            .delay_until(|e| e.send_at);

    listener
        .handle(&ReminderScheduled { send_at })
        .await
        .unwrap();

    let pushed = pushed_with_available_at::<SendReminder>();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].1, send_at);
}

#[tokio::test]
#[serial]
async fn a_unique_listener_job_keeps_the_listener_delay() {
    let clock = TestClock::freeze();
    let _queue = Queue::fake();
    let listener = QueuedListener::<OrderPlaced, ReindexOrder>::new(|e| ReindexOrder {
        order_id: e.order_id,
    })
    .delay(Duration::from_secs(45));

    listener.handle(&OrderPlaced { order_id: 5 }).await.unwrap();

    let pushed = pushed_with_available_at::<ReindexOrder>();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].1, clock.now() + chrono::Duration::seconds(45));
}

/// Declares both a unique id and a debounce window, which no push accepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Conflicted;

#[async_trait]
impl Job for Conflicted {
    fn job_name() -> &'static str {
        "laravel-infra-gaps-conflicted"
    }
    fn unique_id(&self) -> Option<String> {
        Some("one".into())
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_secs(30))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_listener_job_that_is_unique_and_debounced_is_still_refused() {
    let _queue = Queue::fake();
    let listener = QueuedListener::<OrderPlaced, Conflicted>::new(|_| Conflicted);
    let err = listener
        .handle(&OrderPlaced { order_id: 6 })
        .await
        .expect_err("debouncing and uniqueness cannot both apply");
    assert!(err.to_string().contains("debounce_for"), "{err}");
}
