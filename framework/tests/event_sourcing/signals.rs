use super::*;
use crate::live_async_support::{self as transport, Identity, TestServer};
use std::sync::Arc;
use std::time::Duration;
use suprnova::broadcasting::{BroadcastHub, InMemoryBroadcastHub};
use suprnova::live::{LiveComponent, LiveMount, LiveRegistry, LiveTopics, live};
use suprnova::{
    Aggregate, App, DB, EventFacade, EventReplay, Gate, Listener, Model, Projector, Router, attrs,
};

const TOPIC: &str = "es-watch";
const OTHER_TOPIC: &str = "es-watch-summary";
#[derive(LiveComponent)]
#[live(
    name = "tests.es-watch",
    view = "live/tests/async-orders.html",
    minimum_protocol_version = 2,
    streams(stream(
        name = "changes",
        topics("es-watch"),
        events(crate::live_async_support::OrdersUpdated)
    ))
)]
pub struct Watch {
    #[model]
    filter: String,
}
#[live]
impl Watch {
    #[mount]
    pub fn mount() -> Self {
        Self {
            filter: String::new(),
        }
    }
}
#[domain_event(name = "es.signal.added", version = 1)]
#[derive(Serialize, Deserialize)]
struct SignalAdded {
    amount: i64,
}
#[suprnova::model(table = "es_signals", timestamps = false, fillable = ["total"])]
struct SignalRow {
    pub id: i64,
    pub total: i64,
}
#[suprnova::model(table = "es_plain", timestamps = false, fillable = ["total"])]
struct Plain {
    pub id: i64,
    pub total: i64,
}

#[suprnova::projector]
#[derive(Default)]
struct Signals;
#[suprnova::async_trait]
impl Projector for Signals {
    fn name(&self) -> &'static str {
        "es.signals"
    }
    fn handles(&self, fact: &StoredEvent) -> bool {
        fact.event_type == "es.signal.added"
    }
    async fn project(&self, fact: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(fact) = fact.decode::<SignalAdded>()? {
            let mut row = SignalRow::find(1i64)
                .await?
                .ok_or_else(|| FrameworkError::internal("signal read model missing"))?;
            row.total += fact.amount;
            row.save().await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        SignalRow::query()
            .update_all(attrs! { total: 0i64 })
            .await?;
        Ok(())
    }
}
struct Changed(LiveTopics);
#[suprnova::async_trait]
impl Listener<signal_row::events::Saved> for Changed {
    async fn handle(&self, event: &signal_row::events::Saved) -> Result<(), FrameworkError> {
        let prop = if event.model.total % 2 == 0 {
            "even"
        } else {
            "odd"
        };
        self.0.changed(TOPIC, &["total", prop]).await?;
        self.0.changed(OTHER_TOPIC, &[prop]).await?;
        if EventReplay::is_running() {
            for topic in [TOPIC, OTHER_TOPIC] {
                if self.0.cursor(topic).await? != 0 {
                    return Err(FrameworkError::internal(
                        "replay published an intermediate signal",
                    ));
                }
            }
        }
        Ok(())
    }
}
#[suprnova::async_trait]
impl Listener<plain::events::Saved> for Changed {
    async fn handle(&self, _: &plain::events::Saved) -> Result<(), FrameworkError> {
        self.0.changed(TOPIC, &["total"]).await
    }
}

struct Fixture {
    server: TestServer,
    hub: Arc<InMemoryBroadcastHub>,
    signals: tokio::sync::Mutex<tokio::sync::broadcast::Receiver<suprnova::BroadcastEnvelope>>,
    summaries: tokio::sync::Mutex<tokio::sync::broadcast::Receiver<suprnova::BroadcastEnvelope>>,
    topics: LiveTopics,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        EventFacade::forget::<signal_row::events::Saved>();
        EventFacade::forget::<plain::events::Saved>();
    }
}
async fn fixture() -> Fixture {
    App::init();
    transport::ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub.clone());
    App::singleton(
        suprnova::live::LiveConfig::builder()
            .topic_signal_interval_ms(1)
            .build()
            .unwrap(),
    );
    let signals = hub.subscribe_live_topic(TOPIC).unwrap();
    let summaries = hub.subscribe_live_topic(OTHER_TOPIC).unwrap();
    Gate::define::<String, String>("live:tests.es-watch.stream.changes", |_, _| true);
    for topic in [TOPIC, OTHER_TOPIC] {
        Gate::define::<String, String>(&format!("live:topic.{topic}"), |principal, _| {
            !principal.is_empty()
        });
    }
    App::singleton(
        LiveRegistry::builder()
            .register::<Watch>()
            .unwrap()
            .topic(TOPIC)
            .unwrap()
            .topic(OTHER_TOPIC)
            .unwrap()
            .build(),
    );
    let mount = LiveMount::<Watch>::identity_bound("/es-watch", "es-slot", "es-document").unwrap();
    let router: Router = Router::new().post("/es-write", request_write).into();
    let router: Router = router.post("/es-rollback", request_rollback).into();
    let router = router.try_live().unwrap().try_live_mount(&mount).unwrap();
    let runtime = suprnova::live::testing::prepare_live_router_for_test(&router).unwrap();
    App::singleton(runtime.clone());
    let topics = LiveTopics::from_runtime(&runtime);
    EventFacade::listen::<signal_row::events::Saved, _>(Arc::new(Changed(topics.clone()))).await;
    EventFacade::listen::<plain::events::Saved, _>(Arc::new(Changed(topics.clone()))).await;
    let server = transport::spawn_server(Arc::new(router)).await;
    Fixture {
        server,
        topics,
        hub,
        signals: tokio::sync::Mutex::new(signals),
        summaries: tokio::sync::Mutex::new(summaries),
    }
}
async fn tail(fixture: &Fixture) -> Vec<serde_json::Value> {
    let mut receiver = fixture.signals.lock().await;
    let mut signals = Vec::new();
    while let Ok(signal) = receiver.try_recv() {
        signals.push(signal.data);
    }
    signals
}
async fn request_write(_: suprnova::Request) -> suprnova::Response {
    let mut root = Aggregate::<Tally>::new(&uuid())?;
    for _ in 0..4 {
        root.record(SignalAdded { amount: 1 }).await?;
    }
    root.persist().await?;
    LiveTopics::resolve()?.changed(TOPIC, &["request"]).await?;
    Ok(suprnova::HttpResponse::json(
        serde_json::json!({"saved": true}),
    ))
}
async fn request_rollback(_: suprnova::Request) -> suprnova::Response {
    DB::transaction(|_| {
        Box::pin(async {
            Plain::create(attrs! { total: 9i64 }).await?;
            LiveTopics::resolve()?
                .changed(TOPIC, &["rolled_back"])
                .await?;
            Err::<(), _>(FrameworkError::bad_request("rollback signal writes"))
        })
    })
    .await?;
    Ok(suprnova::HttpResponse::json(
        serde_json::json!({"saved": true}),
    ))
}
async fn setup() -> TestDatabase {
    let db = database().await;
    db.execute_unprepared("CREATE TABLE es_signals (id INTEGER PRIMARY KEY, total BIGINT NOT NULL); INSERT INTO es_signals VALUES (1, 0); CREATE TABLE es_plain (id INTEGER PRIMARY KEY AUTOINCREMENT, total BIGINT NOT NULL)").await.unwrap();
    db
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_replay_signals_once_after_all_projector_writes() {
    let _db = setup().await;
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    for _ in 0..10 {
        root.record(SignalAdded { amount: 1 }).await.unwrap();
    }
    root.persist().await.unwrap();
    let fixture = fixture().await;
    EventReplay::projectors(["es.signals"])
        .page_size(2)
        .run()
        .await
        .unwrap();
    assert_eq!(SignalRow::find(1i64).await.unwrap().unwrap().total, 10);
    let log = tail(&fixture).await;
    assert_eq!(
        log.len(),
        1,
        "ten writes produce one committed signal: {log:?}"
    );
    assert_eq!(log[0]["props"], serde_json::json!(["even", "odd", "total"]));
    assert!(log[0].get("total").is_none());
    assert_eq!(fixture.topics.cursor(TOPIC).await.unwrap(), 1);
    let summary = fixture.summaries.lock().await.try_recv().unwrap().data;
    assert_eq!(summary["topic"], OTHER_TOPIC);
    assert_eq!(summary["props"], serde_json::json!(["even", "odd"]));
    assert_eq!(fixture.topics.cursor(OTHER_TOPIC).await.unwrap(), 1);
    assert!(fixture.summaries.lock().await.try_recv().is_err());
}

#[tokio::test]
#[serial_test::serial]
async fn es_004_projector_committed_model_write_signals_a_topic() {
    let _db = setup().await;
    let fixture = fixture().await;
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(SignalAdded { amount: 4 }).await.unwrap();
    root.persist().await.unwrap();
    assert_eq!(SignalRow::find(1i64).await.unwrap().unwrap().total, 4);
    let log = tail(&fixture).await;
    assert_eq!(log.len(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn es_004_plain_model_event_and_explicit_change_need_no_aggregate() {
    let _db = setup().await;
    let fixture = fixture().await;
    Plain::create(attrs! { total: 7i64 }).await.unwrap();
    fixture.topics.changed(TOPIC, &["total"]).await.unwrap();
    assert_eq!(suprnova::EventStore::last_position().await.unwrap(), 0);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(tail(&fixture).await.len(), 2);
}

#[tokio::test]
#[serial_test::serial]
async fn es_004_rolled_back_model_write_does_not_signal() {
    let _db = setup().await;
    let fixture = fixture().await;
    let result: Result<(), FrameworkError> = DB::transaction(|_| {
        Box::pin(async {
            Plain::create(attrs! { total: 9i64 }).await?;
            Err(FrameworkError::internal("rollback plain model"))
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(Plain::query().count().await.unwrap(), 0);
    assert_eq!(tail(&fixture).await.len(), 0);
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_public_suppression_merges_into_an_outer_batch() {
    let _db = setup().await;
    let fixture = fixture().await;
    let topics = fixture.topics.clone();
    topics
        .batch(async {
            topics.changed(TOPIC, &["first"]).await?;
            topics
                .suppress(|| async {
                    for _ in 0..10 {
                        topics.changed(TOPIC, &["total"]).await?;
                    }
                    Ok::<(), FrameworkError>(())
                })
                .await??;
            topics.changed(TOPIC, &["last"]).await?;
            Ok::<(), FrameworkError>(())
        })
        .await
        .unwrap()
        .unwrap();
    let log = tail(&fixture).await;
    assert_eq!(log.len(), 1);
    assert_eq!(
        log[0]["props"],
        serde_json::json!(["first", "last", "total"])
    );
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_request_coalesces_projector_and_explicit_changes() {
    let _db = setup().await;
    let fixture = fixture().await;
    let response = transport::send(
        fixture.server.port,
        &Identity::alice(),
        hyper::Method::POST,
        "/es-write",
        &[],
        bytes::Bytes::new(),
    )
    .await;
    assert_eq!(
        response.status,
        hyper::StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let log = tail(&fixture).await;
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(
        log[0]["props"],
        serde_json::json!(["even", "odd", "request", "total"])
    );
    assert_eq!(fixture.hub.live_topic_cursor(TOPIC).await.unwrap(), 1);
    assert_eq!(SignalRow::find(1i64).await.unwrap().unwrap().total, 4);
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_request_rollback_discards_every_prop_and_signal() {
    let _db = setup().await;
    let fixture = fixture().await;
    let response = transport::send(
        fixture.server.port,
        &Identity::alice(),
        hyper::Method::POST,
        "/es-rollback",
        &[],
        bytes::Bytes::new(),
    )
    .await;
    assert_eq!(response.status, hyper::StatusCode::BAD_REQUEST);
    assert_eq!(Plain::query().count().await.unwrap(), 0);
    assert!(tail(&fixture).await.is_empty());
    assert_eq!(fixture.topics.cursor(TOPIC).await.unwrap(), 0);
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_manual_change_waits_for_commit_and_discards_rollback() {
    let _db = setup().await;
    let fixture = fixture().await;
    let tx = DB::begin_transaction().await.unwrap();
    fixture
        .topics
        .changed_with_tx(&tx, TOPIC, &["discarded"])
        .await
        .unwrap();
    assert!(tail(&fixture).await.is_empty());
    tx.rollback().await.unwrap();
    assert_eq!(fixture.topics.cursor(TOPIC).await.unwrap(), 0);
    let tx = DB::begin_transaction().await.unwrap();
    fixture
        .topics
        .changed_with_tx(&tx, TOPIC, &["committed"])
        .await
        .unwrap();
    assert!(tail(&fixture).await.is_empty());
    tx.commit().await.unwrap();
    let log = tail(&fixture).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["props"], serde_json::json!(["committed"]));
    assert_eq!(fixture.topics.cursor(TOPIC).await.unwrap(), 1);
}
