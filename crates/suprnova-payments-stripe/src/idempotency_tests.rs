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

async fn capture_server_with_response(
    response: Option<String>,
) -> (String, JoinHandle<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind request-capture listener");
    let address = listener.local_addr().expect("capture listener address");
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async move {
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
            assert!(bytes.len() < 65536, "unexpectedly large Stripe request");

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

        let fallback_body = r#"{"error":{"message":"forced response","type":"api_error"}}"#;
        let status = if response.is_some() {
            "200 OK"
        } else {
            "400 Bad Request"
        };
        let response_body = response.as_deref().unwrap_or(fallback_body);
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_body}",
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
        }).await.expect("bounded Stripe capture exchange")
    });

    (format!("http://{address}/"), task)
}

async fn capture_request<T, F, Fut>(call: F) -> (T, CapturedRequest)
where
    F: FnOnce(StripeProvider) -> Fut,
    Fut: Future<Output = T>,
{
    let (base_url, capture) = capture_server_with_response(None).await;
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
    let (base_url, mut capture) = capture_server_with_response(None).await;
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

fn checkout_success_fixture(mode: &str) -> String {
    serde_json::json!({
        "id": "cs_test_contract", "object": "checkout.session",
        "automatic_tax": {"enabled": false, "liability": null, "status": null},
        "created": 1720000000, "expires_at": 1720086400, "livemode": false,
        "mode": mode, "payment_method_types": ["card"], "shipping_options": [],
        "payment_status": "unpaid", "status": "open", "custom_fields": [],
        "custom_text": {"after_submit": null, "shipping_address": null,
            "submit": null, "terms_of_service_acceptance": null},
        "metadata": {}, "url": "https://checkout.stripe.com/c/pay/cs_test_contract"
    })
    .to_string()
}

fn elements_success_fixture() -> String {
    serde_json::json!({
        "id": "pi_contract", "object": "payment_intent", "amount": 2500,
        "amount_capturable": 0, "amount_received": 0, "capture_method": "automatic",
        "client_secret": "pi_contract_secret_fixture", "confirmation_method": "automatic",
        "created": 1720000000, "currency": "usd", "livemode": false,
        "metadata": {}, "payment_method_types": ["card"],
        "status": "requires_payment_method"
    })
    .to_string()
}

async fn successful_checkout_request(
    mode: SessionMode,
    prices: Vec<String>,
    metadata: Option<serde_json::Value>,
    managed_payments: bool,
) -> (suprnova::payments::SessionPayload, HashMap<String, String>) {
    let elements = prices.is_empty();
    let response = if elements {
        elements_success_fixture()
    } else {
        checkout_success_fixture(if mode == SessionMode::Subscription {
            "subscription"
        } else {
            "payment"
        })
    };
    let (base_url, capture) = capture_server_with_response(Some(response)).await;
    let mut provider = provider(base_url);
    provider.managed_payments = managed_payments;
    let mut req = session_request(
        mode,
        prices,
        Some(Money::from_minor_units(2500, Currency::USD)),
        "checkout-contract-key".into(),
    );
    req.success_return_url =
        "https://example.test/return?session_id={CHECKOUT_SESSION_ID}&next=a+b%20c".into();
    req.metadata = metadata;
    let result = provider.start_session(req).await;
    let request = capture.await.expect("capture checkout request");
    assert_key(
        &request,
        "checkout-contract-key",
        if elements {
            "/v1/payment_intents"
        } else {
            "/v1/checkout/sessions"
        },
    );
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/x-www-form-urlencoded")
    );
    let pairs: Vec<_> = form_urlencoded::parse(request.body.as_bytes())
        .into_owned()
        .collect();
    let form: HashMap<_, _> = pairs.iter().cloned().collect();
    assert_eq!(
        pairs.len(),
        form.len(),
        "duplicate form keys hide incorrect serialization"
    );
    (
        result.expect("Stripe success fixture must yield checkout payload"),
        form,
    )
}

async fn assert_hosted_checkout_wire(mode: SessionMode, downstream: &str, managed: bool) {
    let value = "intent + / ? & = % café";
    let (payload, form) = successful_checkout_request(
        mode,
        vec!["price_first".into(), "price_second".into()],
        Some(serde_json::json!({"checkout_intent_id": value})),
        managed,
    )
    .await;
    assert!(
        matches!(payload, suprnova::payments::SessionPayload::StripeCheckoutRedirect { provider_session_id, url } if provider_session_id == "cs_test_contract" && url == "https://checkout.stripe.com/c/pay/cs_test_contract")
    );
    assert_eq!(
        form.get("line_items[0][price]").map(String::as_str),
        Some("price_first")
    );
    assert_eq!(
        form.get("line_items[1][price]").map(String::as_str),
        Some("price_second")
    );
    for index in 0..2 {
        assert_eq!(
            form.get(&format!("line_items[{index}][quantity]"))
                .map(String::as_str),
            Some("1")
        );
    }
    assert_eq!(
        form.get("success_url").map(String::as_str),
        Some("https://example.test/return?session_id={CHECKOUT_SESSION_ID}&next=a+b%20c")
    );
    assert_eq!(
        form.get("metadata[checkout_intent_id]").map(String::as_str),
        Some(value)
    );
    assert_eq!(
        form.get(&format!("{downstream}[metadata][checkout_intent_id]"))
            .map(String::as_str),
        Some(value)
    );
    let other = if managed {
        "subscription_data"
    } else {
        "payment_intent_data"
    };
    assert!(!form.keys().any(|key| key.starts_with(other)));
    assert_eq!(
        form.get("mode").map(String::as_str),
        Some(if managed { "payment" } else { "subscription" })
    );
    assert_eq!(
        form.get("allow_promotion_codes").map(String::as_str),
        managed.then_some("true")
    );
    assert_eq!(
        form.get("managed_payments[enabled]").map(String::as_str),
        managed.then_some("true")
    );
}

#[tokio::test]
async fn hosted_checkout_wire_preserves_metadata_line_items_and_flags() {
    assert_hosted_checkout_wire(SessionMode::OneOff, "payment_intent_data", true).await;
}

#[tokio::test]
async fn subscription_checkout_wire_preserves_metadata_line_items_and_flags() {
    assert_hosted_checkout_wire(SessionMode::Subscription, "subscription_data", false).await;
}

#[tokio::test]
async fn elements_checkout_wire_preserves_metadata() {
    let value = "intent + & = % café";
    let (payload, form) = successful_checkout_request(
        SessionMode::OneOff,
        Vec::new(),
        Some(serde_json::json!({"checkout_intent_id": value})),
        false,
    )
    .await;
    assert!(
        matches!(payload, suprnova::payments::SessionPayload::StripeElements { provider_session_id, client_secret, .. } if provider_session_id == "pi_contract" && client_secret == "pi_contract_secret_fixture")
    );
    assert_eq!(form.get("amount").map(String::as_str), Some("2500"));
    assert_eq!(form.get("currency").map(String::as_str), Some("usd"));
    assert_eq!(
        form.get("automatic_payment_methods[enabled]")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        form.get("metadata[checkout_intent_id]").map(String::as_str),
        Some(value)
    );
    assert!(
        !form
            .keys()
            .any(|key| key.starts_with("payment_intent_data")
                || key.starts_with("subscription_data"))
    );
}

#[tokio::test]
async fn checkout_wire_omits_absent_metadata_and_disabled_managed_payments() {
    for mode in [SessionMode::OneOff, SessionMode::Subscription] {
        let (_, form) =
            successful_checkout_request(mode, vec!["price_first".into()], None, false).await;
        assert!(!form.keys().any(|key| key.contains("metadata")));
        assert!(!form.contains_key("managed_payments[enabled]"));
    }
}

#[tokio::test]
async fn elements_session_status_retrieves_payment_intent_and_only_succeeded_is_paid() {
    use suprnova::payments::CheckoutSessionState;
    for status in [
        "succeeded",
        "processing",
        "canceled",
        "requires_payment_method",
        "requires_capture",
        "requires_action",
        "requires_confirmation",
    ] {
        let mut fixture: serde_json::Value =
            serde_json::from_str(&elements_success_fixture()).unwrap();
        fixture["status"] = status.into();
        fixture["amount_received"] = if status == "succeeded" { 2500 } else { 0 }.into();
        let (base_url, capture) = capture_server_with_response(Some(fixture.to_string())).await;
        let result = provider(base_url).session_status("pi_contract").await;
        let request = capture.await.expect("capture PaymentIntent retrieval");
        assert_eq!(
            request.request_line,
            "GET /v1/payment_intents/pi_contract HTTP/1.1"
        );
        let expected = match status {
            "succeeded" => CheckoutSessionState::Complete {
                paid: true,
                payment_ref: Some("pi_contract".into()),
                amount_total: Some(Money::from_minor_units(2500, Currency::USD)),
            },
            "canceled" => CheckoutSessionState::Expired,
            _ => CheckoutSessionState::Open,
        };
        assert_eq!(
            result.expect("valid PaymentIntent response"),
            expected,
            "status {status}"
        );
    }
}

#[tokio::test]
async fn elements_session_status_preserves_provider_errors() {
    let (base_url, capture) = capture_server_with_response(None).await;
    let result = provider(base_url).session_status("pi_contract").await;
    let request = capture
        .await
        .expect("capture failed PaymentIntent retrieval");
    assert_eq!(
        request.request_line,
        "GET /v1/payment_intents/pi_contract HTTP/1.1"
    );
    assert!(matches!(result, Err(PaymentError::Provider(_))));
}

#[tokio::test]
async fn session_status_rejects_invalid_identifiers_before_network() {
    for identifier in [
        "",
        "pi_",
        "cs_",
        "pi_bad/other",
        "cs_bad?expand[]=customer",
        "pi_bad#fragment",
        "unrecognized",
    ] {
        let (base_url, mut capture) = capture_server_with_response(None).await;
        let result = provider(base_url).session_status(identifier).await;
        let received = tokio::time::timeout(Duration::from_millis(100), &mut capture).await;
        if received.is_err() {
            capture.abort();
        }
        assert!(
            matches!(result, Err(PaymentError::Validation(_))),
            "invalid identifier must fail validation: {identifier}: {result:?}"
        );
        assert!(
            received.is_err(),
            "invalid identifier reached provider: {identifier}"
        );
    }
}
