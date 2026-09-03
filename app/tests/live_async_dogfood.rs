//! Asynchronous updates through the real application: an SSE subscription
//! for the activity feed, a published event delivered over it, a WebSocket
//! membership, and polling through the ordinary fresh render.

mod live_support;

use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use http_body_util::Full;
use hyper::{Method, StatusCode};
use live_support::{
    ActionSpec, MEMBERSHIP_PATH, SOCKET_PATH, SUBSCRIPTION_PATH, SseClient, action_request,
    control_nonce, dashboard_html, decoded_snapshot, fresh_render, idempotency, island_tag,
    request, seed_session, send, setup_app, snapshot_revision,
};
use serde_json::{Value, json};
use suprnova::live::{CanonicalValue, LiveEventTarget, LiveStreams};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use app::live::components::activity_feed::ActivityPosted;

async fn control(
    app: &live_support::TestApp,
    session: &live_support::SeededSession,
    path: &str,
    credential: Option<&str>,
    body: Value,
) -> live_support::Reply {
    let mut builder = request(app, Method::POST, path, Some(session), true)
        .header("content-type", "application/json")
        .header("x-suprnova-live", "async-v1");
    if let Some(credential) = credential {
        builder = builder.header("authorization", format!("SuprnovaAsync {credential}"));
    }
    let request = builder
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&body).expect("encode"),
        )))
        .expect("build control request");
    send(app.addr, request).await
}

fn issue_body(transport: &str, document_instance: &str) -> Value {
    json!({
        "protocol_version": 1,
        "operation": "issue",
        "transport": transport,
        "stream": "activity",
        "island": {"component": "app.activity-feed", "slot": "feed", "document_key": "dashboard-feed"},
        "document_instance": document_instance,
    })
}

fn membership_body(subscription: &Value, nonce: &str, generation: u64) -> Value {
    json!({
        "protocol_version": 1,
        "operation": "subscribe",
        "subscription_id": subscription["subscription_id"],
        "descriptor_binding": subscription["descriptor_binding"],
        "stream": subscription["stream"],
        "control_nonce": nonce,
        "transport_generation": generation,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_feed_receives_events_over_sse_and_websocket_and_polls() {
    let app = setup_app(12).await;
    let session = seed_session(&app).await;
    let html = dashboard_html(&app, &session).await;
    let feed = island_tag(&html, "dashboard-feed");
    let snapshot = decoded_snapshot(feed);
    let revision = snapshot_revision(&snapshot);

    // SSE: issue, open, subscribe, then a server-published event arrives.
    let issued = control(
        &app,
        &session,
        SUBSCRIPTION_PATH,
        None,
        issue_body("sse", "doc-instance-0001"),
    )
    .await;
    assert_eq!(issued.status, StatusCode::CREATED, "{}", issued.text());
    let issued = issued.json();
    let subscription = &issued["subscription"];
    assert_eq!(subscription["document"]["origin"], app.origin());
    let credential = subscription["authorization"]["credential"]
        .as_str()
        .expect("bearer credential")
        .to_owned();

    let mut sse = SseClient::open(&app, &session, &credential, 1).await;
    assert_eq!(sse.status, StatusCode::OK);
    assert_eq!(
        sse.headers
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    let first = sse.next_record().await.expect("heartbeat");
    assert_eq!(first.comment.as_deref(), Some("suprnova-live heartbeat"));

    let ack = control(
        &app,
        &session,
        MEMBERSHIP_PATH,
        Some(&credential),
        membership_body(subscription, &control_nonce(1), 1),
    )
    .await;
    assert_eq!(ack.status, StatusCode::OK, "{}", ack.text());
    assert_eq!(ack.json()["kind"], "authenticated");

    LiveStreams::from_runtime(&app.runtime)
        .event::<ActivityPosted>(
            "activity",
            LiveEventTarget::Island,
            CanonicalValue::String("posted".to_owned()),
        )
        .await
        .expect("publish activity event");
    let delivered = sse.next_data().await.expect("event delivered");
    assert_eq!(delivered["stream"], "activity", "{delivered}");
    assert_eq!(delivered["payload"]["kind"], "browser_event", "{delivered}");
    assert_eq!(
        delivered["payload"]["event"], "activity.posted",
        "{delivered}"
    );
    assert_eq!(delivered["payload"]["payload"], "posted", "{delivered}");

    // WebSocket: a cookie-authorized upgrade with the browser origin.
    let issued = control(
        &app,
        &session,
        SUBSCRIPTION_PATH,
        None,
        issue_body("websocket", "doc-instance-0002"),
    )
    .await;
    assert_eq!(issued.status, StatusCode::CREATED, "{}", issued.text());
    let socket_subscription = issued.json()["subscription"].clone();
    assert_eq!(
        socket_subscription["authorization"]["kind"],
        "session_cookie"
    );
    let url = format!("ws://127.0.0.1:{}{SOCKET_PATH}", app.port);
    let mut upgrade = url.into_client_request().expect("ws request");
    upgrade.headers_mut().insert(
        "cookie",
        format!("suprnova_session={}", session.cookie)
            .parse()
            .expect("cookie header"),
    );
    upgrade
        .headers_mut()
        .insert("origin", app.origin().parse().expect("origin header"));
    let (mut socket, _) = tokio_tungstenite::connect_async(upgrade)
        .await
        .expect("WebSocket upgrade through the real stack");
    let frame = json!({
        "control_nonce": control_nonce(2),
        "descriptor_binding": socket_subscription["descriptor_binding"],
        "kind": "subscribe",
        "stream": socket_subscription["stream"],
        "subscription": socket_subscription["subscription_id"],
        "transport_generation": 1,
    });
    socket
        .send(Message::text(frame.to_string()))
        .await
        .expect("send subscribe frame");
    let ack = loop {
        match socket.next().await.expect("socket frame") {
            Ok(Message::Text(text)) => break text.to_string(),
            Ok(Message::Close(frame)) => panic!("socket closed: {frame:?}"),
            Ok(_) => continue,
            Err(error) => panic!("socket error: {error}"),
        }
    };
    let ack: Value = serde_json::from_str(&ack).expect("ack JSON");
    assert_eq!(ack["kind"], "membership_authenticated", "{ack}");
    let _ = socket.close(None).await;

    // Polling is the ordinary fresh render on the feed island, and it shows
    // the server data recorded since the document rendered.
    let posted = app::live::components::activity_feed::record_post();
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.activity-feed",
                document_key: "dashboard-feed",
                snapshot,
                seed: false,
                base_revision: &revision,
                operations: fresh_render(),
                model_proposals: Value::Object(Default::default()),
                idempotency_key: &idempotency(41),
            },
            Some(&session),
            true,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(
        reply.json()["render"]["html"]
            .as_str()
            .is_some_and(|h| h.contains(&format!("data-posted=\"{posted}\""))),
        "the fresh render shows the server data recorded before it: {}",
        reply.text()
    );
}
