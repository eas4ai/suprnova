use serde::{Deserialize, Serialize};
use suprnova::notifications::testing::{assert_sent_times, assert_sent_to, assert_sent_to_on};
use suprnova::{Notification, Notify};

#[derive(Debug, Serialize, Deserialize)]
struct Invoice {
    total: u64,
    private_note: String,
}
impl Notification for Invoice {
    fn notification_name() -> &'static str {
        "Invoice"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail", "slack"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({"total": self.total})
    }
}
#[derive(Debug, Serialize, Deserialize)]
struct OtherInvoice {
    total: u64,
}
impl Notification for OtherInvoice {
    fn notification_name() -> &'static str {
        "Invoice"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({"total": self.total})
    }
}

#[tokio::test]
async fn typed_queries_filter_contents_recipient_and_concrete_type() {
    let fake = Notify::fake();
    let recipient = Notify::routes([("mail", "alice"), ("slack", "alice")]).expect("routes");
    Notify::send(
        &recipient,
        &Invoice {
            total: 10,
            private_note: "full object".into(),
        },
    )
    .await
    .expect("send");
    Notify::queue(
        &recipient,
        Invoice {
            total: 20,
            private_note: "queued".into(),
        },
    )
    .await
    .expect("queue");
    Notify::send(&recipient, &OtherInvoice { total: 10 })
        .await
        .expect("other type");
    let sent = fake
        .sent::<Invoice>("alice", |_| true)
        .expect("typed records");
    assert_eq!(
        sent.len(),
        2,
        "one notification per send, even with two channels"
    );
    assert_eq!(sent[0].private_note, "full object");
    assert_eq!(sent[1].private_note, "queued");
    assert_eq!(
        fake.sent::<Invoice>("alice", |n| n.total == 10)
            .expect("filtered")
            .len(),
        1
    );
    assert!(
        fake.sent::<Invoice>("bob", |_| true)
            .expect("no recipient")
            .is_empty()
    );
    fake.assert_sent_to::<Invoice>("alice", |n| n.total == 10);
    fake.assert_not_sent_to::<Invoice>("bob");
    assert_sent_to("alice", "Invoice");
    assert_sent_to_on("alice", "mail", "Invoice");
    assert_sent_times("Invoice", 5);
}

#[tokio::test]
#[should_panic(expected = "expected no")]
async fn negative_assertion_fails_when_the_type_was_sent() {
    let fake = Notify::fake();
    let recipient = Notify::route("mail", "alice").expect("route");
    Notify::send(
        &recipient,
        &Invoice {
            total: 20,
            private_note: String::new(),
        },
    )
    .await
    .expect("send");
    fake.assert_not_sent_to::<Invoice>("alice");
}

#[tokio::test]
#[should_panic(expected = "expected at least one")]
async fn positive_assertion_fails_when_contents_do_not_match() {
    let fake = Notify::fake();
    let recipient = Notify::route("mail", "alice").expect("route");
    Notify::send(
        &recipient,
        &Invoice {
            total: 20,
            private_note: String::new(),
        },
    )
    .await
    .expect("send");
    fake.assert_sent_to::<Invoice>("alice", |n| n.total == 10);
}

#[tokio::test]
async fn typed_queries_are_empty_before_delivery_and_allow_nested_queries() {
    let fake = Notify::fake();
    assert!(
        fake.sent::<Invoice>("alice", |_| true)
            .expect("empty fake")
            .is_empty()
    );
    fake.assert_not_sent_to::<Invoice>("alice");
    let recipient = Notify::route("mail", "alice").expect("route");
    Notify::send(
        &recipient,
        &Invoice {
            total: 10,
            private_note: String::new(),
        },
    )
    .await
    .expect("send");
    let matches = fake
        .sent::<Invoice>("alice", |_| {
            assert_sent_to("alice", "Invoice");
            true
        })
        .expect("predicate runs outside the lock");
    assert_eq!(matches.len(), 1);
}

#[derive(Deserialize)]
struct CannotEncode;
impl Serialize for CannotEncode {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("scripted encoding failure"))
    }
}
impl Notification for CannotEncode {
    fn notification_name() -> &'static str {
        "CannotEncode"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

#[derive(Serialize, Deserialize)]
struct CannotDecode {
    #[serde(skip_serializing)]
    required: String,
}
impl Notification for CannotDecode {
    fn notification_name() -> &'static str {
        "CannotDecode"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({"required": self.required})
    }
}

#[tokio::test]
async fn encoding_and_decoding_failures_surface_without_hiding_records() {
    let fake = Notify::fake();
    let recipient = Notify::route("mail", "alice").expect("route");
    assert!(Notify::send(&recipient, &CannotEncode).await.is_err());
    assert!(suprnova::notifications::testing::recorded().is_empty());
    Notify::send(
        &recipient,
        &CannotDecode {
            required: "value".into(),
        },
    )
    .await
    .expect("record encoded notification");
    assert!(fake.sent::<CannotDecode>("alice", |_| true).is_err());
    assert_sent_to("alice", "CannotDecode");
    assert_eq!(suprnova::notifications::testing::recorded().len(), 1);
}
