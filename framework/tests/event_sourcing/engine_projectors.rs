use super::*;

use sea_orm_migration::prelude::{Alias, ColumnDef, Table};
use suprnova::testing::TestContainer;
use suprnova::{
    Aggregate, DB, DatabaseConfig, DbConnection, DomainEvent, EventReplay, FirstOrCreate, Job,
    Model, Projector, Queue, QueuedProjector, attrs,
};

#[domain_event(name = "es.engine.added", version = 1)]
#[derive(Serialize, Deserialize)]
struct Added {
    amount: i64,
}

#[suprnova::model(table = "es_engine_inline", key_type = "String", timestamps = false, fillable = ["id", "total"])]
struct InlineRow {
    pub id: String,
    pub total: i64,
}
#[suprnova::model(table = "es_engine_queued", key_type = "String", timestamps = false, fillable = ["id", "total"])]
struct QueuedRow {
    pub id: String,
    pub total: i64,
}

#[suprnova::projector]
#[derive(Default)]
struct Inline;
#[suprnova::async_trait]
impl Projector for Inline {
    fn name(&self) -> &'static str {
        "es.engine.inline"
    }
    fn handles(&self, event: &StoredEvent) -> bool {
        event.event_type == Added::NAME
    }
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(fact) = event.decode::<Added>()? {
            let total = InlineRow::find(event.aggregate_uuid.clone())
                .await?
                .map_or(0, |row| row.total);
            InlineRow::update_or_create(
                attrs! { id: event.aggregate_uuid.clone() },
                attrs! { total: total + fact.amount },
            )
            .await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        InlineRow::query().delete_all().await?;
        Ok(())
    }
}
#[suprnova::projector(queued)]
#[derive(Default)]
struct Deferred;
#[suprnova::async_trait]
impl Projector for Deferred {
    fn name(&self) -> &'static str {
        "es.engine.queued"
    }
    fn handles(&self, event: &StoredEvent) -> bool {
        event.event_type == Added::NAME
    }
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(fact) = event.decode::<Added>()? {
            let total = QueuedRow::find(event.aggregate_uuid.clone())
                .await?
                .map_or(0, |row| row.total);
            QueuedRow::update_or_create(
                attrs! { id: event.aggregate_uuid.clone() },
                attrs! { total: total + fact.amount },
            )
            .await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        QueuedRow::query().delete_all().await?;
        Ok(())
    }
}
pub(super) async fn tables(db: &sea_orm::DatabaseConnection) {
    let manager = SchemaManager::new(db);
    for name in ["es_engine_inline", "es_engine_queued"] {
        manager
            .create_table(
                Table::create()
                    .if_not_exists()
                    .table(Alias::new(name))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Alias::new("total")).big_integer().not_null())
                    .to_owned(),
            )
            .await
            .unwrap();
    }
}
async fn optional_checkpoint(name: &str) -> Option<i64> {
    DB::table("projector_checkpoints")
        .filter("name", name)
        .first()
        .await
        .unwrap()
        .map(|row| row.get_int("last_event_id").unwrap())
}
async fn checkpoint(name: &str) -> i64 {
    optional_checkpoint(name)
        .await
        .expect("projector checkpoint exists after delivery")
}
async fn engine(url: String) {
    let _container = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .logging(false)
        .build();
    let connection = DbConnection::connect(&config).await.unwrap();
    let manager = SchemaManager::new(connection.inner());
    if !manager.has_table("stored_events").await.unwrap() {
        suprnova::CreateEventSourcingTables
            .up(&manager)
            .await
            .unwrap();
    }
    tables(connection.inner()).await;
    TestContainer::singleton(connection);
    let _queue = Queue::fake();
    lifecycle().await;
}
async fn lifecycle() {
    lifecycle_with_fault(false).await;
}
async fn lifecycle_with_fault(premature_checkpoint: bool) {
    let id = uuid();
    let queued_before = optional_checkpoint("es.engine.queued").await;
    let mut aggregate = Aggregate::<Tally>::new(&id).unwrap();
    aggregate.record(Added { amount: 2 }).await.unwrap();
    let first = aggregate.persist().await.unwrap()[0].id;
    assert_eq!(InlineRow::find(id.clone()).await.unwrap().unwrap().total, 2);
    assert_eq!(checkpoint("es.engine.inline").await, first);
    if premature_checkpoint {
        DB::statement(
            "INSERT INTO projector_checkpoints (name, last_event_id) VALUES (?, ?)",
            vec!["es.engine.queued".into(), first.into()],
        )
        .await
        .unwrap();
    }
    assert_eq!(
        optional_checkpoint("es.engine.queued").await,
        queued_before,
        "queued checkpoint changed before first delivery"
    );
    assert!(QueuedRow::find(id.clone()).await.unwrap().is_none());
    QueuedProjector::new("es.engine.queued", first)
        .unwrap()
        .handle()
        .await
        .unwrap();
    assert_eq!(checkpoint("es.engine.inline").await, first);
    assert_eq!(checkpoint("es.engine.queued").await, first);
    assert_eq!(QueuedRow::find(id.clone()).await.unwrap().unwrap().total, 2);

    aggregate.record(Added { amount: 3 }).await.unwrap();
    let last = aggregate.persist().await.unwrap()[0].id;
    assert_eq!(InlineRow::find(id.clone()).await.unwrap().unwrap().total, 5);
    assert_eq!(checkpoint("es.engine.inline").await, last);
    assert_eq!(checkpoint("es.engine.queued").await, first);
    assert_eq!(QueuedRow::find(id.clone()).await.unwrap().unwrap().total, 2);
    QueuedProjector::new("es.engine.queued", last)
        .unwrap()
        .handle()
        .await
        .unwrap();
    assert_eq!(checkpoint("es.engine.inline").await, last);
    assert_eq!(checkpoint("es.engine.queued").await, last);
    assert_eq!(QueuedRow::find(id.clone()).await.unwrap().unwrap().total, 5);
    for _ in 0..2 {
        EventReplay::projectors(["es.engine.inline", "es.engine.queued"])
            .from(0)
            .page_size(2)
            .run()
            .await
            .unwrap();
        assert_eq!(InlineRow::find(id.clone()).await.unwrap().unwrap().total, 5);
        assert_eq!(QueuedRow::find(id.clone()).await.unwrap().unwrap().total, 5);
        assert_eq!(checkpoint("es.engine.inline").await, last);
        assert_eq!(checkpoint("es.engine.queued").await, last);
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires PG_TEST_URL"]
async fn es_003_postgres_persist_queue_checkpoint_and_replay() {
    engine(std::env::var("PG_TEST_URL").expect("PG_TEST_URL")).await;
}
#[tokio::test]
#[serial_test::serial]
#[ignore = "requires MYSQL_TEST_URL"]
async fn es_003_mysql_persist_queue_checkpoint_and_replay() {
    engine(std::env::var("MYSQL_TEST_URL").expect("MYSQL_TEST_URL")).await;
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_sqlite_persist_queue_checkpoint_and_replay() {
    let _db = database().await;
    let _queue = Queue::fake();
    lifecycle().await;
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_lifecycle_rejects_a_premature_queued_checkpoint() {
    use futures::FutureExt;
    let _db = database().await;
    let _queue = Queue::fake();
    // Replay repairs this fault, so the live-stage checks must catch it first.
    let result = std::panic::AssertUnwindSafe(lifecycle_with_fault(true))
        .catch_unwind()
        .await;
    let panic = result.expect_err("lifecycle accepted a premature queued checkpoint and replay hid the missing live projection");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    assert!(
        message.contains("queued checkpoint changed before first delivery"),
        "unexpected lifecycle failure: {message}"
    );
}
