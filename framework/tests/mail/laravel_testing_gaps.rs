//! Laravel testing gaps in the mail fake: queued mailables read back as their
//! own type (PAR-178).
//!
//! Laravel evidence: `Support/Testing/Fakes/MailFake.php` (`queued($mailable,
//! $callback)` takes a class and a closure, and `assertQueued` /
//! `assertNotQueued` assert on what it returns).

use serde::{Deserialize, Serialize};
use serial_test::serial;
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Welcome {
    name: String,
}

#[async_trait]
impl Mailable for Welcome {
    fn mailable_name() -> &'static str {
        "LaravelTestingGapsWelcome"
    }
    fn subject(&self) -> String {
        format!("Welcome, {}", self.name)
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Hello {{ name }}.".into())
    }
    fn to(&self) -> Vec<Address> {
        vec![Address::new("alice@example.org")]
    }
}

/// A second mailable type, so a typed read can be seen to leave it out.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Receipt {
    order: u32,
}

#[async_trait]
impl Mailable for Receipt {
    fn mailable_name() -> &'static str {
        "LaravelTestingGapsReceipt"
    }
    fn subject(&self) -> String {
        format!("Order {}", self.order)
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Thanks for order {{ order }}.".into())
    }
    fn to(&self) -> Vec<Address> {
        vec![Address::new("alice@example.org")]
    }
}

/// A mailable whose payload never rebuilds: `token` is left out of the
/// serialized form but is required to deserialize it.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Lossy {
    name: String,
    #[serde(skip_serializing)]
    token: String,
}

#[async_trait]
impl Mailable for Lossy {
    fn mailable_name() -> &'static str {
        "LaravelTestingGapsLossy"
    }
    fn subject(&self) -> String {
        format!("Your code is {}", self.token)
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Hello {{ name }}.".into())
    }
    fn to(&self) -> Vec<Address> {
        vec![Address::new("alice@example.org")]
    }
}

fn welcome(name: &str) -> Welcome {
    Welcome { name: name.into() }
}

/// The message of the panic `result` holds. Fails the test when `result` did
/// not panic.
fn panic_message(result: std::thread::Result<()>) -> String {
    let payload = match result {
        Ok(()) => panic!("expected the call to panic"),
        Err(payload) => payload,
    };
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    String::from("<a panic without a message>")
}

#[tokio::test]
#[serial]
async fn queued_of_rebuilds_each_queued_mailable_of_the_type() {
    let fake = Mail::fake();
    Mail::queue(welcome("a")).await.unwrap();
    Mail::queue(Receipt { order: 7 }).await.unwrap();

    assert_eq!(fake.queued_of::<Welcome>(), vec![welcome("a")]);
    assert_eq!(fake.queued_of::<Receipt>(), vec![Receipt { order: 7 }]);
    assert_eq!(
        fake.try_queued_of::<Welcome>().unwrap(),
        vec![welcome("a")],
        "the fallible form returns the same mailables"
    );
}

#[tokio::test]
#[serial]
async fn queued_of_is_empty_when_only_a_send_happened() {
    let fake = Mail::fake();
    Mail::send(welcome("sent")).await.unwrap();

    assert!(fake.queued_of::<Welcome>().is_empty());
    assert!(fake.try_queued_of::<Welcome>().unwrap().is_empty());
}

#[tokio::test]
#[serial]
async fn queued_where_returns_only_the_mailables_the_filter_accepts() {
    let fake = Mail::fake();
    Mail::queue(welcome("a")).await.unwrap();
    Mail::queue(welcome("c")).await.unwrap();

    assert!(fake.queued_where::<Welcome>(|m| m.name == "b").is_empty());
    assert_eq!(
        fake.queued_where::<Welcome>(|m| m.name == "a"),
        vec![welcome("a")]
    );
    assert_eq!(
        fake.try_queued_where::<Welcome>(|m| m.name == "c").unwrap(),
        vec![welcome("c")]
    );
}

#[tokio::test]
#[serial]
async fn assert_queued_mailable_passes_on_a_match_and_names_the_mailable_otherwise() {
    let fake = Mail::fake();
    Mail::queue(welcome("a")).await.unwrap();

    fake.assert_queued_mailable::<Welcome>(|m| m.name == "a");

    let message = panic_message(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || fake.assert_queued_mailable::<Welcome>(|m| m.name == "b"),
    )));
    assert!(
        message.contains("LaravelTestingGapsWelcome"),
        "the failure names the mailable: {message}"
    );
}

#[tokio::test]
#[serial]
async fn assert_not_queued_mailable_fails_when_a_queued_mailable_matches() {
    let fake = Mail::fake();
    fake.assert_not_queued_mailable::<Welcome>(|_| true);

    Mail::queue(welcome("a")).await.unwrap();
    fake.assert_not_queued_mailable::<Welcome>(|m| m.name == "b");
    fake.assert_not_queued_mailable::<Receipt>(|_| true);

    let message = panic_message(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || fake.assert_not_queued_mailable::<Welcome>(|_| true),
    )));
    assert!(
        message.contains("LaravelTestingGapsWelcome"),
        "the failure names the mailable: {message}"
    );
}

#[tokio::test]
#[serial]
async fn a_payload_that_does_not_rebuild_is_reported_not_skipped() {
    let fake = Mail::fake();
    Mail::queue(Lossy {
        name: "a".into(),
        token: "t".into(),
    })
    .await
    .unwrap();

    let error = fake
        .try_queued_of::<Lossy>()
        .expect_err("a payload that does not rebuild is an error")
        .to_string();
    assert!(error.contains("LaravelTestingGapsLossy"), "{error}");
    assert!(error.contains("token"), "names the decode error: {error}");

    let message = panic_message(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || {
            fake.queued_of::<Lossy>();
        },
    )));
    assert!(message.contains("LaravelTestingGapsLossy"), "{message}");
    assert!(message.contains("token"), "{message}");

    // A negative assertion does not pass over a mailable it cannot read.
    let message = panic_message(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || fake.assert_not_queued_mailable::<Lossy>(|_| false),
    )));
    assert!(message.contains("token"), "{message}");
}

#[tokio::test]
#[serial]
async fn queued_still_returns_the_snapshots() {
    let fake = Mail::fake();
    Mail::queue(welcome("a")).await.unwrap();

    let snapshots = fake.queued();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].mailable_name, "LaravelTestingGapsWelcome");
    assert_eq!(snapshots[0].payload["name"], "a");
}
