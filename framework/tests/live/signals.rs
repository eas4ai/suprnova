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
    suprnova::inertia_response!(
        &request,
        "__tests__/TypedProps",
        SignalPage {
            posts: vec![],
            count: 0
        }
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
    let router = router.try_live().unwrap().try_live_mount(&mount).unwrap();
    let runtime = prepare_live_router_for_test(&router).unwrap();
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
            if value["payload"]["kind"] == "topic_change" {
                return value["payload"].clone();
            }
        }
    })
    .await
    .expect("signal arrives")
}

async fn assert_no_signal(stream: &mut SseClient) {
    let _ = tokio::time::timeout(Duration::from_millis(250), async {
        while let Some(value) = stream.next_data().await {
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
            if value["payload"]["kind"] == "topic_change" {
                return value["payload"].clone();
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
        json!({"topic":"signal.posts", "cursor":1, "props":["count", "posts"]})
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
    let event = tokio::time::timeout(Duration::from_secs(2), island_stream.next_data())
        .await
        .unwrap()
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
        assert_eq!(first.data, json!({"topic":topic, "cursor":1, "props":[]}));
        assert!(receiver.try_recv().is_err());
        let last = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(90));
        assert_eq!(last.data, json!({"topic":topic, "cursor":20, "props":[]}));
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
        json!({"topic":"signal.stream-tx", "cursor":1, "props":[]})
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
        .unwrap();
    assert_eq!(topics.cursor("signal.manual").await.unwrap(), 0);
    tx.rollback().await.unwrap();
    assert_eq!(topics.cursor("signal.manual").await.unwrap(), 0);
    let tx = suprnova::DB::begin_transaction().await.unwrap();
    topics
        .changed_with_tx(&tx, "signal.manual", &["posts"])
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
