//! Laravel infrastructure gaps in notifications: a notification's own queue
//! delay per channel (PAR-147), the broadcast channel's event, message and
//! queued publish (PAR-148), and notification mail attachments by path and
//! the subject a mail without one falls back to (PAR-149).
//!
//! Laravel evidence: `Notifications/NotificationSender.php:259`
//! (`withDelay` per channel), `Notifications/Channels/BroadcastChannel.php:37-51`,
//! `Notifications/Messages/MailMessage.php:266-299` (`attach`,
//! `attachMany`) and `Notifications/Channels/MailChannel.php:178-180` (the
//! class basename in title case as the subject).

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::broadcasting::{BroadcastHub, InMemoryBroadcastHub};
use suprnova::events::Listener;
use suprnova::mail::memory::InMemoryMailTransport;
use suprnova::mail::{Attachment, Mail};
use suprnova::notifications::channels::broadcast::{
    BroadcastChannel, BroadcastMessage, BroadcastNotificationJob, NotificationBroadcast,
    register_broadcast_renderer,
};
use suprnova::notifications::channels::mail::{
    AttachOptions, MailChannel, MailRendering, NotificationMailable, register_mail_renderer,
};
use suprnova::notifications::notify_job::SendNotificationJob;
use suprnova::notifications::{BroadcastNotificationCreated, Channel, Notifiable, Notification};
use suprnova::queue::testing::{
    assert_pushed_on_connection, assert_pushed_on_queue, pushed, pushed_with_available_at,
};
use suprnova::testing::{TestClock, TestContainer};
use suprnova::{EventFacade, FrameworkError, Job, Notify, Queue, async_trait};

// ---- PAR-149: the subject a mail without one falls back to ----

#[derive(Serialize, Deserialize, Debug, Clone)]
struct InvoicePaid;

impl Notification for InvoicePaid {
    fn notification_name() -> &'static str {
        "InvoicePaid"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

impl NotificationMailable for InvoicePaid {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            text: Some("Paid.".into()),
            ..Default::default()
        })
    }
}

/// A namespaced name whose last segment mixes `_`, `-` and `.`.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ReminderSent;

impl Notification for ReminderSent {
    fn notification_name() -> &'static str {
        "Billing::invoice-reminder_sent.v2"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

impl NotificationMailable for ReminderSent {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            text: Some("Reminder.".into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn a_notification_mail_without_a_subject_is_sent_with_its_name_in_title_case() {
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());
    register_mail_renderer::<InvoicePaid>().unwrap();
    register_mail_renderer::<ReminderSent>().unwrap();

    let channel = MailChannel::new();
    channel
        .deliver("alice@example.org", &InvoicePaid)
        .await
        .expect("delivers");
    channel
        .deliver("alice@example.org", &ReminderSent)
        .await
        .expect("delivers");

    let subjects: Vec<String> = transport
        .captured()
        .into_iter()
        .map(|m| m.subject)
        .collect();
    assert_eq!(
        subjects,
        vec![
            "Invoice Paid".to_string(),
            "Invoice Reminder Sent V2".to_string()
        ]
    );
    let _ = Mail::clear_transport();
}

// ---- PAR-149: attachments by path, read at delivery ----

/// Attaches two files by path, named by the test.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Statement {
    first: String,
    second: String,
}

impl Notification for Statement {
    fn notification_name() -> &'static str {
        "LaravelInfraGapsStatement"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

impl NotificationMailable for Statement {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: "Your statement".into(),
            text: Some("Attached.".into()),
            ..Default::default()
        }
        .attach_many([
            (self.first.clone(), AttachOptions::default()),
            (
                self.second.clone(),
                AttachOptions {
                    name: Some("report.pdf".into()),
                    ..Default::default()
                },
            ),
        ]))
    }
}

/// Attaches in memory and by path, with a content type given and one
/// left to an extension no table knows.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Mixed {
    path: String,
}

impl Notification for Mixed {
    fn notification_name() -> &'static str {
        "LaravelInfraGapsMixed"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

impl NotificationMailable for Mixed {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: "Mixed".into(),
            text: Some("Mixed.".into()),
            ..Default::default()
        }
        .attach(Attachment::new("a.csv", b"1,2".to_vec(), "text/csv"))
        .attach_data(b"raw".to_vec(), "raw.bin", "application/x-raw")
        .attach_path(self.path.clone(), AttachOptions::default())
        .attach_path(
            self.path.clone(),
            AttachOptions {
                name: Some("typed.data".into()),
                content_type: Some("application/vnd.example".into()),
            },
        ))
    }
}

#[tokio::test]
#[serial]
async fn files_attached_by_path_are_read_at_delivery_with_their_names_and_types() {
    let dir = tempfile::tempdir().expect("temp dir");
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.pdf");
    std::fs::write(&a, b"alpha").unwrap();
    std::fs::write(&b, b"%PDF-1.7 beta").unwrap();
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());
    register_mail_renderer::<Statement>().unwrap();

    MailChannel::new()
        .deliver(
            "alice@example.org",
            &Statement {
                first: a.display().to_string(),
                second: b.display().to_string(),
            },
        )
        .await
        .expect("delivers");

    let captured = transport.captured();
    assert_eq!(captured.len(), 1);
    let msg = &captured[0];
    assert!(
        msg.has_attached_data(b"alpha", "a.txt", "text/plain"),
        "{:#?}",
        msg.attachments
    );
    assert!(
        msg.has_attached_data(b"%PDF-1.7 beta", "report.pdf", "application/pdf"),
        "{:#?}",
        msg.attachments
    );
    assert_eq!(msg.attachments.len(), 2);
    let _ = Mail::clear_transport();
}

#[tokio::test]
#[serial]
async fn attach_and_attach_data_add_in_memory_and_options_override_name_and_type() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("blob.unknownext");
    std::fs::write(&path, b"blob").unwrap();
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());
    register_mail_renderer::<Mixed>().unwrap();

    MailChannel::new()
        .deliver(
            "alice@example.org",
            &Mixed {
                path: path.display().to_string(),
            },
        )
        .await
        .expect("delivers");

    let msg = &transport.captured()[0];
    assert!(msg.has_attached_data(b"1,2", "a.csv", "text/csv"));
    assert!(msg.has_attached_data(b"raw", "raw.bin", "application/x-raw"));
    assert!(
        msg.has_attached_data(b"blob", "blob.unknownext", "application/octet-stream"),
        "an extension no table knows is application/octet-stream: {:#?}",
        msg.attachments
    );
    assert!(msg.has_attached_data(b"blob", "typed.data", "application/vnd.example"));
    let _ = Mail::clear_transport();
}

#[test]
fn attach_path_records_the_path_and_reads_nothing() {
    let rendering =
        MailRendering::default().attach_path("/no/such/file.txt", AttachOptions::default());
    assert!(rendering.attachments.is_empty(), "nothing is read yet");
    assert_eq!(rendering.attachment_paths.len(), 1);
    assert_eq!(
        rendering.attachment_paths[0].path,
        std::path::PathBuf::from("/no/such/file.txt")
    );
}

#[tokio::test]
#[serial]
async fn a_file_that_cannot_be_read_fails_the_delivery_naming_the_path() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("gone.pdf");
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());
    register_mail_renderer::<Statement>().unwrap();
    let present = dir.path().join("here.txt");
    std::fs::write(&present, b"here").unwrap();

    let err = MailChannel::new()
        .deliver(
            "alice@example.org",
            &Statement {
                first: present.display().to_string(),
                second: missing.display().to_string(),
            },
        )
        .await
        .expect_err("a missing file fails the delivery");
    assert!(
        err.to_string().contains(&missing.display().to_string()),
        "the error names the path: {err}"
    );
    assert!(transport.captured().is_empty(), "nothing was sent");
    let _ = Mail::clear_transport();
}

// ---- PAR-147: a notification's delay per channel ----

/// Delays its mail by thirty seconds and nothing else.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct DelayedNote;

impl Notification for DelayedNote {
    fn notification_name() -> &'static str {
        "LaravelInfraGapsDelayedNote"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail", "database"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    fn delay(&self, channel: &str) -> Option<Duration> {
        (channel == "mail").then_some(Duration::from_secs(30))
    }
}

struct Customer;

impl Notifiable for Customer {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "mail" => Some("alice@example.org".into()),
            "database" => Some("7".into()),
            _ => None,
        }
    }
}

#[tokio::test]
#[serial]
async fn notify_queue_delays_each_channel_by_the_notifications_delay_for_it() {
    let clock = TestClock::freeze();
    let _queue = Queue::fake();
    Notify::queue(&Customer, DelayedNote).await.unwrap();

    let pushed = pushed_with_available_at::<SendNotificationJob>();
    assert_eq!(pushed.len(), 2, "one job per channel");
    let at = |channel: &str| {
        pushed
            .iter()
            .find(|(job, _)| job.channels == vec![channel.to_string()])
            .map(|(_, at)| *at)
            .expect("a job for the channel")
    };
    assert_eq!(at("mail"), clock.now() + chrono::Duration::seconds(30));
    assert_eq!(at("database"), clock.now(), "no delay for the database");
}

// ---- PAR-148: the broadcast channel's event, message and queued publish ----

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PingNote;

impl Notification for PingNote {
    fn notification_name() -> &'static str {
        "PingNote"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["broadcast"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "title": "ping" })
    }
}

/// Broadcasts through the queue named `broadcasts`.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct QueuedNote;

impl Notification for QueuedNote {
    fn notification_name() -> &'static str {
        "LaravelInfraGapsQueuedNote"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["broadcast"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "from": "data" })
    }
}

impl NotificationBroadcast for QueuedNote {
    fn to_broadcast(&self) -> BroadcastMessage {
        BroadcastMessage::new(serde_json::json!({ "id": 7 })).on_queue("broadcasts")
    }
}

/// Broadcasts through the connection named `redis`.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ConnectionNote;

impl Notification for ConnectionNote {
    fn notification_name() -> &'static str {
        "LaravelInfraGapsConnectionNote"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["broadcast"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

impl NotificationBroadcast for ConnectionNote {
    fn to_broadcast(&self) -> BroadcastMessage {
        BroadcastMessage::new(serde_json::json!({ "id": 8 })).on_connection("redis")
    }
}

/// Supplies its own data and names neither a queue nor a connection.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct AtOnceNote;

impl Notification for AtOnceNote {
    fn notification_name() -> &'static str {
        "LaravelInfraGapsAtOnceNote"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["broadcast"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "from": "data" })
    }
}

impl NotificationBroadcast for AtOnceNote {
    fn to_broadcast(&self) -> BroadcastMessage {
        BroadcastMessage::new(serde_json::json!({ "from": "to_broadcast" }))
    }
}

/// Records every `BroadcastNotificationCreated` it hears.
#[derive(Default)]
struct Heard(Mutex<Vec<BroadcastNotificationCreated>>);

#[async_trait]
impl Listener<BroadcastNotificationCreated> for Heard {
    async fn handle(&self, event: &BroadcastNotificationCreated) -> Result<(), FrameworkError> {
        self.0.lock().unwrap().push(event.clone());
        Ok(())
    }
}

/// Forgets the `BroadcastNotificationCreated` listeners when the test ends.
struct ForgetCreatedListeners;

impl Drop for ForgetCreatedListeners {
    fn drop(&mut self) {
        EventFacade::forget::<BroadcastNotificationCreated>();
    }
}

async fn listen_for_created() -> (Arc<Heard>, ForgetCreatedListeners) {
    let heard = Arc::new(Heard::default());
    EventFacade::listen::<BroadcastNotificationCreated, _>(heard.clone()).await;
    (heard, ForgetCreatedListeners)
}

#[tokio::test]
#[serial]
async fn broadcasting_a_notification_dispatches_broadcast_notification_created() {
    let (heard, _forget) = listen_for_created().await;
    TestContainer::scope(async {
        let hub = Arc::new(InMemoryBroadcastHub::new());
        let mut rx = hub.subscribe("room.lobby");
        TestContainer::bind::<dyn BroadcastHub>(hub.clone());

        BroadcastChannel::new()
            .deliver("room.lobby", &PingNote)
            .await
            .expect("delivers");

        let env = rx.try_recv().expect("published at once");
        assert_eq!(env.event, "PingNote");
        assert_eq!(
            env.data,
            serde_json::json!({ "title": "ping" }),
            "data() stays the payload"
        );
    })
    .await;

    let heard = heard.0.lock().unwrap().clone();
    assert_eq!(heard.len(), 1, "the listener ran once: {heard:#?}");
    assert_eq!(heard[0].notification, "PingNote");
    assert_eq!(heard[0].route, "room.lobby");
    assert_eq!(heard[0].data, serde_json::json!({ "title": "ping" }));
    assert_eq!(heard[0].queue, None);
    assert_eq!(heard[0].connection, None);
}

#[tokio::test]
#[serial]
async fn a_message_with_a_queue_is_published_by_a_job_on_that_queue_not_at_once() {
    register_broadcast_renderer::<QueuedNote>().unwrap();
    let (heard, _forget) = listen_for_created().await;
    let _queue = Queue::fake();
    TestContainer::scope(async {
        let hub = Arc::new(InMemoryBroadcastHub::new());
        let mut rx = hub.subscribe("orders.7");
        TestContainer::bind::<dyn BroadcastHub>(hub.clone());

        BroadcastChannel::new()
            .deliver("orders.7", &QueuedNote)
            .await
            .expect("delivers");

        assert!(rx.try_recv().is_err(), "nothing reaches the hub at once");
    })
    .await;

    assert_pushed_on_queue::<BroadcastNotificationJob>("broadcasts", |job| {
        job.channel == "orders.7"
            && job.event == "LaravelInfraGapsQueuedNote"
            && job.data == serde_json::json!({ "id": 7 })
    });
    let heard = heard.0.lock().unwrap().clone();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].queue.as_deref(), Some("broadcasts"));
    assert_eq!(heard[0].data, serde_json::json!({ "id": 7 }));
}

#[tokio::test]
#[serial]
async fn a_message_with_a_connection_is_pushed_to_that_connection_even_with_no_hub_bound() {
    register_broadcast_renderer::<ConnectionNote>().unwrap();
    let _queue = Queue::fake();
    TestContainer::scope(async {
        // No hub here: a queued message needs one on the worker, not now.
        BroadcastChannel::new()
            .deliver("orders.8", &ConnectionNote)
            .await
            .expect("a queued message does not need the hub at delivery");
    })
    .await;
    assert_pushed_on_connection::<BroadcastNotificationJob>("redis");
    assert_eq!(pushed::<BroadcastNotificationJob>().len(), 1);
}

#[tokio::test]
#[serial]
async fn the_queued_job_publishes_on_the_worker_and_fails_without_a_hub() {
    let job = BroadcastNotificationJob {
        channel: "orders.9".into(),
        event: "LaravelInfraGapsQueuedNote".into(),
        data: serde_json::json!({ "id": 9 }),
    };
    TestContainer::scope(async {
        let hub = Arc::new(InMemoryBroadcastHub::new());
        let mut rx = hub.subscribe("orders.9");
        TestContainer::bind::<dyn BroadcastHub>(hub.clone());
        job.clone().handle().await.expect("publishes");
        let env = rx.try_recv().expect("the worker published");
        assert_eq!(env.event, "LaravelInfraGapsQueuedNote");
        assert_eq!(env.data, serde_json::json!({ "id": 9 }));
    })
    .await;
    TestContainer::scope(async {
        let err = job.handle().await.expect_err("no hub, no publish");
        assert!(err.to_string().contains("BroadcastHub"), "{err}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_message_with_neither_publishes_its_own_data_at_once_and_fails_without_a_hub() {
    register_broadcast_renderer::<AtOnceNote>().unwrap();
    TestContainer::scope(async {
        let hub = Arc::new(InMemoryBroadcastHub::new());
        let mut rx = hub.subscribe("feed");
        TestContainer::bind::<dyn BroadcastHub>(hub.clone());
        BroadcastChannel::new()
            .deliver("feed", &AtOnceNote)
            .await
            .expect("delivers");
        let env = rx.try_recv().expect("published at once");
        assert_eq!(env.data, serde_json::json!({ "from": "to_broadcast" }));
    })
    .await;
    TestContainer::scope(async {
        let err = BroadcastChannel::new()
            .deliver("feed", &AtOnceNote)
            .await
            .expect_err("no hub bound fails the delivery");
        assert!(err.to_string().contains("BroadcastHub"), "{err}");
    })
    .await;
}
