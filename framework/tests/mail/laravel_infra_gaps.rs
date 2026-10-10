//! Laravel infrastructure gaps in the mail subsystem: equivalent
//! attachments and their de-duplication (PAR-145), a mailable's own
//! recipients and the `Mail` shortcuts that send it alone (PAR-146), and a
//! mailable's own queue delay (PAR-147).
//!
//! Laravel evidence: `Mail/Attachment.php:227` (`isEquivalent`),
//! `Mail/Mailable.php:1164-1172` (`attachData` keeps one per name and
//! data), `Mail/Mailable.php:709` (a mailable's own `bcc`),
//! `Mail/Mailer.php:493-496` (`onQueue`), `Support/Testing/Fakes/MailFake.php:385-388`
//! (`hasSent`) and `Mail/Mailable.php:231` (the `Delay` attribute).

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::async_trait;
use suprnova::mail::memory::InMemoryMailTransport;
use suprnova::mail::{Address, Attachment, Mail, Mailable, OutgoingMessage};
use suprnova::queue::Queue;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::worker::{WorkerConfig, run_worker};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Greeting {
    name: String,
}

#[async_trait]
impl Mailable for Greeting {
    fn mailable_name() -> &'static str {
        "Greeting"
    }
    fn subject(&self) -> String {
        format!("Hello, {}", self.name)
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Welcome aboard, {{ name }}.".into())
    }
    fn from(&self) -> Option<Address> {
        Some("hello@suprnova.dev".into())
    }
}

fn greeting() -> Greeting {
    Greeting {
        name: "Alice".into(),
    }
}

/// A mailable that carries its own report, so the merge of the mailable's
/// `attachments()` with the builder's can be observed.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ReportMail {}

#[async_trait]
impl Mailable for ReportMail {
    fn mailable_name() -> &'static str {
        "LaravelInfraGapsReportMail"
    }
    fn subject(&self) -> String {
        "Your report".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Attached.".into())
    }
    fn attachments(&self) -> Vec<Attachment> {
        vec![report()]
    }
}

fn report() -> Attachment {
    Attachment::new("r.pdf", b"A".to_vec(), "application/pdf")
}

/// Run a worker over `driver` until `capture` holds a message, then stop it.
async fn deliver_queued(driver: Arc<dyn QueueDriver>, capture: &InMemoryMailTransport) {
    let handle = tokio::spawn(run_worker(
        driver,
        WorkerConfig {
            visibility_timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(5),
            max_jobs: None,
            queues: Vec::new(),
        },
        CancellationToken::new(),
    ));
    for _ in 0..400 {
        if !capture.captured().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();
}

fn only_message(messages: Vec<OutgoingMessage>) -> OutgoingMessage {
    assert_eq!(messages.len(), 1, "exactly one message: {messages:#?}");
    messages.into_iter().next().expect("one message")
}

// ---- PAR-145: one attachment per name and bytes ----

#[tokio::test]
#[serial]
async fn the_same_attachment_attached_twice_on_the_builder_is_sent_once() {
    let fake = Mail::fake();
    let x = report();
    Mail::to("alice@example.org")
        .attach(x.clone())
        .attach(x)
        .send(greeting())
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    assert_eq!(
        msg.attachments.len(),
        1,
        "a second attachment with the same name and bytes is dropped"
    );
    assert_eq!(msg.attachments[0].content, b"A");
}

#[tokio::test]
#[serial]
async fn two_attachments_with_one_name_and_different_bytes_are_both_sent() {
    let fake = Mail::fake();
    Mail::to("alice@example.org")
        .attach(Attachment::new("r.csv", b"1,2".to_vec(), "text/csv"))
        .attach(Attachment::new("r.csv", b"3,4".to_vec(), "text/csv"))
        .send(greeting())
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    assert_eq!(
        msg.attachments.len(),
        2,
        "the name alone does not make two attachments one"
    );
}

#[tokio::test]
#[serial]
async fn the_mailables_attachment_and_the_builders_copy_are_sent_once_and_the_first_wins() {
    let fake = Mail::fake();
    // The same name and bytes, a different content type: still one, and the
    // mailable's own (the first) is the one kept.
    Mail::to("alice@example.org")
        .attach(Attachment::new(
            "r.pdf",
            b"A".to_vec(),
            "application/x-other",
        ))
        .send(ReportMail {})
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    assert_eq!(msg.attachments.len(), 1, "{:#?}", msg.attachments);
    assert_eq!(msg.attachments[0].content_type, "application/pdf");
}

#[tokio::test]
#[serial]
async fn the_queued_path_sends_the_mailables_attachment_and_the_builders_copy_once() {
    let capture = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(capture.clone());
    let _ = suprnova::mail::register_mailable_factory::<ReportMail>();
    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Mail::to("alice@example.org")
        .attach(report())
        .queue(ReportMail {})
        .await
        .unwrap();
    deliver_queued(driver, &capture).await;

    let msg = only_message(capture.captured());
    assert_eq!(
        msg.attachments.len(),
        1,
        "the worker merges the mailable's and the builder's attachments without a duplicate"
    );
    let _ = Mail::clear_transport();
}

#[test]
fn an_attachment_is_equivalent_only_with_the_same_name_bytes_and_content_type() {
    let a = report();
    assert!(a.is_equivalent(&report()));
    assert!(!a.is_equivalent(&Attachment::new("r.pdf", b"B".to_vec(), "application/pdf")));
    assert!(!a.is_equivalent(&Attachment::new("r.pdf", b"A".to_vec(), "text/plain")));
    assert!(!a.is_equivalent(&Attachment::new("x.pdf", b"A".to_vec(), "application/pdf")));

    // The given name and type stand in for the other's; its bytes still count.
    let other = Attachment::new("x.bin", b"A".to_vec(), "text/plain");
    assert!(a.is_equivalent_with(&other, "r.pdf", "application/pdf"));
    assert!(!a.is_equivalent_with(&other, "x.bin", "application/pdf"));
    assert!(!a.is_equivalent_with(&other, "r.pdf", "text/plain"));
    let other_bytes = Attachment::new("x.bin", b"B".to_vec(), "text/plain");
    assert!(!a.is_equivalent_with(&other_bytes, "r.pdf", "application/pdf"));
}

#[tokio::test]
#[serial]
async fn a_sent_message_answers_for_an_equivalent_attachment_and_attached_data() {
    let fake = Mail::fake();
    Mail::to("alice@example.org")
        .attach(report())
        .send(greeting())
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    assert!(
        msg.has_equivalent_attachment(&report()),
        "its own attachment"
    );
    assert!(
        !msg.has_equivalent_attachment(&Attachment::new("r.pdf", b"B".to_vec(), "application/pdf")),
        "other bytes under the same name"
    );
    assert!(
        !msg.has_equivalent_attachment(&Attachment::new("r.pdf", b"A".to_vec(), "text/plain")),
        "another content type"
    );
    assert!(msg.has_attached_data(b"A", "r.pdf", "application/pdf"));
    assert!(!msg.has_attached_data(b"A", "r.pdf", "text/plain"));
    assert!(!msg.has_attached_data(b"B", "r.pdf", "application/pdf"));
    assert!(
        msg.has_attachment("r.pdf"),
        "has_attachment still matches by name"
    );
}

#[tokio::test]
#[serial]
async fn attach_data_adds_an_attachment_and_the_same_data_twice_is_sent_once() {
    let fake = Mail::fake();
    Mail::to("alice@example.org")
        .attach_data(b"id,total".to_vec(), "r.csv", "text/csv")
        .attach_data(b"id,total".to_vec(), "r.csv", "text/csv")
        .send(greeting())
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    assert_eq!(msg.attachments.len(), 1, "{:#?}", msg.attachments);
    assert!(msg.has_attached_data(b"id,total", "r.csv", "text/csv"));
}

// ---- PAR-146: a mailable's own recipients ----

/// Carries only its own audit copy.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Audited {}

#[async_trait]
impl Mailable for Audited {
    fn mailable_name() -> &'static str {
        "LaravelInfraGapsAudited"
    }
    fn subject(&self) -> String {
        "Audited".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Body.".into())
    }
    fn bcc(&self) -> Vec<Address> {
        vec!["audit@example.org".into()]
    }
}

/// Carries every recipient list of its own.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Invitation {}

#[async_trait]
impl Mailable for Invitation {
    fn mailable_name() -> &'static str {
        "LaravelInfraGapsInvitation"
    }
    fn subject(&self) -> String {
        "You are invited".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Come along.".into())
    }
    fn to(&self) -> Vec<Address> {
        vec!["alice@example.org".into()]
    }
    fn cc(&self) -> Vec<Address> {
        vec!["manager@example.org".into()]
    }
    fn reply_to(&self) -> Vec<Address> {
        vec!["support@example.org".into()]
    }
}

/// Has a reply-to address and no recipient.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ReplyOnly {}

#[async_trait]
impl Mailable for ReplyOnly {
    fn mailable_name() -> &'static str {
        "LaravelInfraGapsReplyOnly"
    }
    fn subject(&self) -> String {
        "No one".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Nobody reads this.".into())
    }
    fn reply_to(&self) -> Vec<Address> {
        vec!["support@example.org".into()]
    }
}

/// Clears the `Mail::always_*` defaults when the test ends.
struct ForgetAlways;

impl Drop for ForgetAlways {
    fn drop(&mut self) {
        let _ = Mail::forget_always();
    }
}

#[tokio::test]
#[serial]
async fn a_mailables_own_bcc_reaches_a_send_and_a_queue() {
    let fake = Mail::fake();
    Mail::to("alice@example.org")
        .send(Audited {})
        .await
        .unwrap();
    Mail::to("alice@example.org")
        .queue(Audited {})
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    assert!(msg.has_to("alice@example.org"));
    assert!(msg.has_bcc("audit@example.org"), "{msg:#?}");
    let queued = fake.queued();
    assert_eq!(queued.len(), 1);
    assert!(queued[0].has_to("alice@example.org"));
    assert_eq!(queued[0].bcc, vec![Address::new("audit@example.org")]);
}

#[tokio::test]
#[serial]
async fn the_mailables_recipients_come_first_and_an_address_already_present_is_skipped() {
    let fake = Mail::fake();
    Mail::to("bob@example.org")
        .to("ALICE@example.org")
        .cc("manager@example.org")
        .send(Invitation {})
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    let to: Vec<&str> = msg.to.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(to, vec!["alice@example.org", "bob@example.org"]);
    assert_eq!(msg.cc.len(), 1, "{:#?}", msg.cc);
    assert!(msg.has_reply_to("support@example.org"));
}

#[tokio::test]
#[serial]
async fn always_to_still_replaces_every_recipient_on_the_send_path() {
    let _forget = ForgetAlways;
    let fake = Mail::fake();
    Mail::always_to("inbox@example.org").unwrap();
    Mail::to("alice@example.org")
        .send(Audited {})
        .await
        .unwrap();

    let msg = only_message(fake.captured());
    let to: Vec<&str> = msg.to.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(to, vec!["inbox@example.org"]);
    assert!(msg.bcc.is_empty(), "the mailable's bcc is replaced too");
}

#[tokio::test]
#[serial]
async fn the_worker_sends_the_mailables_recipients_once_and_always_to_replaces_them() {
    let _forget = ForgetAlways;
    let _ = suprnova::mail::register_mailable_factory::<Audited>();

    // Without always_to: the worker adds nothing the queue did not carry.
    let capture = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(capture.clone());
    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Mail::to("alice@example.org")
        .queue(Audited {})
        .await
        .unwrap();
    deliver_queued(driver, &capture).await;
    let msg = only_message(capture.captured());
    assert_eq!(
        msg.bcc,
        vec![Address::new("audit@example.org")],
        "the bcc arrives exactly once"
    );

    // With always_to: the worker replaces every recipient.
    Mail::always_to("inbox@example.org").unwrap();
    let capture = Arc::new(InMemoryMailTransport::new());
    let _ = Mail::set_transport(capture.clone());
    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Mail::to("alice@example.org")
        .queue(Audited {})
        .await
        .unwrap();
    deliver_queued(driver, &capture).await;
    let msg = only_message(capture.captured());
    let to: Vec<&str> = msg.to.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(to, vec!["inbox@example.org"]);
    assert!(msg.bcc.is_empty());
    let _ = Mail::clear_transport();
}

#[tokio::test]
#[serial]
async fn mail_send_sends_a_mailable_to_its_own_recipients() {
    let fake = Mail::fake();
    Mail::send(Invitation {}).await.unwrap();

    let msg = only_message(fake.captured());
    assert!(msg.has_to("alice@example.org"));
    assert!(msg.has_cc("manager@example.org"));
    assert!(msg.has_reply_to("support@example.org"));
}

#[tokio::test]
#[serial]
async fn the_mail_queue_shortcuts_queue_a_mailable_to_its_own_recipients() {
    let fake = Mail::fake();
    Mail::on_queue("emails", Invitation {}).await.unwrap();
    Mail::queue_on("emails", Invitation {}).await.unwrap();
    Mail::queue(Invitation {}).await.unwrap();
    Mail::later(Duration::from_secs(10), Invitation {})
        .await
        .unwrap();

    let queued = fake.queued_named("LaravelInfraGapsInvitation");
    assert_eq!(queued.len(), 4, "{queued:#?}");
    for snapshot in &queued {
        assert!(snapshot.has_to("alice@example.org"), "{snapshot:#?}");
    }
    fake.assert_queued_on("LaravelInfraGapsInvitation", "emails");
    assert_eq!(fake.queued_on("emails").len(), 2);
    assert_eq!(queued[2].queue, None);
    assert_eq!(queued[3].delay, Some(Duration::from_secs(10)));
}

#[tokio::test]
#[serial]
async fn a_mailable_with_no_recipients_is_refused_before_anything_is_sent_or_queued() {
    let fake = Mail::fake();
    let refusals = [
        Mail::send(greeting()).await.err(),
        Mail::queue(greeting()).await.err(),
        Mail::later(Duration::from_secs(5), greeting()).await.err(),
        Mail::on_queue("emails", greeting()).await.err(),
        Mail::queue_on("emails", greeting()).await.err(),
        Mail::send(ReplyOnly {}).await.err(),
    ];
    for refusal in refusals {
        let message = refusal.expect("refused").to_string();
        assert!(message.contains("recipient"), "{message}");
    }
    fake.assert_nothing_outgoing();
    drop(fake);

    // On a real queue nothing is pushed either.
    let driver: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let err = Mail::queue(greeting()).await.expect_err("refused");
    assert!(
        err.to_string().contains("Greeting"),
        "names the mailable: {err}"
    );
    assert_eq!(driver.size(None).await.unwrap(), 0);
}

#[tokio::test]
#[serial]
async fn has_sent_and_has_queued_answer_for_a_mailable_by_name() {
    let fake = Mail::fake();
    Mail::to("alice@example.org")
        .queue(greeting())
        .await
        .unwrap();
    assert!(!fake.has_sent("Greeting"), "a queued mailable is not sent");
    assert!(!fake.has_sent_mailable::<Greeting>());
    assert!(fake.has_queued("Greeting"));
    assert!(!fake.has_queued("LaravelInfraGapsInvitation"));

    Mail::to("alice@example.org")
        .send(greeting())
        .await
        .unwrap();
    assert!(fake.has_sent("Greeting"));
    assert!(fake.has_sent_mailable::<Greeting>());
    assert!(!fake.has_sent_mailable::<Invitation>());
}

// ---- PAR-147: a mailable's own delay ----

/// Waits a minute on the queue unless the caller says otherwise.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Digest {}

#[async_trait]
impl Mailable for Digest {
    fn mailable_name() -> &'static str {
        "LaravelInfraGapsDigest"
    }
    fn subject(&self) -> String {
        "Your digest".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Digest.".into())
    }
    fn to(&self) -> Vec<Address> {
        vec!["alice@example.org".into()]
    }
    fn delay(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }
}

#[tokio::test]
#[serial]
async fn a_mailables_delay_applies_to_a_queue_and_an_explicit_later_wins() {
    let fake = Mail::fake();
    Mail::to("bob@example.org").queue(Digest {}).await.unwrap();
    Mail::queue(Digest {}).await.unwrap();
    Mail::to("bob@example.org")
        .later(Duration::from_secs(10), Digest {})
        .await
        .unwrap();
    Mail::to("bob@example.org").queue(greeting()).await.unwrap();

    let delays: Vec<Option<Duration>> = fake.queued().into_iter().map(|q| q.delay).collect();
    assert_eq!(
        delays,
        vec![
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(10)),
            None,
        ]
    );
}

#[tokio::test]
#[serial]
async fn a_mailables_delay_reaches_the_queued_job() {
    let clock = suprnova::testing::TestClock::freeze();
    let _queue = Queue::fake();
    Mail::to("bob@example.org").queue(Digest {}).await.unwrap();

    let pushed = suprnova::queue::testing::pushed_with_available_at::<suprnova::SendMailJob>();
    assert_eq!(pushed.len(), 1);
    assert_eq!(
        pushed[0].1,
        clock.now() + chrono::Duration::seconds(60),
        "available a minute after the push"
    );
}
