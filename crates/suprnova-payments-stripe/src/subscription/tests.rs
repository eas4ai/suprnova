//! `Subscription` against a mocked Stripe API: price changes and the
//! deadline of a call.
//!
//! The mocked API answers each connection with the next canned response
//! and writes the request to the log before the response goes out. The
//! adapter reads each response before it goes on, so every request it made
//! is in the log by the time its call returns, and a request past the
//! canned responses finds no listener and fails. `stalled` is the other
//! shape: a Stripe that takes the connection and never answers, with the
//! paused clock moved past the deadline.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use serde_json::json;
use suprnova::payments::{
    PaymentError, PaymentResult, Proration, Subscription, UpdateSubscriptionRequest,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::deadline::REQUEST_TIMEOUT;
use crate::{DEFAULT_WEBHOOK_SIGNATURE_TOLERANCE_SECONDS, StripeProvider};

static INIT: Once = Once::new();

/// One request as the mocked Stripe API received it.
struct Captured {
    request_line: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Captured {
    /// The form body as a map. A key sent twice fails the test, since the
    /// map would hide it.
    fn form(&self) -> HashMap<String, String> {
        let pairs: Vec<(String, String)> = form_urlencoded::parse(self.body.as_bytes())
            .into_owned()
            .collect();
        let form: HashMap<String, String> = pairs.iter().cloned().collect();
        assert_eq!(pairs.len(), form.len(), "a form key was sent twice");
        form
    }
}

/// The requests the mocked Stripe API received, in order.
type RequestLog = Arc<Mutex<Vec<Captured>>>;

/// A provider that talks to a mocked Stripe API answering with
/// `responses`, one `(status, body)` per request, and the log of what it
/// was sent.
async fn stripe_with(responses: Vec<(u16, String)>) -> (StripeProvider, RequestLog) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mocked Stripe API");
    let address = listener.local_addr().expect("mocked Stripe API address");
    let log = RequestLog::default();
    let server_log = Arc::clone(&log);
    tokio::spawn(async move {
        for (status, body) in responses {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let request = read_request(&mut stream).await;
            server_log.lock().expect("request log").push(request);
            respond(&mut stream, status, &body).await;
        }
    });
    (provider_at(address), log)
}

/// Answer the request on `stream` with `status` and the JSON `body`.
async fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} Mocked\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write mocked Stripe response");
}

/// A provider whose Stripe API refuses every connection: the listener is
/// closed before the first request.
async fn unreachable() -> StripeProvider {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mocked Stripe API");
    let address = listener.local_addr().expect("mocked Stripe API address");
    drop(listener);
    provider_at(address)
}

/// Run `call` against a Stripe API that takes the connection and never
/// answers, move the paused clock past the deadline, and return what the
/// call returned. Nothing waits in real time: the clock is paused before it
/// moves.
async fn stalled<T, F, Fut>(call: F) -> PaymentResult<T>
where
    F: FnOnce(StripeProvider) -> Fut,
    Fut: Future<Output = PaymentResult<T>> + Send + 'static,
    T: Send + 'static,
{
    stalled_after(Vec::new(), call).await
}

/// Like [`stalled`], for a call whose first requests are answered with
/// `responses`, one `(status, body)` per request: the request after them is
/// the one that stalls.
async fn stalled_after<T, F, Fut>(responses: Vec<(u16, String)>, call: F) -> PaymentResult<T>
where
    F: FnOnce(StripeProvider) -> Fut,
    Fut: Future<Output = PaymentResult<T>> + Send + 'static,
    T: Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled Stripe API");
    let address = listener.local_addr().expect("stalled Stripe API address");
    let task = tokio::spawn(call(provider_at(address)));
    for (status, body) in responses {
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("the call reaches Stripe")
            .expect("accept the call");
        read_request(&mut stream).await;
        respond(&mut stream, status, &body).await;
    }
    let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("the call reaches Stripe")
        .expect("accept the call");
    tokio::time::pause();
    tokio::time::advance(REQUEST_TIMEOUT + Duration::from_secs(1)).await;
    let result = timeout(Duration::from_secs(1), task)
        .await
        .expect("the deadline ends the call")
        .expect("the task of the call");
    tokio::time::resume();
    drop(stream);
    result
}

/// A provider that talks to the mocked Stripe API at `address`.
fn provider_at(address: SocketAddr) -> StripeProvider {
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    StripeProvider {
        client: stripe::ClientBuilder::new("sk_test_subscription_update")
            .url(format!("http://{address}/"))
            .build()
            .expect("mocked Stripe API URL"),
        publishable_key: "pk_test_subscription_update".into(),
        webhook_signing_secret: "whsec_subscription_update".into(),
        webhook_signature_tolerance_seconds: DEFAULT_WEBHOOK_SIGNATURE_TOLERANCE_SECONDS,
        managed_payments: false,
    }
}

/// Read one HTTP request: the head, then as many body bytes as its
/// `content-length` names.
async fn read_request(stream: &mut TcpStream) -> Captured {
    let mut bytes = Vec::new();
    let head_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.expect("read Stripe request");
        assert!(read > 0, "Stripe request ended before it was complete");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < 65536, "unexpectedly large Stripe request");
        let Some(head_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let content_length = String::from_utf8_lossy(&bytes[..head_end])
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
        if bytes.len() >= head_end + 4 + content_length {
            break head_end;
        }
    };
    let head = String::from_utf8(bytes[..head_end].to_vec()).expect("request head is UTF-8");
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    Captured {
        request_line,
        headers,
        body: String::from_utf8(bytes[head_end + 4..].to_vec())
            .expect("Stripe request body is UTF-8"),
    }
}

/// The subscription `sub_test` as Stripe returns it, with one item per
/// `(item id, price id, quantity)`.
fn subscription(items: &[(&str, &str, u64)]) -> String {
    let data: Vec<serde_json::Value> = items
        .iter()
        .map(|(item_id, price_id, quantity)| {
            json!({
                "id": item_id, "object": "subscription_item", "created": 1720000000,
                "current_period_start": 1720000000, "current_period_end": 1722678400,
                "discounts": [], "metadata": {}, "quantity": quantity,
                "subscription": "sub_test",
                "plan": {
                    "id": price_id, "object": "plan", "active": true,
                    "billing_scheme": "per_unit", "created": 1720000000, "currency": "usd",
                    "interval": "month", "interval_count": 1, "livemode": false,
                    "usage_type": "licensed"
                },
                "price": {
                    "id": price_id, "object": "price", "active": true,
                    "billing_scheme": "per_unit", "created": 1720000000, "currency": "usd",
                    "livemode": false, "metadata": {}, "product": "prod_test",
                    "type": "recurring", "unit_amount": 1000
                }
            })
        })
        .collect();
    json!({
        "id": "sub_test", "object": "subscription", "automatic_tax": {"enabled": false},
        "billing_cycle_anchor": 1720000000, "billing_mode": {"type": "classic"},
        "cancel_at_period_end": false, "collection_method": "charge_automatically",
        "created": 1720000000, "currency": "usd", "customer": "cus_test", "discounts": [],
        "invoice_settings": {"issuer": {"type": "self"}},
        "items": {
            "object": "list", "data": data, "has_more": false,
            "url": "/v1/subscription_items?subscription=sub_test"
        },
        "livemode": false, "metadata": {}, "start_date": 1720000000, "status": "active"
    })
    .to_string()
}

/// The body Stripe answers a refused request with.
fn stripe_error() -> String {
    json!({"error": {"message": "mocked error", "type": "invalid_request_error"}}).to_string()
}

/// The body Stripe answers with for an error of `code` whose message and
/// URLs name the entity and the request, as Stripe's do.
fn stripe_error_naming(id: &str, code: &str) -> String {
    json!({"error": {
        "message": format!("No such subscription: '{id}'"),
        "type": "invalid_request_error",
        "code": code,
        "param": "id",
        "doc_url": format!("https://stripe.com/docs/error-codes/{code}"),
        "request_log_url": "https://dashboard.stripe.com/test/logs/req_secret"
    }})
    .to_string()
}

/// The form of the update a change to `prices` sends for a subscription
/// that has the `current` `(item id, price id, quantity)` items.
async fn updated_form(current: &[(&str, &str, u64)], prices: &[&str]) -> HashMap<String, String> {
    let before = subscription(current);
    let after = subscription(&[]);
    let (provider, requests) = stripe_with(vec![(200, before), (200, after)]).await;

    let result = provider.update(price_change(prices, None)).await;

    assert!(result.is_ok(), "{result:?}");
    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 2);
    requests[1].form()
}

/// A price change of `sub_test` to `prices`.
fn price_change(prices: &[&str], proration: Option<Proration>) -> UpdateSubscriptionRequest {
    UpdateSubscriptionRequest {
        provider_subscription_id: "sub_test".into(),
        new_price_refs: Some(prices.iter().map(|price| (*price).to_string()).collect()),
        proration,
        cancel_at_period_end: None,
        idempotency_key: None,
    }
}

/// The form a map of `(key, value)` pairs names.
fn form(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

#[tokio::test]
async fn a_price_change_deletes_the_old_item_adds_the_new_price_and_leaves_the_rest() {
    let before = subscription(&[("si_old", "price_old", 1), ("si_keep", "price_keep", 3)]);
    let after = subscription(&[("si_keep", "price_keep", 3), ("si_new", "price_new", 1)]);
    let (provider, requests) = stripe_with(vec![(200, before), (200, after)]).await;
    let mut request = price_change(&["price_keep", "price_new"], None);
    request.idempotency_key = Some("price-change-key".into());

    let result = provider.update(request).await.expect("price change");

    let prices: Vec<&str> = result
        .items
        .iter()
        .map(|item| item.provider_price_id.as_str())
        .collect();
    assert_eq!(prices, ["price_keep", "price_new"]);
    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].request_line,
        "GET /v1/subscriptions/sub_test HTTP/1.1"
    );
    assert!(!requests[0].headers.contains_key("idempotency-key"));
    assert_eq!(
        requests[1].request_line,
        "POST /v1/subscriptions/sub_test HTTP/1.1"
    );
    let key = requests[1].headers.get("idempotency-key");
    assert_eq!(key.map(String::as_str), Some("price-change-key"));
    assert_eq!(
        requests[1].form(),
        form(&[
            ("items[0][id]", "si_old"),
            ("items[0][deleted]", "true"),
            ("items[1][price]", "price_new"),
            ("items[1][quantity]", "1"),
            ("proration_behavior", "create_prorations"),
        ])
    );
}

#[tokio::test]
async fn each_proration_mode_is_sent_as_its_stripe_proration_behavior() {
    for (proration, behavior) in [
        (None, "create_prorations"),
        (Some(Proration::ProrateAtRenewal), "create_prorations"),
        (Some(Proration::ProrateNow), "always_invoice"),
        (Some(Proration::DoNotProrate), "none"),
    ] {
        let before = subscription(&[("si_old", "price_old", 1)]);
        let after = subscription(&[("si_new", "price_new", 1)]);
        let (provider, requests) = stripe_with(vec![(200, before), (200, after)]).await;

        let result = provider
            .update(price_change(&["price_new"], proration))
            .await;

        assert!(result.is_ok(), "{proration:?}: {result:?}");
        let requests = requests.lock().expect("request log");
        assert_eq!(requests.len(), 2, "{proration:?}");
        let sent = requests[1].form();
        assert_eq!(
            sent.get("proration_behavior").map(String::as_str),
            Some(behavior),
            "{proration:?}"
        );
    }
}

#[tokio::test]
async fn a_price_change_and_a_scheduled_cancellation_go_in_one_update() {
    let before = subscription(&[("si_old", "price_old", 1)]);
    let after = subscription(&[("si_new", "price_new", 1)]);
    let (provider, requests) = stripe_with(vec![(200, before), (200, after)]).await;
    let mut request = price_change(&["price_new"], None);
    request.cancel_at_period_end = Some(true);

    let result = provider.update(request).await;

    assert!(result.is_ok(), "{result:?}");
    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].form(),
        form(&[
            ("items[0][id]", "si_old"),
            ("items[0][deleted]", "true"),
            ("items[1][price]", "price_new"),
            ("items[1][quantity]", "1"),
            ("proration_behavior", "create_prorations"),
            ("cancel_at_period_end", "true"),
        ])
    );
}

#[tokio::test]
async fn an_empty_or_repeated_price_set_is_refused_before_any_request() {
    for prices in [vec![], vec!["price_new", "price_new"]] {
        let current = subscription(&[("si_old", "price_old", 1)]);
        let (provider, requests) = stripe_with(vec![(200, current.clone()), (200, current)]).await;

        let result = provider.update(price_change(&prices, None)).await;

        assert!(
            matches!(result, Err(PaymentError::Validation(_))),
            "{prices:?}: {result:?}"
        );
        assert!(
            requests.lock().expect("request log").is_empty(),
            "{prices:?} reached Stripe"
        );
    }
}

#[tokio::test]
async fn a_refused_update_is_an_error() {
    let before = subscription(&[("si_old", "price_old", 1)]);
    let (provider, requests) = stripe_with(vec![(200, before), (400, stripe_error())]).await;

    let result = provider.update(price_change(&["price_new"], None)).await;

    assert!(
        matches!(result, Err(PaymentError::Provider(_))),
        "{result:?}"
    );
    assert_eq!(requests.lock().expect("request log").len(), 2);
}

#[tokio::test]
async fn a_failed_read_sends_no_update() {
    let after = subscription(&[("si_new", "price_new", 1)]);
    let (provider, requests) = stripe_with(vec![(400, stripe_error()), (200, after)]).await;

    let result = provider.update(price_change(&["price_new"], None)).await;

    assert!(
        matches!(result, Err(PaymentError::Provider(_))),
        "{result:?}"
    );
    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 1, "an update was sent");
    assert!(requests[0].request_line.starts_with("GET "));
}

#[tokio::test]
async fn the_same_prices_send_no_update() {
    let current = subscription(&[("si_keep", "price_keep", 3)]);
    let (provider, requests) = stripe_with(vec![(200, current.clone()), (200, current)]).await;

    let result = provider
        .update(price_change(&["price_keep"], None))
        .await
        .expect("unchanged prices");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].quantity, 3);
    assert_eq!(requests.lock().expect("request log").len(), 1);
}

#[tokio::test]
async fn a_cancel_that_runs_out_of_time_has_an_unknown_outcome() {
    let result = stalled(|provider| async move { provider.cancel("sub_test", false).await }).await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert!(message.contains("timed out"), "{message}");
    assert!(message.contains("outcome is unknown"), "{message}");
    assert!(!message.contains("failed"), "{message}");
}

#[tokio::test]
async fn a_read_that_runs_out_of_time_is_an_error() {
    let result = stalled(|provider| async move { provider.get("sub_test").await }).await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert!(
        message.contains("subscriptions.retrieve timed out"),
        "{message}"
    );
}

#[tokio::test]
async fn a_swap_of_one_price_for_another_keeps_the_quantity() {
    let sent = updated_form(&[("si_a", "price_a", 10)], &["price_b"]).await;

    assert_eq!(
        sent,
        form(&[
            ("items[0][id]", "si_a"),
            ("items[0][deleted]", "true"),
            ("items[1][price]", "price_b"),
            ("items[1][quantity]", "10"),
            ("proration_behavior", "create_prorations"),
        ])
    );
}

#[tokio::test]
async fn a_swap_keeps_the_quantity_of_the_price_it_replaces_not_of_the_kept_ones() {
    let current = [("si_a", "price_a", 10), ("si_keep", "price_keep", 3)];

    let sent = updated_form(&current, &["price_keep", "price_b"]).await;

    assert_eq!(
        sent,
        form(&[
            ("items[0][id]", "si_a"),
            ("items[0][deleted]", "true"),
            ("items[1][price]", "price_b"),
            ("items[1][quantity]", "10"),
            ("proration_behavior", "create_prorations"),
        ])
    );
}

#[tokio::test]
async fn one_price_replaced_by_two_gives_both_new_prices_quantity_one() {
    let sent = updated_form(&[("si_a", "price_a", 10)], &["price_b", "price_c"]).await;

    assert_eq!(
        sent,
        form(&[
            ("items[0][id]", "si_a"),
            ("items[0][deleted]", "true"),
            ("items[1][price]", "price_b"),
            ("items[1][quantity]", "1"),
            ("items[2][price]", "price_c"),
            ("items[2][quantity]", "1"),
            ("proration_behavior", "create_prorations"),
        ])
    );
}

#[tokio::test]
async fn two_prices_replaced_by_one_gives_the_new_price_quantity_one() {
    let current = [("si_a", "price_a", 10), ("si_b", "price_b", 4)];

    let sent = updated_form(&current, &["price_c"]).await;

    assert_eq!(
        sent.get("items[2][price]").map(String::as_str),
        Some("price_c")
    );
    assert_eq!(
        sent.get("items[2][quantity]").map(String::as_str),
        Some("1")
    );
}

#[tokio::test]
async fn a_price_added_only_sends_the_new_item_and_leaves_the_others() {
    let sent = updated_form(
        &[("si_keep", "price_keep", 3)],
        &["price_keep", "price_new"],
    )
    .await;

    assert_eq!(
        sent,
        form(&[
            ("items[0][price]", "price_new"),
            ("items[0][quantity]", "1"),
            ("proration_behavior", "create_prorations"),
        ])
    );
}

#[tokio::test]
async fn a_price_removed_only_sends_the_deletion_and_leaves_the_others() {
    let current = [("si_keep", "price_keep", 3), ("si_old", "price_old", 2)];

    let sent = updated_form(&current, &["price_keep"]).await;

    assert_eq!(
        sent,
        form(&[
            ("items[0][id]", "si_old"),
            ("items[0][deleted]", "true"),
            ("proration_behavior", "create_prorations"),
        ])
    );
}

#[tokio::test]
async fn a_subscription_with_more_than_one_page_of_items_is_refused_before_any_write() {
    let mut paged: serde_json::Value =
        serde_json::from_str(&subscription(&[("si_old", "price_old", 1)])).expect("fixture JSON");
    paged["items"]["has_more"] = json!(true);
    let (provider, requests) =
        stripe_with(vec![(200, paged.to_string()), (200, subscription(&[]))]).await;

    let result = provider.update(price_change(&["price_new"], None)).await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert!(message.contains("more than one page"), "{message}");
    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 1, "a write was sent");
    assert!(requests[0].request_line.starts_with("GET "));
}

#[tokio::test]
async fn a_price_change_that_runs_out_of_time_has_an_unknown_outcome() {
    let before = subscription(&[("si_old", "price_old", 1)]);

    let result = stalled_after(vec![(200, before)], |provider| async move {
        provider.update(price_change(&["price_new"], None)).await
    })
    .await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert!(
        message.contains("subscriptions.update timed out"),
        "{message}"
    );
    assert!(message.contains("outcome is unknown"), "{message}");
    assert!(!message.contains("failed"), "{message}");
}

#[tokio::test]
async fn an_error_of_stripe_carries_its_kind_and_code_but_no_message_id_or_url() {
    let refused = stripe_error_naming("sub_secret_id", "resource_missing");
    let (provider, _) = stripe_with(vec![(404, refused)]).await;

    let result = provider.get("sub_secret_id").await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert_eq!(
        message,
        "stripe subscriptions.retrieve: api error invalid_request_error resource_missing, \
         status 404"
    );
}

#[tokio::test]
async fn a_transport_error_carries_neither_the_id_nor_the_url() {
    let provider = unreachable().await;

    let result = provider.get("sub_secret_id").await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert!(
        message.starts_with("stripe subscriptions.retrieve: "),
        "{message}"
    );
    assert!(!message.contains("sub_secret_id"), "{message}");
    assert!(!message.contains("127.0.0.1"), "{message}");
    assert!(!message.contains("http"), "{message}");
}
