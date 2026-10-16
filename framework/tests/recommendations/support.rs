use sea_orm_migration::{MigrationName, MigrationTrait, MigratorTrait, SchemaManager};
use std::{future::Future, sync::Arc};
use suprnova::{
    Actor, AttributeValue, CacheStore, CollectionEngine, EventType, Filter, Model, Recommend,
    RecommendConfig, RecommendationEvent, RecommendationTarget, SearchConfig, SearchDriver,
    SearchRegistry, Searchable, Viewer,
    testing::{TestContainer, TestDatabase},
};

fn scope(viewer: &Viewer) -> Filter {
    Filter::Or(vec![
        Filter::Eq("public".into(), AttributeValue::Bool(true)),
        Filter::Eq(
            "owner".into(),
            AttributeValue::Text(viewer.id().unwrap_or("").into()),
        ),
    ])
}

#[suprnova::model(table = "recommendation_notes", timestamps = false,
    fillable = ["title", "category", "public", "owner", "published_at"])]
#[derive(Searchable)]
#[searchable(index = "recommendation_notes", scope = "scope")]
pub struct Note {
    pub id: i64,
    #[searchable(text, embed, title)]
    pub title: String,
    #[searchable(facet)]
    pub category: String,
    #[searchable(filterable)]
    pub public: bool,
    #[searchable(filterable)]
    pub owner: String,
    #[searchable(sortable)]
    pub published_at: i64,
}

struct Schema;
impl MigrationName for Schema {
    fn name(&self) -> &str {
        "recommendations_test_notes"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Schema {
    async fn up(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        use sea_orm::ConnectionTrait;
        manager.get_connection().execute_unprepared("CREATE TABLE recommendation_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, category TEXT NOT NULL, public BOOLEAN NOT NULL, owner TEXT NOT NULL, published_at BIGINT NOT NULL)").await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        use sea_orm::ConnectionTrait;
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE recommendation_notes")
            .await?;
        Ok(())
    }
}
pub struct Migrator;
#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations: Vec<Box<dyn MigrationTrait>> = vec![Box::new(Schema)];
        migrations.extend(suprnova::recommendations::migrations());
        migrations
    }
}

pub fn setup<F: Future<Output = ()>>(future: F) -> impl Future<Output = ()> {
    let future = Box::pin(future);
    async move {
        let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
        TestContainer::bind::<dyn suprnova::UserProvider>(Arc::new(Users));
        TestContainer::bind::<dyn CacheStore>(Arc::new(
            suprnova::cache::memory::InMemoryCache::new(),
        ));
        SearchConfig::scope(
            SearchConfig {
                driver: SearchDriver::Collection,
                queue: false,
                ..Default::default()
            },
            SearchRegistry::isolated(async {
                SearchRegistry::register_engine("collection", Arc::new(CollectionEngine::new()))
                    .unwrap();
                RecommendConfig::scope(RecommendConfig::default(), future)
                    .await
                    .unwrap();
            }),
        )
        .await
        .unwrap();
    }
}

pub async fn note(title: &str, category: &str, public: bool, owner: &str, time: i64) -> Note {
    let note = Box::pin(Note::create(suprnova::attrs! {title: title, category: category, public: public, owner: owner, published_at: time})).await.unwrap();
    suprnova::searchable::<Note>(&[note.id.to_string()])
        .await
        .unwrap();
    note
}
pub fn key(note: &Note) -> RecommendationTarget {
    RecommendationTarget {
        index: "recommendation_notes".into(),
        key: note.id.to_string(),
    }
}
pub async fn event(actor: &str, note: &Note, kind: EventType, dedupe: &str) -> RecommendationEvent {
    RecommendationEvent::new(
        kind,
        key(note),
        Some(Actor::User(actor.into())),
        "home",
        dedupe,
    )
    .await
    .unwrap()
}
pub async fn record(actor: &str, note: &Note, kind: EventType, dedupe: &str) {
    Recommend::record(event(actor, note, kind, dedupe).await)
        .await
        .unwrap();
}
struct User(String);
impl suprnova::Authenticatable for User {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }
}
pub fn viewer(id: &str) -> Viewer {
    Viewer::Authenticated {
        id: id.into(),
        user: Arc::new(User(id.into())),
    }
}

struct Users;
#[async_trait::async_trait]
impl suprnova::UserProvider for Users {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn suprnova::Authenticatable>>, suprnova::FrameworkError> {
        Ok(Some(Arc::new(User(id.into()))))
    }
}
