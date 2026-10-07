use crate::{PaddleEnvironment, PaddleProvider};
use paddle_rust_sdk::Paddle;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use suprnova::payments::{
    Checkout, CheckoutSessionState, Currency, Money, PaymentError, SessionMode, SessionPayload,
    StartSessionRequest,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::timeout,
};

fn transaction(status: &str) -> Value {
    json!({"data": {
        "id": "txn_test", "status": status, "customer_id": "ctm_test",
        "currency_code": "USD", "origin": "api", "collection_mode": "automatic",
        "items": [], "payments": [], "checkout": {"url": "https://checkout.example.test"},
        "created_at": "2026-09-09T12:00:00Z", "updated_at": "2026-09-09T12:01:00Z",
        "details": {"tax_rates_used": [], "line_items": [],
            "totals": {"subtotal": "2900", "discount": "0", "tax": "0", "total": "2900",
                "credit": "0", "credit_to_balance": "0", "balance": "0", "grand_total": "2900", "currency_code": "USD"},
            "adjusted_totals": {"subtotal": "2900", "tax": "0", "total": "2900", "grand_total": "2900", "currency_code": "USD"}}
    }, "meta": {"request_id": "test-request"}})
}

async fn server(body: Value, status: u16) -> (PaddleProvider, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let capture = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 4096];
                let read = stream.read(&mut buf).await.unwrap();
                assert!(read > 0, "request ended before complete body");
                request.extend_from_slice(&buf[..read]);
                assert!(request.len() < 65536, "unexpectedly large request");
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                    }).unwrap_or(0);
                    if request.len() >= end + 4 + length { break; }
                }
            }
            let body = body.to_string();
            let response = format!("HTTP/1.1 {status} Test\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        }).await.expect("bounded mock server")
    });
    (
        PaddleProvider {
            client: Arc::new(
                Paddle::new("pdl_sdbx_apikey_test", format!("http://{address}")).unwrap(),
            ),
            webhook_key: "test-webhook".into(),
            client_token: "test-client".into(),
            environment: PaddleEnvironment::Sandbox,
        },
        capture,
    )
}

#[tokio::test]
async fn checkout_forwards_correlation_metadata() {
    for mode in [SessionMode::OneOff, SessionMode::Subscription] {
        let (provider, capture) = server(transaction("draft"), 200).await;
        let metadata = json!({"attempt": "listing & revision=1 + café"});
        let result = timeout(
            Duration::from_secs(5),
            provider.start_session(StartSessionRequest {
                mode,
                customer_ref: "ctm_test".into(),
                price_refs: vec!["pri_first".into(), "pri_second".into()],
                success_return_url: "https://app.example.test/success".into(),
                cancel_return_url: "https://app.example.test/cancel".into(),
                amount_hint: None,
                idempotency_key: None,
                metadata: Some(metadata),
            }),
        )
        .await
        .unwrap();
        let request = capture.await.unwrap();
        assert!(
            matches!(result, Ok(SessionPayload::PaddleInline { transaction_id, .. }) if transaction_id == "txn_test")
        );
        assert!(request.starts_with("POST /transactions HTTP/1.1"));
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body["custom_data"],
            json!({"attempt": "listing & revision=1 + café"})
        );
        assert_eq!(
            body["items"],
            json!([{"price_id":"pri_first","quantity":1},{"price_id":"pri_second","quantity":1}])
        );
        assert_eq!(body["customer_id"], "ctm_test");
    }
}

#[tokio::test]
async fn retrieval_maps_each_transaction_status() {
    for status in [
        "draft",
        "ready",
        "billed",
        "past_due",
        "canceled",
        "paid",
        "completed",
    ] {
        let (provider, capture) = server(transaction(status), 200).await;
        let result = timeout(Duration::from_secs(5), provider.session_status("txn_test"))
            .await
            .unwrap();
        // Await capture only after checking the default NotSupported regression.
        assert!(result.is_ok(), "{status}: {result:?}");
        let request = capture.await.unwrap();
        assert!(
            request.starts_with("GET /transactions/txn_test? HTTP/1.1"),
            "{request}"
        );
        let expected = match status {
            "paid" | "completed" => CheckoutSessionState::Complete {
                paid: true,
                payment_ref: Some("txn_test".into()),
                amount_total: Some(Money::from_minor_units(2900, Currency::USD)),
            },
            "canceled" => CheckoutSessionState::Expired,
            _ => CheckoutSessionState::Open,
        };
        assert_eq!(result.unwrap(), expected, "{status}");
    }
}

#[tokio::test]
async fn retrieval_rejects_invalid_totals_and_provider_errors() {
    for total in [
        Value::Null,
        json!("not-a-number"),
        json!("9223372036854775808"),
    ] {
        let mut body = transaction("completed");
        body["data"]["details"]["totals"]["total"] = total;
        let (provider, capture) = server(body, 200).await;
        let result = timeout(Duration::from_secs(5), provider.session_status("txn_test"))
            .await
            .unwrap();
        assert!(
            matches!(result, Err(PaymentError::Provider(_))),
            "{result:?}"
        );
        capture.await.unwrap();
    }
    let (provider, capture) = server(
        json!({"error":{"type":"request_error","code":"not_found","detail":"missing transaction"}}),
        404,
    )
    .await;
    let result = timeout(Duration::from_secs(5), provider.session_status("txn_test"))
        .await
        .unwrap();
    assert!(
        matches!(result, Err(PaymentError::Provider(_))),
        "{result:?}"
    );
    capture.await.unwrap();
}

#[tokio::test]
async fn checkout_does_not_mislabel_customer_id_as_auth_token() {
    let (provider, capture) = server(transaction("draft"), 200).await;
    let payload = provider
        .start_session(StartSessionRequest {
            mode: SessionMode::OneOff,
            customer_ref: "ctm_test".into(),
            price_refs: vec!["pri_test".into()],
            success_return_url: "https://app.example.test/success".into(),
            cancel_return_url: "https://app.example.test/cancel".into(),
            amount_hint: None,
            idempotency_key: None,
            metadata: None,
        })
        .await
        .unwrap();
    capture.await.unwrap();
    assert!(matches!(
        payload,
        SessionPayload::PaddleInline {
            customer_token: None,
            ..
        }
    ));
}

#[tokio::test]
async fn checkout_metadata_uses_existing_string_map_policy() {
    for (metadata, expected) in [
        (None, Value::Null),
        (
            Some(json!({"count": 2, "active": true, "nested": {"revision": 1}, "empty": null})),
            json!({"count":"2", "active":"true", "nested":"{\"revision\":1}"}),
        ),
    ] {
        let (provider, capture) = server(transaction("draft"), 200).await;
        provider
            .start_session(StartSessionRequest {
                mode: SessionMode::OneOff,
                customer_ref: "ctm_test".into(),
                price_refs: vec!["pri_test".into()],
                success_return_url: "https://app.example.test/success".into(),
                cancel_return_url: "https://app.example.test/cancel".into(),
                amount_hint: None,
                idempotency_key: None,
                metadata,
            })
            .await
            .unwrap();
        let request = capture.await.unwrap();
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["custom_data"], expected);
    }
}

#[tokio::test]
async fn retrieval_rejects_malformed_transaction_identifiers_before_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = PaddleProvider {
        client: Arc::new(
            Paddle::new(
                "test-api",
                format!("http://{}", listener.local_addr().unwrap()),
            )
            .unwrap(),
        ),
        webhook_key: "test-webhook".into(),
        client_token: "test-client".into(),
        environment: PaddleEnvironment::Sandbox,
    };
    for id in [
        "",
        "txn_",
        "other_test",
        "txn_a/../customers",
        "txn_a?include=customer",
        "txn_a#fragment",
        "txn_a%2f",
    ] {
        let result = timeout(Duration::from_millis(200), provider.session_status(id)).await;
        assert!(
            matches!(result, Ok(Err(PaymentError::Validation(_)))),
            "{id}: {result:?}"
        );
    }
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

async fn stalled_transaction_request(
    create: bool,
) -> Result<super::PaymentResult<()>, tokio::time::error::Elapsed> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = PaddleProvider {
        client: Arc::new(
            Paddle::new(
                "test-api",
                format!("http://{}", listener.local_addr().unwrap()),
            )
            .unwrap(),
        ),
        webhook_key: "test-webhook".into(),
        client_token: "test-client".into(),
        environment: PaddleEnvironment::Sandbox,
    };
    let task = tokio::spawn(async move {
        if create {
            provider
                .start_session(StartSessionRequest {
                    mode: SessionMode::OneOff,
                    customer_ref: "ctm_test".into(),
                    price_refs: vec!["pri_test".into()],
                    success_return_url: "https://app.example.test/success".into(),
                    cancel_return_url: "https://app.example.test/cancel".into(),
                    amount_hint: None,
                    idempotency_key: None,
                    metadata: None,
                })
                .await
                .map(|_| ())
        } else {
            provider.session_status("txn_test").await.map(|_| ())
        }
    });
    let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    let result = timeout(Duration::from_secs(1), task)
        .await
        .map(|result| result.unwrap());
    tokio::time::resume();
    drop(stream);
    result
}

#[tokio::test]
async fn create_timeout_reports_an_uncertain_outcome() {
    let result = stalled_transaction_request(true).await;
    assert!(
        matches!(result, Ok(Err(PaymentError::Provider(ref message))) if message.contains("timed out") && message.contains("reconcile")),
        "{result:?}"
    );
}

#[tokio::test]
async fn retrieval_timeout_is_bounded() {
    let result = stalled_transaction_request(false).await;
    assert!(
        matches!(result, Ok(Err(PaymentError::Provider(ref message))) if message.contains("timed out")),
        "{result:?}"
    );
}
