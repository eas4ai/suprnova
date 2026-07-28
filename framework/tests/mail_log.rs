//! SEC-03: the `log` transport must not turn a log file into a credential
//! store. A password-reset or email-verification body carries a single-use
//! bearer link; once that link is in a log it is a working account takeover
//! for anyone with log access. These tests pin the envelope-only contract.

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::mail::log::LogMailTransport;
use suprnova::mail::{Address, Mail, Mailable};
use tracing_test::traced_test;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Ping {
    msg: String,
}

#[async_trait]
impl Mailable for Ping {
    fn mailable_name() -> &'static str {
        "Ping"
    }
    fn subject(&self) -> String {
        "ping".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("pong".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct EmptyBody;

#[async_trait]
impl Mailable for EmptyBody {
    fn mailable_name() -> &'static str {
        "EmptyBody"
    }
    fn subject(&self) -> String {
        "nope".into()
    }
}

/// Shaped like the framework's own password-reset mail: a one-time bearer
/// link in the body, in both the text and HTML alternatives.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct ResetPassword {
    // Tera needs a JSON object for its context; an empty named struct
    // serialises to `{}` where a unit struct would serialise to `null`.
    _placeholder: (),
}

const RESET_TOKEN: &str = "R3S3T-T0K3N-2f8a91";
const RESET_SIGNATURE: &str = "51gn4tur3-deadbeef";
const RESET_URL_BASE: &str = "https://app.example.com/password/reset";

#[async_trait]
impl Mailable for ResetPassword {
    fn mailable_name() -> &'static str {
        "ResetPassword"
    }
    fn subject(&self) -> String {
        "Reset your password".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some(format!(
            "Reset it here: {RESET_URL_BASE}?token={RESET_TOKEN}&signature={RESET_SIGNATURE}"
        ))
    }
    fn html_template_source(&self) -> Option<String> {
        Some(format!(
            "<a href=\"{RESET_URL_BASE}?token={RESET_TOKEN}&signature={RESET_SIGNATURE}\">Reset</a>"
        ))
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
}

/// A mailable that smuggles a signed link into the one free-text field the
/// log line still emits.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct MagicLinkSubject {
    _placeholder: (),
}

#[async_trait]
impl Mailable for MagicLinkSubject {
    fn mailable_name() -> &'static str {
        "MagicLinkSubject"
    }
    fn subject(&self) -> String {
        format!("Sign in: {RESET_URL_BASE}?token={RESET_TOKEN}")
    }
    fn text_template_source(&self) -> Option<String> {
        Some("body".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
}

#[tokio::test]
#[serial]
#[traced_test]
async fn log_transport_emits_the_envelope_but_not_the_body() {
    let _ = Mail::set_transport(Arc::new(LogMailTransport::new()));
    Mail::to("alice@example.org")
        .send(Ping {
            msg: "hello".into(),
        })
        .await
        .unwrap();

    assert!(logs_contain("mail (log driver): would send"));
    assert!(logs_contain("alice@example.org"), "recipient stays");
    assert!(logs_contain("noreply@suprnova.dev"), "sender stays");
    assert!(logs_contain("ping"), "subject stays");
    // The body does not. It is summarised by length instead.
    assert!(
        !logs_contain("pong"),
        "the rendered body must never reach the log (SEC-03)"
    );
    assert!(logs_contain("text_bytes=4"), "body size is still reported");
}

#[tokio::test]
#[serial]
#[traced_test]
async fn log_transport_never_emits_a_reset_link_or_its_token() {
    let _ = Mail::set_transport(Arc::new(LogMailTransport::new()));
    Mail::to("alice@example.org")
        .send(ResetPassword::default())
        .await
        .unwrap();

    assert!(
        logs_contain("mail (log driver): would send"),
        "sanity: the send was logged at all"
    );
    assert!(
        !logs_contain(RESET_TOKEN),
        "a single-use reset token in a log file is a working credential"
    );
    assert!(
        !logs_contain(RESET_SIGNATURE),
        "the URL signature must not leak either"
    );
    assert!(
        !logs_contain(RESET_URL_BASE),
        "the reset URL itself must not reach the log"
    );
    // The HTML alternative carried the same link — it must not slip through
    // by another route.
    assert!(!logs_contain("<a href"), "no HTML body in the log");
    // But the operator can still see that a reset mail went to this address.
    assert!(logs_contain("alice@example.org"));
    assert!(logs_contain("Reset your password"));
}

#[tokio::test]
#[serial]
#[traced_test]
async fn log_transport_redacts_url_credentials_in_the_subject() {
    let _ = Mail::set_transport(Arc::new(LogMailTransport::new()));
    Mail::to("alice@example.org")
        .send(MagicLinkSubject::default())
        .await
        .unwrap();

    assert!(
        !logs_contain(RESET_TOKEN),
        "a token smuggled through the subject must be redacted too"
    );
    assert!(
        logs_contain("token=[redacted]"),
        "the parameter name survives so the link is still recognisable"
    );
}

#[tokio::test]
#[serial]
async fn mailbuilder_rejects_mailable_without_any_body() {
    let _ = Mail::set_transport(Arc::new(LogMailTransport::new()));
    let err = Mail::to("alice@example.org")
        .send(EmptyBody)
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("EmptyBody"),
        "error mentions the Mailable name: {msg}"
    );
    assert!(
        msg.contains("text_template_source") || msg.contains("html_template_source"),
        "error suggests which methods to implement: {msg}"
    );
}
