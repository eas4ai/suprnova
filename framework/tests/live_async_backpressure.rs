//! Bounded fan-in, fairness, coalescing, cancellation, and membership ceilings.

mod live_async_support;

use hyper::StatusCode;
use live_async_support::*;
use serde_json::Value;
use suprnova::live::testing::{
    await_async_transport_retirement_for_test, inspect_async_transports_for_test,
};
use suprnova::live::{CanonicalValue, LiveEventTarget, LiveStreams};

/// Chatty deliveries published before the quiet sibling joins the transport.
const BACKLOG: usize = 40;

async fn next_envelope(stream: &mut SseClient) -> Value {
    loop {
        let record = stream.next_record().await.expect("stream stays open");
        if let Some(data) = record.data {
            let value: Value = serde_json::from_str(&data).expect("envelope JSON");
            if value["payload"]["kind"] != "heartbeat" {
                return value;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn a_chatty_island_cannot_starve_its_sibling_on_one_transport() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let orders = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let inventory = issue(
        server.port,
        &alice,
        inventory_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = orders.credential.clone().expect("bearer");
    let streams = LiveStreams::resolve().expect("Live streams publisher");
    for index in 0..BACKLOG {
        streams
            .event::<OrdersUpdated>(
                "orders",
                LiveEventTarget::Document,
                CanonicalValue::String(format!("backlog-{index}")),
            )
            .await
            .expect("publish backlog");
    }
    streams.refresh("inventory").await.expect("publish refresh");

    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;
    assert_eq!(
        subscribe(server.port, &alice, &credential, &orders, "nonce-orders", 1)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        subscribe(
            server.port,
            &alice,
            &credential,
            &inventory,
            "nonce-inventory",
            1
        )
        .await
        .status,
        StatusCode::OK
    );

    let mut seen_inventory_at = None;
    let mut orders_seen = 0;
    for index in 0..41 {
        let envelope = next_envelope(&mut stream).await;
        if envelope["subscription"] == inventory.subscription_id {
            seen_inventory_at = Some(index);
            break;
        }
        orders_seen += 1;
    }
    let position = seen_inventory_at.expect("the quiet island is served");
    // The chatty island joined first and its whole backlog may already have
    // been admitted before the sibling existed, so the bound is that backlog:
    // the sibling is never held behind deliveries admitted after it joined.
    assert!(
        position <= BACKLOG,
        "the sibling was served only after {orders_seen} chatty deliveries"
    );

    for index in 0..40 {
        streams
            .event::<OrdersUpdated>(
                "orders",
                LiveEventTarget::Document,
                CanonicalValue::String(format!("live-{index}")),
            )
            .await
            .expect("publish live burst");
    }
    streams
        .event::<StockChanged>(
            "inventory",
            LiveEventTarget::Island,
            CanonicalValue::String("restock".to_owned()),
        )
        .await
        .expect("publish sibling event");
    let mut deliveries_before_sibling = 0;
    loop {
        let envelope = next_envelope(&mut stream).await;
        if envelope["subscription"] == inventory.subscription_id
            && envelope["payload"]["kind"] == "browser_event"
        {
            break;
        }
        deliveries_before_sibling += 1;
        assert!(
            deliveries_before_sibling < 80,
            "the sibling event never surfaced behind the chatty backlog"
        );
    }
    assert!(
        deliveries_before_sibling <= 42,
        "fairness bounds the sibling's wait to the backlog already queued"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn refresh_bursts_coalesce_and_the_document_queue_stays_bounded() {
    let (router, runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let orders = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = orders.credential.clone().expect("bearer");
    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;
    assert_eq!(
        subscribe(server.port, &alice, &credential, &orders, "nonce-1", 1)
            .await
            .status,
        StatusCode::OK
    );

    let streams = LiveStreams::resolve().expect("Live streams publisher");
    for _ in 0..200 {
        streams.refresh("orders").await.expect("publish refresh");
    }
    for index in 0..200 {
        streams
            .event::<OrdersUpdated>(
                "orders",
                LiveEventTarget::Document,
                CanonicalValue::String(format!("burst-{index}")),
            )
            .await
            .expect("publish event");
    }
    let reports = inspect_async_transports_for_test(&runtime);
    let report = reports
        .iter()
        .find(|report| report.credential_matches(&credential))
        .expect("open transport");
    assert_eq!(report.memberships, 1);
    assert!(
        report.retained_events <= 64,
        "retained {} events",
        report.retained_events
    );
    assert!(
        report.retained_bytes <= 256 * 1024,
        "retained {} bytes",
        report.retained_bytes
    );

    let mut refreshes = 0;
    let mut events = 0;
    let mut degraded = false;
    while events < 10 {
        let envelope = next_envelope(&mut stream).await;
        match envelope["payload"]["kind"].as_str().expect("payload kind") {
            "refresh" => refreshes += 1,
            "browser_event" => events += 1,
            "error" => {
                assert_eq!(envelope["payload"]["code"], "backpressure");
                degraded = true;
                break;
            }
            other => panic!("unexpected payload kind {other}"),
        }
    }
    assert!(
        refreshes < 200,
        "{refreshes} refreshes reached the wire instead of coalescing"
    );
    let reports = inspect_async_transports_for_test(&runtime);
    let report = reports
        .iter()
        .find(|report| report.credential_matches(&credential))
        .expect("open transport");
    assert!(report.retained_events <= 64);
    // Reading envelopes is the barrier proving the document drained the burst.
    assert!(report.coalesced > 0, "burst refreshes coalesce");
    assert!(
        degraded || report.degraded || events == 10,
        "delivery either drained or degraded truthfully"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn membership_ceilings_and_retirement_bound_every_transport() {
    let (router, runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let first = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = first.credential.clone().expect("bearer");
    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;

    let mut issued = vec![first];
    for _ in 1..128 {
        issued.push(
            issue(
                server.port,
                &alice,
                orders_issue_body("sse", "doc-instance-0001"),
            )
            .await,
        );
    }
    for (index, subscription) in issued.iter().enumerate() {
        let reply = subscribe(
            server.port,
            &alice,
            &credential,
            subscription,
            &format!("nonce-{index}"),
            1,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "membership {index}");
    }
    let overflow = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let reply = subscribe(
        server.port,
        &alice,
        &credential,
        &overflow,
        "nonce-overflow",
        1,
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error_code(), "async_membership_limit");

    let report = inspect_async_transports_for_test(&runtime)
        .into_iter()
        .find(|report| report.credential_matches(&credential))
        .expect("open transport");
    assert_eq!(report.memberships, 128);
    assert!(report.reader_active);

    drop(stream);
    await_async_transport_retirement_for_test(&runtime, &credential).await;
    assert!(
        inspect_async_transports_for_test(&runtime)
            .iter()
            .all(|report| !report.credential_matches(&credential) || !report.reader_active),
        "a disconnected reader releases its memberships"
    );
    let report = inspect_async_transports_for_test(&runtime)
        .into_iter()
        .find(|report| report.credential_matches(&credential));
    assert!(
        report.is_none_or(|report| report.memberships == 0),
        "retirement releases every membership exactly once"
    );
}
