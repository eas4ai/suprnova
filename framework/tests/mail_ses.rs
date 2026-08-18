use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::mail::ses::SesMailTransport;
use suprnova::mail::{Address, Mail, Mailable};
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct M {
    // Tera context requires a JSON object; a unit struct serializes to `null`
    // and `Context::from_value` rejects that. Empty named struct → `{}`.
    _placeholder: (),
}

#[async_trait]
impl Mailable for M {
    fn mailable_name() -> &'static str {
        "M"
    }
    fn subject(&self) -> String {
        "s".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("b".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
}

#[tokio::test]
#[serial]
async fn ses_emits_sigv4_signed_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/email/outbound-emails"))
        .and(header_exists("authorization")) // sigv4 puts the sig here
        .and(header_exists("x-amz-date")) // sigv4 timestamp
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "0000018a-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .send(M::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let auth = reqs[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(auth.starts_with("AWS4-HMAC-SHA256"), "got: {auth}");

    // A message that sets no custom headers must not grow a
    // `Content.Simple.Headers` field at all — `skip_serializing_if` on
    // `SesSimple::headers` keeps the request body byte-identical to what
    // it was before this task's header support landed.
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert!(
        body["Content"]["Simple"]["Headers"].is_null(),
        "Content.Simple.Headers must be absent for a header-less message: {body}"
    );
}

#[tokio::test]
#[serial]
async fn ses_maps_4xx_to_framework_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "__type": "MessageRejected",
            "message": "Email address is not verified."
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    let err = Mail::to("u@unverified.example")
        .send(M::default())
        .await
        .unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("SES"), "error mentions provider: {s}");
    assert!(s.contains("400"), "error includes HTTP status: {s}");
    assert!(
        s.contains("Email address is not verified"),
        "error surfaces upstream body: {s}"
    );
}

// Attachments must ride the Raw MIME path — SES `Content.Simple` has no
// attachment support, so dropping silently is a data-loss bug.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct MWithPdf {
    _placeholder: (),
}

#[async_trait]
impl Mailable for MWithPdf {
    fn mailable_name() -> &'static str {
        "MWithPdf"
    }
    fn subject(&self) -> String {
        "invoice".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("see attached".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
    fn attachments(&self) -> Vec<suprnova::mail::Attachment> {
        vec![suprnova::mail::Attachment {
            filename: "invoice.pdf".into(),
            content: b"%PDF-1.4\n%test-content".to_vec(),
            content_type: "application/pdf".into(),
        }]
    }
}

#[tokio::test]
#[serial]
async fn ses_uses_raw_mime_when_attachments_present() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/email/outbound-emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "raw-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .send(MWithPdf::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();

    // Must use the Raw variant when attachments are present — Simple has
    // no attachment support and would silently drop them.
    assert!(
        body["Content"]["Simple"].is_null(),
        "Simple variant must be absent: {body}"
    );
    let raw_b64 = body["Content"]["Raw"]["Data"]
        .as_str()
        .expect("Content.Raw.Data is a string");

    use base64::Engine;
    let mime = base64::engine::general_purpose::STANDARD
        .decode(raw_b64)
        .expect("Raw.Data is valid base64");
    let mime_str = String::from_utf8_lossy(&mime);

    assert!(
        mime_str.contains("Content-Disposition: attachment"),
        "MIME has attachment disposition: {mime_str}"
    );
    assert!(
        mime_str.contains("invoice.pdf"),
        "MIME contains filename: {mime_str}"
    );
    assert!(
        mime_str.contains("application/pdf"),
        "MIME contains content-type: {mime_str}"
    );
    // Body and subject must still ride the MIME envelope too.
    assert!(
        mime_str.contains("invoice"),
        "MIME contains subject: {mime_str}"
    );
    assert!(
        mime_str.contains("see attached"),
        "MIME contains text body: {mime_str}"
    );
}

// ---------------------------------------------------------------------------
// Custom headers must survive BOTH SES content paths
// ---------------------------------------------------------------------------
//
// SES picks Simple or Raw based on whether the message has attachments. A
// caller who sets List-Unsubscribe does not know or care which path their
// message takes, so a header that only rides one of them is a header that
// disappears the day somebody attaches a PDF.

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct MWithHeaderDefault {
    _placeholder: (),
}

#[async_trait]
impl Mailable for MWithHeaderDefault {
    fn mailable_name() -> &'static str {
        "MWithHeaderDefault"
    }
    fn subject(&self) -> String {
        "s".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("b".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-Origin".into(), "warehouse".into())]
    }
}

#[tokio::test]
#[serial]
async fn ses_simple_content_carries_custom_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/email/outbound-emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "simple-headers-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .header("List-Unsubscribe", "<https://example.org/unsub/abc>")
        .header("X-Campaign", "spring")
        .send(M::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();

    let headers = body["Content"]["Simple"]["Headers"]
        .as_array()
        .unwrap_or_else(|| panic!("Content.Simple.Headers must be a list: {body}"));

    // SES v2 spells a MessageHeader `{"Name": ..., "Value": ...}` — the same
    // pair Postmark uses. A field-name typo here fails the whole send call at
    // AWS, so assert the exact JSON shape rather than "contains the string".
    assert!(
        headers
            .iter()
            .any(|h| h["Name"] == "List-Unsubscribe"
                && h["Value"] == "<https://example.org/unsub/abc>"),
        "List-Unsubscribe missing from Content.Simple.Headers: {body}"
    );
    assert!(
        headers
            .iter()
            .any(|h| h["Name"] == "X-Campaign" && h["Value"] == "spring"),
        "X-Campaign missing from Content.Simple.Headers: {body}"
    );
    assert_eq!(
        headers.len(),
        2,
        "exactly the two caller-set headers, no extras: {body}"
    );
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct MPdfHeaders {
    _placeholder: (),
}

#[async_trait]
impl Mailable for MPdfHeaders {
    fn mailable_name() -> &'static str {
        "MPdfHeaders"
    }
    fn subject(&self) -> String {
        "invoice".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("see attached".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
    fn attachments(&self) -> Vec<suprnova::mail::Attachment> {
        vec![suprnova::mail::Attachment {
            filename: "invoice.pdf".into(),
            content: b"%PDF-1.4\n%test-content".to_vec(),
            content_type: "application/pdf".into(),
        }]
    }
}

#[tokio::test]
#[serial]
async fn ses_raw_mime_carries_custom_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/email/outbound-emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "raw-headers-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .header("List-Unsubscribe", "<https://example.org/unsub/abc>")
        .header("X-Campaign", "spring")
        .send(MPdfHeaders::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let raw_b64 = body["Content"]["Raw"]["Data"]
        .as_str()
        .expect("attachments force the Raw path");

    use base64::Engine;
    let mime = base64::engine::general_purpose::STANDARD
        .decode(raw_b64)
        .expect("Raw.Data is valid base64");
    let mime_str = String::from_utf8_lossy(&mime);

    // Line-anchored, not a bare substring match: lettre terminates each
    // header line with CRLF (`lettre::message::header::Headers`'s `Display`
    // impl), so a real header line — as opposed to the value merely
    // appearing inside another header or the body — starts right after one.
    assert!(
        mime_str.contains("\r\nList-Unsubscribe: <https://example.org/unsub/abc>"),
        "raw MIME dropped List-Unsubscribe: {mime_str}"
    );
    // Exactly once. A header emitted by both a builder loop and a fallback
    // would be a duplicate field, which some MTAs reject outright.
    assert_eq!(
        mime_str.matches("X-Campaign:").count(),
        1,
        "X-Campaign must appear exactly once in the MIME envelope: {mime_str}"
    );
}

#[tokio::test]
#[serial]
async fn ses_does_not_duplicate_a_header_set_twice() {
    // `MailBuilder::send` unions the Mailable's headers with the builder's and
    // de-dupes exact (name, value) pairs (framework/src/mail/mod.rs:416-420).
    // The SES mapping must not re-introduce the duplicate.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/email/outbound-emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "dedupe-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .header("X-Origin", "warehouse")
        .send(MWithHeaderDefault::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let headers = body["Content"]["Simple"]["Headers"]
        .as_array()
        .unwrap_or_else(|| panic!("Content.Simple.Headers must be a list: {body}"));

    assert_eq!(
        headers.iter().filter(|h| h["Name"] == "X-Origin").count(),
        1,
        "the same header on the Mailable and the builder must produce one \
         entry, not two: {body}"
    );
}

#[tokio::test]
#[serial]
async fn ses_rejects_a_header_name_carrying_crlf() {
    // A CR/LF in a header name is how a caller-supplied string becomes a
    // second header. Mailgun already refuses it (mail/mailgun.rs:68); SES must
    // refuse it identically on BOTH content paths, so that attaching a file
    // never changes whether a message is accepted.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "never-sent"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));

    for injected in ["X-Bad\r\nInjected", "X-Bad\nHeader", "X-Bad\0Header"] {
        // Content.Simple path: no attachments.
        let err = Mail::to("alice@example.org")
            .header(injected, "value")
            .send(M::default())
            .await
            .unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("SES") && s.contains("header name"),
            "the error must name the transport and the offending field (Simple path): {s}"
        );

        // Content.Raw path: attachments present. The up-front guard in
        // `send` runs before the content branch, so this must reject
        // identically rather than only failing once inside `build_mime`.
        let err = Mail::to("alice@example.org")
            .header(injected, "value")
            .send(MPdfHeaders::default())
            .await
            .unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("SES") && s.contains("header name"),
            "the error must name the transport and the offending field (Raw path): {s}"
        );
    }

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a rejected header must abort before anything reaches AWS"
    );
}

#[tokio::test]
#[serial]
async fn ses_rejects_a_header_name_with_space_or_colon() {
    // `HeaderName::new_from_ascii` — the check the raw MIME path runs when
    // it builds a real header line — rejects a `:` or a space in a name in
    // addition to CR/LF/NUL. The up-front guard in `send` has to reject the
    // same names on the Simple path, or "X-Foo: bar" would be forwarded to
    // AWS today and only start failing the day somebody attaches a file.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "never-sent"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));

    for injected in ["X-Foo: bar", "X-Foo Bar"] {
        let err = Mail::to("alice@example.org")
            .header(injected, "value")
            .send(M::default())
            .await
            .unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("SES") && s.contains("header name"),
            "the error must name the transport and the offending field: {s}"
        );
    }

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a rejected header must abort before anything reaches AWS (Simple path, no attachment)"
    );
}

// ---- SES v2 send options (Laravel 13.25 #60886 + the ConfigurationSet /
//      ListManagementOptions fold) -----------------------------------------

/// A mailable that pins SES control headers at the type level, the way
/// an app scopes a whole mail class to one tenant.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct TenantMail {
    _placeholder: (),
}

#[async_trait]
impl Mailable for TenantMail {
    fn mailable_name() -> &'static str {
        "TenantMail"
    }
    fn subject(&self) -> String {
        "tenanted".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("body".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-SES-TENANT-NAME".into(), "acme".into())]
    }
}

/// Stand up a mock SES endpoint, run one send, and return the decoded
/// request body.
async fn ses_body_for<F>(build: F) -> serde_json::Value
where
    F: FnOnce(SesMailTransport) -> SesMailTransport,
{
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/email/outbound-emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "opts-stub"
        })))
        .mount(&server)
        .await;

    let transport = build(SesMailTransport::with_endpoint(
        "AKIATEST",
        "secret",
        "us-east-1",
        server.uri(),
    ));
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .send(TenantMail::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    serde_json::from_slice(&reqs[0].body).unwrap()
}

#[tokio::test]
#[serial]
async fn ses_reads_tenant_name_from_the_mailable_header() {
    let body = ses_body_for(|t| t).await;
    assert_eq!(body["TenantName"], "acme", "body: {body}");
}

#[tokio::test]
#[serial]
async fn ses_header_tenant_name_beats_the_transport_default() {
    // Laravel merges the header over the configured options, so a
    // per-message tenant overrides the transport's.
    let body = ses_body_for(|t| t.tenant_name("transport-default")).await;
    assert_eq!(body["TenantName"], "acme", "body: {body}");
}

#[tokio::test]
#[serial]
async fn ses_falls_back_to_the_transport_tenant_name() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "fallback-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri())
            .tenant_name("transport-default");
    let _ = Mail::set_transport(Arc::new(transport));
    // `M` declares no headers, so only the transport default applies.
    Mail::to("alice@example.org")
        .send(M::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["TenantName"], "transport-default", "body: {body}");
}

#[tokio::test]
#[serial]
async fn ses_omits_every_option_key_when_none_is_configured() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "bare-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .send(M::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert!(body.get("TenantName").is_none(), "body: {body}");
    assert!(body.get("ConfigurationSetName").is_none(), "body: {body}");
    assert!(body.get("ListManagementOptions").is_none(), "body: {body}");
}

#[tokio::test]
#[serial]
async fn ses_configuration_set_comes_from_transport_and_from_the_header() {
    let from_transport = ses_body_for(|t| t.configuration_set_name("prod-set")).await;
    assert_eq!(from_transport["ConfigurationSetName"], "prod-set");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "cfg-stub"
        })))
        .mount(&server)
        .await;
    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri())
            .configuration_set_name("prod-set");
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .header("X-SES-CONFIGURATION-SET", "per-message-set")
        .send(M::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(
        body["ConfigurationSetName"], "per-message-set",
        "body: {body}"
    );
}

#[tokio::test]
#[serial]
async fn ses_parses_the_laravel_list_management_header_shapes() {
    for (header, expect_list, expect_topic) in [
        ("news", "news", None),
        ("contactListName=news", "news", None),
        ("news; topicName=weekly", "news", Some("weekly")),
        (
            "contactListName=news;topicName=weekly",
            "news",
            Some("weekly"),
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MessageId": "lm-stub"
            })))
            .mount(&server)
            .await;
        let transport =
            SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
        let _ = Mail::set_transport(Arc::new(transport));
        Mail::to("alice@example.org")
            .header("X-SES-LIST-MANAGEMENT-OPTIONS", header)
            .send(M::default())
            .await
            .unwrap();

        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(
            body["ListManagementOptions"]["ContactListName"], expect_list,
            "header {header:?} body: {body}"
        );
        match expect_topic {
            Some(t) => assert_eq!(
                body["ListManagementOptions"]["TopicName"], t,
                "header {header:?} body: {body}"
            ),
            None => assert!(
                body["ListManagementOptions"].get("TopicName").is_none(),
                "header {header:?} must not emit TopicName; body: {body}"
            ),
        }
    }
}

#[tokio::test]
#[serial]
async fn ses_transport_list_management_default_applies_without_a_header() {
    let body = ses_body_for(|t| t.list_management("newsletter", Some("weekly"))).await;
    assert_eq!(
        body["ListManagementOptions"]["ContactListName"],
        "newsletter"
    );
    assert_eq!(body["ListManagementOptions"]["TopicName"], "weekly");
}

/// SES control headers select the tenant / configuration set /
/// subscription list for the send. They are transport directives, not
/// message content, and must never reach the recipient's inbox.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct TenantMailWithPdf {
    _placeholder: (),
}

#[async_trait]
impl Mailable for TenantMailWithPdf {
    fn mailable_name() -> &'static str {
        "TenantMailWithPdf"
    }
    fn subject(&self) -> String {
        "tenanted invoice".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("see attached".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-SES-TENANT-NAME".into(), "acme".into())]
    }
    fn attachments(&self) -> Vec<suprnova::mail::Attachment> {
        vec![suprnova::mail::Attachment {
            filename: "invoice.pdf".into(),
            content: b"%PDF-1.4\n%test-content".to_vec(),
            content_type: "application/pdf".into(),
        }]
    }
}

#[tokio::test]
#[serial]
async fn ses_control_headers_never_reach_the_raw_mime() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MessageId": "raw-tenant-stub"
        })))
        .mount(&server)
        .await;

    let transport =
        SesMailTransport::with_endpoint("AKIATEST", "secret", "us-east-1", server.uri());
    let _ = Mail::set_transport(Arc::new(transport));
    Mail::to("alice@example.org")
        .send(TenantMailWithPdf::default())
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();

    // The directive rode the request as a top-level option…
    assert_eq!(body["TenantName"], "acme", "body: {body}");

    // …and not as a MIME header inside the message.
    let raw_b64 = body["Content"]["Raw"]["Data"]
        .as_str()
        .expect("Content.Raw.Data is a string");
    use base64::Engine;
    let mime = base64::engine::general_purpose::STANDARD
        .decode(raw_b64)
        .expect("Raw.Data is valid base64");
    let mime_str = String::from_utf8_lossy(&mime);
    assert!(
        !mime_str.to_ascii_uppercase().contains("X-SES-TENANT-NAME"),
        "SES control headers must not be rendered into the message: {mime_str}"
    );
}
