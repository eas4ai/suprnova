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
use suprnova::FrameworkError;
use suprnova::mail::address::{Address, Attachment};
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

/// A fake provider that accepts every request.
///
/// It answers `Connection: close`. The mail transports share one pooled
/// HTTP client, and these tests start and drop many servers on ephemeral
/// ports; a keep-alive connection left in the pool could be reused against
/// a later server that happened to get the same port, and fail that test.
async fn accepting_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_json(serde_json::json!({
                    "MessageId": "stub", "id": "stub"
                })),
        )
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
async fn every_provider_refuses_a_display_name_with_a_control_character() {
    // A line break in a name is collapsed (see below); any other control
    // character has no meaning in a name and is refused.
    for name in ["Vic\0tim", "Vic\x01tim", "Vic\x7ftim"] {
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

// ---------------------------------------------------------------------------
// Probes: send one message through every HTTP provider and read what each
// one received.
// ---------------------------------------------------------------------------

/// One provider's outcome: the result `send` returned and the requests the
/// fake provider saw.
struct Sent {
    provider: &'static str,
    result: Result<(), FrameworkError>,
    requests: Vec<wiremock::Request>,
}

/// Send `msg` through each HTTP provider, each against its own fake server.
async fn through_every_provider(msg: &OutgoingMessage) -> Vec<Sent> {
    let mut sent = Vec::new();
    for provider in ["Postmark", "Mailgun", "SES", "Resend", "SendGrid"] {
        let server = accepting_server().await;
        let transport = http_transports(&server)
            .into_iter()
            .find(|(name, _)| *name == provider)
            .map(|(_, transport)| transport)
            .unwrap();
        let result = transport.send(msg).await;
        let requests = server.received_requests().await.unwrap();
        sent.push(Sent {
            provider,
            result,
            requests,
        });
    }
    sent
}

fn json_of(sent: &Sent) -> serde_json::Value {
    assert_eq!(sent.requests.len(), 1, "{}: one request", sent.provider);
    serde_json::from_slice(&sent.requests[0].body).unwrap()
}

fn form_of(sent: &Sent) -> Vec<(String, String)> {
    assert_eq!(sent.requests.len(), 1, "{}: one request", sent.provider);
    url::form_urlencoded::parse(&sent.requests[0].body)
        .into_owned()
        .collect()
}

/// The `To` recipients as the provider will read them: every string field
/// is parsed as an RFC 5322 mailbox list; SendGrid's objects are read as
/// they are.
fn to_on_wire(sent: &Sent) -> Vec<(Option<String>, String)> {
    let strings: Vec<String> = match sent.provider {
        "SendGrid" => {
            return json_of(sent)["personalizations"][0]["to"]
                .as_array()
                .unwrap()
                .iter()
                .map(|to| {
                    (
                        to["name"].as_str().map(str::to_string),
                        to["email"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
        }
        "Mailgun" => form_of(sent)
            .into_iter()
            .filter(|(k, _)| k == "to")
            .map(|(_, v)| v)
            .collect(),
        "Postmark" => vec![json_of(sent)["To"].as_str().unwrap().to_string()],
        "SES" => json_of(sent)["Destination"]["ToAddresses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect(),
        "Resend" => json_of(sent)["to"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect(),
        other => panic!("unknown provider {other}"),
    };
    strings
        .iter()
        .flat_map(|field| {
            lettre::message::Mailboxes::from_str(field)
                .unwrap_or_else(|e| panic!("{field:?} is not a mailbox list: {e:?}"))
                .into_iter()
                .map(|m| (m.name, m.email.to_string()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The raw `To` strings of the providers that take recipients as text.
fn to_text_on_wire(sent: &Sent) -> Vec<String> {
    match sent.provider {
        "Mailgun" => form_of(sent)
            .into_iter()
            .filter(|(k, _)| k == "to")
            .map(|(_, v)| v)
            .collect(),
        "Postmark" => vec![json_of(sent)["To"].as_str().unwrap().to_string()],
        "SES" => json_of(sent)["Destination"]["ToAddresses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect(),
        "Resend" => json_of(sent)["to"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn single_recipient(name: &str) -> OutgoingMessage {
    let mut msg = message();
    msg.to = vec![Address::new(VICTIM).with_name(name)];
    msg.cc.clear();
    msg.bcc.clear();
    msg.reply_to.clear();
    msg
}

#[tokio::test]
async fn every_provider_collapses_line_breaks_in_a_display_name() {
    // A line break in a name cannot change who receives the message once
    // the name is quoted, so it is collapsed to one space instead of
    // failing the whole send.
    let msg = single_recipient("Victim\r\nBcc: attacker@evil.example\u{2028}\u{2029}x\ny");
    for sent in through_every_provider(&msg).await {
        assert!(sent.result.is_ok(), "{}: {:?}", sent.provider, sent.result);
        assert_eq!(
            to_on_wire(&sent),
            [(
                Some("Victim Bcc: attacker@evil.example x y".to_string()),
                VICTIM.to_string()
            )],
            "{}",
            sent.provider
        );
    }
}

#[tokio::test]
async fn every_provider_trims_whitespace_around_an_email() {
    let mut msg = single_recipient("Victim");
    msg.to = vec![Address::new("  victim@example.com\t").with_name("Victim")];
    for sent in through_every_provider(&msg).await {
        assert!(sent.result.is_ok(), "{}: {:?}", sent.provider, sent.result);
        assert_eq!(
            to_on_wire(&sent),
            [(Some("Victim".to_string()), VICTIM.to_string())],
            "{}",
            sent.provider
        );
    }
}

#[tokio::test]
async fn every_provider_refuses_a_quoted_local_part_or_a_domain_literal() {
    for email in [
        r#""attacker@evil.example, Victim <v@evil.example>"@example.com"#,
        r#""victim"@example.com"#,
        "victim@[127.0.0.1]",
    ] {
        let mut msg = message();
        msg.to = vec![Address::new(email)];
        assert_every_provider_refuses(&msg, "email address").await;
    }
}

#[tokio::test]
async fn every_provider_quotes_a_display_name_that_holds_an_encoded_word() {
    // `=?utf-8?b?...?=` decodes to `attacker@evil.example, V`. Unquoted, a
    // provider that decodes encoded words before it splits the list would
    // read two recipients; inside a quoted string it is never decoded.
    let encoded = "=?utf-8?b?YXR0YWNrZXJAZXZpbC5leGFtcGxlLCBW?=";
    let msg = single_recipient(encoded);
    for sent in through_every_provider(&msg).await {
        assert!(sent.result.is_ok(), "{}: {:?}", sent.provider, sent.result);
        for text in to_text_on_wire(&sent) {
            assert_eq!(
                text,
                format!("\"{encoded}\" <{VICTIM}>"),
                "{}",
                sent.provider
            );
        }
    }
}

#[tokio::test]
async fn every_provider_refuses_a_subject_with_a_line_break() {
    for subject in ["Hello\r\nBcc: attacker@evil.example", "Hello\nX", "Hello\0"] {
        let mut msg = message();
        msg.subject = subject.into();
        assert_every_provider_refuses(&msg, "Subject").await;
    }
}

#[tokio::test]
async fn every_provider_refuses_control_characters_in_a_header_value() {
    for value in ["a\x01b", "a\x7fb", "a\u{2028}b", "a\u{2029}b"] {
        let mut msg = message();
        msg.headers = vec![("X-Fine".into(), value.into())];
        assert_every_provider_refuses(&msg, "header").await;
    }
}

#[tokio::test]
async fn every_provider_refuses_a_header_the_message_structure_owns() {
    // `.header("Bcc", ...)` added a recipient on SMTP; the typed builder
    // methods are the only way to set these.
    for name in [
        "Bcc",
        "to",
        "CC",
        "From",
        "Sender",
        "Reply-To",
        "Return-Path",
        "Subject",
        "Date",
        "Message-ID",
        "MIME-Version",
        "Content-Type",
        "content-transfer-encoding",
    ] {
        let mut msg = message();
        msg.headers = vec![(name.into(), "attacker@evil.example".into())];
        assert_every_provider_refuses(&msg, "builder").await;
    }
}

#[tokio::test]
async fn every_provider_refuses_control_characters_in_tags_and_metadata() {
    for (tag, key, value) in [
        ("bad\r\ntag", "ok", "ok"),
        ("ok", "ok", "bad\nvalue"),
        ("ok", "ok", "bad\tvalue"),
        ("ok", "bad\u{2028}key", "ok"),
    ] {
        let mut msg = message();
        msg.tags = vec![tag.into()];
        msg.metadata.insert(key.into(), value.into());
        assert_every_provider_refuses(&msg, "").await;
    }
}

#[tokio::test]
async fn every_transport_refuses_a_metadata_key_that_is_not_a_header_name() {
    // SMTP and the file transport write the key into an `X-Metadata-<key>`
    // header name, so every transport holds it to that grammar.
    let mut msg = message();
    msg.metadata.insert("order id".into(), "42".into());
    assert_every_provider_refuses(&msg, "metadata key").await;
    assert_file_transport_refuses_with(&msg, "metadata key").await;
    let err = suprnova::mail::log::LogMailTransport::new()
        .send(&msg)
        .await
        .expect_err("the log transport must refuse what SMTP refuses");
    assert!(format!("{err}").contains("metadata key"), "{err}");
}

#[tokio::test]
async fn every_provider_refuses_an_attachment_name_with_a_line_break() {
    let mut msg = message();
    msg.attachments = vec![Attachment::new(
        "invoice.pdf\r\nContent-Type: text/html",
        b"%PDF".to_vec(),
        "application/pdf",
    )];
    assert_every_provider_refuses(&msg, "attachment").await;
}

#[tokio::test]
async fn every_provider_refuses_an_attachment_type_that_is_not_a_mime_type() {
    let mut msg = message();
    msg.attachments = vec![Attachment::new(
        "invoice.pdf",
        b"%PDF".to_vec(),
        "pdf please",
    )];
    assert_every_provider_refuses(&msg, "attachment").await;
}

#[tokio::test]
async fn a_refused_message_is_a_server_error_whose_text_stays_in_the_logs() {
    // A bad address is the application's fault, not the HTTP client's: a
    // 500 with a hidden body, like Laravel's RfcComplianceException,
    // instead of a 400 that echoes the address to the client.
    let mut msg = message();
    msg.to = vec![Address::new("victim@example.com, attacker@evil.example")];
    for sent in through_every_provider(&msg).await {
        let err = sent.result.expect_err(sent.provider);
        assert_eq!(err.status_code(), 500, "{}: {err}", sent.provider);
        assert!(
            matches!(err, FrameworkError::Internal { .. }),
            "{}: {err:?}",
            sent.provider
        );
    }
}

#[tokio::test]
async fn postmark_refuses_more_than_one_tag() {
    // Postmark carries one tag per email; sending the first and dropping
    // the rest lost data silently.
    let mut msg = message();
    msg.tags = vec!["welcome".into(), "onboarding".into()];
    let server = accepting_server().await;
    let err = PostmarkMailTransport::with_endpoint("token", server.uri())
        .send(&msg)
        .await
        .expect_err("two tags on Postmark");
    assert!(format!("{err}").contains("single tag"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn ses_refuses_a_tag_or_metadata_it_cannot_carry() {
    // SES tags allow only [A-Za-z0-9_-]; dropping the rest with a warning
    // lost data silently.
    for (tag, key, value) in [
        ("has space", "ok", "ok"),
        ("ok", "order.id", "ok"),
        ("ok", "ok", "a/b"),
    ] {
        let mut msg = message();
        msg.tags = vec![tag.into()];
        msg.metadata.insert(key.into(), value.into());
        let server = accepting_server().await;
        let err = SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri())
            .send(&msg)
            .await
            .expect_err("SES must refuse what it cannot carry");
        assert!(format!("{err}").contains("SES"), "{err}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn the_serializer_is_public_for_third_party_transports() {
    let list = suprnova::mail_wire::mailbox_list(
        "acme",
        &[
            Address::new(VICTIM).with_name(HOSTILE_NAME),
            Address::new("jane@example.com").with_name(ORDINARY_NAME),
        ],
    )
    .unwrap();
    assert_eq!(list, format!("{}, {ORDINARY_WIRE}", hostile_wire(VICTIM)));
    assert_eq!(
        suprnova::mail_wire::email("acme", &Address::new(" victim@example.com ")).unwrap(),
        VICTIM
    );
    assert!(suprnova::mail_wire::check_header("acme", "Bcc", "x@example.com").is_err());
    assert!(suprnova::mail_wire::check_message("acme", &message()).is_ok());
}

async fn assert_file_transport_refuses_with(msg: &OutgoingMessage, expect: &str) {
    let dir = tempfile::tempdir().unwrap();
    let err = FileMailTransport::new(dir.path())
        .send(msg)
        .await
        .expect_err("the file transport must refuse the message");
    assert!(format!("{err}").contains(expect), "{err}");
    let written = std::fs::read_dir(dir.path())
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "a refused message must not be written");
}
