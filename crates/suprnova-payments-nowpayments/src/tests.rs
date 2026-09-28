use super::*;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha512;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use suprnova::payments::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SECRET: &str = "test-ipn-secret";
const FIXTURE: &str = r#"{"payment_status":"finished","payment_id":5745459419,"invoice_id":"4522625843","order_id":"order-1001","price_amount":10.00,"price_currency":"usd","nested":{"z":1e-7,"a":[{"z":0.0,"a":"é"},true,null],"10":"ten","2":"two"}}"#;
// Generated independently using node:crypto and the provider's JavaScript
// sorting algorithm, not this crate's canonicalizer.
const FIXTURE_SIGNATURE: &str = "86df434c8817f5b5e86570268be37fce2360bcf7e7d04e173f86aa7aa18c6fc61ff6aabb721f6799f636da19404337d75b74d8b3e90835b005aa3b30e926655c";

fn provider() -> NowPaymentsProvider {
    NowPaymentsProvider::new(
        "test-api-key",
        SECRET,
        "https://merchant.example/webhooks/payments/nowpayments",
        NowPaymentsEnvironment::Sandbox,
    )
    .unwrap()
}

fn request() -> StartSessionRequest {
    StartSessionRequest {
        mode: SessionMode::OneOff,
        customer_ref: String::new(),
        price_refs: vec![],
        amount_hint: Some(Money::from_minor_units(1234, Currency::USD)),
        success_return_url: "https://merchant.example/checkout/return?order=order-1001".into(),
        cancel_return_url: "https://merchant.example/checkout/cancel".into(),
        idempotency_key: None,
        metadata: Some(json!({"order_id":"order-1001"})),
    }
}

fn payment(status: &str) -> Value {
    json!({"payment_id": "5745459419", "invoice_id": "4522625843", "order_id": "order-1001", "payment_status": status, "price_amount": "12.34", "price_currency": "usd"})
}

fn signature(body: &[u8]) -> String {
    let value = crate::json::parse(body).unwrap();
    let mut mac = Hmac::<Sha512>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(&crate::json::canonical(&value).unwrap());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn verify(body: &[u8], signature: &str) -> PaymentResult<()> {
    let mut headers = http::HeaderMap::new();
    headers.insert("x-nowpayments-sig", signature.parse().unwrap());
    provider().verify(&WebhookContext {
        body,
        headers: &headers,
        remote_addr: None,
    })
}

#[test]
fn ipn_matches_independent_javascript_fixture() {
    verify(FIXTURE.as_bytes(), FIXTURE_SIGNATURE).unwrap();
    let event = provider().parse_event(FIXTURE.as_bytes()).unwrap();
    assert_eq!(event.neutral, Some(NeutralEventKind::PaymentSucceeded));
    assert_eq!(event.provider_event_id, "5745459419:finished");
}

#[test]
fn ipn_rejects_tampering_missing_duplicate_and_invalid_signatures() {
    assert!(
        verify(
            FIXTURE.replace("10.00", "11.00").as_bytes(),
            FIXTURE_SIGNATURE
        )
        .is_err()
    );
    for invalid in ["", "00", &"z".repeat(128), &"0".repeat(128)] {
        assert!(matches!(
            verify(FIXTURE.as_bytes(), invalid),
            Err(PaymentError::WebhookSignature(_))
        ));
    }
    let mut headers = http::HeaderMap::new();
    let p = provider();
    assert!(
        p.verify(&WebhookContext {
            body: FIXTURE.as_bytes(),
            headers: &headers,
            remote_addr: None
        })
        .is_err()
    );
    headers.append("x-nowpayments-sig", FIXTURE_SIGNATURE.parse().unwrap());
    headers.append("x-nowpayments-sig", FIXTURE_SIGNATURE.parse().unwrap());
    assert!(
        p.verify(&WebhookContext {
            body: FIXTURE.as_bytes(),
            headers: &headers,
            remote_addr: None
        })
        .is_err()
    );
}

#[test]
fn ipn_rejects_duplicate_keys_invalid_json_and_oversized_bodies() {
    for body in [
        r#"{"a":1,"a":2}"#,
        r#"{"nested":{"a":1,"a":2}}"#,
        "[]",
        "null",
        "{",
        "{} {}",
    ] {
        assert!(
            verify(body.as_bytes(), FIXTURE_SIGNATURE).is_err(),
            "{body}"
        );
    }
    assert!(verify(&vec![b' '; MAX_BODY_BYTES + 1], FIXTURE_SIGNATURE).is_err());
    let huge = format!("{{\"n\":{}}}", 9_007_199_254_740_993u64);
    assert!(verify(huge.as_bytes(), FIXTURE_SIGNATURE).is_err());
}

#[test]
fn status_mapping_never_promotes_partial_or_unsettled_payments() {
    let p = provider();
    for (name, expected) in [
        ("waiting", None),
        ("confirming", None),
        ("confirmed", None),
        ("sending", None),
        ("partially_paid", None),
        ("new_provider_status", None),
        ("finished", Some(NeutralEventKind::PaymentSucceeded)),
        ("failed", Some(NeutralEventKind::PaymentFailed)),
        ("expired", Some(NeutralEventKind::PaymentFailed)),
        ("cancelled", Some(NeutralEventKind::PaymentFailed)),
        ("refunded", Some(NeutralEventKind::PaymentRefunded)),
    ] {
        let body = serde_json::to_vec(&payment(name)).unwrap();
        verify(&body, &signature(&body)).unwrap();
        let event = p.parse_event(&body).unwrap();
        assert_eq!(event.neutral, expected, "{name}");
        assert_eq!(
            p.extract_payload_ids(&event).transaction_id.as_deref(),
            Some("5745459419")
        );
    }
}

#[test]
fn ipn_delivery_timestamps_do_not_change_receipt_identity() {
    let p = provider();
    let first = payment("finished");
    let mut second = first.clone();
    second["updated_at"] = json!("2026-09-10T12:00:00Z");
    assert_eq!(
        p.parse_event(&serde_json::to_vec(&first).unwrap())
            .unwrap()
            .provider_event_id,
        p.parse_event(&serde_json::to_vec(&second).unwrap())
            .unwrap()
            .provider_event_id
    );
}

#[test]
fn malformed_identity_status_and_price_are_rejected() {
    let p = provider();
    for field in [
        "payment_id",
        "payment_status",
        "price_amount",
        "price_currency",
    ] {
        for invalid in [Value::Null, json!(true), json!([]), json!(""), json!(" ")] {
            let mut raw = payment("finished");
            raw[field] = invalid;
            assert!(
                p.parse_event(&serde_json::to_vec(&raw).unwrap()).is_err(),
                "{field}: {raw}"
            );
        }
    }
    for amount in [
        "0",
        "-1",
        "1.001",
        "NaN",
        "9999999999999999999999999999999999999999",
        "1e100",
    ] {
        let mut raw = payment("finished");
        raw["price_amount"] = json!(amount);
        assert!(
            p.parse_event(&serde_json::to_vec(&raw).unwrap()).is_err(),
            "{amount}"
        );
    }
    for id in ["../auth", "12/34", "01", "0", "1?x=y", " 12"] {
        let mut raw = payment("finished");
        raw["payment_id"] = json!(id);
        assert!(
            p.parse_event(&serde_json::to_vec(&raw).unwrap()).is_err(),
            "{id}"
        );
    }
}

#[test]
fn fiat_amounts_convert_without_assuming_two_decimal_places() {
    for (code, major, minor) in [
        ("usd", "12.34", 1234),
        ("jpy", "123", 123),
        ("kwd", "1.234", 1234),
    ] {
        let mut raw = payment("finished");
        raw["price_currency"] = json!(code);
        raw["price_amount"] = json!(major);
        let body = serde_json::to_vec(&raw).unwrap();
        assert_eq!(
            crate::status::parse_payment(raw, &body)
                .unwrap()
                .price
                .minor_units(),
            minor
        );
    }
}

#[test]
fn price_validation_preserves_lexemes_and_rejects_silent_rounding() {
    let p = provider();
    for amount in [
        "90071992547409.91",
        "1.00000000000000000000000000001e0",
        "\"1.00000000000000000000000000001e0\"",
        "\"1.00000000000000000000000000001\"",
    ] {
        let body = format!(
            r#"{{"payment_id":"1","payment_status":"finished","price_currency":"usd","price_amount":{amount}}}"#
        );
        assert!(
            p.parse_event(body.as_bytes()).is_err(),
            "{amount} must not be rounded"
        );
    }
    for (amount, minor) in [
        ("1e2", 10000),
        ("1.23e0", 123),
        ("\"90071992547409.91\"", 9007199254740991),
        ("0.01", 1),
    ] {
        let body = format!(
            r#"{{"payment_id":"1","payment_status":"finished","price_currency":"usd","price_amount":{amount}}}"#
        );
        let raw = crate::json::parse(body.as_bytes()).unwrap();
        assert_eq!(
            crate::status::parse_payment(raw, body.as_bytes())
                .unwrap()
                .price
                .minor_units(),
            minor
        );
    }
}

#[test]
fn constructor_rejects_bad_config_and_debug_redacts_secrets() {
    for secret in ["", " ", "a\nb"] {
        assert!(
            NowPaymentsProvider::new(
                secret,
                SECRET,
                "https://merchant.example/ipn",
                NowPaymentsEnvironment::Sandbox
            )
            .is_err()
        );
        assert!(
            NowPaymentsProvider::new(
                "key",
                secret,
                "https://merchant.example/ipn",
                NowPaymentsEnvironment::Sandbox
            )
            .is_err()
        );
    }
    for url in [
        "http://merchant.example/ipn",
        "https://user:pass@merchant.example/ipn",
        "https://merchant.example/ipn#x",
        "/ipn",
        " https://merchant.example/ipn",
    ] {
        assert!(
            NowPaymentsProvider::new("key", SECRET, url, NowPaymentsEnvironment::Sandbox).is_err(),
            "{url}"
        );
    }
    let debug = format!("{:?}", provider());
    assert!(!debug.contains(SECRET));
    assert!(!debug.contains("test-api-key"));
    assert!(provider().api_key.is_sensitive());
}

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
    headers: String,
    delay: Duration,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: value.to_string(),
            headers: String::new(),
            delay: Duration::ZERO,
        }
    }
}
#[derive(Debug)]
struct Captured {
    head: String,
    body: Value,
}
struct Server {
    url: Url,
    requests: Arc<Mutex<Vec<Captured>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn new(replies: Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/v1/", listener.local_addr().unwrap())).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let mut replies = VecDeque::from(replies);
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0u8; 1024];
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break index + 4;
                    }
                    assert!(bytes.len() < 16384);
                };
                let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let size: usize = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + size {
                    let mut buffer = [0u8; 1024];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let body = if size == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + size]).unwrap()
                };
                received.lock().unwrap().push(Captured { head, body });
                let reply = replies.pop_front().unwrap_or_else(|| Reply {
                    status: 500,
                    ..Reply::json(json!({}))
                });
                tokio::time::sleep(reply.delay).await;
                let response = format!(
                    "HTTP/1.1 {} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                    reply.status,
                    reply.body.len(),
                    reply.headers,
                    reply.body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    fn provider(&self) -> NowPaymentsProvider {
        let mut provider = provider();
        provider.api_url = self.url.clone();
        provider
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

fn invoice() -> Value {
    json!({"id":"4522625843", "order_id":"order-1001", "invoice_url":"https://sandbox.nowpayments.io/payment/?iid=4522625843"})
}

#[tokio::test]
async fn checkout_posts_exact_fields_and_returns_generic_redirect() {
    let server = Server::new(vec![Reply::json(invoice())]).await;
    let session = server.provider().start_session(request()).await.unwrap();
    assert!(
        matches!(session, SessionPayload::Redirect { provider_session_id, .. } if provider_session_id == "4522625843")
    );
    let requests = server.requests.lock().unwrap();
    let sent = &requests[0];
    assert!(sent.head.starts_with("POST /v1/invoice HTTP/1.1"));
    assert!(
        sent.head
            .to_ascii_lowercase()
            .contains("x-api-key: test-api-key")
    );
    assert_eq!(
        sent.body,
        json!({
            "price_amount":"12.34", "price_currency":"usd", "order_id":"order-1001",
            "ipn_callback_url":"https://merchant.example/webhooks/payments/nowpayments",
            "success_url":"https://merchant.example/checkout/return?order=order-1001",
            "cancel_url":"https://merchant.example/checkout/cancel"
        })
    );
}

#[tokio::test]
async fn invalid_or_unsupported_checkout_never_reaches_provider() {
    let server = Server::new(vec![]).await;
    let p = server.provider();
    let mut inputs = Vec::new();
    let mut bad = request();
    bad.mode = SessionMode::Subscription;
    inputs.push(bad);
    let mut bad = request();
    bad.customer_ref = "cus_1".into();
    inputs.push(bad);
    let mut bad = request();
    bad.price_refs = vec!["price_1".into()];
    inputs.push(bad);
    let mut bad = request();
    bad.idempotency_key = Some("retry-1".into());
    inputs.push(bad);
    let mut bad = request();
    bad.amount_hint = None;
    inputs.push(bad);
    let mut bad = request();
    bad.amount_hint = Some(Money::from_minor_units(0, Currency::USD));
    inputs.push(bad);
    let mut bad = request();
    bad.metadata = None;
    inputs.push(bad);
    let mut bad = request();
    bad.metadata = Some(json!({"order_id":"order-1001","unknown":true}));
    inputs.push(bad);
    let mut bad = request();
    bad.metadata = Some(json!({"order_id":"order-1001","pay_currency":"../btc"}));
    inputs.push(bad);
    for url in [
        "http://merchant.example/x",
        "https://attacker.example/x",
        "https://merchant.example.evil/x",
        "https://merchant.example:444/x",
        "https://merchant.example/x#fragment",
    ] {
        let mut bad = request();
        bad.success_return_url = url.into();
        inputs.push(bad);
    }
    for input in inputs {
        assert!(matches!(
            p.create_invoice(input).await,
            Err(InvoiceCreationError::Rejected(_))
        ));
    }
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn checkout_rejects_untrusted_or_mismatched_success_response_as_unknown() {
    let mut responses = Vec::new();
    for url in [
        "https://attacker.example/payment/?iid=4522625843",
        "https://sandbox.nowpayments.io.evil/payment/?iid=4522625843",
        "https://nowpayments.io/payment/?iid=4522625843",
        "http://sandbox.nowpayments.io/payment/?iid=4522625843",
        "https://sandbox.nowpayments.io/payment/?iid=999",
        "https://sandbox.nowpayments.io/payment/?iid=4522625843&iid=999",
    ] {
        let mut raw = invoice();
        raw["invoice_url"] = json!(url);
        responses.push(raw);
    }
    let mut wrong_order = invoice();
    wrong_order["order_id"] = json!("other-order");
    responses.push(wrong_order);
    let mut wrong_id = invoice();
    wrong_id["id"] = json!([]);
    responses.push(wrong_id);
    for raw in responses {
        let server = Server::new(vec![Reply::json(raw)]).await;
        assert!(matches!(
            server.provider().create_invoice(request()).await,
            Err(InvoiceCreationError::Unknown(_))
        ));
        assert_eq!(server.count(), 1);
    }
}

#[tokio::test]
async fn invoice_errors_distinguish_rejection_from_ambiguous_and_never_retry() {
    for (status, rejected) in [
        (400, true),
        (401, true),
        (403, true),
        (404, true),
        (422, true),
        (408, false),
        (429, false),
        (500, false),
        (503, false),
    ] {
        let server = Server::new(vec![Reply {
            status,
            ..Reply::json(json!({"message":"secret-response-do-not-log"}))
        }])
        .await;
        let error = server
            .provider()
            .create_invoice(request())
            .await
            .unwrap_err();
        assert_eq!(
            matches!(error, InvoiceCreationError::Rejected(_)),
            rejected,
            "{status}"
        );
        assert!(!error.to_string().contains("secret-response-do-not-log"));
        assert_eq!(server.count(), 1);
    }
}

#[tokio::test]
async fn malformed_large_timeout_and_redirect_responses_are_bounded_without_retries() {
    for reply in [
        Reply {
            body: "not-json".into(),
            ..Reply::json(json!({}))
        },
        Reply {
            body: format!("{{\"body\":\"{}\"}}", "x".repeat(MAX_BODY_BYTES)),
            ..Reply::json(json!({}))
        },
        Reply {
            delay: Duration::from_secs(1),
            ..Reply::json(invoice())
        },
        Reply {
            status: 302,
            headers: "Location: https://attacker.example/\r\n".into(),
            ..Reply::json(json!({}))
        },
    ] {
        let server = Server::new(vec![reply]).await;
        let mut p = server.provider();
        p.client = build_client(Duration::from_millis(100)).unwrap();
        assert!(matches!(
            p.create_invoice(request()).await,
            Err(InvoiceCreationError::Unknown(_))
        ));
        assert_eq!(server.count(), 1);
    }
}

#[tokio::test]
async fn payment_lookup_authenticates_validates_identity_and_keeps_invoice_separate() {
    let server = Server::new(vec![
        Reply::json(payment("finished")),
        Reply::json(payment("partially_paid")),
        Reply::json(payment("finished")),
    ])
    .await;
    let p = server.provider();
    let paid = p.payment_status("5745459419").await.unwrap();
    assert_eq!(paid.status, NowPaymentsStatus::Finished);
    assert_eq!(paid.invoice_id.as_deref(), Some("4522625843"));
    assert_eq!(paid.price, Money::from_minor_units(1234, Currency::USD));
    assert_eq!(
        p.payment_status("5745459419").await.unwrap().status,
        NowPaymentsStatus::PartiallyPaid
    );
    assert!(p.payment_status("123").await.is_err());
    assert!(matches!(
        p.session_status("4522625843").await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(p.payment_status("../auth").await.is_err());
    assert_eq!(server.count(), 3);
    let requests = server.requests.lock().unwrap();
    assert!(
        requests[0]
            .head
            .starts_with("GET /v1/payment/5745459419 HTTP/1.1")
    );
    assert!(
        requests[0]
            .head
            .to_ascii_lowercase()
            .contains("x-api-key: test-api-key")
    );
}

#[tokio::test]
async fn every_unsupported_capability_is_explicit_and_offline() {
    let server = Server::new(vec![]).await;
    let p = server.provider();
    assert!(p.as_payment().is_none());
    assert!(p.as_promotions().is_none());
    assert!(!p.mirrors_payment_transactions());
    assert!(matches!(
        p.create_customer(CreateCustomerRequest {
            user_id: "u".into(),
            email: "u@example.com".into(),
            name: None,
            metadata: None
        })
        .await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(matches!(
        p.update_customer(UpdateCustomerRequest {
            provider_customer_id: "u".into(),
            email: None,
            name: None,
            metadata: None
        })
        .await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(matches!(
        p.get_customer("u").await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(matches!(
        p.delete_customer("u").await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(matches!(
        p.subscribe(SubscribeRequest {
            customer_ref: "u".into(),
            price_refs: vec![],
            trial_days: None,
            idempotency_key: None,
            metadata: None
        })
        .await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(matches!(
        p.update(UpdateSubscriptionRequest {
            provider_subscription_id: "s".into(),
            new_price_refs: None,
            cancel_at_period_end: None,
            idempotency_key: None
        })
        .await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(matches!(
        p.cancel("s", true).await,
        Err(PaymentError::NotSupported(_))
    ));
    assert!(matches!(
        p.get("s").await,
        Err(PaymentError::NotSupported(_))
    ));
    assert_eq!(server.count(), 0);
}
