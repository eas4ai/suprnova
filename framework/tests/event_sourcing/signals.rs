use super::*;
use crate::live_async_support::{self as transport, Identity, Issued, TestServer};
use std::sync::Arc;
use suprnova::live::{LiveComponent, LiveMount, LiveRegistry, LiveTopics, live};
use suprnova::{
    Aggregate, App, DB, EventFacade, EventReplay, Gate, Listener, Model, Projector, Router, attrs,
};

const TOPIC: &str = "es-watch";
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
    async fn handle(&self, _: &signal_row::events::Saved) -> Result<(), FrameworkError> {
        self.0.changed(TOPIC, &["total"]).await
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
    issued: Issued,
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
    Gate::define::<String, String>("live:tests.es-watch.stream.changes", |_, _| true);
    App::singleton(LiveRegistry::builder().register::<Watch>().unwrap().build());
    let mount = LiveMount::<Watch>::identity_bound("/es-watch", "es-slot", "es-document").unwrap();
    let router = Router::new()
        .try_live()
        .unwrap()
        .try_live_mount(&mount)
        .unwrap();
    let runtime = suprnova::live::testing::prepare_live_router_for_test(&router).unwrap();
    App::singleton(runtime.clone());
    let topics = LiveTopics::from_runtime(&runtime);
    EventFacade::listen::<signal_row::events::Saved, _>(Arc::new(Changed(topics.clone()))).await;
    EventFacade::listen::<plain::events::Saved, _>(Arc::new(Changed(topics.clone()))).await;
    let server = transport::spawn_server(Arc::new(router)).await;
    let issued = transport::issue(
        server.port,
        &Identity::alice(),
        transport::issue_body(
            "sse",
            "tests.es-watch",
            "es-slot",
            "es-document",
            "changes",
            "es-document-0001",
        ),
    )
    .await;
    Fixture {
        server,
        issued,
        topics,
    }
}
async fn tail(fixture: &Fixture) -> Vec<serde_json::Value> {
    let prior = &fixture.issued;
    let body = serde_json::json!({
        "protocol_version": 1, "operation": "renew", "transport": "sse", "stream": "changes",
        "island": { "component": "tests.es-watch", "slot": "es-slot", "document_key": "es-document" },
        "document_instance": "es-document-0001",
        "prior": { "subscription_id": prior.subscription_id, "descriptor_binding": prior.descriptor_binding },
        "position": { "epoch": prior.baseline.0, "sequence": "0" }
    });
    let reply = transport::post_control(
        fixture.server.port,
        &Identity::alice(),
        transport::SUBSCRIPTION_PATH,
        None,
        body,
    )
    .await;
    assert!(
        reply.status.is_success(),
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json()["replay"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| serde_json::from_str(value.as_str().unwrap()).unwrap())
        .collect()
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
    assert!(log[0]["payload"].get("total").is_none());
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
    assert_eq!(tail(&fixture).await.len(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn es_004_plain_model_event_and_explicit_change_need_no_aggregate() {
    let _db = setup().await;
    let fixture = fixture().await;
    Plain::create(attrs! { total: 7i64 }).await.unwrap();
    fixture.topics.changed(TOPIC, &["total"]).await.unwrap();
    assert_eq!(suprnova::EventStore::last_position().await.unwrap(), 0);
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
    assert_eq!(tail(&fixture).await.len(), 1);
}
