//! The dispatch path validates every message before any transport sees it.
//!
//! Validation used to live only inside the built-in transports, so a custom
//! transport bound with `Mail::set_transport`, and the in-memory transport
//! behind `Mail::fake`, accepted what production refused. `MessageSending`
//! also fired before the transport had a chance to refuse, and `Mail::queue`
//! accepted a message the worker would then fail `max_tries` times.

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::events::{EventFacade, dispatched};
use suprnova::mail::memory::InMemoryMailTransport;
use suprnova::mail::transport::{MailTransport, OutgoingMessage};
use suprnova::mail::{Address, Mail, Mailable, MessageSending};
use suprnova::queue::Queue;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::{FrameworkError, async_trait};

const LIST_AS_EMAIL: &str = "victim@example.com, attacker@evil.example";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Notice {
    name: String,
}

#[async_trait]
impl Mailable for Notice {
    fn mailable_name() -> &'static str {
        "DispatchValidationNotice"
    }
    fn subject(&self) -> String {
        "Notice".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Hi {{ name }}".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
}

/// A transport an application might write: it trusts what it is given.
#[derive(Default)]
struct Trusting {
    calls: AtomicUsize,
}

#[async_trait]
impl MailTransport for Trusting {
    async fn send(&self, _msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_custom_transport_never_receives_a_message_the_serializer_refuses() {
    let _ = Mail::forget_always();
    let _events = EventFacade::fake();
    let transport = Arc::new(Trusting::default());
    let _ = Mail::set_transport(transport.clone());

    let err = Mail::to(LIST_AS_EMAIL)
        .send(Notice { name: "A".into() })
        .await
        .expect_err("the dispatch path must refuse a list as one address");
    assert!(format!("{err}").contains("email address"), "{err}");
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(
        dispatched::<MessageSending>(|_| true).is_empty(),
        "MessageSending fires only for a message that validated"
    );
}

#[tokio::test]
#[serial]
async fn mail_fake_refuses_what_production_refuses() {
    let _ = Mail::forget_always();
    let fake = Mail::fake();

    let err = Mail::to(LIST_AS_EMAIL)
        .send(Notice { name: "A".into() })
        .await
        .expect_err("a test under Mail::fake must see the production refusal");
    assert!(format!("{err}").contains("email address"), "{err}");
    assert_eq!(fake.count(), 0);
}

#[tokio::test]
#[serial]
async fn the_dispatch_path_hands_transports_a_normalized_message() {
    // A custom transport that reads `Address` fields directly must still
    // never see a line break in a name or whitespace around an email.
    let _ = Mail::forget_always();
    let capture = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(capture.clone());

    Mail::to(Address::new(" victim@example.com ").with_name("Vic\r\ntim"))
        .send(Notice { name: "A".into() })
        .await
        .unwrap();

    let captured = capture.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].to[0].email, "victim@example.com");
    assert_eq!(captured[0].to[0].name.as_deref(), Some("Vic tim"));
}

#[tokio::test]
#[serial]
async fn mail_queue_refuses_an_invalid_message_when_it_queues() {
    // The caller gets the error; the worker never fails the job max_tries
    // times over a message that can never be sent.
    let _ = Mail::forget_always();
    let _ = Mail::set_transport(Arc::new(InMemoryMailTransport::new()));
    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let err = Mail::to(LIST_AS_EMAIL)
        .queue(Notice { name: "A".into() })
        .await
        .expect_err("Mail::queue must validate at push time");
    assert!(format!("{err}").contains("email address"), "{err}");
    let err = Mail::to("victim@example.com")
        .header("Bcc", "attacker@evil.example")
        .later(Duration::from_secs(60), Notice { name: "A".into() })
        .await
        .expect_err("Mail::later must validate at push time");
    assert!(format!("{err}").contains("builder"), "{err}");
    assert_eq!(driver.size().await.unwrap(), 0, "nothing reached the queue");

    let fake = Mail::fake();
    assert!(
        Mail::to(LIST_AS_EMAIL)
            .queue(Notice { name: "A".into() })
            .await
            .is_err()
    );
    assert_eq!(fake.queued_count(), 0, "Mail::fake records no refused push");
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TemplatedSubject {
    name: String,
}

#[async_trait]
impl Mailable for TemplatedSubject {
    fn mailable_name() -> &'static str {
        "DispatchValidationTemplatedSubject"
    }
    fn subject(&self) -> String {
        String::new()
    }
    fn subject_template_source(&self) -> Option<String> {
        // A template file usually ends with a newline.
        Some("  Welcome, {{ name }}\n".into())
    }
    fn text_template_source(&self) -> Option<String> {
        Some("hi".into())
    }
}

#[tokio::test]
#[serial]
async fn a_templated_subject_is_trimmed_so_its_trailing_newline_is_not_refused() {
    let subject = TemplatedSubject {
        name: "Alice".into(),
    };
    assert_eq!(subject.render_subject().unwrap(), "Welcome, Alice");

    let _ = Mail::forget_always();
    let fake = Mail::fake();
    Mail::to("alice@example.org").send(subject).await.unwrap();
    assert_eq!(fake.captured()[0].subject, "Welcome, Alice");
}
