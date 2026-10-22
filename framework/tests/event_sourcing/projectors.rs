use super::*;
use suprnova::{
    Aggregate, DB, EventReplay, EventStore, Job, Model, Projector, Queue, QueuedProjector, Reactor,
    attrs,
};

#[domain_event(name = "es.projection.added", version = 1)]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Projected {
    amount: i64,
}

#[suprnova::model(table = "es_projection", timestamps = false, fillable = ["total"])]
struct ReadModel {
    pub id: i64,
    pub total: i64,
}

#[suprnova::projector]
#[derive(Default)]
struct Inline;
#[suprnova::async_trait]
impl Projector for Inline {
    fn name(&self) -> &'static str {
        "es.inline"
    }
    fn handles(&self, event: &StoredEvent) -> bool {
        event.event_type == "es.projection.added"
    }
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(event) = event.decode::<Projected>()? {
            let mut row = ReadModel::find(1i64)
                .await?
                .ok_or_else(|| FrameworkError::internal("read model missing"))?;
            row.total += event.amount;
            row.save().await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        ReadModel::query()
            .update_all(attrs! { total: 0i64 })
            .await?;
        DB::table("es_lifecycle")
            .insert(attrs! { stage: "reset" })
            .await?;
        Ok(())
    }
    async fn replay_started(&self) -> Result<(), FrameworkError> {
        DB::table("es_lifecycle")
            .insert(attrs! { stage: "start" })
            .await?;
        Ok(())
    }
    async fn replay_finished(&self) -> Result<(), FrameworkError> {
        DB::table("es_lifecycle")
            .insert(attrs! { stage: "finish" })
            .await?;
        Ok(())
    }
}
#[suprnova::projector(queued)]
#[derive(Default)]
struct Deferred;
#[suprnova::async_trait]
impl Projector for Deferred {
    fn name(&self) -> &'static str {
        "es.queued"
    }
    fn handles(&self, event: &StoredEvent) -> bool {
        event.event_type == "es.projection.added"
    }
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(fact) = event.decode::<Projected>()? {
            // This observation fixture checks scheduling, not a supported projection dependency.
            let observed = if event
                .metadata
                .get("observe_s")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                ReadModel::find(1i64)
                    .await?
                    .ok_or_else(|| FrameworkError::internal("read model missing"))?
                    .total
            } else {
                0
            };
            DB::table("es_order")
                .insert(attrs! { event_id: event.id, amount: fact.amount, observed: observed })
                .await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        DB::table("es_order").delete().await?;
        Ok(())
    }
}
#[suprnova::reactor]
#[derive(Default)]
struct Notify;
#[suprnova::async_trait]
impl Reactor for Notify {
    fn name(&self) -> &'static str {
        "es.notify"
    }
    async fn react(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if event.decode::<Projected>()?.is_some() {
            if DB::transaction_level() != 0 || EventReplay::is_running() {
                return Err(FrameworkError::internal(
                    "reactor ran before commit or during replay",
                ));
            }
            assert!(!EventStore::events(&event.aggregate_uuid).await?.is_empty());
            DB::table("es_reactions")
                .insert(attrs! { event_id: event.id })
                .await?;
        }
        Ok(())
    }
}

pub(super) async fn setup() -> TestDatabase {
    let db = database().await;
    db.execute_unprepared("CREATE TABLE es_projection (id INTEGER PRIMARY KEY, total BIGINT NOT NULL); INSERT INTO es_projection VALUES (1, 0); CREATE TABLE es_order (id INTEGER PRIMARY KEY AUTOINCREMENT, event_id BIGINT NOT NULL, amount BIGINT NOT NULL, observed BIGINT NOT NULL); CREATE TABLE es_reactions (id INTEGER PRIMARY KEY AUTOINCREMENT, event_id BIGINT NOT NULL); CREATE TABLE es_lifecycle (id INTEGER PRIMARY KEY AUTOINCREMENT, stage TEXT NOT NULL)").await.unwrap();
    db
}
async fn append(amounts: &[i64]) -> Vec<StoredEvent> {
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    for amount in amounts {
        root.record(Projected { amount: *amount }).unwrap();
    }
    root.persist().await.unwrap()
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_replay_resets_without_double_counting_or_reactors() {
    let _queue = Queue::fake();
    let _db = setup().await;
    append(&[2, 3, 5]).await;
    assert_eq!(ReadModel::find(1i64).await.unwrap().unwrap().total, 10);
    assert_eq!(DB::table("es_reactions").count().await.unwrap(), 3);
    for _ in 0..2 {
        assert_eq!(
            EventReplay::projectors(["es.inline"])
                .page_size(1)
                .run()
                .await
                .unwrap()
                .events,
            3
        );
        assert_eq!(ReadModel::find(1i64).await.unwrap().unwrap().total, 10);
    }
    assert_eq!(DB::table("es_reactions").count().await.unwrap(), 3);
    let stages = DB::table("es_lifecycle")
        .order_by_asc("id")
        .get()
        .await
        .unwrap();
    let stages: Vec<String> = stages
        .iter()
        .map(|row| row.get_string("stage").unwrap())
        .collect();
    assert_eq!(
        stages,
        ["start", "reset", "finish", "start", "reset", "finish"]
    );
    assert!(!EventReplay::is_running());
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_projector_failure_rolls_back_facts_and_read_models() {
    let _queue = Queue::fake();
    let db = setup().await;
    db.execute_unprepared("CREATE TRIGGER fail_projection BEFORE UPDATE ON es_projection WHEN NEW.total > 3 BEGIN SELECT RAISE(ABORT, 'projection refused'); END").await.unwrap();
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(Projected { amount: 2 }).unwrap();
    root.record(Projected { amount: 3 }).unwrap();
    assert!(root.persist().await.is_err());
    assert!(EventStore::events(root.uuid()).await.unwrap().is_empty());
    assert_eq!(ReadModel::find(1i64).await.unwrap().unwrap().total, 0);
    assert_eq!(DB::table("es_reactions").count().await.unwrap(), 0);
    assert!(suprnova::queue::testing::pushed::<QueuedProjector>().is_empty());
    assert_eq!(root.recorded().len(), 2);
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_queued_projector_shuffled_delivery_retries_in_order_once() {
    let _queue = Queue::fake();
    let db = setup().await;
    let first = append(&[10]).await;
    let second = append(&[20]).await;
    let third = append(&[30]).await;
    let jobs = suprnova::queue::testing::pushed::<QueuedProjector>();
    assert_eq!(jobs.len(), 3);
    assert_eq!(DB::table("es_order").count().await.unwrap(), 0);
    db.execute_unprepared("CREATE TRIGGER fail_queue BEFORE INSERT ON es_order WHEN NEW.amount = 20 BEGIN SELECT RAISE(ABORT, 'transient queue error'); END").await.unwrap();
    let dispatch = |job: &QueuedProjector| {
        let value = serde_json::to_value(job).unwrap();
        suprnova::queue::worker::dispatch_by_name(QueuedProjector::job_name(), value)
    };
    assert!(dispatch(&jobs[2]).await.is_err());
    assert_eq!(DB::table("es_order").count().await.unwrap(), 0);
    db.execute_unprepared("DROP TRIGGER fail_queue")
        .await
        .unwrap();
    dispatch(&jobs[2]).await.unwrap();
    dispatch(&jobs[0]).await.unwrap();
    dispatch(&jobs[1]).await.unwrap();
    dispatch(&jobs[2]).await.unwrap();
    let rows = DB::table("es_order")
        .order_by_asc("id")
        .get()
        .await
        .unwrap();
    let ids: Vec<i64> = rows
        .iter()
        .map(|row| row.get_int("event_id").unwrap())
        .collect();
    assert_eq!(ids, [first[0].id, second[0].id, third[0].id]);
    assert_eq!(QueuedProjector::max_tries(), 5);
    assert_eq!(
        DB::table("projector_checkpoints")
            .filter("name", "es.queued")
            .first()
            .await
            .unwrap()
            .unwrap()
            .get_int("last_event_id")
            .unwrap(),
        third[0].id
    );
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_replay_failure_rolls_back_reset_and_checkpoint() {
    let _queue = Queue::fake();
    let db = setup().await;
    append(&[1, 2]).await;
    let checkpoint = DB::table("projector_checkpoints")
        .filter("name", "es.inline")
        .first()
        .await
        .unwrap()
        .unwrap()
        .get_int("last_event_id")
        .unwrap();
    db.execute_unprepared("CREATE TRIGGER fail_replay BEFORE UPDATE ON es_projection WHEN NEW.total = 1 BEGIN SELECT RAISE(ABORT, 'replay failed'); END").await.unwrap();
    assert!(
        EventReplay::projectors(["es.inline"])
            .page_size(1)
            .run()
            .await
            .is_err()
    );
    assert_eq!(ReadModel::find(1i64).await.unwrap().unwrap().total, 3);
    assert_eq!(
        DB::table("projector_checkpoints")
            .filter("name", "es.inline")
            .first()
            .await
            .unwrap()
            .unwrap()
            .get_int("last_event_id")
            .unwrap(),
        checkpoint
    );
    assert!(!EventReplay::is_running());
    db.execute_unprepared("DROP TRIGGER fail_replay")
        .await
        .unwrap();
    EventReplay::projectors(["es.inline"]).run().await.unwrap();
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_reactor_failure_preserves_the_committed_version_without_recording_twice() {
    let _queue = Queue::fake();
    let db = setup().await;
    db.execute_unprepared("CREATE TRIGGER fail_reactor BEFORE INSERT ON es_reactions BEGIN SELECT RAISE(ABORT, 'reactor failed'); END").await.unwrap();
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(Projected { amount: 2 }).unwrap();
    let error = root.persist().await.unwrap_err().to_string();
    assert!(error.contains("reactor failed"), "{error}");
    assert_eq!(root.persisted_version(), 1);
    assert!(root.recorded().is_empty());
    assert_eq!(EventStore::events(root.uuid()).await.unwrap().len(), 1);
    assert!(root.persist().await.unwrap().is_empty());
    assert_eq!(ReadModel::find(1i64).await.unwrap().unwrap().total, 2);
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_checkpoint_names_reject_padding_and_control_characters() {
    let _db = database().await;
    for name in ["es.queued ", " es.queued", "es.\0queued"] {
        assert!(
            QueuedProjector::new(name, 1).is_err(),
            "ambiguous checkpoint name: {name:?}"
        );
    }
    assert!(QueuedProjector::new("es.queued", 1).is_ok());
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_live_and_replay_keep_the_same_projector_event_order() {
    use crate::render_cache_middleware_support::{Post, post};
    use std::sync::{Arc, Mutex};
    use suprnova::{EventFacade, Listener};
    let _queue = Queue::fake();
    let db = setup().await;
    db.execute_unprepared("CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, views BIGINT NOT NULL DEFAULT 0)").await.unwrap();
    let post = Post::create(attrs! { title: "initial" }).await.unwrap();
    struct Trace(Arc<Mutex<Vec<String>>>);
    #[suprnova::async_trait]
    impl Listener<read_model::events::Saved> for Trace {
        async fn handle(&self, event: &read_model::events::Saved) -> Result<(), FrameworkError> {
            self.0
                .lock()
                .unwrap()
                .push(format!("total:{}", event.model.total));
            Ok(())
        }
    }
    #[suprnova::async_trait]
    impl Listener<post::events::Saved> for Trace {
        async fn handle(&self, event: &post::events::Saved) -> Result<(), FrameworkError> {
            self.0
                .lock()
                .unwrap()
                .push(format!("title:{}", event.model.title));
            Ok(())
        }
    }
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            EventFacade::forget::<read_model::events::Saved>();
            EventFacade::forget::<post::events::Saved>();
        }
    }
    let _cleanup = Cleanup;
    let trace = Arc::new(Mutex::new(Vec::new()));
    EventFacade::listen::<read_model::events::Saved, _>(Arc::new(Trace(trace.clone()))).await;
    EventFacade::listen::<post::events::Saved, _>(Arc::new(Trace(trace.clone()))).await;
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(Projected { amount: 2 }).unwrap();
    root.record(super::cache::Title {
        id: post.id,
        title: "middle".into(),
    })
    .unwrap();
    root.record(Projected { amount: 3 }).unwrap();
    root.persist().await.unwrap();
    let live = trace.lock().unwrap().clone();
    assert_eq!(live, ["total:2", "title:middle", "total:5"]);
    trace.lock().unwrap().clear();
    EventReplay::projectors(["es.inline", "es.titles"])
        .page_size(1)
        .run()
        .await
        .unwrap();
    assert_eq!(*trace.lock().unwrap(), live);
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_mixed_replay_matches_live_batch_observations() {
    let _queue = Queue::fake();
    let _db = setup().await;
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    for amount in [2, 3] {
        root.record_with_metadata(Projected { amount }, serde_json::json!({"observe_s":true}))
            .unwrap();
    }
    let events = root.persist().await.unwrap();
    QueuedProjector::new("es.queued", events.last().unwrap().id)
        .unwrap()
        .handle()
        .await
        .unwrap();
    async fn observations() -> Vec<i64> {
        DB::table("es_order")
            .order_by_asc("id")
            .get()
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.get_int("observed").unwrap())
            .collect()
    }
    let live = observations().await;
    assert_eq!(live, [5, 5]);
    EventReplay::projectors(["es.inline", "es.queued"])
        .page_size(2)
        .run()
        .await
        .unwrap();
    assert_eq!(observations().await, live);
}
