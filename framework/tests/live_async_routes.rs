//! Real Suprnova routes for Live subscriptions, SSE, WebSocket, and fallback polling.

mod live_async_support;

use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use hyper::{Method, StatusCode};
use live_async_support::*;
use serde_json::{Value, json};
use suprnova::live::{CanonicalValue, LiveEventTarget, LiveStreams};

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis(),
    )
    .expect("millis fit")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn issuance_returns_a_document_scoped_bearer_subscription() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    assert_eq!(
        reply.headers.get("cache-control").expect("Cache-Control"),
        "no-store"
    );
    let value = reply.json();
    assert_eq!(value["proof"], "authoritative_no_tail");
    assert!(value["replay"].as_array().expect("replay").is_empty());
    let subscription = &value["subscription"];
    assert!(
        subscription.get("descriptor").is_none(),
        "the signed descriptor never leaves the server"
    );
    assert_eq!(subscription["authorization"]["kind"], "bearer");
    let credential = subscription["authorization"]["credential"]
        .as_str()
        .expect("bearer credential");
    assert!((16..=1024).contains(&credential.len()));
    assert_eq!(subscription["stream"], "orders");
    assert_eq!(subscription["document"]["transport"], "sse");
    assert_eq!(subscription["document"]["origin"], server.origin());
    assert!(
        subscription["document"]["authorization_scope"]
            .as_str()
            .expect("document scope")
            .len()
            >= 16
    );
    assert!(
        subscription["baseline"]["epoch"]
            .as_str()
            .expect("epoch")
            .parse::<u64>()
            .expect("decimal epoch")
            >= 1
    );
    assert_eq!(subscription["baseline"]["sequence"], "0");
    assert!(subscription["expires_at"].as_u64().expect("expires_at") > now_ms());
    assert!(
        (22..=43).contains(&subscription["subscription_id"].as_str().expect("id").len()),
        "subscription identity is canonical base64url"
    );
    assert!(
        (22..=43).contains(
            &subscription["descriptor_binding"]
                .as_str()
                .expect("binding")
                .len()
        )
    );

    let events = subscription["events"].as_array().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "orders.updated");
    assert_eq!(events[0]["version"], 1);
    assert_eq!(events[0]["source"], "stream");
    assert_eq!(events[0]["order"], "per_source_sequence");
    assert_eq!(events[0]["schema"], "json");
    assert_eq!(events[0]["payloadContract"], "orders.updated");
    assert_eq!(events[0]["maximumFanout"], 4);
    assert_eq!(
        events[0]["cycle"],
        json!({ "kind": "forbid_repeated_island" })
    );
    assert_eq!(events[0]["targets"], json!(["self", "document"]));

    assert_eq!(subscription["fallback_poll"]["interval_ms"], 30_000);
    assert_eq!(subscription["fallback_poll"]["initial"], "wait");
    assert_eq!(subscription["fallback_poll"]["visibility"], "visible");
    assert_eq!(subscription["fallback_poll"]["jitter_ratio"], 0.2);
    assert_eq!(subscription["heartbeat_timeout_ms"], 15_000);
    assert_eq!(subscription["reconnect"]["kind"], "resume_or_refresh");
    assert_eq!(subscription["reconnect"]["maximum_attempts"], 4);
    assert_eq!(subscription["reconnect"]["minimum_delay_ms"], 250);
    assert_eq!(subscription["reconnect"]["maximum_delay_ms"], 5_000);
    assert!(
        subscription["presentation_signals"]
            .as_array()
            .expect("signals")
            .is_empty()
    );

    let sibling = issue(
        server.port,
        &alice,
        inventory_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    assert_eq!(
        sibling.credential.as_deref(),
        Some(credential),
        "islands of one browser document share one transport credential"
    );
    assert_ne!(sibling.subscription_id, subscription["subscription_id"]);
    assert_eq!(
        sibling.value["subscription"]["reconnect"]["kind"],
        "refresh_on_reconnect"
    );
    let other_document = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0002"),
    )
    .await;
    assert_ne!(
        other_document.credential.as_deref(),
        Some(credential),
        "a second browser document owns its own transport credential"
    );

    let poll = send(
        server.port,
        &alice,
        Method::POST,
        "/__live/v1/async/poll",
        &[("content-type", "application/json")],
        Bytes::from_static(b"{}"),
    )
    .await;
    assert_eq!(
        poll.status,
        StatusCode::NOT_FOUND,
        "polling is the ordinary fresh-render route, never a dedicated poll route"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn one_sse_transport_multiplexes_two_islands_and_delivers_typed_events_in_order() {
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

    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    assert_eq!(
        stream.headers.get("content-type").expect("content type"),
        "text/event-stream; charset=utf-8"
    );
    assert_eq!(
        stream.headers.get("cache-control").expect("cache control"),
        "no-store, no-transform"
    );
    assert_eq!(
        stream
            .headers
            .get("x-content-type-options")
            .expect("nosniff"),
        "nosniff"
    );
    assert_eq!(
        stream.headers.get("x-accel-buffering").expect("buffering"),
        "no"
    );
    let opened = stream.next_record().await.expect("stream opened");
    assert_eq!(opened.comment.as_deref(), Some("suprnova-live heartbeat"));

    let ack = subscribe(
        server.port,
        &alice,
        &credential,
        &orders,
        "nonce-orders-1",
        1,
    )
    .await;
    assert_eq!(
        ack.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&ack.body)
    );
    assert_eq!(
        ack.json(),
        json!({
            "kind": "authenticated",
            "operation": "subscribe",
            "subscription_id": orders.subscription_id,
            "descriptor_binding": orders.descriptor_binding,
            "stream": "orders",
            "control_nonce": "nonce-orders-1",
            "transport_generation": 1,
        })
    );
    let ack = subscribe(
        server.port,
        &alice,
        &credential,
        &inventory,
        "nonce-inventory-1",
        1,
    )
    .await;
    assert_eq!(ack.status, StatusCode::OK);

    let streams = LiveStreams::resolve().expect("Live streams publisher");
    streams
        .event::<OrdersUpdated>(
            "orders",
            LiveEventTarget::Document,
            CanonicalValue::String("first".to_owned()),
        )
        .await
        .expect("publish first event");
    streams.refresh("inventory").await.expect("publish refresh");
    streams
        .event::<OrdersUpdated>(
            "orders/alice",
            LiveEventTarget::Island,
            CanonicalValue::String("second".to_owned()),
        )
        .await
        .expect("publish second event");

    let (record, first) = next_envelope(&mut stream).await;
    assert_eq!(record.event.as_deref(), Some("suprnova-live-async"));
    assert_eq!(
        record.id.as_deref(),
        Some(format!("{}/{}/1", orders.subscription_id, orders.baseline.0).as_str())
    );
    assert_eq!(first["protocol_version"], 1);
    assert_eq!(first["subscription"], orders.subscription_id);
    assert_eq!(first["stream"], "orders");
    assert_eq!(first["position"]["sequence"], "1");
    assert_eq!(
        first["payload"],
        json!({
            "event": "orders.updated",
            "kind": "browser_event",
            "payload": "first",
            "schema_version": 1,
            "target": "document",
        })
    );
    let (_, refresh) = next_envelope(&mut stream).await;
    assert_eq!(refresh["subscription"], inventory.subscription_id);
    assert_eq!(refresh["stream"], "inventory");
    assert_eq!(
        refresh["payload"],
        json!({ "kind": "refresh", "name": "refresh" })
    );
    let (_, second) = next_envelope(&mut stream).await;
    assert_eq!(second["subscription"], orders.subscription_id);
    assert_eq!(second["position"]["sequence"], "2");
    assert_eq!(second["payload"]["target"], "self");
    assert_eq!(second["payload"]["payload"], "second");

    let unsubscribe = post_control(
        server.port,
        &alice,
        MEMBERSHIP_PATH,
        Some(&credential),
        membership_body("unsubscribe", &inventory, "nonce-inventory-2", 1),
    )
    .await;
    assert_eq!(unsubscribe.status, StatusCode::OK);
    assert_eq!(unsubscribe.json()["operation"], "unsubscribe");
    streams.refresh("inventory").await.expect("publish refresh");
    streams
        .event::<OrdersUpdated>(
            "orders",
            LiveEventTarget::Document,
            CanonicalValue::String("third".to_owned()),
        )
        .await
        .expect("publish third event");
    let (_, third) = next_envelope(&mut stream).await;
    assert_eq!(
        third["subscription"], orders.subscription_id,
        "a removed membership receives nothing more"
    );
    assert_eq!(third["payload"]["payload"], "third");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn a_productive_sse_batch_is_followed_by_a_comment_trailer() {
    let (router, _runtime) = router_and_runtime();
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
    let opened = stream.next_record().await.expect("stream opened");
    assert_eq!(opened.comment.as_deref(), Some("suprnova-live heartbeat"));
    let ack = subscribe(
        server.port,
        &alice,
        &credential,
        &orders,
        "nonce-orders-1",
        1,
    )
    .await;
    assert_eq!(ack.status, StatusCode::OK);

    let streams = LiveStreams::resolve().expect("Live streams publisher");
    streams
        .event::<OrdersUpdated>(
            "orders",
            LiveEventTarget::Document,
            CanonicalValue::String("first".to_owned()),
        )
        .await
        .expect("publish first event");
    let (_, first) = next_envelope(&mut stream).await;
    assert_eq!(first["payload"]["payload"], "first");

    // WebKit hands a fetch stream's buffered bytes to the page only when more
    // bytes arrive, so every productive batch is followed shortly by a
    // non-authoritative comment, long before any idle heartbeat is due.
    let trailer = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next_record())
        .await
        .expect("a comment trailer follows the batch within its delay")
        .expect("stream stays open");
    assert_eq!(trailer.comment.as_deref(), Some("suprnova-live heartbeat"));
    assert!(trailer.data.is_none(), "the trailer carries no envelope");
}

async fn next_envelope(stream: &mut SseClient) -> (SseRecord, Value) {
    loop {
        let record = stream.next_record().await.expect("stream stays open");
        if let Some(data) = record.data.clone() {
            let value: Value = serde_json::from_str(&data).expect("envelope JSON");
            if value["payload"]["kind"] != "heartbeat" {
                return (record, value);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn renewal_replays_the_bounded_log_tail_and_reissues_authority() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let streams = LiveStreams::resolve().expect("Live streams publisher");
    for index in 0..3 {
        streams
            .event::<OrdersUpdated>(
                "orders",
                LiveEventTarget::Document,
                CanonicalValue::String(format!("event-{index}")),
            )
            .await
            .expect("publish");
    }

    let renew = |prior: &Issued, sequence: &str| {
        json!({
            "protocol_version": 1,
            "operation": "renew",
            "transport": "sse",
            "stream": "orders",
            "island": {
                "component": ORDERS_COMPONENT,
                "slot": "orders-slot",
                "document_key": "orders-document",
            },
            "document_instance": "doc-instance-0001",
            "prior": {
                "subscription_id": prior.subscription_id,
                "descriptor_binding": prior.descriptor_binding,
            },
            "position": { "epoch": prior.baseline.0, "sequence": sequence },
        })
    };
    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        renew(&issued, "0"),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let value = reply.json();
    assert_eq!(value["proof"], "complete_replay");
    let replay = value["replay"].as_array().expect("replay");
    assert_eq!(replay.len(), 3);
    for (index, encoded) in replay.iter().enumerate() {
        let envelope: Value =
            serde_json::from_str(encoded.as_str().expect("encoded envelope")).expect("envelope");
        assert_eq!(envelope["subscription"], issued.subscription_id);
        assert_eq!(
            envelope["position"]["sequence"],
            (index + 1).to_string(),
            "replay is contiguous from the observed position"
        );
        assert_eq!(envelope["payload"]["payload"], format!("event-{index}"));
    }
    let renewed = parse_issued(&value);
    assert_eq!(renewed.subscription_id, issued.subscription_id);
    assert_ne!(renewed.descriptor_binding, issued.descriptor_binding);
    assert_eq!(renewed.credential, issued.credential);
    assert_eq!(
        renewed.baseline,
        (issued.baseline.0.clone(), "0".to_owned())
    );
    assert!(
        renewed.value["subscription"]["expires_at"]
            .as_u64()
            .expect("expiry")
            >= issued.value["subscription"]["expires_at"]
                .as_u64()
                .expect("expiry")
    );

    let current = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        renew(&renewed, "3"),
    )
    .await;
    assert_eq!(current.status, StatusCode::OK);
    assert_eq!(current.json()["proof"], "authoritative_no_tail");
    assert!(
        current.json()["replay"]
            .as_array()
            .expect("replay")
            .is_empty()
    );
    let current = parse_issued(&current.json());
    assert_ne!(
        current.descriptor_binding, renewed.descriptor_binding,
        "every renewal rotates the descriptor binding"
    );

    let ahead = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        renew(&current, "9"),
    )
    .await;
    assert_eq!(ahead.status, StatusCode::BAD_REQUEST);
    assert_eq!(ahead.error_code(), "async_position_invalid");

    let superseded = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        renew(&issued, "0"),
    )
    .await;
    assert_eq!(
        superseded.status,
        StatusCode::NOT_FOUND,
        "a consumed predecessor binding is never accepted again"
    );
    assert_eq!(superseded.error_code(), "async_subscription_unknown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn websocket_transport_authenticates_memberships_and_delivers_envelopes() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("websocket", "doc-instance-ws01"),
    )
    .await;
    assert_eq!(
        issued.value["subscription"]["authorization"],
        json!({ "kind": "session_cookie" })
    );
    assert_eq!(
        issued.value["subscription"]["document"]["transport"],
        "websocket"
    );

    let mut ws = connect_ws(server.port, &alice, Some(&server.origin()))
        .await
        .expect("same-origin cookie upgrade");
    ws_send(&mut ws, ws_subscribe_frame(&issued, "0000000000000001", 1)).await;
    let ack_text = ws_next_text(&mut ws)
        .await
        .expect("membership acknowledgment");
    let ack: Value = serde_json::from_str(&ack_text).expect("acknowledgment JSON");
    assert_eq!(
        serde_json::to_string(&ack).expect("canonical"),
        ack_text,
        "the acknowledgment is canonical JSON"
    );
    assert_eq!(
        ack,
        json!({
            "control_nonce": "0000000000000001",
            "descriptor_binding": issued.descriptor_binding,
            "kind": "membership_authenticated",
            "stream": "orders",
            "subscription": issued.subscription_id,
            "transport_generation": 1,
        })
    );

    let streams = LiveStreams::resolve().expect("Live streams publisher");
    streams
        .event::<OrdersUpdated>(
            "orders/alice",
            LiveEventTarget::Island,
            CanonicalValue::Null,
        )
        .await
        .expect("publish");
    let envelope: Value =
        serde_json::from_str(&ws_next_text(&mut ws).await.expect("envelope frame"))
            .expect("envelope JSON");
    assert_eq!(envelope["subscription"], issued.subscription_id);
    assert_eq!(envelope["position"]["sequence"], "1");
    assert_eq!(envelope["payload"]["kind"], "browser_event");
    assert_eq!(envelope["payload"]["target"], "self");

    ws_send(
        &mut ws,
        json!({ "kind": "unsubscribe", "subscription": issued.subscription_id }).to_string(),
    )
    .await;
    ws_send(&mut ws, ws_subscribe_frame(&issued, "0000000000000002", 1)).await;
    let ack: Value = serde_json::from_str(&ws_next_text(&mut ws).await.expect("second ack"))
        .expect("acknowledgment JSON");
    assert_eq!(ack["control_nonce"], "0000000000000002");
    ws.close(None).await.expect("client close");
    // A heartbeat envelope may already be in flight when the client closes,
    // so the barrier is the server's close frame (or the closed stream), not
    // the absence of any further text.
    let closed = ws_next_close(&mut ws).await;
    assert!(
        matches!(closed, None | Some((1000 | 1005, _))),
        "the server acknowledges the client close: {closed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn sse_disconnect_retires_the_transport_and_a_reconnect_reauthenticates_memberships() {
    let (router, runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = issued.credential.clone().expect("bearer");
    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;
    let ack = subscribe(server.port, &alice, &credential, &issued, "nonce-1", 1).await;
    assert_eq!(ack.status, StatusCode::OK);
    drop(stream);
    suprnova::live::testing::await_async_transport_retirement_for_test(&runtime, &credential).await;

    let stale = subscribe(server.port, &alice, &credential, &issued, "nonce-2", 1).await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.error_code(), "async_transport_closed");

    let mut stream = SseClient::open(server.port, &alice, &credential, 2, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;
    let old_generation = subscribe(server.port, &alice, &credential, &issued, "nonce-3", 1).await;
    assert_eq!(old_generation.status, StatusCode::CONFLICT);
    assert_eq!(old_generation.error_code(), "async_generation_stale");
    let ack = subscribe(server.port, &alice, &credential, &issued, "nonce-4", 2).await;
    assert_eq!(ack.status, StatusCode::OK);
    LiveStreams::resolve()
        .expect("Live streams publisher")
        .refresh("orders")
        .await
        .expect("publish refresh");
    let (_, envelope) = next_envelope(&mut stream).await;
    assert_eq!(envelope["payload"]["kind"], "refresh");
}
