//! The real application records facts, projects a total and refreshes its bound Live view.
mod live_support;
mod test_environment;

use app::event_sourcing::{
    self as domain, TALLY_UUID, TallyActivity, TallyReactor, TallyState, TallyTotal,
};
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Method, StatusCode};
use live_support::{
    ActionSpec, MEMBERSHIP_PATH, SUBSCRIPTION_PATH, SseClient, action_request, control_nonce,
    decoded_snapshot, empty, fresh_render, get, idempotency, island_tag, request, seed_session,
    send, setup_app, snapshot_revision,
};
use serde_json::{Value, json};
use suprnova::{Aggregate, EventStore, Model, Reactor, console};

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
    send(
        app.addr,
        builder
            .body(Full::new(Bytes::from(serde_json::to_vec(&body).unwrap())))
            .unwrap(),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn es_005_aggregate_projector_reactor_and_bound_live_page_reload() {
    let app = setup_app(12).await;
    let session = seed_session(&app).await;
    let page = get(&app, app::live::EVENT_TALLY_PATH, Some(&session)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text());
    assert!(page.text().contains("data-event-tally=\"0\""));
    let snapshot = decoded_snapshot(island_tag(&page.text(), "event-tally"));
    let revision = snapshot_revision(&snapshot);
    let issue = control(&app, &session, SUBSCRIPTION_PATH, None, json!({
        "protocol_version": 1, "operation": "issue", "transport": "sse", "stream": "changes",
        "island": { "component": "app.event-tally", "slot": "event-tally", "document_key": "event-tally" },
        "document_instance": "event-tally-page-0001"
    })).await;
    assert_eq!(issue.status, StatusCode::CREATED, "{}", issue.text());
    let subscription = issue.json()["subscription"].clone();
    let credential = subscription["authorization"]["credential"]
        .as_str()
        .unwrap();
    let mut stream = SseClient::open(&app, &session, credential, 1).await;
    assert_eq!(stream.status, StatusCode::OK);
    let _ = stream.next_record().await;
    let subscribed = control(&app, &session, MEMBERSHIP_PATH, Some(credential), json!({
        "protocol_version": 1, "operation": "subscribe", "stream": "changes",
        "subscription_id": subscription["subscription_id"], "descriptor_binding": subscription["descriptor_binding"],
        "control_nonce": control_nonce(1), "transport_generation": 1
    })).await;
    assert_eq!(subscribed.status, StatusCode::OK, "{}", subscribed.text());
    let added = send(
        app.addr,
        request(&app, Method::POST, domain::ADD_PATH, Some(&session), true)
            .header("x-csrf-token", &session.csrf)
            .body(empty())
            .unwrap(),
    )
    .await;
    assert!(added.status.is_redirection(), "{}", added.text());
    let signal = loop {
        let envelope = stream.next_data().await.unwrap();
        if envelope["payload"]["kind"] == "topic_change"
            && envelope["payload"]["cursor"]
                .as_u64()
                .is_some_and(|cursor| cursor > 0)
        {
            break envelope;
        }
    };
    assert_eq!(signal["stream"], "changes");
    assert_eq!(signal["payload"]["kind"], "topic_change");
    assert_eq!(signal["payload"]["topic"], domain::TALLY_TOPIC);
    assert_eq!(signal["payload"]["props"], json!(["total"]));
    let fresh = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.event-tally",
                document_key: "event-tally",
                snapshot,
                seed: false,
                base_revision: &revision,
                operations: fresh_render(),
                model_proposals: json!({}),
                idempotency_key: &idempotency(51),
            },
            Some(&session),
            true,
        ),
    )
    .await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text());
    assert!(
        fresh.json()["render"]["html"]
            .as_str()
            .unwrap()
            .contains("data-event-tally=\"1\""),
        "{}",
        fresh.text()
    );
    assert_eq!(
        Aggregate::<TallyState>::retrieve(TALLY_UUID)
            .await
            .unwrap()
            .state()
            .total,
        1
    );
    assert_eq!(
        TallyTotal::find(TALLY_UUID.to_owned())
            .await
            .unwrap()
            .unwrap()
            .total,
        1
    );
    assert_eq!(TallyActivity::query().count().await.unwrap(), 1);
    assert_eq!(EventStore::events(TALLY_UUID).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn es_005_snapshot_commands_replay_and_reactor_idempotency() {
    let _app = setup_app(2).await;
    assert_eq!(domain::add(3).await.unwrap(), 3);
    assert_eq!(domain::add(2).await.unwrap(), 5);
    console::test(["event-sourcing:snapshot", TALLY_UUID])
        .run()
        .await
        .assert_successful();
    assert_eq!(
        EventStore::snapshot(TALLY_UUID)
            .await
            .unwrap()
            .unwrap()
            .aggregate_version,
        2
    );
    assert_eq!(domain::add(4).await.unwrap(), 9);
    let mut projection = TallyTotal::find(TALLY_UUID.to_owned())
        .await
        .unwrap()
        .unwrap();
    projection.total = 99;
    projection.save().await.unwrap();
    console::test([
        "event-sourcing:replay",
        "--projector",
        "dogfood.tally-total",
    ])
    .run()
    .await
    .assert_successful();
    assert_eq!(domain::total().await.unwrap(), 9);
    assert_eq!(TallyActivity::query().count().await.unwrap(), 3);
    let rebuilt = Aggregate::<TallyState>::retrieve(TALLY_UUID).await.unwrap();
    let full = Aggregate::<TallyState>::retrieve_without_snapshot(TALLY_UUID)
        .await
        .unwrap();
    assert_eq!(rebuilt.state().total, full.state().total);
    let events = EventStore::events(TALLY_UUID).await.unwrap();
    TallyReactor.react(&events[0]).await.unwrap();
    assert_eq!(TallyActivity::query().count().await.unwrap(), 3);
    let listed = console::test(["event-sourcing:events", TALLY_UUID])
        .run()
        .await;
    listed.assert_successful();
    assert!(listed.output().contains("dogfood.tally.added"));
    assert_eq!(listed.output().lines().count(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn es_005_command_route_requires_authentication_and_csrf() {
    let app = setup_app(8).await;
    let topic = domain::TALLY_TOPIC.to_owned();
    assert!(suprnova::Gate::allows(
        "live:topic.event-tally",
        &"signed-in".to_owned(),
        &topic
    ));
    assert!(!suprnova::Gate::allows(
        "live:topic.event-tally",
        &String::new(),
        &topic
    ));
    let anonymous = get(&app, app::live::EVENT_TALLY_PATH, None).await;
    assert!(anonymous.status.is_redirection());
    let session = seed_session(&app).await;
    let rejected = send(
        app.addr,
        request(&app, Method::POST, domain::ADD_PATH, Some(&session), false)
            .body(empty())
            .unwrap(),
    )
    .await;
    assert_eq!(rejected.status.as_u16(), 419, "{}", rejected.text());
    assert!(EventStore::events(TALLY_UUID).await.unwrap().is_empty());
    let accepted = send(
        app.addr,
        request(&app, Method::POST, domain::ADD_PATH, Some(&session), true)
            .header("x-csrf-token", &session.csrf)
            .body(empty())
            .unwrap(),
    )
    .await;
    assert!(accepted.status.is_redirection(), "{}", accepted.text());
    assert_eq!(domain::total().await.unwrap(), 1);
    assert!(
        TallyTotal::query()
            .filter("id", TALLY_UUID)
            .exists()
            .await
            .unwrap()
    );
}
