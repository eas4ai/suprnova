use suprnova::container::testing::TestContainer;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{LiveRegistry, LiveTopics};
use suprnova::{Gate, Router};

#[tokio::test]
#[serial_test::serial]
async fn signal_003_private_topic_without_authorizer_fails_boot() {
    let _container = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    TestContainer::singleton(
        LiveRegistry::builder()
            .topic("signal.private-missing")
            .unwrap()
            .build(),
    );
    let error = prepare_live_router_for_test(&Router::new()).unwrap_err();
    assert!(
        error.to_string().contains("signal.private-missing"),
        "{error}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn signal_002_render_cursor_is_topic_wide_and_changes_without_subscribers() {
    let _container = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    Gate::define::<String, String>("live:topic.signal.posts", |_, _| true);
    TestContainer::singleton(
        LiveRegistry::builder()
            .topic("signal.posts")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    assert_eq!(topics.cursor("signal.posts").await.unwrap(), 0);
    topics.changed("signal.posts", &["posts"]).await.unwrap();
    assert_eq!(topics.cursor("signal.posts").await.unwrap(), 1);
}

use crate::live_async_support::*;
use bytes::Bytes;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use suprnova::broadcasting::{BroadcastHub, InMemoryBroadcastHub};
use suprnova::live::{LiveComponent, LiveMount, LiveRuntime, live};
use suprnova::{App, FrameworkError, HttpResponse, Request, Response};

pub struct SignalEvent;
impl suprnova::live::EventPayloadMetadata for SignalEvent {
    const NAME: &'static str = "signal.event";
    const VERSION: u16 = 1;
}
#[derive(LiveComponent)]
#[live(
    name = "tests.signal-island",
    view = "live/tests/signal-island.html",
    minimum_protocol_version = 2,
    streams(stream(
        name = "posts",
        topics("signal.posts"),
        events(SignalEvent),
        targets("self")
    ))
)]
pub struct SignalIsland {
    #[model]
    count: u64,
}

#[test]
fn signal_001_component_and_topic_registration_share_a_fallible_builder() {
    fn registry() -> Result<LiveRegistry, FrameworkError> {
        Ok(LiveRegistry::builder()
            .register::<SignalIsland>()?
            .public_topic("signal.builder")?
            .build())
    }
    assert!(registry().is_ok());
}
#[live]
impl SignalIsland {
    #[mount]
    pub fn mount() -> Self {
        Self { count: 0 }
    }
}

#[derive(suprnova::InertiaProps)]
#[inertia(live(topic = "signal.posts", only = ["posts", "count"]))]
struct SignalPage {
    posts: Vec<String>,
    count: u64,
}

async fn page_handler(request: Request) -> Response {
    Ok(
        suprnova::InertiaResponse::live::<SignalPage>("__tests__/TypedProps")
            .load(|| async {
                Ok(SignalPage {
                    posts: vec![],
                    count: 0,
                })
            })
            .await?
            .resolve(&request)
            .await?,
    )
}
async fn write_handler(_request: Request) -> Response {
    let topics = LiveTopics::resolve()?;
    for n in 0..10 {
        topics
            .batch(topics.changed(
                "signal.posts",
                if n % 2 == 0 { &["posts"] } else { &["count"] },
            ))
            .await??;
    }
    Ok(HttpResponse::json(json!({"saved": true})))
}
fn signal_runtime() -> (Arc<Router>, LiveRuntime, Arc<InMemoryBroadcastHub>) {
    signal_runtime_with_clock(None)
}

fn signal_runtime_with_clock(
    clock: Option<Arc<suprnova::live::testing::AdjustableTestClock>>,
) -> (Arc<Router>, LiveRuntime, Arc<InMemoryBroadcastHub>) {
    App::init();
    ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub.clone());
    Gate::define::<String, String>("live:topic.signal.posts", |_, _| true);
    App::singleton(
        LiveRegistry::builder()
            .register::<SignalIsland>()
            .unwrap()
            .topic("signal.posts")
            .unwrap()
            .public_topic("signal.public")
            .unwrap()
            .build(),
    );
    let mount =
        LiveMount::<SignalIsland>::identity_bound("/signal/page", "signal-slot", "signal-document")
            .unwrap();
    let router: Router = Router::new().get("/signal/page", page_handler).into();
    let router: Router = router.post("/signal/write", write_handler).into();
    let router: Router = router
        .post("/signal/owned-write", socket_origin_write)
        .into();
    let router = router.try_live().unwrap().try_live_mount(&mount).unwrap();
    let runtime = match clock {
        Some(clock) => {
            suprnova::live::testing::prepare_live_router_with_clock_for_test(&router, clock)
                .unwrap()
        }
        None => prepare_live_router_for_test(&router).unwrap(),
    };
    App::singleton(runtime.clone());
    (Arc::new(router), runtime, hub)
}
async fn page_binding(server: &TestServer) -> Value {
    let reply = send(
        server.port,
        &Identity::alice(),
        Method::GET,
        "/signal/page",
        &[("x-inertia", "true")],
        Bytes::new(),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json()["liveBindings"]["signal.posts"].clone()
}
async fn issue_page(
    server: &TestServer,
    binding: &Value,
    document: &str,
    identity: &Identity,
) -> Issued {
    let island = &binding["island"];
    issue(
        server.port,
        identity,
        issue_body(
            "sse",
            island["component"].as_str().unwrap(),
            island["slot"].as_str().unwrap(),
            island["document_key"].as_str().unwrap(),
            binding["stream"].as_str().unwrap(),
            document,
        ),
    )
    .await
}
async fn open_membership(
    server: &TestServer,
    identity: &Identity,
    issued: &Issued,
    nonce: &str,
) -> SseClient {
    let credential = issued.credential.as_deref().unwrap();
    let mut stream = SseClient::open(server.port, identity, credential, 1, &[]).await;
    assert_eq!(stream.status, StatusCode::OK);
    stream.next_record().await.unwrap();
    let ack = subscribe(server.port, identity, credential, issued, nonce, 1).await;
    assert_eq!(
        ack.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&ack.body)
    );
    stream
}
async fn next_signal(stream: &mut SseClient) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = stream.next_data().await.expect("stream remains open");
            if value["payload"]["kind"] == "topic_change" && value["payload"]["cursor"] != 0 {
                let mut payload = value["payload"].clone();
                assert!(
                    payload["generation"]
                        .as_u64()
                        .is_some_and(|generation| generation > 0)
                );
                payload.as_object_mut().unwrap().remove("generation");
                return payload;
            }
        }
    })
    .await
    .expect("signal arrives")
}

async fn assert_no_signal(stream: &mut SseClient) {
    let _ = tokio::time::timeout(Duration::from_millis(250), async {
        while let Some(value) = stream.next_data().await {
            if value["payload"]["cursor"] == 0 {
                continue;
            }
            assert_ne!(
                value["payload"]["kind"], "topic_change",
                "unexpected signal: {value}"
            );
        }
    })
    .await;
}

async fn open_topic_socket(
    server: &TestServer,
    identity: &Identity,
    binding: &Value,
    document: &str,
) -> WsStream {
    let island = &binding["island"];
    let issued = issue(
        server.port,
        identity,
        issue_body(
            "websocket",
            island["component"].as_str().unwrap(),
            island["slot"].as_str().unwrap(),
            island["document_key"].as_str().unwrap(),
            binding["stream"].as_str().unwrap(),
            document,
        ),
    )
    .await;
    let mut socket = connect_ws(server.port, identity, Some(&server.origin()))
        .await
        .unwrap();
    ws_send(
        &mut socket,
        ws_subscribe_frame(&issued, &control_nonce(1), 1),
    )
    .await;
    let acknowledgment = tokio::time::timeout(Duration::from_secs(2), ws_next_text(&mut socket))
        .await
        .unwrap()
        .unwrap();
    let acknowledgment: Value = serde_json::from_str(&acknowledgment).unwrap();
    assert_eq!(acknowledgment["kind"], "membership_authenticated");
    socket
}

async fn next_socket_signal(socket: &mut WsStream) -> Value {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let text = ws_next_text(socket).await.expect("socket remains open");
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["payload"]["kind"] == "topic_change" && value["payload"]["cursor"] != 0 {
                let mut payload = value["payload"].clone();
                assert!(
                    payload["generation"]
                        .as_u64()
                        .is_some_and(|generation| generation > 0)
                );
                payload.as_object_mut().unwrap().remove("generation");
                return payload;
            }
        }
    })
    .await
    .expect("socket receives signal")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_001_page_and_island_share_topic_log_and_revocation() {
    let (router, runtime, _) = signal_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    let page = issue_page(&server, &binding, "signal-page-0001", &alice).await;
    let island = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.signal-island",
            "signal-slot",
            "signal-document",
            "posts",
            "signal-island-0001",
        ),
    )
    .await;
    let mut page_stream = open_membership(&server, &alice, &page, "nonce-page").await;
    let mut island_stream = open_membership(&server, &alice, &island, "nonce-island").await;
    let topics = LiveTopics::from_runtime(&runtime);
    topics.changed("signal.posts", &["posts"]).await.unwrap();
    assert_eq!(
        next_signal(&mut page_stream).await,
        next_signal(&mut island_stream).await
    );
    Gate::define::<String, String>("live:topic.signal.posts", |_, _| false);
    topics.changed("signal.posts", &["count"]).await.unwrap();
    assert_no_signal(&mut page_stream).await;
    assert_no_signal(&mut island_stream).await;
    assert!(
        suprnova::live::testing::inspect_async_transports_for_test(&runtime)
            .iter()
            .all(|report| report.memberships == 0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_003_ten_request_writes_signal_once_with_prop_union_and_no_model_data() {
    let (router, runtime, hub) = signal_runtime();
    let mut signals = hub.subscribe_live_topic("signal.posts").unwrap();
    let server = spawn_server(router).await;
    let response = send(
        server.port,
        &Identity::alice(),
        Method::POST,
        "/signal/write",
        &[],
        Bytes::new(),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    let signal = tokio::time::timeout(Duration::from_secs(1), signals.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        signal.data,
        json!({"topic":"signal.posts", "generation": hub.live_topic_positions(&["signal.posts".into()]).await.unwrap()[0].generation, "cursor":1, "props":["count", "posts"]})
    );
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(signals.try_recv().is_err());
    assert_eq!(
        LiveTopics::from_runtime(&runtime)
            .cursor("signal.posts")
            .await
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_003_socket_identity_excludes_only_the_senders_live_transport() {
    let (router, runtime, _) = signal_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    let sender = issue_page(&server, &binding, "signal-sender-0001", &alice).await;
    let other = issue_page(&server, &binding, "signal-other-0001", &alice).await;
    let mut sender_stream = open_membership(&server, &alice, &sender, "nonce-sender").await;
    let mut other_stream = open_membership(&server, &alice, &other, "nonce-other").await;
    let response = send(
        server.port,
        &alice,
        Method::POST,
        "/signal/write",
        &[("x-socket-id", "signal-sender-0001")],
        Bytes::new(),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(next_signal(&mut other_stream).await["cursor"], 1);
    assert_no_signal(&mut sender_stream).await;
    assert_eq!(
        LiveTopics::from_runtime(&runtime)
            .cursor("signal.posts")
            .await
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_003_websocket_preserves_the_issued_sender_socket_identity() {
    let (router, _, _) = signal_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    let mut sender = open_topic_socket(&server, &alice, &binding, "signal-ws-sender-0001").await;
    let mut other = open_topic_socket(&server, &alice, &binding, "signal-ws-other-0001").await;
    let response = send(
        server.port,
        &alice,
        Method::POST,
        "/signal/write",
        &[("x-socket-id", "signal-ws-sender-0001")],
        Bytes::new(),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(next_socket_signal(&mut other).await["cursor"], 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(250), next_socket_signal(&mut sender))
            .await
            .is_err()
    );
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_rollback_discards_changes_and_commit_keeps_one_union() {
    let _guard = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.tx")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let change = topics.clone();
    let rolled_back: Result<(), FrameworkError> = suprnova::DB::transaction(move |_| {
        Box::pin(async move {
            change.changed("signal.tx", &["posts"]).await?;
            Err(FrameworkError::internal("rollback probe"))
        })
    })
    .await;
    assert!(rolled_back.is_err());
    assert_eq!(topics.cursor("signal.tx").await.unwrap(), 0);
    let change = topics.clone();
    topics
        .batch(async move {
            suprnova::DB::transaction(move |_| {
                Box::pin(async move {
                    for _ in 0..10 {
                        change.changed("signal.tx", &["posts"]).await?;
                    }
                    Ok::<(), FrameworkError>(())
                })
            })
            .await
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(topics.cursor("signal.tx").await.unwrap(), 1);
    drop(db);
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_replay_emits_one_final_signal_per_touched_topic() {
    let _guard = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.replay")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    topics
        .suppress(|| async {
            for _ in 0..10 {
                topics.changed("signal.replay", &["posts"]).await.unwrap();
                assert_eq!(topics.cursor("signal.replay").await.unwrap(), 0);
            }
        })
        .await
        .unwrap();
    assert_eq!(topics.cursor("signal.replay").await.unwrap(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_replay_inside_a_transaction_waits_then_signals_once() {
    let _guard = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.outer-tx")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let change = topics.clone();
    suprnova::DB::transaction(move |_| {
        Box::pin(async move {
            change
                .suppress(|| async {
                    for _ in 0..10 {
                        change.changed("signal.outer-tx", &["posts"]).await.unwrap();
                    }
                })
                .await?;
            assert_eq!(change.cursor("signal.outer-tx").await?, 0);
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();
    assert_eq!(topics.cursor("signal.outer-tx").await.unwrap(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_replay_waits_for_late_manual_commit_and_keeps_the_outer_union() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.late-replay")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let mut signals = hub.subscribe_live_topic("signal.late-replay").unwrap();
    topics
        .batch(async {
            let tx = suprnova::DB::begin_transaction().await.unwrap();
            topics
                .suppress(|| async {
                    for prop in ["posts", "count", "total"] {
                        topics
                            .changed_with_tx(&tx, "signal.late-replay", &[prop])
                            .await
                            .unwrap();
                    }
                })
                .await
                .unwrap();
            tx.commit().await.unwrap();
            assert_eq!(topics.cursor("signal.late-replay").await.unwrap(), 0);
            topics
                .changed("signal.late-replay", &["summary"])
                .await
                .unwrap();
        })
        .await
        .unwrap();
    let signal = tokio::time::timeout(Duration::from_secs(2), signals.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        signal.data["props"],
        json!(["count", "posts", "summary", "total"])
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(topics.cursor("signal.late-replay").await.unwrap(), 1);
    assert!(signals.try_recv().is_err());
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_replay_late_manual_rollback_does_not_leak_props() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.replay-rollback")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    topics
        .suppress(|| async {
            topics
                .changed_with_tx(&tx, "signal.replay-rollback", &["rolled_back"])
                .await
                .unwrap();
        })
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(topics.cursor("signal.replay-rollback").await.unwrap(), 0);
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_replay_retains_one_union_after_its_batch_and_manual_commit() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.closed-batch")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let mut signals = hub.subscribe_live_topic("signal.closed-batch").unwrap();
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    topics
        .batch(async {
            topics
                .changed("signal.closed-batch", &["summary"])
                .await
                .unwrap();
            topics
                .suppress(|| async {
                    for prop in ["posts", "count", "total"] {
                        topics
                            .changed_with_tx(&tx, "signal.closed-batch", &[prop])
                            .await
                            .unwrap();
                    }
                })
                .await
                .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(topics.cursor("signal.closed-batch").await.unwrap(), 0);
    tx.commit().await.unwrap();
    let signal = tokio::time::timeout(Duration::from_secs(2), signals.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        signal.data["props"],
        json!(["count", "posts", "summary", "total"])
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(topics.cursor("signal.closed-batch").await.unwrap(), 1);
    assert!(signals.try_recv().is_err());
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_replay_late_rollback_releases_only_committed_props() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.replay-mixed")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let mut signals = hub.subscribe_live_topic("signal.replay-mixed").unwrap();
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    topics
        .suppress(|| async {
            topics
                .changed("signal.replay-mixed", &["committed"])
                .await
                .unwrap();
            topics
                .changed_with_tx(&tx, "signal.replay-mixed", &["rolled_back"])
                .await
                .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(topics.cursor("signal.replay-mixed").await.unwrap(), 0);
    tx.rollback().await.unwrap();
    let signal = tokio::time::timeout(Duration::from_secs(2), signals.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(signal.data["props"], json!(["committed"]));
    assert_eq!(topics.cursor("signal.replay-mixed").await.unwrap(), 1);
}

#[suprnova::model(table = "signal_posts", fillable = ["title"])]
pub struct SignalPost {
    pub id: i64,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_model_saved_and_deleted_hooks_publish_without_model_fields() {
    use suprnova::Model;
    let _guard = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    db.execute_unprepared("CREATE TABLE signal_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)").await.unwrap();
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.models")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    topics
        .on_model::<SignalPost>("signal.models", &["posts"])
        .unwrap();
    let post = SignalPost::create(suprnova::attrs! { title: "private model value" })
        .await
        .unwrap();
    assert_eq!(topics.cursor("signal.models").await.unwrap(), 1);
    post.delete().await.unwrap();
    assert_eq!(topics.cursor("signal.models").await.unwrap(), 2);
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_empty_change_keeps_all_bound_props_when_coalesced() {
    let _guard = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.all")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let mut receiver = hub.subscribe_live_topic("signal.all").unwrap();
    topics
        .batch(async {
            topics.changed("signal.all", &[]).await.unwrap();
            topics.changed("signal.all", &["posts"]).await.unwrap();
        })
        .await
        .unwrap();
    let envelope = receiver.recv().await.unwrap();
    assert_eq!(envelope.data["props"], json!([]));
    assert_eq!(envelope.data["cursor"], 1);
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_serialized_prop_names_are_not_topic_names() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.renamed")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let mut receiver = hub.subscribe_live_topic("signal.renamed").unwrap();
    topics
        .changed("signal.renamed", &["post count", "投稿"])
        .await
        .unwrap();
    let envelope = receiver.recv().await.unwrap();
    assert_eq!(envelope.data["props"], json!(["post count", "投稿"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_001_anonymous_subscriptions_require_an_explicit_public_topic() {
    use sha2::Digest;
    let (router, runtime, _) = signal_runtime();
    let server = spawn_server(router).await;
    let anonymous = Identity::alice().anonymous();
    let component = format!(
        "suprnova.topic.{}",
        hex::encode(sha2::Sha256::digest(b"signal.public"))
    );
    let body = issue_body(
        "sse",
        &component,
        "topic",
        "topic",
        "changes",
        "public-page-0001",
    );
    let public = post_control(server.port, &anonymous, SUBSCRIPTION_PATH, None, body).await;
    assert_eq!(
        public.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&public.body)
    );
    let public = parse_issued(&public.json());
    let mut stream = open_membership(&server, &anonymous, &public, "nonce-public").await;
    LiveTopics::from_runtime(&runtime)
        .changed("signal.public", &["posts"])
        .await
        .unwrap();
    assert_eq!(next_signal(&mut stream).await["topic"], "signal.public");
    let private = format!(
        "suprnova.topic.{}",
        hex::encode(sha2::Sha256::digest(b"signal.posts"))
    );
    let body = issue_body(
        "sse",
        &private,
        "topic",
        "topic",
        "changes",
        "private-page-0001",
    );
    let private = post_control(server.port, &anonymous, SUBSCRIPTION_PATH, None, body).await;
    assert_eq!(private.status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_001_public_topics_allow_anonymous_websocket_memberships() {
    use sha2::Digest;
    let (router, runtime, _) = signal_runtime();
    let server = spawn_server(router).await;
    let anonymous = Identity::alice().anonymous();
    let component = format!(
        "suprnova.topic.{}",
        hex::encode(sha2::Sha256::digest(b"signal.public"))
    );
    let binding = json!({"island": {"component": component, "slot": "topic", "document_key": "topic"}, "stream": "changes"});
    let mut socket =
        open_topic_socket(&server, &anonymous, &binding, "signal-public-ws-0001").await;
    LiveTopics::from_runtime(&runtime)
        .changed("signal.public", &["posts"])
        .await
        .unwrap();
    assert_eq!(
        next_socket_signal(&mut socket).await["topic"],
        "signal.public"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_001_parameterized_component_streams_do_not_retain_unused_hub_feeds() {
    App::init();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub.clone());
    let (router, _) = router_and_runtime();
    let server = spawn_server(router).await;
    issue(
        server.port,
        &Identity::alice(),
        orders_issue_body("sse", "signal-raw-stream-0001"),
    )
    .await;
    assert_eq!(hub.subscriber_count("__live.topic.orders"), 0);
    assert_eq!(hub.subscriber_count("__live.topic.orders/alice"), 0);
    assert!(hub.subscriber_count(&format!("__live.topic.{ORDERS_COMPONENT}::orders")) > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_002_signal_between_render_and_subscribe_uses_the_current_topic_cursor() {
    let (router, runtime, _) = signal_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    assert_eq!(binding["cursor"], 0);
    LiveTopics::from_runtime(&runtime)
        .changed("signal.posts", &["posts"])
        .await
        .unwrap();
    let page = issue_page(&server, &binding, "signal-raced-0001", &alice).await;
    let mut stream = open_membership(&server, &alice, &page, "nonce-raced").await;
    let signal = next_signal(&mut stream).await;
    assert_eq!(signal["cursor"], 1);
    assert_eq!(signal["props"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_005_membership_baseline_does_not_republish_to_existing_subscribers() {
    let (router, runtime, _) = signal_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    let first = issue_page(&server, &binding, "signal-existing-0001", &alice).await;
    let mut existing = open_membership(&server, &alice, &first, "nonce-existing").await;
    LiveTopics::from_runtime(&runtime)
        .changed("signal.posts", &["posts"])
        .await
        .unwrap();
    assert_eq!(next_signal(&mut existing).await["cursor"], 1);
    let second = issue_page(&server, &binding, "signal-joining-0001", &alice).await;
    let mut joining = open_membership(&server, &alice, &second, "nonce-joining").await;
    assert_eq!(next_signal(&mut joining).await["cursor"], 1);
    assert_no_signal(&mut existing).await;
}

#[tokio::test]
#[serial_test::serial]
async fn signal_005_rate_bound_keeps_the_union_for_the_next_permitted_signal() {
    let _guard = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.hot")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let mut receiver = hub.subscribe_live_topic("signal.hot").unwrap();
    topics.changed("signal.hot", &["first"]).await.unwrap();
    let first = receiver.recv().await.unwrap();
    assert_eq!(first.data["cursor"], 1);
    let start = tokio::time::Instant::now();
    for _ in 0..9 {
        topics.changed("signal.hot", &["posts"]).await.unwrap();
        topics.changed("signal.hot", &["count"]).await.unwrap();
    }
    assert!(receiver.try_recv().is_err());
    let trailing = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(start.elapsed() >= Duration::from_millis(90));
    assert_eq!(trailing.data["props"], json!(["count", "posts"]));
    assert_eq!(trailing.data["cursor"], 19);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_005_second_runtime_membership_receives_first_nodes_signal() {
    let (router, first, _) = signal_runtime();
    let second = prepare_live_router_for_test(&Router::new()).unwrap();
    App::singleton(second);
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    let issued = issue_page(&server, &binding, "signal-node-two-0001", &alice).await;
    let mut stream = open_membership(&server, &alice, &issued, "nonce-node-two").await;
    LiveTopics::from_runtime(&first)
        .changed("signal.posts", &["posts"])
        .await
        .unwrap();
    assert_eq!(
        next_signal(&mut stream).await,
        json!({"kind":"topic_change", "topic":"signal.posts", "cursor":1, "props":["posts"]})
    );
    suprnova::live::LiveStreams::from_runtime(&first)
        .refresh("signal.posts")
        .await
        .unwrap();
    assert_eq!(
        next_signal(&mut stream).await,
        json!({"kind":"topic_change", "topic":"signal.posts", "cursor":2, "props":[]})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_002_stream_refresh_delivers_topic_cursor_to_page_and_island() {
    let (router, runtime, _) = signal_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    let page = issue_page(&server, &binding, "signal-stream-page-0001", &alice).await;
    let island = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.signal-island",
            "signal-slot",
            "signal-document",
            "posts",
            "signal-stream-island-0001",
        ),
    )
    .await;
    let mut page_stream = open_membership(&server, &alice, &page, "nonce-stream-page").await;
    let mut island_stream = open_membership(&server, &alice, &island, "nonce-stream-island").await;
    suprnova::live::LiveStreams::from_runtime(&runtime)
        .refresh("signal.posts")
        .await
        .unwrap();
    let expected = json!({"kind":"topic_change", "topic":"signal.posts", "cursor":1, "props":[]});
    assert_eq!(next_signal(&mut page_stream).await, expected);
    assert_eq!(next_signal(&mut island_stream).await, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_001_component_page_binding_receives_only_its_stream_alias() {
    let (router, runtime, _) = signal_runtime();
    Gate::define::<String, String>("live:tests.signal-island.stream.posts", |_, _| true);
    let topics = LiveTopics::from_runtime(&runtime);
    topics.changed("signal.posts", &["posts"]).await.unwrap();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let page = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.signal-island",
            "topic",
            "topic",
            "posts",
            "signal-alias-page-0001",
        ),
    )
    .await;
    let mut page_stream = open_membership(&server, &alice, &page, "nonce-alias-page").await;
    assert_no_signal(&mut page_stream).await;
    let island = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.signal-island",
            "signal-slot",
            "signal-document",
            "posts",
            "signal-alias-island-0001",
        ),
    )
    .await;
    let mut island_stream = open_membership(&server, &alice, &island, "nonce-alias-island").await;
    assert_eq!(
        next_signal(&mut island_stream).await["topic"],
        "signal.posts"
    );
    suprnova::live::LiveStreams::from_runtime(&runtime)
        .refresh("tests.signal-island::posts")
        .await
        .unwrap();
    let expected = json!({"kind":"topic_change", "topic":"tests.signal-island::posts", "cursor":1, "props":[]});
    assert_eq!(next_signal(&mut page_stream).await, expected);
    assert_eq!(next_signal(&mut island_stream).await, expected);
    topics.changed("signal.posts", &["count"]).await.unwrap();
    assert_eq!(
        next_signal(&mut island_stream).await["topic"],
        "signal.posts"
    );
    assert_no_signal(&mut page_stream).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_001_component_alias_delivery_rechecks_the_islands_stream_ability() {
    let (router, runtime, _) = signal_runtime();
    Gate::define::<String, String>("live:tests.signal-island.stream.posts", |_, _| true);
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let island = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.signal-island",
            "signal-slot",
            "signal-document",
            "posts",
            "signal-alias-revoked-0001",
        ),
    )
    .await;
    let mut stream = open_membership(&server, &alice, &island, "nonce-alias-revoked").await;
    Gate::define::<String, String>("live:tests.signal-island.stream.posts", |_, _| false);
    suprnova::live::LiveStreams::from_runtime(&runtime)
        .refresh("tests.signal-island::posts")
        .await
        .unwrap();
    assert_no_signal(&mut stream).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_002_component_page_binding_does_not_receive_model_event_data() {
    use suprnova_live::canonical::CanonicalValue;
    let (router, runtime, _) = signal_runtime();
    Gate::define::<String, String>("live:tests.signal-island.stream.posts", |_, _| true);
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let page = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.signal-island",
            "topic",
            "topic",
            "posts",
            "signal-alias-data-page-0001",
        ),
    )
    .await;
    let mut page_stream = open_membership(&server, &alice, &page, "nonce-alias-data-page").await;
    let island = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.signal-island",
            "signal-slot",
            "signal-document",
            "posts",
            "signal-alias-data-island-0001",
        ),
    )
    .await;
    let mut island_stream =
        open_membership(&server, &alice, &island, "nonce-alias-data-island").await;
    let payload = CanonicalValue::Object(std::collections::BTreeMap::from([(
        "title".to_owned(),
        CanonicalValue::String("private model field".to_owned()),
    )]));
    suprnova::live::LiveStreams::from_runtime(&runtime)
        .event::<SignalEvent>(
            "tests.signal-island::posts",
            suprnova::live::LiveEventTarget::Island,
            payload,
        )
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = island_stream.next_data().await.unwrap();
            if event["payload"]["kind"] == "browser_event" {
                break event;
            }
            assert_eq!(event["payload"]["kind"], "topic_change");
            assert_eq!(event["payload"]["cursor"], 0);
            assert!(event["payload"]["generation"].as_u64().unwrap() > 0);
        }
    })
    .await
    .unwrap();
    assert_eq!(event["payload"]["payload"]["title"], "private model field");
    let _ = tokio::time::timeout(Duration::from_millis(250), async {
        while let Some(event) = page_stream.next_data().await {
            assert_ne!(
                event["payload"]["kind"], "browser_event",
                "page receives model event data: {event}"
            );
        }
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn signal_005_stream_refresh_uses_cursor_and_rate_for_both_topic_forms() {
    let (_router, runtime, hub) = signal_runtime();
    let topics = LiveTopics::from_runtime(&runtime);
    let streams = suprnova::live::LiveStreams::from_runtime(&runtime);
    for topic in ["signal.posts", "tests.signal-island::posts"] {
        let mut receiver = hub.subscribe_live_topic(topic).unwrap();
        let start = tokio::time::Instant::now();
        for _ in 0..20 {
            streams.refresh(topic).await.unwrap();
        }
        assert_eq!(topics.cursor(topic).await.unwrap(), 20, "{topic}");
        let first = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            first.data,
            json!({"topic":topic, "generation": hub.live_topic_positions(&[topic.into()]).await.unwrap()[0].generation, "cursor":1, "props":[]})
        );
        assert!(receiver.try_recv().is_err());
        let last = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(90));
        assert_eq!(
            last.data,
            json!({"topic":topic, "generation": first.data["generation"], "cursor":20, "props":[]})
        );
        assert!(receiver.try_recv().is_err());
    }
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_stream_refresh_obeys_rollback_and_the_outer_scope() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.stream-tx")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let streams = suprnova::live::LiveStreams::from_runtime(&runtime);
    let topics = LiveTopics::from_runtime(&runtime);
    let rolled = streams.clone();
    let result: Result<(), FrameworkError> = suprnova::DB::transaction(move |_| {
        Box::pin(async move {
            rolled.refresh("signal.stream-tx").await.unwrap();
            Err(FrameworkError::internal("rollback stream refresh"))
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(topics.cursor("signal.stream-tx").await.unwrap(), 0);
    let mut receiver = hub.subscribe_live_topic("signal.stream-tx").unwrap();
    topics
        .batch(async {
            for _ in 0..10 {
                streams.refresh("signal.stream-tx").await.unwrap();
            }
            assert_eq!(topics.cursor("signal.stream-tx").await.unwrap(), 0);
            topics
                .changed("signal.stream-tx", &["posts"])
                .await
                .unwrap();
        })
        .await
        .unwrap();
    let signal = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        signal.data,
        json!({"topic":"signal.stream-tx", "generation": hub.live_topic_positions(&["signal.stream-tx".into()]).await.unwrap()[0].generation, "cursor":1, "props":[]})
    );
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SignalJob;
#[suprnova::async_trait]
impl suprnova::queue::Job for SignalJob {
    fn job_name() -> &'static str {
        "signal.test-job"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        let topics = LiveTopics::resolve()?;
        for _ in 0..10 {
            topics
                .batch(topics.changed("signal.job", &["posts"]))
                .await??;
            topics.changed("signal.job", &["count"]).await?;
        }
        Ok(())
    }
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_worker_attempt_coalesces_one_signal_for_the_job() {
    use suprnova::queue::Job;
    let _guard = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.job")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    suprnova::queue::worker::register_job::<SignalJob>();
    let env = suprnova::queue::Envelope {
        schema_version: suprnova::queue::CURRENT_SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        job_name: SignalJob::job_name().into(),
        queue: None,
        payload: serde_json::to_value(SignalJob).unwrap(),
        dispatched_at: chrono::Utc::now(),
        available_at: chrono::Utc::now(),
        attempts: 0,
        max_tries: 1,
        backoff: suprnova::queue::BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        message_group: None,
        deduplication_id: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
        context: None,
    };
    let mut receiver = hub.subscribe_live_topic("signal.job").unwrap();
    suprnova::queue::worker::run_through_middleware(env)
        .await
        .unwrap();
    let signal = receiver.recv().await.unwrap();
    assert_eq!(signal.data["cursor"], 1);
    assert_eq!(signal.data["props"], json!(["count", "posts"]));
    assert_eq!(
        LiveTopics::from_runtime(&runtime)
            .cursor("signal.job")
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_manual_transaction_handle_waits_for_commit_and_discards_rollback() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.manual")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    topics
        .changed_with_tx(&tx, "signal.manual", &["posts"])
        .await
        .unwrap();
    assert_eq!(topics.cursor("signal.manual").await.unwrap(), 0);
    tx.rollback().await.unwrap();
    assert_eq!(topics.cursor("signal.manual").await.unwrap(), 0);
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    topics
        .changed_with_tx(&tx, "signal.manual", &["posts"])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(topics.cursor("signal.manual").await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_001_component_stream_binding_uses_its_own_stream_ability() {
    let (router, _, _) = signal_runtime();
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    Gate::define::<String, String>("live:tests.signal-island.stream.posts", |_, _| false);
    let body = issue_body(
        "sse",
        "tests.signal-island",
        "topic",
        "topic",
        "posts",
        "signal-alias-0001",
    );
    let denied = post_control(server.port, &alice, SUBSCRIPTION_PATH, None, body.clone()).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    Gate::define::<String, String>("live:topic.signal.posts", |_, _| false);
    Gate::define::<String, String>("live:tests.signal-island.stream.posts", |_, _| true);
    let allowed = post_control(server.port, &alice, SUBSCRIPTION_PATH, None, body).await;
    assert_eq!(
        allowed.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&allowed.body)
    );
}

#[tokio::test]
#[serial_test::serial]
async fn signal_005_large_pending_union_recovers_all_bound_props_without_dropping() {
    let hub = InMemoryBroadcastHub::new();
    let mut receiver = hub.subscribe_live_topic("signal.large-union").unwrap();
    let interval = Duration::from_millis(20);
    hub.publish_live_topic("signal.large-union", &["seed".into()], None, interval)
        .await
        .unwrap();
    receiver.recv().await.unwrap();
    for start in [0, 200] {
        let props = (start..start + 200)
            .map(|n| format!("prop_{n}"))
            .collect::<Vec<_>>();
        hub.publish_live_topic("signal.large-union", &props, None, interval)
            .await
            .unwrap();
    }
    let envelope = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(envelope.data["cursor"], 3);
    assert_eq!(envelope.data["props"], json!([]));
}

#[tokio::test]
#[serial_test::serial]
async fn signal_003_model_hooks_follow_an_explicit_write_transaction() {
    use suprnova::Model;
    let _guard = TestContainer::fake();
    ensure_crypt();
    let db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    db.execute_unprepared("CREATE TABLE signal_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)").await.unwrap();
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.model-tx")
            .unwrap()
            .build(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    topics
        .on_model::<SignalPost>("signal.model-tx", &["posts"])
        .unwrap();
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    SignalPost::create_with_tx(&tx, suprnova::attrs! {title: "rolled back"})
        .await
        .unwrap();
    assert_eq!(topics.cursor("signal.model-tx").await.unwrap(), 0);
    tx.rollback().await.unwrap();
    assert_eq!(topics.cursor("signal.model-tx").await.unwrap(), 0);
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    let post = SignalPost::create_with_tx(&tx, suprnova::attrs! {title: "committed"})
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(topics.cursor("signal.model-tx").await.unwrap(), 1);
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    post.delete_with_tx(&tx).await.unwrap();
    assert_eq!(topics.cursor("signal.model-tx").await.unwrap(), 1);
    tx.rollback().await.unwrap();
    assert_eq!(topics.cursor("signal.model-tx").await.unwrap(), 1);
}

#[suprnova::model(table = "signal_owned_posts", fillable = ["title", "user_id"])]
pub struct SignalOwnedPost {
    pub id: i64,
    pub title: String,
    pub user_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(suprnova::InertiaProps)]
#[inertia(live(topic = "signal.owned", only = ["posts"]))]
struct OwnedPage {
    posts: Vec<String>,
}

async fn owned_page_handler(request: Request) -> Response {
    let owner = suprnova::Auth::id().ok_or(FrameworkError::Unauthorized)?;
    Ok(
        suprnova::InertiaResponse::live::<OwnedPage>("__tests__/TypedProps")
            .scope("signal.owned", owner)
            .load(|| async { Ok(OwnedPage { posts: vec![] }) })
            .await?
            .resolve(&request)
            .await?,
    )
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix_002_two_users_receive_only_their_own_model_changes_over_sse() {
    use suprnova::Model;
    let _guard = TestContainer::fake();
    App::init();
    ensure_crypt();
    let db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    db.execute_unprepared("CREATE TABLE signal_owned_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, user_id TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)").await.unwrap();
    Gate::define::<String, String>("live:topic.signal.owned", |principal, instance| {
        instance == &format!("signal.owned.{principal}")
    });
    App::singleton(
        LiveRegistry::builder()
            .topic_family("signal.owned")
            .unwrap()
            .build(),
    );
    let router: Router = Router::new()
        .get("/signal/owned", owned_page_handler)
        .into();
    let router = router.try_live().unwrap();
    let runtime = prepare_live_router_for_test(&router).unwrap();
    App::singleton(runtime.clone());
    let topics = LiveTopics::from_runtime(&runtime);
    topics
        .on_model_scoped::<SignalOwnedPost>("signal.owned", |post| post.user_id.clone(), &["posts"])
        .unwrap();
    let server = spawn_server(Arc::new(router)).await;
    let alice = Identity::alice();
    let bob = Identity::alice()
        .with_principal("bob")
        .with_session("bob-owned-session");
    let mut bindings = Vec::new();
    for identity in [&alice, &bob] {
        let reply = send(
            server.port,
            identity,
            Method::GET,
            "/signal/owned",
            &[("x-inertia", "true")],
            Bytes::new(),
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        let page = reply.json();
        let (topic, binding) = page["liveBindings"]
            .as_object()
            .unwrap()
            .iter()
            .next()
            .unwrap();
        bindings.push((topic.clone(), binding.clone()));
    }
    let a = issue_page(&server, &bindings[0].1, "owned-alice-document-0001", &alice).await;
    let b = issue_page(&server, &bindings[1].1, "owned-bob-document-0001", &bob).await;
    let mut a_stream = open_membership(&server, &alice, &a, "owned-alice-nonce").await;
    let mut b_stream = open_membership(&server, &bob, &b, "owned-bob-nonce").await;
    let post =
        SignalOwnedPost::create(suprnova::attrs! { title: "private alice note", user_id: "alice" })
            .await
            .unwrap();
    assert_eq!(next_signal(&mut a_stream).await["topic"], bindings[0].0);
    assert_no_signal(&mut b_stream).await;
    assert_eq!(topics.cursor(&bindings[1].0).await.unwrap(), 0);
    post.delete().await.unwrap();
    next_signal(&mut a_stream).await;
    assert_no_signal(&mut b_stream).await;
    let _ = SignalOwnedPost::create(suprnova::attrs! { title: "private bob note", user_id: "bob" })
        .await
        .unwrap();
    assert_eq!(next_signal(&mut b_stream).await["topic"], bindings[1].0);
    assert_no_signal(&mut a_stream).await;
    assert_eq!(topics.cursor(&bindings[0].0).await.unwrap(), 2);
    assert_eq!(topics.cursor(&bindings[1].0).await.unwrap(), 1);
}

#[derive(Clone)]
struct SocketOrigin(LiveTopics);
async fn socket_origin_write(_request: Request) -> Response {
    App::resolve::<SocketOrigin>()?
        .0
        .changed("signal.posts", &["posts"])
        .await?;
    Ok(HttpResponse::json(json!({"saved": true})))
}

async fn forged_socket_regression(second_runtime: bool) {
    let (router, first, _) = signal_runtime();
    if second_runtime {
        App::singleton(prepare_live_router_for_test(&Router::new()).unwrap());
    }
    App::singleton(SocketOrigin(LiveTopics::from_runtime(&first)));
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let bob = Identity::alice()
        .with_principal("bob")
        .with_session("bob-socket-session");
    let binding = page_binding(&server).await;
    let a = issue_page(&server, &binding, "ownership-alice-document", &alice).await;
    let b = issue_page(&server, &binding, "ownership-bob-document", &bob).await;
    let mut a_stream = open_membership(&server, &alice, &a, "ownership-alice-nonce").await;
    let mut b_stream = open_membership(&server, &bob, &b, "ownership-bob-nonce").await;
    let reply = send(
        server.port,
        &alice,
        Method::POST,
        "/signal/owned-write",
        &[("x-socket-id", "ownership-bob-document")],
        Bytes::new(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(next_signal(&mut a_stream).await["cursor"], 1);
    assert_eq!(
        next_signal(&mut b_stream).await["cursor"],
        1,
        "another principal cannot suppress this membership"
    );
    let reply = send(
        server.port,
        &alice,
        Method::POST,
        "/signal/owned-write",
        &[("x-socket-id", "ownership-alice-document")],
        Bytes::new(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(next_signal(&mut b_stream).await["cursor"], 2);
    assert_no_signal(&mut a_stream).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_fix_003_forged_socket_cannot_exclude_another_principal() {
    forged_socket_regression(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_fix_003_forged_socket_cannot_exclude_a_fanout_membership() {
    forged_socket_regression(true).await;
}

#[tokio::test(start_paused = true)]
#[serial_test::serial]
async fn signal_fix2_001_publications_never_create_family_listeners() {
    let _container = TestContainer::fake();
    ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub.clone());
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic_family("churn")
            .unwrap()
            .build(),
    );
    TestContainer::singleton(
        suprnova::live::LiveConfig::builder()
            .topic_instance_retention_secs(2)
            .build()
            .unwrap(),
    );
    let runtime = prepare_live_router_for_test(&Router::new()).unwrap();
    let topics = LiveTopics::from_runtime(&runtime);
    for key in 0..64 {
        let topic = format!("churn.{key}");
        topics.changed(&topic, &["stats"]).await.unwrap();
        assert_eq!(topics.cursor(&topic).await.unwrap(), 1);
        assert_eq!(
            hub.subscriber_count(&format!("__live.topic.{topic}")),
            0,
            "publication retained a listener for {topic}"
        );
    }
    assert_eq!(
        suprnova::live::testing::prune_live_topic_listeners_for_test(&runtime),
        0
    );
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(hub.prune_live_topics().unwrap(), 64);
    assert_eq!(hub.prune_live_topics().unwrap(), 0);
    assert_eq!(
        suprnova::live::testing::prune_live_topic_listeners_for_test(&runtime),
        0
    );
    assert_eq!(topics.cursor("churn.0").await.unwrap(), 0);
    topics.changed("churn.0", &["stats"]).await.unwrap();
    assert_eq!(topics.cursor("churn.0").await.unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn signal_fix2_001_baselines_memberships_pending_and_static_state_survive_pruning() {
    let hub = InMemoryBroadcastHub::new();
    hub.configure_live_topic_family("retained", Duration::from_secs(2))
        .unwrap();
    let interval = Duration::from_millis(1);
    for topic in ["retained.read", "retained.member", "static"] {
        hub.publish_live_topic(topic, &[], None, interval)
            .await
            .unwrap();
    }
    hub.live_topic_membership("retained.member", true).unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        hub.live_topic_cursors(&["retained.read".into()])
            .await
            .unwrap(),
        [1]
    );
    tokio::time::advance(Duration::from_millis(1500)).await;
    assert_eq!(hub.prune_live_topics().unwrap(), 0);
    hub.live_topic_membership("retained.member", false).unwrap();
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(hub.prune_live_topics().unwrap(), 2);
    assert_eq!(hub.live_topic_cursor("static").await.unwrap(), 1);
    let mut receiver = hub.subscribe_live_topic("retained.pending").unwrap();
    hub.publish_live_topic(
        "retained.pending",
        &["first".into()],
        None,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    receiver.recv().await.unwrap();
    hub.publish_live_topic(
        "retained.pending",
        &["last".into()],
        None,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(
        hub.prune_live_topics().unwrap(),
        0,
        "pending invalidations must survive a shorter retention window"
    );
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(
        receiver.recv().await.unwrap().data["props"],
        json!(["last"])
    );
}

#[test]
fn signal_fix2_001_retention_configuration_is_positive_and_defaults_to_one_day() {
    assert_eq!(
        suprnova::live::LiveConfig::default().topic_instance_retention_secs(),
        86400
    );
    let error = suprnova::live::LiveConfig::builder()
        .topic_instance_retention_secs(0)
        .build()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("LIVE_TOPIC_INSTANCE_RETENTION_SECS")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn signal_fix2_001_pruning_the_last_issued_membership_releases_its_listener() {
    let clock = Arc::new(suprnova::live::testing::AdjustableTestClock::new(
        1_700_000_000_000,
    ));
    let (router, runtime, hub) = signal_runtime_with_clock(Some(clock.clone()));
    let server = spawn_server(router).await;
    let alice = Identity::alice();
    let binding = page_binding(&server).await;
    let _issued = issue_page(&server, &binding, "retention-document-0001", &alice).await;
    assert_eq!(hub.subscriber_count("__live.topic.signal.posts"), 1);
    assert_eq!(
        suprnova::live::testing::prune_live_topic_listeners_for_test(&runtime),
        1
    );
    clock.advance_ms(120001);
    assert_eq!(
        suprnova::live::testing::prune_live_topic_listeners_for_test(&runtime),
        0
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while hub.subscriber_count("__live.topic.signal.posts") != 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "aborted listener still owns its receiver"
        );
        tokio::task::yield_now().await;
    }
}

#[test]
fn signal_fix2_001_retention_environment_is_checked() {
    if let Ok(value) = std::env::var("SIGNAL_RETENTION_CHILD") {
        let result = suprnova::live::LiveConfig::from_env();
        if value == "0" {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("LIVE_TOPIC_INSTANCE_RETENTION_SECS")
            );
        } else {
            assert_eq!(result.unwrap().topic_instance_retention_secs(), 2);
        }
        return;
    }
    for value in ["0", "2"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "signals::signal_fix2_001_retention_environment_is_checked",
                "--nocapture",
            ])
            .env_clear()
            .env("SIGNAL_RETENTION_CHILD", value)
            .env("LIVE_TOPIC_INSTANCE_RETENTION_SECS", value)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn signal_fix3_003_sweep_is_shared_throttled_and_visits_only_idle_candidates() {
    let hub = InMemoryBroadcastHub::new();
    hub.configure_live_topic_family("sweep", Duration::from_secs(2))
        .unwrap();
    for key in 0..1000 {
        hub.live_topic_membership(&format!("sweep.active.{key}"), true)
            .unwrap();
    }
    for key in 0..20 {
        hub.live_topic_cursor(&format!("sweep.idle.{key}"))
            .await
            .unwrap();
    }
    hub.live_topic_cursor("static").await.unwrap();
    let before = hub.live_topic_sweep_stats_for_test();
    for key in 0..100 {
        hub.prune_live_topics().unwrap();
        let _receiver = hub
            .subscribe_live_topic(&format!("unrelated.{key}"))
            .unwrap();
        hub.live_topic_cursors(&[format!("sweep.active.{key}")])
            .await
            .unwrap();
    }
    let now = hub.live_topic_sweep_stats_for_test();
    assert!(
        now.0 - before.0 <= 1,
        "sweep ran per caller: {before:?} -> {now:?}"
    );
    assert_eq!(now.1 - before.1, 0, "nothing is eligible yet");
    tokio::time::advance(Duration::from_millis(1100)).await;
    hub.prune_live_topics().unwrap();
    let middle = hub.live_topic_sweep_stats_for_test();
    assert_eq!(middle.0 - now.0, 1);
    assert_eq!(
        middle.1 - now.1,
        0,
        "active instances must not enter the idle index"
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(hub.prune_live_topics().unwrap(), 20);
    let after = hub.live_topic_sweep_stats_for_test();
    assert_eq!(after.0 - middle.0, 1);
    assert_eq!(after.1 - middle.1, 20);
    for _ in 0..100 {
        assert_eq!(hub.prune_live_topics().unwrap(), 0);
    }
    assert_eq!(hub.live_topic_sweep_stats_for_test(), after);
}

#[derive(LiveComponent)]
#[live(
    name = "tests.family-recovery",
    view = "live/tests/signal-island.html",
    minimum_protocol_version = 2,
    streams(stream(
        name = "changes",
        topics("recovery.alice"),
        events(SignalEvent),
        targets("self"),
        fanout = 4
    ))
)]
pub struct FamilyRecoveryIsland {
    #[model]
    count: u64,
}
#[live]
impl FamilyRecoveryIsland {
    #[mount]
    pub fn mount() -> Self {
        Self { count: 0 }
    }
}

#[tokio::test]
#[serial_test::serial]
async fn signal_fix3_001_island_recovery_seed_survives_retirement_at_cursor_zero() {
    let _guard = TestContainer::fake();
    ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub.clone());
    App::singleton(
        suprnova::live::LiveConfig::builder()
            .topic_instance_retention_secs(1)
            .build()
            .unwrap(),
    );
    App::singleton(
        LiveRegistry::builder()
            .register::<FamilyRecoveryIsland>()
            .unwrap()
            .public_topic_family("recovery")
            .unwrap()
            .build(),
    );
    Gate::define::<String, String>("live:tests.family-recovery.stream.changes", |_, _| true);
    let mount = LiveMount::<FamilyRecoveryIsland>::identity_bound(
        "/recovery",
        "recovery-slot",
        "recovery-document",
    )
    .unwrap();
    let router = Router::new()
        .try_live()
        .unwrap()
        .try_live_mount(&mount)
        .unwrap();
    let runtime = prepare_live_router_for_test(&router).unwrap();
    App::singleton(runtime.clone());
    for _ in 0..10 {
        hub.publish_live_topic("recovery.alice", &[], None, Duration::from_millis(1))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let old = hub
        .live_topic_positions(&["recovery.alice".into()])
        .await
        .unwrap()[0];
    assert_eq!(old.cursor, 10);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(hub.prune_live_topics().unwrap(), 1);
    let server = spawn_server(Arc::new(router)).await;
    let alice = Identity::alice();
    let issued = issue(
        server.port,
        &alice,
        issue_body(
            "sse",
            "tests.family-recovery",
            "recovery-slot",
            "recovery-document",
            "changes",
            "retired-island-0001",
        ),
    )
    .await;
    let mut stream = open_membership(&server, &alice, &issued, "nonce-retired-island").await;
    let seed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let frame = stream.next_data().await.unwrap();
            if frame["payload"]["topic"] == "recovery.alice" {
                break frame["payload"].clone();
            }
        }
    })
    .await
    .expect("zero-cursor authority must still seed island recovery");
    assert_eq!(seed["kind"], "topic_change");
    assert_eq!(seed["cursor"], 0);
    assert_eq!(seed["props"], json!([]));
    assert!(seed["generation"].as_u64().unwrap() > old.generation);
    LiveTopics::from_runtime(&runtime)
        .changed("recovery.alice", &["stats"])
        .await
        .unwrap();
    let change = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let frame = stream.next_data().await.unwrap();
            if frame["payload"]["topic"] == "recovery.alice" {
                break frame["payload"].clone();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(change["generation"], seed["generation"]);
    assert_eq!(change["cursor"], 1);
}

#[tokio::test(start_paused = true)]
async fn signal_fix4_001_recreation_within_one_millisecond_gets_a_new_generation() {
    let hub = InMemoryBroadcastHub::new();
    hub.configure_live_topic_family("allocation", Duration::from_nanos(1))
        .unwrap();
    hub.set_live_topic_clock_for_test(1000).unwrap();
    let names = ["allocation.owner".into()];
    let first = hub.live_topic_positions(&names).await.unwrap()[0];
    tokio::time::advance(Duration::from_nanos(1)).await;
    let recreated = hub.live_topic_positions(&names).await.unwrap()[0];
    assert_eq!(first.cursor, 0);
    assert_eq!(recreated.cursor, 0);
    assert!(
        recreated.generation > first.generation,
        "{first:?} -> {recreated:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn signal_fix4_001_clock_rollback_keeps_generations_increasing() {
    let hub = InMemoryBroadcastHub::new();
    hub.configure_live_topic_family("allocation", Duration::from_nanos(1))
        .unwrap();
    hub.set_live_topic_clock_for_test(10000).unwrap();
    let names = ["allocation.owner".into()];
    let first = hub.live_topic_positions(&names).await.unwrap()[0];
    hub.set_live_topic_clock_for_test(10).unwrap();
    tokio::time::advance(Duration::from_nanos(1)).await;
    let recreated = hub.live_topic_positions(&names).await.unwrap()[0];
    assert!(
        recreated.generation > first.generation,
        "{first:?} -> {recreated:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn signal_fix5_001_authority_restart_in_the_same_millisecond_keeps_generations_distinct() {
    let hub = InMemoryBroadcastHub::new();
    hub.set_live_topic_clock_for_test(10000).unwrap();
    let positions = hub
        .live_topic_positions(&["restart.first".into(), "restart.last".into()])
        .await
        .unwrap();
    let old = positions[1];
    drop(hub);

    let restarted = InMemoryBroadcastHub::new();
    restarted.set_live_topic_clock_for_test(10000).unwrap();
    let new = restarted
        .live_topic_positions(&["restart.last".into()])
        .await
        .unwrap()[0];
    assert!(new.generation > old.generation, "{old:?} -> {new:?}");
}

#[tokio::test(start_paused = true)]
async fn signal_fix5_001_authority_restart_after_clock_rollback_keeps_generations_increasing() {
    let hub = InMemoryBroadcastHub::new();
    hub.set_live_topic_clock_for_test(10000).unwrap();
    let old = hub
        .live_topic_positions(&["restart.owner".into()])
        .await
        .unwrap()[0];
    drop(hub);

    let restarted = InMemoryBroadcastHub::new();
    restarted.set_live_topic_clock_for_test(1000).unwrap();
    let new = restarted
        .live_topic_positions(&["restart.owner".into()])
        .await
        .unwrap()[0];
    assert!(new.generation > old.generation, "{old:?} -> {new:?}");
}
