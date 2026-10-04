//! `MailChannel` integration tests.
//!
//! These tests share the process-global mail renderer registry, so
//! every test runs `#[serial]` and uses a unique `notification_name()`
//! to avoid colliding with prior registrations.

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use suprnova::FrameworkError;
use suprnova::mail::memory::InMemoryMailTransport;
use suprnova::mail::{Address, Attachment, Mail};
use suprnova::notifications::channels::mail::{
    MailChannel, MailRendering, NotificationMailable, register_mail_renderer,
};
use suprnova::notifications::{Notifiable, Notification, NotificationDispatcher};

struct User {
    email: String,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        if channel == "mail" {
            Some(self.email.clone())
        } else {
            None
        }
    }
}

// ============================================================================
// Test 1: happy path - registered renderer dispatches through transport
// ============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HappyPath {
    tracking: String,
}

impl Notification for HappyPath {
    fn notification_name() -> &'static str {
        "HappyPath"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("HappyPath serializes")
    }
}

impl NotificationMailable for HappyPath {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: format!("Your order shipped ({})", self.tracking),
            text: Some(format!("Tracking number: {}", self.tracking)),
            from: Some(Address::new("orders@suprnova.dev").with_name("Suprnova Orders")),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn mail_channel_dispatches_via_registered_renderer_through_in_memory_transport() {
    // Capture our own Arc to the transport before binding it globally
    // so we can read the captured-message buffer post-dispatch without
    // racing against the next test's `set_transport`.
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());

    let _ = register_mail_renderer::<HappyPath>();

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));

    let recipient = User {
        email: "alice@example.org".into(),
    };
    dispatcher
        .notify(
            &recipient,
            &HappyPath {
                tracking: "1Z999".into(),
            },
        )
        .await
        .unwrap();

    let captured = transport.captured();
    assert_eq!(captured.len(), 1, "exactly one message captured");
    let msg = &captured[0];
    assert_eq!(msg.from.email, "orders@suprnova.dev");
    assert_eq!(msg.from.name.as_deref(), Some("Suprnova Orders"));
    assert_eq!(msg.to.len(), 1);
    assert_eq!(msg.to[0].email, "alice@example.org");
    assert_eq!(msg.subject, "Your order shipped (1Z999)");
    assert_eq!(msg.text.as_deref(), Some("Tracking number: 1Z999"));
    assert!(msg.html.is_none(), "html intentionally absent");
}

// ============================================================================
// Test 2: empty-body guard fires on the rendering
// ============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
struct EmptyBody;

impl Notification for EmptyBody {
    fn notification_name() -> &'static str {
        "EmptyBody"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("EmptyBody serializes")
    }
}

impl NotificationMailable for EmptyBody {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: "ignored".into(),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn mail_channel_empty_body_guard_fires() {
    // Bind a transport so any error must come from the empty-body
    // guard, not from a missing transport. Without this binding a
    // future reorder of the guard / transport-lookup could silently
    // change which error surfaces.
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());

    let _ = register_mail_renderer::<EmptyBody>();

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));

    let err = dispatcher
        .notify(
            &User {
                email: "bob@example.org".into(),
            },
            &EmptyBody,
        )
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("MailChannel"),
        "expected MailChannel error context, got: {msg}"
    );
    assert!(
        msg.contains("EmptyBody"),
        "expected notification name in error, got: {msg}"
    );
    assert!(
        msg.contains("no html or text body"),
        "expected guard message, got: {msg}"
    );
    assert!(
        transport.captured().is_empty(),
        "guard must fire before transport.send is invoked"
    );
}

// ============================================================================
// Test 3: renderer Err propagates verbatim
// ============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RenderErr;

impl Notification for RenderErr {
    fn notification_name() -> &'static str {
        "RenderErr"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("RenderErr serializes")
    }
}

impl NotificationMailable for RenderErr {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Err(FrameworkError::internal("renderer boom"))
    }
}

#[tokio::test]
#[serial]
async fn mail_channel_renderer_error_propagates() {
    // Same precaution as above - bind a transport so the test pins
    // renderer-error propagation rather than masking it with a
    // missing-transport error.
    let _ = Mail::set_transport(Arc::new(InMemoryMailTransport::new()));

    let _ = register_mail_renderer::<RenderErr>();

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));

    let err = dispatcher
        .notify(
            &User {
                email: "carol@example.org".into(),
            },
            &RenderErr,
        )
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("renderer boom"),
        "expected renderer error to surface verbatim, got: {msg}"
    );
}

// ============================================================================
// Test 4: unregistered notification surfaces a helpful error
// ============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Unregistered;

impl Notification for Unregistered {
    fn notification_name() -> &'static str {
        "Unregistered"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("Unregistered serializes")
    }
}
// Note: NO NotificationMailable impl here - we never call to_mail on
// this type. The point is that nothing ever registers a renderer for
// the `"Unregistered"` name, so the channel must produce a helpful
// error when it tries to look one up.

#[tokio::test]
#[serial]
async fn mail_channel_errors_on_unregistered_notification() {
    // Bind a transport so we know any error is from the missing
    // renderer, not from a missing transport.
    let _ = Mail::set_transport(Arc::new(InMemoryMailTransport::new()));

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));

    let err = dispatcher
        .notify(
            &User {
                email: "dave@example.org".into(),
            },
            &Unregistered,
        )
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("no mail renderer for notification"),
        "expected missing-renderer error, got: {msg}"
    );
    assert!(
        msg.contains("Unregistered"),
        "expected notification name in error, got: {msg}"
    );
    assert!(
        msg.contains("register_mail_renderer"),
        "expected register_mail_renderer hint in error, got: {msg}"
    );
}

// ============================================================================
// Test 5: `to_mail` runs on the notification itself, not on `data()`
// ============================================================================
//
// `data()` is the public payload the database and broadcast channels
// persist; nothing requires it to round-trip the notification. A field the
// mail rendering needs but `data()` leaves out must still reach `to_mail`,
// on the synchronous path and on the queued one, where the worker rebuilds
// the whole notification from its serialized form.

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PublicDataSubset {
    order_id: i64,
    // Needed by the mail body, deliberately kept out of `data()`.
    tracking: String,
}

impl Notification for PublicDataSubset {
    fn notification_name() -> &'static str {
        "PublicDataSubset"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "order_id": self.order_id })
    }
}

impl NotificationMailable for PublicDataSubset {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: format!("Order {} shipped", self.order_id),
            text: Some(format!("Tracking number: {}", self.tracking)),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn mail_channel_renders_the_notification_itself_not_its_public_data() {
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());
    register_mail_renderer::<PublicDataSubset>().unwrap();

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));
    dispatcher
        .notify(
            &User {
                email: "eve@example.org".into(),
            },
            &PublicDataSubset {
                order_id: 42,
                tracking: "1Z999".into(),
            },
        )
        .await
        .expect("to_mail renders from the notification's own fields");

    let captured = transport.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].subject, "Order 42 shipped");
    assert_eq!(captured[0].text.as_deref(), Some("Tracking number: 1Z999"));
}

#[tokio::test]
#[serial]
async fn queued_mail_channel_renders_the_rebuilt_notification_not_its_public_data() {
    use std::collections::HashMap;
    use suprnova::notifications::notify_job::SendNotificationJob;
    use suprnova::queue::Job;

    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());
    register_mail_renderer::<PublicDataSubset>().unwrap();
    suprnova::notifications::register_notification_factory::<PublicDataSubset>().unwrap();
    let _ = suprnova::notifications::set_dispatcher(Arc::new(
        NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new())),
    ));

    // The payload `Notify::queue` stores: the whole notification.
    let job = SendNotificationJob {
        notifiable_route_per_channel: HashMap::from([(
            "mail".to_string(),
            "eve@example.org".to_string(),
        )]),
        notification_name: "PublicDataSubset".to_string(),
        notification_payload: serde_json::to_value(PublicDataSubset {
            order_id: 42,
            tracking: "1Z999".into(),
        })
        .unwrap(),
        channels: vec!["mail".to_string()],
    };
    job.handle()
        .await
        .expect("the worker renders from the rebuilt notification");

    let captured = transport.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].text.as_deref(), Some("Tracking number: 1Z999"));
}

// ============================================================================
// Test 6: re-registering for the same name is last-write-wins
// ============================================================================
//
// Two distinct concrete types share a single notification_name so the
// renderer registry is keyed identically. We register A, then B, then
// dispatch through the channel using A. Because the fn-pointer slot is
// keyed by name, A's dispatch must run B's renderer - captured by
// observing B's distinguishable subject.

#[derive(Serialize, Deserialize, Debug, Clone)]
struct LastWriteWinsA;

impl Notification for LastWriteWinsA {
    fn notification_name() -> &'static str {
        "LastWriteWinsNotif"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("LastWriteWinsA serializes")
    }
}

impl NotificationMailable for LastWriteWinsA {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: "rendered-by-A".into(),
            text: Some("body-from-A".into()),
            ..Default::default()
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct LastWriteWinsB;

impl Notification for LastWriteWinsB {
    fn notification_name() -> &'static str {
        "LastWriteWinsNotif"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("LastWriteWinsB serializes")
    }
}

impl NotificationMailable for LastWriteWinsB {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: "rendered-by-B".into(),
            text: Some("body-from-B".into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn register_mail_renderer_is_last_write_wins() {
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());

    let _ = register_mail_renderer::<LastWriteWinsA>();
    let _ = register_mail_renderer::<LastWriteWinsB>();

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));

    // Dispatch via A - but B's renderer should run because B was the
    // last write under the same notification name.
    dispatcher
        .notify(
            &User {
                email: "frank@example.org".into(),
            },
            &LastWriteWinsA,
        )
        .await
        .unwrap();

    let captured = transport.captured();
    assert_eq!(captured.len(), 1, "exactly one message captured");
    assert_eq!(
        captured[0].subject, "rendered-by-B",
        "last registered renderer (B) must win",
    );
    assert_eq!(
        captured[0].text.as_deref(),
        Some("body-from-B"),
        "last registered renderer (B) must win",
    );
}

// ============================================================================
// Test 7: cc / bcc / reply_to / attachments thread through the channel
// ============================================================================
//
// Pins the MailRendering → OutgoingMessage wiring for the optional
// fields. A future refactor that drops one of these fields when
// assembling the OutgoingMessage would fail this test.

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FullEnvelope;

impl Notification for FullEnvelope {
    fn notification_name() -> &'static str {
        "FullEnvelope"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("FullEnvelope serializes")
    }
}

impl NotificationMailable for FullEnvelope {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: "with the full envelope".into(),
            text: Some("body".into()),
            from: Some(Address::new("sender@suprnova.dev")),
            cc: vec![Address::new("carbon@example.org")],
            bcc: vec![Address::new("blind@example.org")],
            reply_to: vec![Address::new("replies@suprnova.dev")],
            attachments: vec![Attachment {
                filename: "receipt.pdf".into(),
                content: b"%PDF-1.4\nreceipt".to_vec(),
                content_type: "application/pdf".into(),
            }],
            ..Default::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn mail_channel_threads_cc_bcc_reply_to_and_attachments_into_outgoing() {
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());

    let _ = register_mail_renderer::<FullEnvelope>();

    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));

    dispatcher
        .notify(
            &User {
                email: "primary@example.org".into(),
            },
            &FullEnvelope,
        )
        .await
        .unwrap();

    let captured = transport.captured();
    assert_eq!(captured.len(), 1, "exactly one message captured");
    let msg = &captured[0];

    assert_eq!(msg.to.len(), 1, "single primary recipient");
    assert_eq!(msg.to[0].email, "primary@example.org");

    assert_eq!(msg.cc.len(), 1, "cc threaded through");
    assert_eq!(msg.cc[0].email, "carbon@example.org");

    assert_eq!(msg.bcc.len(), 1, "bcc threaded through");
    assert_eq!(msg.bcc[0].email, "blind@example.org");

    assert_eq!(msg.reply_to.len(), 1, "reply_to threaded through");
    assert_eq!(msg.reply_to[0].email, "replies@suprnova.dev");

    assert_eq!(msg.attachments.len(), 1, "attachments threaded through");
    assert_eq!(msg.attachments[0].filename, "receipt.pdf");
    assert_eq!(msg.attachments[0].content_type, "application/pdf");
    assert_eq!(msg.attachments[0].content, b"%PDF-1.4\nreceipt");
}

// ============================================================================
// Mail events: a notification sent by mail is a dispatched mail
// ============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ObservedNotice;

impl Notification for ObservedNotice {
    fn notification_name() -> &'static str {
        "ObservedNotice"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ObservedNotice serializes")
    }
}

impl NotificationMailable for ObservedNotice {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: "Observed notice".into(),
            text: Some("body".into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn mail_channel_fires_message_sending_and_message_sent() {
    // The mail manual promises both events for every successful dispatch,
    // and Laravel's mail channel sends through the mailer, which fires them.
    use suprnova::events::{EventFacade, dispatched};
    use suprnova::mail::{MessageSending, MessageSent};

    let _events = EventFacade::fake();
    let transport = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(transport.clone());
    let _ = register_mail_renderer::<ObservedNotice>();
    let dispatcher = NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new()));

    dispatcher
        .notify(
            &User {
                email: "alice@example.org".into(),
            },
            &ObservedNotice,
        )
        .await
        .unwrap();

    assert_eq!(transport.captured().len(), 1);
    assert_eq!(
        dispatched::<MessageSending>(|e| e.subject == "Observed notice").len(),
        1,
        "MessageSending fires for a notification sent by mail"
    );
    assert_eq!(
        dispatched::<MessageSent>(|e| e.subject == "Observed notice").len(),
        1,
        "MessageSent fires for a notification sent by mail"
    );
}
