//! `FileMailTransport` writes one RFC 5322 `.eml` per message. The header
//! set has to match what SMTP emits - a preview that quietly drops headers
//! misrepresents the real send - and enabling its zero-part guard must not
//! change SMTP's output.

use suprnova::mail::Attachment;
use suprnova::mail::address::Address;
use suprnova::mail::file::FileMailTransport;
use suprnova::mail::transport::{MailTransport, OutgoingMessage};

fn addr(email: &str) -> Address {
    Address {
        email: email.to_string(),
        name: None,
    }
}

fn base_message() -> OutgoingMessage {
    OutgoingMessage {
        from: addr("from@example.test"),
        to: vec![addr("to@example.test")],
        cc: vec![],
        bcc: vec![],
        reply_to: vec![],
        subject: "Hello there".to_string(),
        html: Some("<p>hi</p>".to_string()),
        text: Some("hi".to_string()),
        attachments: vec![],
        tags: vec![],
        metadata: Default::default(),
        priority: None,
        headers: vec![],
        return_path: None,
    }
}

async fn send_and_read(dir: &std::path::Path, msg: &OutgoingMessage) -> String {
    let t = FileMailTransport::new(dir);
    t.send(msg).await.expect("send writes a file");
    let mut entries = std::fs::read_dir(dir).expect("dir exists");
    let path = entries
        .next()
        .expect("exactly one message written")
        .expect("readable entry")
        .path();
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("eml"));
    String::from_utf8_lossy(&std::fs::read(path).expect("readable")).into_owned()
}

#[tokio::test]
async fn writes_an_eml_with_envelope_and_both_bodies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let raw = send_and_read(dir.path(), &base_message()).await;

    assert!(raw.contains("To: to@example.test"), "missing To:\n{raw}");
    assert!(
        raw.contains("Subject: Hello there"),
        "missing Subject:\n{raw}"
    );
    assert!(
        raw.contains("multipart/alternative"),
        "not alternative:\n{raw}"
    );
    assert!(raw.contains("hi"), "missing text body:\n{raw}");
    assert!(raw.contains("<p>hi</p>"), "missing html body:\n{raw}");
}

#[tokio::test]
async fn emits_the_same_header_superset_smtp_does() {
    // The regression the `build_mime` hoist would have shipped: SES's
    // builder omits all four of these, so a preview built on it would show
    // an inbox the real send never produces.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut msg = base_message();
    msg.priority = Some(1);
    msg.tags = vec!["welcome".to_string()];
    msg.metadata
        .insert("campaign".to_string(), "spring".to_string());
    msg.return_path = Some(addr("bounces@example.test"));
    msg.headers = vec![("X-Custom".to_string(), "yes".to_string())];

    let raw = send_and_read(dir.path(), &msg).await;

    assert!(raw.contains("X-Priority: 1"), "missing X-Priority:\n{raw}");
    assert!(
        raw.contains("Importance: High"),
        "missing Importance:\n{raw}"
    );
    assert!(raw.contains("X-Tag: welcome"), "missing X-Tag:\n{raw}");
    assert!(
        raw.contains("X-Metadata-campaign: spring"),
        "missing X-Metadata-*:\n{raw}"
    );
    assert!(
        raw.contains("Return-Path: bounces@example.test"),
        "missing Return-Path:\n{raw}"
    );
    assert!(
        raw.contains("X-Custom: yes"),
        "missing custom header:\n{raw}"
    );
}

#[tokio::test]
async fn an_attachment_survives_the_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut msg = base_message();
    msg.attachments = vec![Attachment::new(
        "note.txt",
        b"attached bytes".to_vec(),
        "text/plain",
    )];

    let raw = send_and_read(dir.path(), &msg).await;

    assert!(raw.contains("multipart/mixed"), "not mixed:\n{raw}");
    assert!(raw.contains("note.txt"), "missing filename:\n{raw}");
}

#[tokio::test]
async fn a_missing_directory_is_created() {
    let dir = tempfile::tempdir().expect("tempdir");
    let nested = dir.path().join("deeply").join("nested");
    assert!(!nested.exists());

    let t = FileMailTransport::new(&nested);
    t.send(&base_message())
        .await
        .expect("creates the directory");

    assert!(nested.is_dir(), "directory was not created");
}

#[tokio::test]
async fn an_unwritable_directory_returns_an_error_rather_than_panicking() {
    // A file where the directory should be: `create_dir_all` fails, and the
    // transport has to surface that as a Result.
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"x").expect("write blocker");

    let t = FileMailTransport::new(&blocker);
    let err = t.send(&base_message()).await.expect_err("must not panic");
    assert!(
        err.to_string().to_lowercase().contains("mail"),
        "error should name the subsystem: {err}"
    );
}

#[tokio::test]
async fn a_message_with_no_bodies_does_not_emit_a_zero_part_alternative() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut msg = base_message();
    msg.text = None;
    msg.html = None;

    let raw = send_and_read(dir.path(), &msg).await;

    // A zero-part multipart is not valid MIME and no client will open it.
    // The guard substitutes a single empty text/plain part.
    assert!(
        raw.contains("text/plain"),
        "expected a substituted text/plain part:\n{raw}"
    );
}
