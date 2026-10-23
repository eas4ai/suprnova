//! In-process BroadcastHub tests - publish + subscribe round-trip,
//! multiple subscribers, channel isolation, unsubscribe.

use serde_json::json;
use suprnova::broadcasting::{BroadcastEnvelope, BroadcastHub, InMemoryBroadcastHub};

#[tokio::test]
async fn publish_to_subscribed_channel_delivers_envelope() {
    let hub = InMemoryBroadcastHub::new();
    let mut rx = hub.subscribe("chat.42");

    hub.publish(BroadcastEnvelope::new(
        "chat.42",
        "MessagePosted",
        json!({ "text": "hello" }),
    ))
    .await
    .unwrap();

    let received = rx.recv().await.expect("envelope arrives");
    assert_eq!(received.channel, "chat.42");
    assert_eq!(received.event, "MessagePosted");
    assert_eq!(received.data["text"], "hello");
}

#[tokio::test]
async fn publish_to_unsubscribed_channel_is_ignored_by_other_subscribers() {
    let hub = InMemoryBroadcastHub::new();
    let mut chat_rx = hub.subscribe("chat.42");
    let mut presence_rx = hub.subscribe("presence.lobby");

    hub.publish(BroadcastEnvelope::new(
        "presence.lobby",
        "MemberJoined",
        json!({ "user_id": 7 }),
    ))
    .await
    .unwrap();

    let presence_msg = presence_rx
        .recv()
        .await
        .expect("presence subscriber receives");
    assert_eq!(presence_msg.event, "MemberJoined");

    use tokio::time::{Duration, timeout};
    assert!(
        timeout(Duration::from_millis(50), chat_rx.recv())
            .await
            .is_err(),
        "chat subscriber should not receive presence-channel events"
    );
}

#[tokio::test]
async fn unsubscribe_via_drop_releases_slot() {
    let hub = InMemoryBroadcastHub::new();
    let rx1 = hub.subscribe("chat.42");
    let rx2 = hub.subscribe("chat.42");
    assert_eq!(hub.subscriber_count("chat.42"), 2);

    drop(rx1);
    // tokio::sync::broadcast detects drops on the next publish (or
    // explicit poll). Yield + publish to push the count refresh.
    tokio::task::yield_now().await;
    hub.publish(BroadcastEnvelope::new("chat.42", "Tick", json!({})))
        .await
        .unwrap();
    tokio::task::yield_now().await;

    assert_eq!(hub.subscriber_count("chat.42"), 1);
    drop(rx2);
}

#[tokio::test]
async fn subscriber_count_returns_zero_for_unknown_channel() {
    let hub = InMemoryBroadcastHub::new();
    assert_eq!(hub.subscriber_count("nobody.here"), 0);
}

#[tokio::test]
async fn signal_fix_003_hub_preserves_sender_and_clears_mixed_owners() {
    use std::time::Duration;
    use suprnova::broadcasting::{LiveTopicExclusion, LiveTopicSender, LiveTopicSignal};
    let hub = InMemoryBroadcastHub::new();
    let mut receiver = hub.subscribe_live_topic("owned").unwrap();
    let interval = Duration::from_millis(30);
    let owner = LiveTopicExclusion {
        socket: "shared-document".into(),
        sender: LiveTopicSender::Session("session-a".into()),
    };
    hub.publish_live_topic("owned", &["stats".into()], Some(owner.clone()), interval)
        .await
        .unwrap();
    let envelope = receiver.recv().await.unwrap();
    let signal: LiveTopicSignal = serde_json::from_value(envelope.data).unwrap();
    assert_eq!(signal.exclusion, Some(owner.clone()));
    hub.publish_live_topic("owned", &["stats".into()], Some(owner), interval)
        .await
        .unwrap();
    hub.publish_live_topic(
        "owned",
        &["count".into()],
        Some(LiveTopicExclusion {
            socket: "shared-document".into(),
            sender: LiveTopicSender::Session("session-b".into()),
        }),
        interval,
    )
    .await
    .unwrap();
    let envelope = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(envelope.except.is_none());
    let signal: LiveTopicSignal = serde_json::from_value(envelope.data).unwrap();
    assert!(signal.exclusion.is_none());
    assert_eq!(signal.props, ["count", "stats"]);
}
