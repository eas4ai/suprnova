//! Hostile, stale, foreign, and malformed asynchronous-transport requests fail closed.

mod live_async_support;

use futures_util::SinkExt as _;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use hyper::{Method, StatusCode};
use live_async_support::*;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::protocol::Message;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn control_requests_are_gated_by_method_header_media_and_shape_before_authority() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();

    let reply = send(
        server.port,
        &alice,
        Method::GET,
        SUBSCRIPTION_PATH,
        &[],
        Bytes::new(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(reply.headers.get("allow").expect("Allow"), "POST");

    let reply = send(
        server.port,
        &alice,
        Method::POST,
        SUBSCRIPTION_PATH,
        &[("content-type", "application/json")],
        Bytes::from_static(b"{}"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_protocol_invalid");

    let reply = send(
        server.port,
        &alice,
        Method::POST,
        SUBSCRIPTION_PATH,
        &[
            ("content-type", "text/plain"),
            ("x-suprnova-live", "async-v1"),
        ],
        Bytes::from_static(b"{}"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let reply = send(
        server.port,
        &alice,
        Method::POST,
        SUBSCRIPTION_PATH,
        &[
            ("content-type", "application/json"),
            ("x-suprnova-live", "async-v1"),
        ],
        Bytes::from(vec![b'{'; 16 * 1024 + 1]),
    )
    .await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);

    let reply = post_control(server.port, &alice, SUBSCRIPTION_PATH, None, json!([])).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_protocol_invalid");

    let mut unknown_field = orders_issue_body("sse", "doc-instance-0001");
    unknown_field["baseline"] = json!({ "epoch": "1", "sequence": "99" });
    let reply = post_control(server.port, &alice, SUBSCRIPTION_PATH, None, unknown_field).await;
    assert_eq!(
        reply.status,
        StatusCode::BAD_REQUEST,
        "a browser cannot propose its baseline"
    );
    assert_eq!(reply.error_code(), "async_protocol_invalid");

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        orders_issue_body("carrier-pigeon", "doc-instance-0001"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_transport_invalid");

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        orders_issue_body("sse", "short"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_protocol_invalid");

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        issue_body(
            "sse",
            "tests.unregistered",
            "orders-slot",
            "orders-document",
            "orders",
            "doc-instance-0001",
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_code(), "async_mount_unknown");

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        issue_body(
            "sse",
            ORDERS_COMPONENT,
            "orders-slot",
            "orders-document",
            "shipping",
            "doc-instance-0001",
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_code(), "async_stream_unknown");

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
        inventory_issue_body("websocket", "doc-instance-0001"),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::BAD_REQUEST,
        "a stream registered for SSE only cannot be issued for WebSocket"
    );
    assert_eq!(reply.error_code(), "async_transport_unsupported");

    let reply = post_control(
        server.port,
        &alice.clone().anonymous(),
        SUBSCRIPTION_PATH,
        None,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_code(), "async_authorization_denied");

    let reply = post_control(
        server.port,
        &alice.clone().with_principal("mallory"),
        SUBSCRIPTION_PATH,
        None,
        inventory_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_code(), "async_authorization_denied");

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
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
        }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_protocol_invalid");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn sse_streams_reject_missing_foreign_or_duplicate_authority_before_allocation() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = issued.credential.clone().expect("bearer");

    let reply = SseClient::open(
        server.port,
        &alice,
        &credential,
        1,
        &[("accept", "application/json")],
    )
    .await
    .rejection()
    .await;
    assert_eq!(reply.status, StatusCode::NOT_ACCEPTABLE);

    let reply = SseClient::open(server.port, &alice, "", 1, &[])
        .await
        .rejection()
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.error_code(), "async_authority_missing");

    let reply = SseClient::open(server.port, &alice, "not-a-real-credential-value", 1, &[])
        .await
        .rejection()
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_code(), "async_authority_invalid");

    let reply = SseClient::open(server.port, &alice, &credential, 0, &[])
        .await
        .rejection()
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_generation_invalid");

    let bob = alice
        .clone()
        .with_session("bob-session")
        .with_principal("bob");
    let reply = SseClient::open(server.port, &bob, &credential, 1, &[])
        .await
        .rejection()
        .await;
    assert_eq!(
        reply.status,
        StatusCode::FORBIDDEN,
        "a bearer minted for another scope is never accepted"
    );
    assert_eq!(reply.error_code(), "async_authority_invalid");

    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;
    let duplicate = SseClient::open(server.port, &alice, &credential, 2, &[])
        .await
        .rejection()
        .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(duplicate.error_code(), "async_transport_reader_exists");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn membership_controls_reject_forged_stale_replayed_or_foreign_controls() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = issued.credential.clone().expect("bearer");
    let other_document = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0002"),
    )
    .await;
    let socket_issued = issue(
        server.port,
        &alice,
        orders_issue_body("websocket", "doc-instance-ws01"),
    )
    .await;

    let before_reader = subscribe(server.port, &alice, &credential, &issued, "nonce-0", 1).await;
    assert_eq!(before_reader.status, StatusCode::CONFLICT);
    assert_eq!(before_reader.error_code(), "async_transport_closed");

    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;

    let missing = post_control(
        server.port,
        &alice,
        MEMBERSHIP_PATH,
        None,
        membership_body("subscribe", &issued, "nonce-1", 1),
    )
    .await;
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing.error_code(), "async_authority_missing");

    let mut forged = issued.clone();
    forged.descriptor_binding = other_document.descriptor_binding.clone();
    let reply = subscribe(server.port, &alice, &credential, &forged, "nonce-2", 1).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_code(), "async_membership_invalid");

    let reply = subscribe(
        server.port,
        &alice,
        &credential,
        &other_document,
        "nonce-3",
        1,
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::FORBIDDEN,
        "a subscription issued for another browser document cannot join this transport"
    );
    assert_eq!(reply.error_code(), "async_membership_invalid");

    let reply = subscribe(
        server.port,
        &alice,
        &credential,
        &socket_issued,
        "nonce-4",
        1,
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_transport_mismatch");

    let unknown = post_control(
        server.port,
        &alice,
        MEMBERSHIP_PATH,
        Some(&credential),
        membership_body("unsubscribe", &issued, "nonce-5", 1),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.error_code(), "async_membership_unknown");

    let bob = alice
        .clone()
        .with_session("bob-session")
        .with_principal("bob");
    let reply = subscribe(server.port, &bob, &credential, &issued, "nonce-6", 1).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_code(), "async_authority_invalid");

    let reply = subscribe(server.port, &alice, &credential, &issued, "nonce-7", 2).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error_code(), "async_generation_stale");

    let reply = subscribe(server.port, &alice, &credential, &issued, "nonce-8", 1).await;
    assert_eq!(reply.status, StatusCode::OK);
    let replayed = subscribe(server.port, &alice, &credential, &issued, "nonce-8", 1).await;
    assert_eq!(replayed.status, StatusCode::CONFLICT);
    assert_eq!(replayed.error_code(), "async_control_replayed");
    let duplicate = subscribe(server.port, &alice, &credential, &issued, "nonce-9", 1).await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(duplicate.error_code(), "async_membership_duplicate");

    let mut nonce_too_long = membership_body("subscribe", &issued, "nonce-10", 1);
    nonce_too_long["control_nonce"] = Value::String("x".repeat(129));
    let reply = post_control(
        server.port,
        &alice,
        MEMBERSHIP_PATH,
        Some(&credential),
        nonce_too_long,
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_code(), "async_protocol_invalid");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn websocket_upgrades_reject_hostile_origins_before_any_session_authority() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("websocket", "doc-instance-ws01"),
    )
    .await;
    let facts_before = FACTS_RECORDED.load(Ordering::SeqCst);

    for origin in [None, Some("null"), Some("*"), Some("http://evil.example")] {
        let error = connect_ws(server.port, &alice, origin)
            .await
            .err()
            .unwrap_or_else(|| panic!("origin {origin:?} must be rejected"));
        match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                assert_eq!(
                    response.status(),
                    StatusCode::FORBIDDEN,
                    "origin {origin:?}"
                );
            }
            other => panic!("unexpected upgrade failure for {origin:?}: {other}"),
        }
    }
    assert_eq!(
        FACTS_RECORDED.load(Ordering::SeqCst),
        facts_before,
        "hostile origins are rejected before session middleware runs"
    );

    let mut anonymous = connect_ws(
        server.port,
        &alice.clone().anonymous(),
        Some(&server.origin()),
    )
    .await
    .expect("upgrade with a valid origin");
    ws_send(
        &mut anonymous,
        ws_subscribe_frame(&issued, "0000000000000001", 1),
    )
    .await;
    assert_eq!(
        ws_next_close(&mut anonymous).await,
        Some((1008, "membership_authority_invalid".to_owned()))
    );

    let bob = alice
        .clone()
        .with_session("bob-session")
        .with_principal("bob");
    let mut foreign = connect_ws(server.port, &bob, Some(&server.origin()))
        .await
        .expect("upgrade with a valid origin");
    ws_send(
        &mut foreign,
        ws_subscribe_frame(&issued, "0000000000000001", 1),
    )
    .await;
    assert_eq!(
        ws_next_close(&mut foreign).await,
        Some((1008, "membership_authority_invalid".to_owned()))
    );

    let sse_issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let mut mismatched = connect_ws(server.port, &alice, Some(&server.origin()))
        .await
        .expect("upgrade with a valid origin");
    ws_send(
        &mut mismatched,
        ws_subscribe_frame(&sse_issued, "0000000000000001", 1),
    )
    .await;
    assert_eq!(
        ws_next_close(&mut mismatched).await,
        Some((1008, "membership_authority_invalid".to_owned()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn websocket_frames_are_bounded_typed_and_capacity_limited() {
    let (router, _runtime) = router_and_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("websocket", "doc-instance-ws01"),
    )
    .await;

    let mut binary = connect_ws(server.port, &alice, Some(&server.origin()))
        .await
        .expect("upgrade");
    binary
        .send(Message::binary(vec![0_u8; 8]))
        .await
        .expect("send binary frame");
    assert_eq!(
        ws_next_close(&mut binary).await,
        Some((1008, "unsupported_frame".to_owned()))
    );

    let mut oversized = connect_ws(server.port, &alice, Some(&server.origin()))
        .await
        .expect("upgrade");
    ws_send(&mut oversized, "x".repeat(513)).await;
    assert_eq!(
        ws_next_close(&mut oversized).await,
        Some((1008, "frame_too_large".to_owned()))
    );

    let mut malformed = connect_ws(server.port, &alice, Some(&server.origin()))
        .await
        .expect("upgrade");
    ws_send(&mut malformed, "{\"kind\":\"subscribe\"}".to_owned()).await;
    assert_eq!(
        ws_next_close(&mut malformed).await,
        Some((1008, "invalid_envelope".to_owned()))
    );

    let mut capacity = connect_ws(server.port, &alice, Some(&server.origin()))
        .await
        .expect("upgrade");
    for index in 0..32_u64 {
        ws_send(
            &mut capacity,
            ws_subscribe_frame(&issued, &control_nonce(index + 1), 1),
        )
        .await;
        let ack: Value = serde_json::from_str(
            &ws_next_text(&mut capacity)
                .await
                .expect("membership acknowledgment"),
        )
        .expect("acknowledgment JSON");
        assert_eq!(ack["kind"], "membership_authenticated");
        ws_send(
            &mut capacity,
            json!({ "kind": "unsubscribe", "subscription": issued.subscription_id }).to_string(),
        )
        .await;
    }
    ws_send(
        &mut capacity,
        ws_subscribe_frame(&issued, &control_nonce(99), 1),
    )
    .await;
    assert_eq!(
        ws_next_close(&mut capacity).await,
        Some((1008, "control_capacity_exceeded".to_owned()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn expired_authority_is_rejected_at_every_boundary() {
    let (router, _runtime, clock) = router_and_runtime_with_clock();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    let credential = issued.credential.clone().expect("bearer");
    let expires_at = issued.value["subscription"]["expires_at"]
        .as_u64()
        .expect("expires_at");
    assert_eq!(expires_at, clock.now_ms() + 120_000);

    let mut stream = SseClient::open(server.port, &alice, &credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;
    clock.advance_ms(120_000);

    let reply = subscribe(server.port, &alice, &credential, &issued, "nonce-1", 1).await;
    assert_eq!(reply.status, StatusCode::GONE);
    assert_eq!(reply.error_code(), "async_authority_expired");

    let reply = post_control(
        server.port,
        &alice,
        SUBSCRIPTION_PATH,
        None,
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
                "subscription_id": issued.subscription_id,
                "descriptor_binding": issued.descriptor_binding,
            },
            "position": { "epoch": issued.baseline.0, "sequence": "0" },
        }),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::GONE,
        "an expired descriptor cannot be renewed; the browser must issue afresh"
    );
    assert_eq!(reply.error_code(), "async_authority_expired");

    drop(stream);
    let reply = SseClient::open(server.port, &alice, &credential, 2, &[])
        .await
        .rejection()
        .await;
    assert_eq!(reply.status, StatusCode::GONE);
    assert_eq!(reply.error_code(), "async_authority_expired");

    let fresh = issue(
        server.port,
        &alice,
        orders_issue_body("sse", "doc-instance-0001"),
    )
    .await;
    assert_ne!(fresh.credential, issued.credential);
}
