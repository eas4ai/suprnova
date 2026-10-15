use super::delivery::receiver;
use std::time::Duration;
use suprnova::serde_json::json;
use suprnova::{ReportingConfig, ReportingEvent, ReportingKind};

#[test]
fn report_004_error_text_never_sends_header_or_quoted_credentials() {
    let (dsn, received) = receiver();
    let driver = ReportingConfig::default().dsn(dsn).build().expect("driver");
    driver.report(ReportingEvent {kind:ReportingKind::Event,payload:json!({
        "message":"upstream rejected\nAuthorization: Bearer private-auth\nretry after 5 seconds",
        "exception":{"values":[{"type":"Error","value":"Cookie: SID=private-cookie; theme=private-theme"}]},
        "extra":{
            "upstream":r#"{"password":"private-json second-json-word"}"#,
            "diagnostic":r#"password="private-quoted second-quoted-word""#,
            "failure":"password=private-first second-unquoted-word"
        }
    })}).expect("report");
    assert!(driver.flush(Duration::from_secs(3)).expect("flush"));
    let body = String::from_utf8(
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("envelope"),
    )
    .expect("UTF-8");
    for secret in [
        "private-auth",
        "private-cookie",
        "private-theme",
        "private-json",
        "second-json-word",
        "private-quoted",
        "second-quoted-word",
        "private-first",
        "second-unquoted-word",
    ] {
        assert!(!body.contains(secret), "outbound event leaked {secret}");
    }
    assert!(body.contains("upstream rejected"));
    assert!(body.contains("retry after 5 seconds"));
}

#[test]
fn report_004_public_events_drop_bodies_and_flattened_credentials_and_default_pii() {
    let (dsn, received) = receiver();
    let driver = ReportingConfig::default().dsn(dsn).build().expect("driver");
    driver.report(ReportingEvent {kind:ReportingKind::Event,payload:json!({
        "message":"privacy boundary",
        "request":{"url":"https://app.example/orders?email=private-mail@example.test","method":"POST","data":"private-opaque-body","headers":{
            "cf-connecting-ip":"203.0.113.17","cf-connecting-ipv6":"2001:db8::17","true-client-ip":"203.0.113.18","x-client-ip":"203.0.113.19"
        }},
        "extra":{"http.request.header.authorization":"Bearer private-flat-auth","http.request.header.cookie":"a=private-cookie-a; b=private-cookie-b"}
    })}).expect("report");
    assert!(driver.flush(Duration::from_secs(3)).expect("flush"));
    let body = String::from_utf8(
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("envelope"),
    )
    .expect("UTF-8");
    for secret in [
        "private-opaque-body",
        "private-flat-auth",
        "private-cookie-a",
        "private-cookie-b",
        "203.0.113.17",
        "2001:db8::17",
        "203.0.113.18",
        "203.0.113.19",
        "private-mail",
    ] {
        assert!(!body.contains(secret), "outbound event leaked {secret}");
    }
}

#[test]
fn report_003_direct_builder_applies_its_release_and_environment_and_pii_opt_in() {
    let (dsn, received) = receiver();
    let driver = ReportingConfig::default()
        .dsn(dsn)
        .release("shop@v2")
        .environment("staging")
        .send_pii(true)
        .build()
        .expect("driver");
    driver.report(ReportingEvent{kind:ReportingKind::Event,payload:json!({"message":"metadata","user":{"id":"42","email":"visible@example.test","ip_address":"203.0.113.42"}})}).expect("report");
    assert!(driver.flush(Duration::from_secs(3)).expect("flush"));
    let body = String::from_utf8(
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("envelope"),
    )
    .expect("UTF-8");
    assert!(body.contains("shop@v2"));
    assert!(body.contains("staging"));
    assert!(body.contains("visible@example.test"));
    assert!(body.contains("203.0.113.42"));
}
