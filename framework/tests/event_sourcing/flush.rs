use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use suprnova::broadcasting::{BroadcastHub, InMemoryBroadcastHub};
use suprnova::live::{LiveConfig, LiveRegistry, LiveTopics};
use suprnova::{Aggregate, App, BroadcastEnvelope, DB, EventReplay, Gate, Projector, Router};

const FIRST: &str = "es.flush.a";
const SECOND: &str = "es.flush.b";

struct ParkHub {
    inner: InMemoryBroadcastHub,
    first: AtomicBool,
    fail: bool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    attempts: Mutex<Vec<String>>,
}
impl ParkHub {
    fn new(fail: bool) -> Self {
        Self {
            inner: InMemoryBroadcastHub::new(),
            first: AtomicBool::new(true),
            fail,
            entered: Default::default(),
            resume: Default::default(),
            attempts: Default::default(),
        }
    }
}
#[suprnova::async_trait]
impl BroadcastHub for ParkHub {
    fn subscribe(&self, channel: &str) -> tokio::sync::broadcast::Receiver<BroadcastEnvelope> {
        self.inner.subscribe(channel)
    }
    async fn publish(&self, envelope: BroadcastEnvelope) -> Result<(), FrameworkError> {
        self.inner.publish(envelope).await
    }
    fn subscribe_live_topic(
        &self,
        topic: &str,
    ) -> Result<tokio::sync::broadcast::Receiver<BroadcastEnvelope>, FrameworkError> {
        self.inner.subscribe_live_topic(topic)
    }
    async fn live_topic_cursor(&self, topic: &str) -> Result<u64, FrameworkError> {
        self.inner.live_topic_cursor(topic).await
    }
    async fn live_topic_cursors(&self, topics: &[String]) -> Result<Vec<u64>, FrameworkError> {
        self.inner.live_topic_cursors(topics).await
    }
    async fn live_topic_positions(
        &self,
        topics: &[String],
    ) -> Result<Vec<suprnova::LiveTopicPosition>, FrameworkError> {
        self.inner.live_topic_positions(topics).await
    }
    async fn publish_live_topic(
        &self,
        topic: &str,
        props: &[String],
        except: Option<suprnova::LiveTopicExclusion>,
        interval: Duration,
    ) -> Result<(), FrameworkError> {
        self.attempts.lock().unwrap().push(topic.to_owned());
        if self.first.swap(false, Ordering::AcqRel) {
            self.entered.notify_one();
            self.resume.notified().await;
            if self.fail {
                return Err(FrameworkError::internal("blocked hub flush failure"));
            }
        }
        self.inner
            .publish_live_topic(topic, props, except, interval)
            .await
    }
}

#[domain_event(name = "es.flush.fact", version = 1)]
#[derive(Serialize, Deserialize)]
struct FlushFact;
#[suprnova::projector]
#[derive(Default)]
struct FlushProjection;
#[suprnova::async_trait]
impl Projector for FlushProjection {
    fn name(&self) -> &'static str {
        "es.flush"
    }
    fn handles(&self, event: &StoredEvent) -> bool {
        event.event_type == "es.flush.fact"
    }
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if EventReplay::is_running() {
            DB::statement("UPDATE es_flush_rows SET total = total + 1", vec![]).await?;
            let prop = if event.aggregate_version == 1 {
                "first"
            } else {
                "last"
            };
            let topics = LiveTopics::resolve()?;
            topics.changed(FIRST, &[prop]).await?;
            topics.changed(SECOND, &[prop, "summary"]).await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

fn publisher(hub: Arc<ParkHub>) -> LiveTopics {
    App::bind::<dyn BroadcastHub>(hub);
    App::singleton(
        LiveConfig::builder()
            .topic_signal_interval_ms(1)
            .build()
            .unwrap(),
    );
    for topic in [FIRST, SECOND] {
        Gate::define::<String, String>(&format!("live:topic.{topic}"), |principal, _| {
            !principal.is_empty()
        });
    }
    App::singleton(
        LiveRegistry::builder()
            .topic(FIRST)
            .unwrap()
            .topic(SECOND)
            .unwrap()
            .build(),
    );
    let runtime = suprnova::live::testing::prepare_live_router_for_test(&Router::new()).unwrap();
    App::singleton(runtime.clone());
    LiveTopics::from_runtime(&runtime)
}
async fn seed() -> TestDatabase {
    let db = database().await;
    db.execute_unprepared(
        "CREATE TABLE es_flush_rows (total BIGINT NOT NULL); INSERT INTO es_flush_rows VALUES (0)",
    )
    .await
    .unwrap();
    let mut aggregate = Aggregate::<Tally>::new(&uuid()).unwrap();
    aggregate.record(FlushFact).await.unwrap();
    aggregate.record(FlushFact).await.unwrap();
    aggregate.persist().await.unwrap();
    db
}
async fn durable_rows() -> i64 {
    DB::scalar("SELECT total FROM es_flush_rows", vec![])
        .await
        .unwrap()
}
async fn cancelled_flush(replay: bool) {
    let _db = seed().await;
    let hub = Arc::new(ParkHub::new(false));
    let topics = publisher(hub.clone());
    let mut first = hub.subscribe_live_topic(FIRST).unwrap();
    let mut second = hub.subscribe_live_topic(SECOND).unwrap();
    {
        let work = async {
            if replay {
                EventReplay::projectors(["es.flush"])
                    .page_size(1)
                    .run()
                    .await?;
            } else {
                let change = topics.clone();
                topics
                    .batch(async move {
                        DB::transaction(move |_| {
                            Box::pin(async move {
                                DB::statement("UPDATE es_flush_rows SET total = 2", vec![]).await?;
                                for prop in ["first", "last"] {
                                    change.changed(FIRST, &[prop]).await?;
                                    change.changed(SECOND, &[prop, "summary"]).await?;
                                }
                                Ok::<(), FrameworkError>(())
                            })
                        })
                        .await
                    })
                    .await??;
            }
            Ok::<(), FrameworkError>(())
        };
        let mut work = std::pin::pin!(work);
        tokio::select! {
            result = &mut work => panic!("flush completed before the publisher was released: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(5), hub.entered.notified()) => result.unwrap(),
        }
        assert_eq!(
            durable_rows().await,
            2,
            "the transaction commits before the final flush"
        );
    }
    hub.resume.notify_one();
    let a = tokio::time::timeout(Duration::from_secs(3), first.recv())
        .await
        .expect("first committed union survives cancellation")
        .unwrap();
    let b = tokio::time::timeout(Duration::from_secs(3), second.recv())
        .await
        .expect("remaining committed union survives cancellation")
        .unwrap();
    assert_eq!(a.data["props"], serde_json::json!(["first", "last"]));
    assert_eq!(
        b.data["props"],
        serde_json::json!(["first", "last", "summary"])
    );
    assert_eq!(*hub.attempts.lock().unwrap(), [FIRST, SECOND]);
    assert!(first.try_recv().is_err());
    assert!(second.try_recv().is_err());
    assert_eq!(topics.cursor(FIRST).await.unwrap(), 1);
    assert_eq!(topics.cursor(SECOND).await.unwrap(), 1);
}
#[tokio::test]
#[serial_test::serial]
async fn es_003_cancelled_replay_final_flush_delivers_every_union() {
    cancelled_flush(true).await;
}
#[tokio::test]
#[serial_test::serial]
async fn es_003_cancelled_batch_final_flush_delivers_every_union() {
    cancelled_flush(false).await;
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_final_flush_returns_failure_and_attempts_remaining_topics() {
    let _db = seed().await;
    let hub = Arc::new(ParkHub::new(true));
    let _topics = publisher(hub.clone());
    let mut second = hub.subscribe_live_topic(SECOND).unwrap();
    let replay = EventReplay::projectors(["es.flush"]).run();
    let mut replay = std::pin::pin!(replay);
    tokio::select! {
        result = &mut replay => panic!("flush completed before the publisher was released: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(5), hub.entered.notified()) => result.unwrap(),
    }
    assert_eq!(durable_rows().await, 2);
    hub.resume.notify_one();
    let error = replay.await.unwrap_err();
    assert!(
        error.to_string().contains("blocked hub flush failure"),
        "{error}"
    );
    assert_eq!(*hub.attempts.lock().unwrap(), [FIRST, SECOND]);
    let signal = second.try_recv().unwrap();
    assert_eq!(
        signal.data["props"],
        serde_json::json!(["first", "last", "summary"])
    );
    assert!(second.try_recv().is_err());
}
