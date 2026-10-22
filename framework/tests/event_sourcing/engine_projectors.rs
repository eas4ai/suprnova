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
async fn checkpoint(name: &str) -> i64 {
    DB::table("projector_checkpoints")
        .filter("name", name)
        .first()
        .await
        .unwrap()
        .unwrap()
        .get_int("last_event_id")
        .unwrap()
}
async fn engine(url: String) {
    let _container = TestContainer::fake();
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
    let id = uuid();
    let mut aggregate = Aggregate::<Tally>::new(&id).unwrap();
    aggregate.record(Added { amount: 2 }).await.unwrap();
    let first = aggregate.persist().await.unwrap()[0].id;
    assert_eq!(InlineRow::find(id.clone()).await.unwrap().unwrap().total, 2);
    assert_eq!(checkpoint("es.engine.inline").await, first);
    QueuedProjector::new("es.engine.queued", first)
        .unwrap()
        .handle()
        .await
        .unwrap();
    assert_eq!(checkpoint("es.engine.queued").await, first);
    aggregate.record(Added { amount: 3 }).await.unwrap();
    let last = aggregate.persist().await.unwrap()[0].id;
    QueuedProjector::new("es.engine.queued", last)
        .unwrap()
        .handle()
        .await
        .unwrap();
    assert_eq!(checkpoint("es.engine.inline").await, last);
    assert_eq!(checkpoint("es.engine.queued").await, last);
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
