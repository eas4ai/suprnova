//! Every transport builds addresses and headers through one validating
//! serializer. These tests pin what reaches the wire.
//!
//! The defect they guard: Postmark and Mailgun received recipient lists
//! joined from `Address`'s unquoted `Display` text, so a display name such as
//! `attacker@evil.example, Victim` added the attacker as a recipient, and an
//! ordinary `Doe, Jane` split into two broken entries. SMTP and the file
//! transport wrote header names carrying CR and LF verbatim.

use std::str::FromStr;
use std::sync::Arc;
use suprnova::mail::address::Address;
use suprnova::mail::file::FileMailTransport;
use suprnova::mail::mailgun::MailgunMailTransport;
use suprnova::mail::postmark::PostmarkMailTransport;
use suprnova::mail::resend::ResendMailTransport;
use suprnova::mail::sendgrid::SendGridMailTransport;
use suprnova::mail::ses::SesMailTransport;
use suprnova::mail::transport::{MailTransport, OutgoingMessage};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const VICTIM: &str = "victim@example.com";
/// A display name with a comma, quotes, angle brackets and an address of
/// its own - everything a list parser could split on.
const HOSTILE_NAME: &str = r#"attacker@evil.example, "Victim" <x@evil.example>"#;
/// An ordinary name that the unquoted form also broke.
const ORDINARY_NAME: &str = "Doe, Jane";
const ORDINARY_WIRE: &str = r#""Doe, Jane" <jane@example.com>"#;
const FROM_WIRE: &str = r#""Suprnova, Inc." <noreply@suprnova.dev>"#;

/// The one quoted mailbox [`HOSTILE_NAME`] must become for `email`.
fn hostile_wire(email: &str) -> String {
    format!(r#""attacker@evil.example, \"Victim\" <x@evil.example>" <{email}>"#)
}

fn hostile(email: &str) -> Address {
    Address::new(email).with_name(HOSTILE_NAME)
}

fn message() -> OutgoingMessage {
    let mut msg =
        OutgoingMessage::new(Address::new("noreply@suprnova.dev").with_name("Suprnova, Inc."));
    msg.to = vec![
        hostile(VICTIM),
        Address::new("jane@example.com").with_name(ORDINARY_NAME),
    ];
    msg.cc = vec![hostile("cc@example.com")];
    msg.bcc = vec![hostile("bcc@example.com")];
    msg.reply_to = vec![hostile("reply@example.com")];
    msg.subject = "Verify your email".into();
    msg.text = Some("https://app.example/verify/secret-token".into());
    msg
}

/// Read a recipient field the way a provider does and list the mailboxes it
/// names. A field the injection split would name the attacker here too.
fn emails_in(field: &str) -> Vec<String> {
    lettre::message::Mailboxes::from_str(field)
        .unwrap_or_else(|e| panic!("{field:?} is not a mailbox list: {e:?}"))
        .into_iter()
        .map(|mailbox| mailbox.email.to_string())
        .collect()
}

async fn accepting_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "stub", "id": "stub"
        })))
        .mount(&server)
        .await;
    server
}

async fn only_json_body(server: &MockServer) -> serde_json::Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "exactly one provider request");
    serde_json::from_slice(&requests[0].body).unwrap()
}

/// One transport per HTTP provider, all pointed at `server`.
fn http_transports(server: &MockServer) -> Vec<(&'static str, Arc<dyn MailTransport>)> {
    let uri = server.uri();
    vec![
        (
            "Postmark",
            Arc::new(PostmarkMailTransport::with_endpoint("token", &uri)),
        ),
        (
            "Mailgun",
            Arc::new(MailgunMailTransport::with_endpoint(
                "key",
                "example.com",
                &uri,
            )),
        ),
        (
            "SES",
            Arc::new(SesMailTransport::with_endpoint(
                "AKIATEST",
                "secret",
                "us-east-1",
                &uri,
            )),
        ),
        (
            "Resend",
            Arc::new(ResendMailTransport::with_endpoint("key", &uri)),
        ),
        (
            "SendGrid",
            Arc::new(SendGridMailTransport::with_endpoint("key", &uri)),
        ),
    ]
}

/// Send `msg` through every HTTP provider and assert each one refuses it
/// with an error naming the provider and containing `expect`, before any
/// request leaves the process.
async fn assert_every_provider_refuses(msg: &OutgoingMessage, expect: &str) {
    let server = accepting_server().await;
    for (provider, transport) in http_transports(&server) {
        let err = transport
            .send(msg)
            .await
            .expect_err(&format!("{provider} must refuse the message"));
        let text = format!("{err}");
        assert!(
            text.contains(provider) && text.contains(expect),
            "{provider}: the error must name the provider and the field: {text}"
        );
    }
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a refused message must not reach any provider"
    );
}

#[tokio::test]
async fn postmark_sends_every_display_name_as_one_quoted_mailbox() {
    let server = accepting_server().await;
    PostmarkMailTransport::with_endpoint("token", server.uri())
        .send(&message())
        .await
        .unwrap();

    let body = only_json_body(&server).await;
    assert_eq!(
        body["To"],
        format!("{}, {ORDINARY_WIRE}", hostile_wire(VICTIM))
    );
    assert_eq!(
        emails_in(body["To"].as_str().unwrap()),
        [VICTIM, "jane@example.com"]
    );
    for (field, email) in [
        ("Cc", "cc@example.com"),
        ("Bcc", "bcc@example.com"),
        ("ReplyTo", "reply@example.com"),
    ] {
        assert_eq!(body[field], hostile_wire(email), "{field}");
        assert_eq!(emails_in(body[field].as_str().unwrap()), [email], "{field}");
    }
    assert_eq!(body["From"], FROM_WIRE);
}

#[tokio::test]
async fn mailgun_sends_every_display_name_as_one_quoted_mailbox() {
    let server = accepting_server().await;
    MailgunMailTransport::with_endpoint("key", "example.com", server.uri())
        .send(&message())
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let form: Vec<(String, String)> = url::form_urlencoded::parse(&requests[0].body)
        .into_owned()
        .collect();
    let field = |name: &str| -> Vec<&str> {
        form.iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    };

    let to = format!("{}, {ORDINARY_WIRE}", hostile_wire(VICTIM));
    assert_eq!(field("to"), [to.as_str()]);
    assert_eq!(emails_in(&to), [VICTIM, "jane@example.com"]);
    for (name, email) in [
        ("cc", "cc@example.com"),
        ("bcc", "bcc@example.com"),
        ("h:Reply-To", "reply@example.com"),
    ] {
        let expected = hostile_wire(email);
        assert_eq!(field(name), [expected.as_str()], "{name}");
        assert_eq!(emails_in(&expected), [email], "{name}");
    }
    assert_eq!(field("from"), [FROM_WIRE]);
}

#[tokio::test]
async fn resend_sends_every_display_name_as_one_quoted_mailbox() {
    let server = accepting_server().await;
    ResendMailTransport::with_endpoint("key", server.uri())
        .send(&message())
        .await
        .unwrap();

    let body = only_json_body(&server).await;
    assert_eq!(
        body["to"],
        serde_json::json!([hostile_wire(VICTIM), ORDINARY_WIRE])
    );
    for (field, email) in [
        ("cc", "cc@example.com"),
        ("bcc", "bcc@example.com"),
        ("reply_to", "reply@example.com"),
    ] {
        assert_eq!(
            body[field],
            serde_json::json!([hostile_wire(email)]),
            "{field}"
        );
    }
    assert_eq!(body["from"], FROM_WIRE);
}

#[tokio::test]
async fn ses_sends_every_display_name_as_one_quoted_mailbox() {
    let server = accepting_server().await;
    SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri())
        .send(&message())
        .await
        .unwrap();

    let body = only_json_body(&server).await;
    let destination = &body["Destination"];
    assert_eq!(
        destination["ToAddresses"],
        serde_json::json!([hostile_wire(VICTIM), ORDINARY_WIRE])
    );
    assert_eq!(
        destination["CcAddresses"],
        serde_json::json!([hostile_wire("cc@example.com")])
    );
    assert_eq!(
        destination["BccAddresses"],
        serde_json::json!([hostile_wire("bcc@example.com")])
    );
    assert_eq!(
        body["ReplyToAddresses"],
        serde_json::json!([hostile_wire("reply@example.com")])
    );
    assert_eq!(body["FromEmailAddress"], FROM_WIRE);
}

#[tokio::test]
async fn sendgrid_sends_every_display_name_in_its_own_name_field() {
    // SendGrid takes structured `{email, name}` objects, so the name never
    // shares a string with the address. This pins that it stays that way.
    let server = accepting_server().await;
    SendGridMailTransport::with_endpoint("key", server.uri())
        .send(&message())
        .await
        .unwrap();

    let body = only_json_body(&server).await;
    let personalization = &body["personalizations"][0];
    assert_eq!(
        personalization["to"],
        serde_json::json!([
            { "email": VICTIM, "name": HOSTILE_NAME },
            { "email": "jane@example.com", "name": ORDINARY_NAME },
        ])
    );
    assert_eq!(
        personalization["cc"],
        serde_json::json!([{ "email": "cc@example.com", "name": HOSTILE_NAME }])
    );
    assert_eq!(
        personalization["bcc"],
        serde_json::json!([{ "email": "bcc@example.com", "name": HOSTILE_NAME }])
    );
    assert_eq!(
        body["reply_to"],
        serde_json::json!({ "email": "reply@example.com", "name": HOSTILE_NAME })
    );
}

#[tokio::test]
async fn every_provider_refuses_a_display_name_with_a_line_break() {
    for name in [
        "Victim\r\nBcc: attacker@evil.example",
        "Victim\nX",
        "Vic\0tim",
    ] {
        let mut msg = message();
        msg.to = vec![Address::new(VICTIM).with_name(name)];
        assert_every_provider_refuses(&msg, "display name").await;
    }
}

#[tokio::test]
async fn every_provider_refuses_an_email_that_is_not_one_address() {
    for email in [
        "victim@example.com, attacker@evil.example",
        "Victim <victim@example.com>",
    ] {
        let mut msg = message();
        msg.to = vec![Address::new(email)];
        assert_every_provider_refuses(&msg, "email address").await;
    }
}

#[tokio::test]
async fn every_provider_refuses_a_header_that_carries_a_line_break() {
    for (name, value) in [
        ("X-Bad\r\nReply-To", "attacker@evil.example"),
        ("X-Bad\0Header", "value"),
        ("X-Fine", "value\r\nBcc: attacker@evil.example"),
    ] {
        let mut msg = message();
        msg.headers = vec![(name.into(), value.into())];
        assert_every_provider_refuses(&msg, "header").await;
    }
}

#[tokio::test]
async fn resend_refuses_a_metadata_key_that_would_inject_a_header() {
    // Resend has no metadata field, so metadata rides as `X-Metadata-<key>`
    // headers, and the key becomes part of a header name.
    let server = accepting_server().await;
    let mut msg = message();
    msg.metadata
        .insert("ref\r\nReply-To".into(), "attacker@evil.example".into());
    let err = ResendMailTransport::with_endpoint("key", server.uri())
        .send(&msg)
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("header name"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// The file transport writes the bytes SMTP would send, so it must refuse
/// what SMTP refuses and leave no `.eml` behind.
async fn assert_file_transport_refuses(msg: &OutgoingMessage) {
    let dir = tempfile::tempdir().unwrap();
    let err = FileMailTransport::new(dir.path())
        .send(msg)
        .await
        .expect_err("the file transport must refuse the message");
    assert!(format!("{err}").contains("header"), "{err}");
    let written = std::fs::read_dir(dir.path())
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "a refused message must not be written");
}

#[tokio::test]
async fn the_file_transport_refuses_a_header_name_with_a_line_break() {
    let mut msg = message();
    msg.headers = vec![("X-A\r\nReply-To".into(), "attacker@evil.example".into())];
    assert_file_transport_refuses(&msg).await;
}

#[tokio::test]
async fn the_file_transport_refuses_a_metadata_key_with_a_line_break() {
    let mut msg = message();
    msg.metadata
        .insert("k\r\nReply-To".into(), "attacker@evil.example".into());
    assert_file_transport_refuses(&msg).await;
}

#[tokio::test]
async fn the_file_transport_refuses_a_header_value_with_a_line_break() {
    let mut msg = message();
    msg.headers = vec![("X-A".into(), "a\r\nReply-To: attacker@evil.example".into())];
    assert_file_transport_refuses(&msg).await;
}

#[tokio::test]
async fn the_file_transport_quotes_a_display_name_into_one_mailbox() {
    let dir = tempfile::tempdir().unwrap();
    FileMailTransport::new(dir.path())
        .send(&message())
        .await
        .unwrap();
    let path = std::fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let raw = std::fs::read_to_string(path).unwrap();
    // Unfold the header block: a long header continues on lines that start
    // with whitespace.
    let mut headers: Vec<String> = Vec::new();
    for line in raw.lines().take_while(|line| !line.is_empty()) {
        match headers.last_mut() {
            Some(last) if line.starts_with([' ', '\t']) => last.push_str(line),
            _ => headers.push(line.to_string()),
        }
    }
    let to = headers
        .iter()
        .find_map(|header| header.strip_prefix("To: "))
        .unwrap_or_else(|| panic!("no To header:\n{raw}"));
    assert_eq!(emails_in(to), [VICTIM, "jane@example.com"], "{raw}");
}
