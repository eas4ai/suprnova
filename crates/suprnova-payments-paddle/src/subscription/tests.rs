//! `Subscription` against the mocked Paddle API: price changes, cancels and
//! the deadline of a call.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use suprnova::payments::{
    PaymentError, Proration, Subscription, SubscriptionStatus, UpdateSubscriptionRequest,
};

use crate::mocked_paddle::{
    paddle_error, paddle_error_with_detail, paddle_with, stalled, unreachable,
};

/// The subscription `sub_test` as Paddle returns it, with one item per
/// `(price id, quantity)`.
fn subscription(items: &[(&str, i64)]) -> Value {
    let entries: Vec<Value> = items
        .iter()
        .map(|(price_id, quantity)| {
            json!({
                "status": "active", "quantity": quantity, "recurring": true,
                "created_at": "2026-09-01T12:00:00Z", "updated_at": "2026-09-01T12:00:00Z",
                "price": {
                    "id": price_id, "product_id": "pro_test", "description": "Monthly plan",
                    "type": "standard", "tax_mode": "account_setting",
                    "unit_price": {"amount": "1000", "currency_code": "USD"},
                    "quantity": {"minimum": 1, "maximum": 100}, "status": "active",
                    "created_at": "2026-09-01T12:00:00Z", "updated_at": "2026-09-01T12:00:00Z"
                },
                "product": {
                    "id": "pro_test", "name": "Plan", "type": "standard",
                    "tax_category": "standard", "status": "active",
                    "created_at": "2026-09-01T12:00:00Z", "updated_at": "2026-09-01T12:00:00Z"
                }
            })
        })
        .collect();
    json!({"data": {
        "id": "sub_test", "status": "active", "customer_id": "ctm_test",
        "address_id": "add_test", "currency_code": "USD", "collection_mode": "automatic",
        "created_at": "2026-09-01T12:00:00Z", "updated_at": "2026-09-01T12:00:00Z",
        "billing_cycle": {"interval": "month", "frequency": 1},
        "current_billing_period": {
            "starts_at": "2026-09-01T12:00:00Z", "ends_at": "2026-10-01T12:00:00Z"
        },
        "items": entries
    }, "meta": {"request_id": "test-request"}})
}

/// The `items` of the PATCH that a change to `prices` sends for a
/// subscription that has the `current` `(price id, quantity)` items.
async fn patched_items(current: &[(&str, i64)], prices: &[&str]) -> Value {
    let before = subscription(current);
    let after = subscription(&[]);
    let (provider, requests) = paddle_with(vec![(200, before), (200, after)]).await;

    let result = provider.update(price_change(prices, None)).await;

    assert!(result.is_ok(), "{result:?}");
    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 2);
    requests[1].json()["items"].clone()
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

#[tokio::test]
async fn a_price_change_patches_the_new_items_with_the_kept_quantities() {
    let before = subscription(&[("pri_old", 1), ("pri_keep", 3)]);
    let after = subscription(&[("pri_keep", 3), ("pri_new", 1)]);
    let (provider, requests) = paddle_with(vec![(200, before), (200, after)]).await;
    let request = price_change(&["pri_keep", "pri_new"], None);

    let result = provider.update(request).await.expect("price change");

    let prices: Vec<&str> = result
        .items
        .iter()
        .map(|item| item.provider_price_id.as_str())
        .collect();
    assert_eq!(prices, ["pri_keep", "pri_new"]);
    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 2);
    let read = &requests[0].request_line;
    assert!(read.starts_with("GET /subscriptions/sub_test"), "{read}");
    assert_eq!(
        requests[1].request_line,
        "PATCH /subscriptions/sub_test HTTP/1.1"
    );
    assert_eq!(
        requests[1].json(),
        json!({
            "items": [
                {"price_id": "pri_keep", "quantity": 3},
                {"price_id": "pri_new", "quantity": 1}
            ],
            "proration_billing_mode": "prorated_next_billing_period"
        })
    );
}

#[tokio::test]
async fn each_proration_mode_is_sent_as_its_paddle_proration_billing_mode() {
    for (proration, mode) in [
        (None, "prorated_next_billing_period"),
        (
            Some(Proration::ProrateAtRenewal),
            "prorated_next_billing_period",
        ),
        (Some(Proration::ProrateNow), "prorated_immediately"),
        (Some(Proration::DoNotProrate), "do_not_bill"),
    ] {
        let before = subscription(&[("pri_old", 1)]);
        let after = subscription(&[("pri_new", 1)]);
        let (provider, requests) = paddle_with(vec![(200, before), (200, after)]).await;
        let request = price_change(&["pri_new"], proration);

        let result = provider.update(request).await;

        assert!(result.is_ok(), "{proration:?}: {result:?}");
        let requests = requests.lock().expect("request log");
        assert_eq!(requests.len(), 2, "{proration:?}");
        let sent = requests[1].json();
        assert_eq!(sent["proration_billing_mode"], mode, "{proration:?}");
    }
}

#[tokio::test]
async fn an_empty_or_repeated_price_set_is_refused_before_any_request() {
    for prices in [vec![], vec!["pri_new", "pri_new"]] {
        let current = subscription(&[("pri_old", 1)]);
        let (provider, requests) = paddle_with(vec![(200, current.clone()), (200, current)]).await;

        let result = provider.update(price_change(&prices, None)).await;

        assert!(
            matches!(result, Err(PaymentError::Validation(_))),
            "{prices:?}: {result:?}"
        );
        assert!(
            requests.lock().expect("request log").is_empty(),
            "{prices:?} reached Paddle"
        );
    }
}

#[tokio::test]
async fn a_price_change_with_a_cancellation_or_a_key_is_refused_before_any_request() {
    let mut with_cancellation = price_change(&["pri_new"], None);
    with_cancellation.cancel_at_period_end = Some(true);
    let mut with_key = price_change(&["pri_new"], None);
    with_key.idempotency_key = Some("price-change-key".into());
    for request in [with_cancellation, with_key] {
        let current = subscription(&[("pri_old", 1)]);
        let (provider, requests) = paddle_with(vec![(200, current.clone()), (200, current)]).await;

        let result = provider.update(request).await;

        assert!(
            matches!(result, Err(PaymentError::NotSupported(_))),
            "{result:?}"
        );
        assert!(requests.lock().expect("request log").is_empty());
    }
}

#[tokio::test]
async fn a_refused_update_is_an_error() {
    let before = subscription(&[("pri_old", 1)]);
    let refused = paddle_error("invalid_field");
    let (provider, requests) = paddle_with(vec![(200, before), (400, refused)]).await;

    let result = provider.update(price_change(&["pri_new"], None)).await;

    assert!(
        matches!(result, Err(PaymentError::Provider(_))),
        "{result:?}"
    );
    assert_eq!(requests.lock().expect("request log").len(), 2);
}

#[tokio::test]
async fn a_failed_read_sends_no_update() {
    let after = subscription(&[("pri_new", 1)]);
    let missing = paddle_error("not_found");
    let (provider, requests) = paddle_with(vec![(404, missing), (200, after)]).await;

    let result = provider.update(price_change(&["pri_new"], None)).await;

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
    let current = subscription(&[("pri_keep", 3)]);
    let (provider, requests) = paddle_with(vec![(200, current.clone()), (200, current)]).await;

    let result = provider
        .update(price_change(&["pri_keep"], None))
        .await
        .expect("unchanged prices");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].quantity, 3);
    assert_eq!(requests.lock().expect("request log").len(), 1);
}

/// The end of the billing period of the `subscription` fixture.
fn period_end() -> DateTime<Utc> {
    "2026-10-01T12:00:00Z".parse().expect("fixture period end")
}

#[tokio::test]
async fn a_cancel_at_the_end_of_the_period_keeps_the_subscription_active() {
    let mut scheduled = subscription(&[("pri_keep", 1)]);
    scheduled["data"]["scheduled_change"] =
        json!({"action": "cancel", "effective_at": "2026-10-01T12:00:00Z"});
    let (provider, requests) = paddle_with(vec![(200, scheduled)]).await;

    let result = provider.cancel("sub_test", true).await.expect("cancel");

    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request_line,
        "POST /subscriptions/sub_test/cancel HTTP/1.1"
    );
    assert_eq!(
        requests[0].json(),
        json!({"effective_from": "next_billing_period"})
    );
    assert_eq!(result.status, SubscriptionStatus::Active);
    assert!(result.cancel_at_period_end);
    assert_eq!(result.current_period_end, period_end());
}

#[tokio::test]
async fn a_cancel_at_once_ends_the_subscription_in_the_period_it_was_in() {
    let before = subscription(&[("pri_keep", 1)]);
    let mut canceled = subscription(&[("pri_keep", 1)]);
    canceled["data"]["status"] = json!("canceled");
    canceled["data"]["canceled_at"] = json!("2026-09-28T12:00:00Z");
    canceled["data"]["current_billing_period"] = Value::Null;
    let (provider, requests) = paddle_with(vec![(200, before), (200, canceled)]).await;

    let result = provider.cancel("sub_test", false).await.expect("cancel");

    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 2);
    let read = &requests[0].request_line;
    assert!(read.starts_with("GET /subscriptions/sub_test"), "{read}");
    assert_eq!(
        requests[1].request_line,
        "POST /subscriptions/sub_test/cancel HTTP/1.1"
    );
    assert_eq!(requests[1].json(), json!({"effective_from": "immediately"}));
    assert_eq!(result.status, SubscriptionStatus::Canceled);
    assert!(!result.cancel_at_period_end);
    assert_eq!(result.current_period_end, period_end());
}

#[tokio::test]
async fn a_cancel_at_once_whose_read_fails_cancels_nothing() {
    let missing = paddle_error("not_found");
    let (provider, requests) = paddle_with(vec![(404, missing), (200, subscription(&[]))]).await;

    let result = provider.cancel("sub_test", false).await;

    assert!(
        matches!(result, Err(PaymentError::Provider(_))),
        "{result:?}"
    );
    assert_eq!(requests.lock().expect("request log").len(), 1);
}

#[tokio::test]
async fn a_cancel_that_runs_out_of_time_has_an_unknown_outcome() {
    let result = stalled(|provider| async move { provider.cancel("sub_test", true).await }).await;

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
    assert!(message.contains("subscription_get timed out"), "{message}");
}

#[tokio::test]
async fn a_swap_of_one_price_for_another_keeps_the_quantity() {
    let items = patched_items(&[("pri_a", 10)], &["pri_b"]).await;

    assert_eq!(items, json!([{"price_id": "pri_b", "quantity": 10}]));
}

#[tokio::test]
async fn a_swap_keeps_the_quantity_of_the_price_it_replaces_and_of_the_kept_ones() {
    let items = patched_items(&[("pri_a", 10), ("pri_keep", 3)], &["pri_keep", "pri_b"]).await;

    assert_eq!(
        items,
        json!([
            {"price_id": "pri_keep", "quantity": 3},
            {"price_id": "pri_b", "quantity": 10}
        ])
    );
}

#[tokio::test]
async fn one_price_replaced_by_two_gives_both_new_prices_quantity_one() {
    let items = patched_items(&[("pri_a", 10)], &["pri_b", "pri_c"]).await;

    assert_eq!(
        items,
        json!([
            {"price_id": "pri_b", "quantity": 1},
            {"price_id": "pri_c", "quantity": 1}
        ])
    );
}

#[tokio::test]
async fn two_prices_replaced_by_one_gives_the_new_price_quantity_one() {
    let items = patched_items(&[("pri_a", 10), ("pri_b", 4)], &["pri_c"]).await;

    assert_eq!(items, json!([{"price_id": "pri_c", "quantity": 1}]));
}

#[tokio::test]
async fn a_price_added_only_keeps_the_others_and_starts_at_quantity_one() {
    let items = patched_items(&[("pri_keep", 3)], &["pri_keep", "pri_new"]).await;

    assert_eq!(
        items,
        json!([
            {"price_id": "pri_keep", "quantity": 3},
            {"price_id": "pri_new", "quantity": 1}
        ])
    );
}

#[tokio::test]
async fn a_price_removed_only_is_left_out_of_the_list() {
    let items = patched_items(&[("pri_keep", 3), ("pri_old", 2)], &["pri_keep"]).await;

    assert_eq!(items, json!([{"price_id": "pri_keep", "quantity": 3}]));
}

#[tokio::test]
async fn a_cancellation_update_without_prices_cancels_from_the_next_billing_period() {
    let mut scheduled = subscription(&[("pri_keep", 1)]);
    scheduled["data"]["scheduled_change"] =
        json!({"action": "cancel", "effective_at": "2026-10-01T12:00:00Z"});
    let (provider, requests) = paddle_with(vec![(200, scheduled)]).await;
    let request = UpdateSubscriptionRequest {
        provider_subscription_id: "sub_test".into(),
        new_price_refs: None,
        proration: None,
        cancel_at_period_end: Some(true),
        idempotency_key: None,
    };

    let result = provider.update(request).await.expect("cancellation");

    let requests = requests.lock().expect("request log");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request_line,
        "POST /subscriptions/sub_test/cancel HTTP/1.1"
    );
    assert_eq!(
        requests[0].json(),
        json!({"effective_from": "next_billing_period"})
    );
    assert!(result.cancel_at_period_end);
}

#[tokio::test]
async fn an_error_of_paddle_carries_its_type_and_code_but_not_its_detail() {
    let refused = paddle_error_with_detail("conflict", "subscription sub_secret_id is locked");
    let (provider, _) = paddle_with(vec![(409, refused)]).await;

    let result = provider.get("sub_secret_id").await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert!(
        message.starts_with("paddle subscription_get: "),
        "{message}"
    );
    assert!(message.contains("request_error conflict"), "{message}");
    assert!(!message.contains("sub_secret_id"), "{message}");
}

#[tokio::test]
async fn a_transport_error_carries_neither_the_id_nor_the_url() {
    let provider = unreachable().await;

    let result = provider.cancel("sub_secret_id", true).await;

    let Err(PaymentError::Provider(message)) = &result else {
        panic!("expected a provider error: {result:?}");
    };
    assert!(
        message.starts_with("paddle subscription_cancel: request error"),
        "{message}"
    );
    assert!(!message.contains("sub_secret_id"), "{message}");
    assert!(!message.contains("127.0.0.1"), "{message}");
    assert!(!message.contains("http"), "{message}");
}
