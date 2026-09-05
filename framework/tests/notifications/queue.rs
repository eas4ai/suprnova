use async_trait::async_trait;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use suprnova::BackoffSchedule;
use suprnova::events::{EventFacade, dispatched};
use suprnova::notifications::channels::database::DatabaseChannel;
use suprnova::notifications::notify_job::SendNotificationJob;
use suprnova::notifications::{
    Channel, DynNotification, Notifiable, Notification, NotificationDispatcher,
};
use suprnova::queue::Job;
use suprnova::queue::Queue;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::events::{JobFailed, JobTimedOut};
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::{FrameworkError, Notify};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OrderShipped {
    tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["database"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User {
    id: i64,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        if channel == "database" {
            Some(self.id.to_string())
        } else {
            None
        }
    }
}

async fn fresh_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.execute_unprepared(
        r"
        CREATE TABLE notifications (
            id CHAR(36) PRIMARY KEY,
            type VARCHAR(255) NOT NULL,
            notifiable_type VARCHAR(255) NOT NULL,
            notifiable_id VARCHAR(64) NOT NULL,
            data TEXT NOT NULL,
            read_at TIMESTAMP NULL,
            created_at TIMESTAMP NOT NULL,
            updated_at TIMESTAMP NOT NULL
        )
        ",
    )
    .await
    .unwrap();
    db
}

#[tokio::test]
#[serial]
async fn notification_queue_dispatches_through_queue_and_lands_in_db() {
    let db = fresh_db().await;
    let dispatcher = NotificationDispatcher::new()
        .register_channel(Arc::new(DatabaseChannel::new(db.clone(), "users")));
    let _ = suprnova::notifications::set_dispatcher(Arc::new(dispatcher));

    let _ = suprnova::notifications::register_notification_factory::<OrderShipped>();
    register_job::<SendNotificationJob>();

    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Notify::queue(
        &User { id: 7 },
        OrderShipped {
            tracking: "1Z".into(),
        },
    )
    .await
    .unwrap();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        WorkerConfig {
            visibility_timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(5),
            max_jobs: None,
            queues: Vec::new(),
        },
        CancellationToken::new(),
    ));

    for _ in 0..200 {
        let row = db
            .query_one_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) FROM notifications".to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        let n: i64 = row.try_get_by_index(0).unwrap();
        if n > 0 {
            break;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();

    let row = db
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT type, notifiable_type, notifiable_id, data FROM notifications".to_string(),
        ))
        .await
        .unwrap()
        .expect("row present");
    assert_eq!(row.try_get_by_index::<String>(0).unwrap(), "OrderShipped");
    assert_eq!(row.try_get_by_index::<String>(1).unwrap(), "users");
    assert_eq!(row.try_get_by_index::<String>(2).unwrap(), "7");
    let data_json: String = row.try_get_by_index(3).unwrap();
    let data: serde_json::Value = serde_json::from_str(&data_json).unwrap();
    assert_eq!(data["tracking"], "1Z");
}

#[tokio::test]
#[serial]
async fn notification_queue_unregistered_notification_surfaces_unknown_error_from_job() {
    // If a Notification's factory isn't registered, SendNotificationJob's
    // handle path returns `unknown notification: {name}` from the
    // registry lookup. This protects against silent retry loops on a
    // typo'd notification_name. We invoke handle directly rather than
    // round-tripping through the worker so the assertion is targeted at
    // the registry lookup, not at end-to-end retry/dead-letter behavior.
    use std::collections::HashMap;
    use suprnova::queue::Job;

    // The dispatcher binding is required by handle() before the factory
    // lookup runs - bind a minimal one so the assertion targets the
    // factory error, not the missing-dispatcher error.
    let _ = suprnova::notifications::set_dispatcher(Arc::new(NotificationDispatcher::new()));

    let job = SendNotificationJob {
        notifiable_route_per_channel: HashMap::new(),
        notification_name: "TotallyUnregisteredNotification".to_string(),
        notification_payload: serde_json::json!({}),
        channels: vec![],
    };
    let err = job.handle().await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown notification"),
        "error names the missing registry entry: {msg}"
    );
    assert!(
        msg.contains("TotallyUnregisteredNotification"),
        "error names the missing notification: {msg}"
    );
}

static SEND_HITS: AtomicU32 = AtomicU32::new(0);

struct CountingChannel;

#[async_trait]
impl Channel for CountingChannel {
    fn name(&self) -> &'static str {
        "database"
    }
    async fn deliver(
        &self,
        _route: &str,
        _notification: &dyn DynNotification,
    ) -> Result<(), FrameworkError> {
        SEND_HITS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn notify_send_delivers_synchronously_through_bound_dispatcher() {
    // Notify::send is the sync sibling of Notify::queue - it must forward
    // to the bound dispatcher in-process with no queue round-trip.
    SEND_HITS.store(0, Ordering::SeqCst);

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(CountingChannel));
    let _ = suprnova::notifications::set_dispatcher(Arc::new(dispatcher));

    Notify::send(
        &User { id: 42 },
        &OrderShipped {
            tracking: "SYNC-1".into(),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        SEND_HITS.load(Ordering::SeqCst),
        1,
        "Notify::send must invoke the bound dispatcher exactly once"
    );
}

// Multi-channel notification with two routed channels.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct DualChannelAlert {
    body: String,
}

impl Notification for DualChannelAlert {
    fn notification_name() -> &'static str {
        "DualChannelAlert"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["database", "mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "body": self.body })
    }
}

struct DualUser {
    id: i64,
    email: String,
}

impl Notifiable for DualUser {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "database" => Some(self.id.to_string()),
            "mail" => Some(self.email.clone()),
            _ => None,
        }
    }
}

// Regression: Notify::queue must push ONE SendNotificationJob per declared,
// routed channel. Before the fix, a single envelope carried the full
// channel list - so any per-channel failure restarted ALL channels on
// retry, causing the database channel to insert twice and the recipient
// to receive the same email twice.
#[tokio::test]
#[serial]
async fn notify_queue_pushes_one_envelope_per_routed_channel() {
    use suprnova::queue::driver::Reservation;

    // Dispatcher binding is needed only to satisfy register_notification_factory.
    let _ = suprnova::notifications::set_dispatcher(Arc::new(NotificationDispatcher::new()));
    let _ = suprnova::notifications::register_notification_factory::<DualChannelAlert>();
    register_job::<SendNotificationJob>();

    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Notify::queue(
        &DualUser {
            id: 99,
            email: "x@example.org".into(),
        },
        DualChannelAlert {
            body: "ping".into(),
        },
    )
    .await
    .unwrap();

    // Drain the driver and count envelopes + assert each carries exactly
    // one channel.
    let mut popped: Vec<Reservation> = Vec::new();
    while let Some(r) = driver.pop(Duration::from_secs(1)).await.unwrap() {
        popped.push(r);
    }
    assert_eq!(
        popped.len(),
        2,
        "queue must hold one envelope per routed channel (database + mail = 2)",
    );
    for r in &popped {
        let job: SendNotificationJob = serde_json::from_value(r.envelope.payload.clone())
            .expect("payload decodes to SendNotificationJob");
        assert_eq!(
            job.channels.len(),
            1,
            "each envelope must carry exactly one channel for retry isolation",
        );
        assert_eq!(
            job.notifiable_route_per_channel.len(),
            1,
            "each envelope must carry exactly one route for its own channel",
        );
    }
}

// Regression: a recipient whose `route_for` returns None for a declared
// channel must not produce an envelope for that channel - matches the
// pre-fix behaviour where the handle path skipped unrouted channels.
#[tokio::test]
#[serial]
async fn notify_queue_skips_channels_with_no_route() {
    // Dispatcher binding is needed only to satisfy register_notification_factory.
    let _ = suprnova::notifications::set_dispatcher(Arc::new(NotificationDispatcher::new()));
    let _ = suprnova::notifications::register_notification_factory::<DualChannelAlert>();
    register_job::<SendNotificationJob>();

    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // User resolves only the database channel; mail returns None.
    Notify::queue(
        &User { id: 99 },
        DualChannelAlert {
            body: "ping".into(),
        },
    )
    .await
    .unwrap();

    let r = driver
        .pop(Duration::from_secs(1))
        .await
        .unwrap()
        .expect("the database channel must produce an envelope");
    let job: SendNotificationJob =
        serde_json::from_value(r.envelope.payload.clone()).expect("decode");
    assert_eq!(job.channels, vec!["database".to_string()]);
    assert!(
        driver.pop(Duration::from_secs(1)).await.unwrap().is_none(),
        "no envelope for the mail channel (route_for returned None)",
    );
}

// Regression: `Notification::queue()` must ride the push as an
// `EnvelopeOverrides`, landing the envelope on the named queue instead
// of the driver default.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct RoutedNotification;

impl Notification for RoutedNotification {
    fn notification_name() -> &'static str {
        "RoutedNotification"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["database"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn queue(&self) -> Option<&'static str> {
        Some("notifications")
    }
}

#[tokio::test]
#[serial]
async fn notify_queue_honors_the_notifications_own_queue_override() {
    let _ = suprnova::notifications::set_dispatcher(Arc::new(NotificationDispatcher::new()));
    let _ = suprnova::notifications::register_notification_factory::<RoutedNotification>();
    register_job::<SendNotificationJob>();

    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Notify::queue(&User { id: 5 }, RoutedNotification)
        .await
        .unwrap();

    let default_pop = driver
        .pop_from(Duration::from_secs(60), &["default".to_string()])
        .await
        .unwrap();
    assert!(
        default_pop.is_none(),
        "default must not drain a push routed to \"notifications\""
    );

    let routed = driver
        .pop_from(Duration::from_secs(60), &["notifications".to_string()])
        .await
        .unwrap()
        .expect("\"notifications\" must drain the routed push");
    assert_eq!(routed.envelope.queue.as_deref(), Some("notifications"));
}

// `OrderShipped` (defined at the top of this file) overrides none of the
// five queue-tuning methods. Its envelope must come out identical to
// what a bare `Queue::push` would have produced - proving `Notify::queue`'s
// always-`Some` overlay for `fail_on_timeout`/`max_tries`/`backoff`
// (Design note 2) doesn't silently change behavior for the common case.
#[tokio::test]
#[serial]
async fn notify_queue_leaves_envelope_defaults_untouched_for_a_notification_that_overrides_nothing()
{
    let _ = suprnova::notifications::set_dispatcher(Arc::new(NotificationDispatcher::new()));
    let _ = suprnova::notifications::register_notification_factory::<OrderShipped>();
    register_job::<SendNotificationJob>();

    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Notify::queue(
        &User { id: 8 },
        OrderShipped {
            tracking: "1Z".into(),
        },
    )
    .await
    .unwrap();

    let popped = driver
        .pop_from(Duration::from_secs(60), &["default".to_string()])
        .await
        .unwrap()
        .expect("an unrouted notification still lands on the driver default queue");
    let env = popped.envelope;
    assert_eq!(
        env.queue, None,
        "Notification::queue()'s None default must not force a queue"
    );
    assert_eq!(
        env.max_tries, 3,
        "Notification::max_tries()'s default must match Job::max_tries()'s default"
    );
    assert!(
        !env.fail_on_timeout,
        "Notification::fail_on_timeout()'s default must match Job::fail_on_timeout()'s default"
    );
    assert_eq!(
        env.timeout_secs, None,
        "Notification::timeout()'s None default must not set a budget"
    );
    assert_eq!(
        env.backoff,
        BackoffSchedule::default(),
        "Notification::backoff()'s default must match Job::backoff()'s default"
    );
}

// The real invariant pin for Design note 2's "no double application" rule.
// The test above only checks the overlay against `Notification`'s own
// defaults, so it stays green even if `Notification` and
// `SendNotificationJob`'s `Job` impl drifted apart together. This test
// compares the two sides directly: `Notification`'s defaults (read off a
// bare notification that overrides none of the five) against what
// `SendNotificationJob` actually reports through its `Job` impl. If either
// side changes without the other, this fails. See the "must not override"
// comment on `impl Job for SendNotificationJob` in `notify_job.rs`.
#[test]
fn notification_defaults_match_send_notification_jobs_job_defaults() {
    let bare = OrderShipped {
        tracking: "1Z".into(),
    };
    assert_eq!(
        bare.queue(),
        <SendNotificationJob as Job>::queue(),
        "Notification::queue()'s default must match SendNotificationJob's Job::queue()"
    );
    assert_eq!(
        bare.timeout(),
        <SendNotificationJob as Job>::timeout(),
        "Notification::timeout()'s default must match SendNotificationJob's Job::timeout()"
    );
    assert_eq!(
        bare.fail_on_timeout(),
        <SendNotificationJob as Job>::fail_on_timeout(),
        "Notification::fail_on_timeout()'s default must match SendNotificationJob's Job::fail_on_timeout()"
    );
    assert_eq!(
        bare.max_tries(),
        <SendNotificationJob as Job>::max_tries(),
        "Notification::max_tries()'s default must match SendNotificationJob's Job::max_tries()"
    );
    assert_eq!(
        bare.backoff(),
        <SendNotificationJob as Job>::backoff(),
        "Notification::backoff()'s default must match SendNotificationJob's Job::backoff()"
    );
}

// The Q12 (#61072) proof: `fail_on_timeout(&self) == true` plus a
// `timeout()` the channel exceeds dead-letters on the FIRST timeout -
// exactly one `JobFailed`, zero retries. `max_tries()` is left at its
// default (3) deliberately: if `fail_on_timeout`'s override were dropped
// on the floor, `attempts(1) < max_tries(3)` would let the job retry
// instead of dead-lettering, so this test actually exercises the
// `fail_on_timeout` wiring rather than `max_tries` exhaustion.
struct SlowChannel;

#[async_trait]
impl Channel for SlowChannel {
    fn name(&self) -> &'static str {
        "slow"
    }
    async fn deliver(
        &self,
        _route: &str,
        _notification: &dyn DynNotification,
    ) -> Result<(), FrameworkError> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(())
    }
}

struct SlowUser {
    id: i64,
}

impl Notifiable for SlowUser {
    fn route_for(&self, channel: &str) -> Option<String> {
        if channel == "slow" {
            Some(self.id.to_string())
        } else {
            None
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct SlowNotification;

impl Notification for SlowNotification {
    fn notification_name() -> &'static str {
        "SlowNotification"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["slow"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(1))
    }
    fn fail_on_timeout(&self) -> bool {
        true
    }
}

#[tokio::test]
#[serial]
async fn notify_queue_fail_on_timeout_dead_letters_on_the_first_timeout_with_zero_retries() {
    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(SlowChannel));
    let _ = suprnova::notifications::set_dispatcher(Arc::new(dispatcher));
    let _ = suprnova::notifications::register_notification_factory::<SlowNotification>();
    register_job::<SendNotificationJob>();

    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _events = EventFacade::fake();
    Notify::queue(&SlowUser { id: 1 }, SlowNotification)
        .await
        .unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    run_worker(driver.clone(), cfg, CancellationToken::new()).await;

    let timed_out = dispatched::<JobTimedOut>(|_| true);
    assert_eq!(timed_out.len(), 1, "one dispatch attempt, one timeout");
    assert_eq!(
        timed_out[0].timeout,
        Duration::from_secs(1),
        "the worker read Notification::timeout() through the envelope override"
    );

    let failed = dispatched::<JobFailed>(|_| true);
    assert_eq!(
        failed.len(),
        1,
        "fail_on_timeout(true) dead-letters on the FIRST timeout, not after \
         exhausting max_tries (left at its default of 3)"
    );

    assert_eq!(
        driver.size().await.unwrap(),
        0,
        "zero retries: a job nacked for retry would still be held by the \
         driver (delayed), not gone"
    );
}
