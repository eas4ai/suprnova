use std::collections::HashMap;
use std::future::Future;
use std::sync::Once;
use std::time::Duration;

use suprnova::payments::{
    ChargeRequest, Checkout, Currency, Money, Payment, PaymentError, RefundRequest, SessionMode,
    StartSessionRequest, SubscribeRequest, Subscription, UpdateSubscriptionRequest,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::{DEFAULT_WEBHOOK_SIGNATURE_TOLERANCE_SECONDS, StripeProvider};

static INIT: Once = Once::new();

#[derive(Debug)]
struct CapturedRequest {
    request_line: String,
    headers: HashMap<String, String>,
    body: String,
}

fn init_crypto() {
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn provider(base_url: String) -> StripeProvider {
    init_crypto();
    StripeProvider {
        client: stripe::ClientBuilder::new("sk_test_request_capture")
            .url(base_url)
            .build()
            .expect("valid capture-server URL"),
        publishable_key: "pk_test_request_capture".into(),
        webhook_signing_secret: "whsec_request_capture".into(),
        webhook_signature_tolerance_seconds: DEFAULT_WEBHOOK_SIGNATURE_TOLERANCE_SECONDS,
        managed_payments: false,
    }
}

async fn capture_server() -> (String, JoinHandle<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind request-capture listener");
    let address = listener.local_addr().expect("capture listener address");
    let task = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .expect("Stripe request reached capture server")
            .expect("accept Stripe request");
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await.expect("read Stripe request");
            assert!(read > 0, "Stripe request ended before its headers");
            bytes.extend_from_slice(&chunk[..read]);

            let Some(header_start) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let header_end = header_start + 4;
            let headers = String::from_utf8_lossy(&bytes[..header_start]);
            let content_length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| {
                    value
                        .trim()
                        .parse::<usize>()
                        .expect("numeric content-length")
                })
                .unwrap_or(0);
            if bytes.len() >= header_end + content_length {
                break header_end;
            }
        };

        let response_body = r#"{"error":{"message":"forced response","type":"api_error"}}"#;
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write Stripe response");

        let head = String::from_utf8(bytes[..header_end - 4].to_vec())
            .expect("Stripe request headers are UTF-8");
        let mut lines = head.lines();
        let request_line = lines.next().expect("Stripe request line").to_string();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
            .collect();

        CapturedRequest {
            request_line,
            headers,
            body: String::from_utf8(bytes[header_end..].to_vec())
                .expect("Stripe request body is UTF-8"),
        }
    });

    (format!("http://{address}/"), task)
}

async fn capture_request<T, F, Fut>(call: F) -> (T, CapturedRequest)
where
    F: FnOnce(StripeProvider) -> Fut,
    Fut: Future<Output = T>,
{
    let (base_url, capture) = capture_server().await;
    let result = call(provider(base_url)).await;
    let request = capture.await.expect("request-capture task");
    (result, request)
}

fn assert_key(request: &CapturedRequest, key: &str, path: &str) {
    assert!(
        request.request_line.starts_with(&format!("POST {path} ")),
        "unexpected request line: {}",
        request.request_line
    );
    assert_eq!(
        request.headers.get("idempotency-key").map(String::as_str),
        Some(key),
        "missing or changed idempotency key on {}",
        request.request_line
    );
}

fn charge_request(idempotency_key: Option<String>) -> ChargeRequest {
    ChargeRequest {
        customer_ref: "cus_test".into(),
        payment_method_ref: "pm_test".into(),
        amount: Money::from_minor_units(1_299, Currency::USD),
        description: Some("request capture".into()),
        idempotency_key,
        metadata: None,
    }
}

fn refund_request(idempotency_key: Option<String>) -> RefundRequest {
    RefundRequest {
        provider_transaction_id: "pi_test".into(),
        amount: None,
        reason: Some("requested_by_customer".into()),
        idempotency_key,
    }
}

fn session_request(
    mode: SessionMode,
    price_refs: Vec<String>,
    amount_hint: Option<Money>,
    idempotency_key: String,
) -> StartSessionRequest {
    StartSessionRequest {
        mode,
        customer_ref: "cus_test".into(),
        price_refs,
        success_return_url: "https://example.test/success".into(),
        cancel_return_url: "https://example.test/cancel".into(),
        amount_hint,
        idempotency_key: Some(idempotency_key),
        metadata: None,
    }
}

#[tokio::test]
async fn idempotency_keys_are_request_headers_on_every_supported_mutation() {
    let (_, charge) = capture_request(|provider| async move {
        provider
            .charge(charge_request(Some("charge-key".into())))
            .await
    })
    .await;
    assert_key(&charge, "charge-key", "/v1/payment_intents");
    assert!(
        !charge.body.contains("idempotency_key"),
        "idempotency data must not be serialized into the Stripe form body"
    );

    let (_, refund) = capture_request(|provider| async move {
        provider
            .refund(refund_request(Some("refund-key".into())))
            .await
    })
    .await;
    assert_key(&refund, "refund-key", "/v1/refunds");

    let (_, hosted) = capture_request(|provider| async move {
        provider
            .start_session(session_request(
                SessionMode::OneOff,
                vec!["price_test".into()],
                None,
                "h".into(),
            ))
            .await
    })
    .await;
    assert_key(&hosted, "h", "/v1/checkout/sessions");

    let (_, elements) = capture_request(|provider| async move {
        provider
            .start_session(session_request(
                SessionMode::OneOff,
                Vec::new(),
                Some(Money::from_minor_units(2_500, Currency::USD)),
                "elements-key".into(),
            ))
            .await
    })
    .await;
    assert_key(&elements, "elements-key", "/v1/payment_intents");

    let (_, subscription_checkout) = capture_request(|provider| async move {
        provider
            .start_session(session_request(
                SessionMode::Subscription,
                vec!["price_monthly".into()],
                None,
                "checkout-sub-key".into(),
            ))
            .await
    })
    .await;
    assert_key(
        &subscription_checkout,
        "checkout-sub-key",
        "/v1/checkout/sessions",
    );

    let (_, subscribe) = capture_request(|provider| async move {
        provider
            .subscribe(SubscribeRequest {
                customer_ref: "cus_test".into(),
                price_refs: vec!["price_monthly".into()],
                trial_days: Some(7),
                idempotency_key: Some("subscribe-key".into()),
                metadata: None,
            })
            .await
    })
    .await;
    assert_key(&subscribe, "subscribe-key", "/v1/subscriptions");

    let longest_key = "x".repeat(255);
    let expected_key = longest_key.clone();
    let (_, update) = capture_request(|provider| async move {
        provider
            .update(UpdateSubscriptionRequest {
                provider_subscription_id: "sub_test".into(),
                new_price_refs: None,
                cancel_at_period_end: Some(true),
                idempotency_key: Some(longest_key),
            })
            .await
    })
    .await;
    assert_key(&update, &expected_key, "/v1/subscriptions/sub_test");

    let (_, absent) =
        capture_request(|provider| async move { provider.refund(refund_request(None)).await })
            .await;
    assert!(
        !absent.headers.contains_key("idempotency-key"),
        "an absent key must not add an idempotency header"
    );
}

async fn assert_invalid_key_is_rejected(key: String) {
    let (base_url, mut capture) = capture_server().await;
    let result = provider(base_url).refund(refund_request(Some(key))).await;
    let received = tokio::time::timeout(Duration::from_millis(100), &mut capture).await;
    if received.is_err() {
        capture.abort();
    }

    assert!(
        matches!(result, Err(PaymentError::Validation(_))),
        "invalid idempotency keys must be rejected as request validation errors: {result:?}"
    );
    assert!(received.is_err(), "invalid key reached the Stripe endpoint");
}

#[tokio::test]
async fn invalid_idempotency_keys_are_rejected_before_network_io() {
    assert_invalid_key_is_rejected(String::new()).await;
    assert_invalid_key_is_rejected("   ".into()).await;
    assert_invalid_key_is_rejected("x".repeat(256)).await;
}
