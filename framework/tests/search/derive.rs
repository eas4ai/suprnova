//! Declarations are exercised on expanded Eloquent models rather than hand-written descriptors.

use chrono::{DateTime, Utc};
use sea_orm_migration::{MigrationName, MigrationTrait, MigratorTrait, SchemaManager};
use std::sync::{Arc, Mutex};
use suprnova::{
    AsArray, FrameworkError, Model, Searchable,
    search::{AttributeType, AttributeValue, Filter, Viewer},
    testing::TestDatabase,
};

tokio::task_local! { pub(crate) static CHUNKS: Arc<Mutex<Vec<usize>>>; }

fn note_scope(viewer: &Viewer) -> Filter {
    let owner = match viewer {
        Viewer::Guest => 0,
        Viewer::Authenticated { id, .. } => id.parse::<i64>().unwrap_or(-1),
    };
    Filter::Eq("owner_id".into(), AttributeValue::Integer(owner))
}

#[suprnova::model(
    table = "search_notes", timestamps = false, soft_deletes,
    fillable = ["title", "body", "category", "rank", "enabled", "owner_id", "tags", "lat", "lng"],
    casts = { tags = AsArray<String> }
)]
#[derive(Searchable)]
#[searchable(
    index = "test_search_notes",
    soft_delete,
    should_index = "is_enabled",
    with = "load_for_search",
    scope = "note_scope",
    geo(name = "location", lat = "lat", lng = "lng")
)]
pub(crate) struct SearchNote {
    pub id: i64,
    #[searchable(text, weight = 2.0, embed, title)]
    pub title: String,
    #[searchable(text, embed)]
    pub body: Option<String>,
    #[searchable(facet, sortable)]
    pub category: String,
    #[searchable(facet, sortable)]
    pub rank: i64,
    #[searchable(filterable)]
    pub enabled: bool,
    #[searchable(filterable)]
    pub owner_id: i64,
    #[searchable(text, facet)]
    pub tags: Vec<String>,
    pub lat: f64,
    pub lng: f64,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl SearchNote {
    fn is_enabled(&self) -> Result<bool, FrameworkError> {
        Ok(self.enabled)
    }

    async fn load_for_search(models: Vec<Self>) -> Result<Vec<Self>, FrameworkError> {
        CHUNKS
            .try_with(|chunks| chunks.lock().unwrap().push(models.len()))
            .ok();
        Ok(models)
    }
}

#[suprnova::model(table = "search_eligibility", timestamps = false, fillable = ["title", "eligible"])]
#[derive(Searchable)]
#[searchable(should_index = "is_eligible")]
pub(crate) struct EligibilityNote {
    pub id: i64,
    #[searchable(text)]
    pub title: String,
    pub eligible: bool,
}

impl EligibilityNote {
    fn is_eligible(&self) -> Result<bool, FrameworkError> {
        Ok(self.eligible)
    }
}

pub(crate) struct SearchMigrator;
struct SearchSchema;

impl MigrationName for SearchSchema {
    fn name(&self) -> &str {
        "search_core_test_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for SearchSchema {
    async fn up(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        use sea_orm::ConnectionTrait;
        manager.get_connection().execute_unprepared(
            "CREATE TABLE search_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, body TEXT NULL, category TEXT NOT NULL, rank INTEGER NOT NULL, enabled BOOLEAN NOT NULL, owner_id INTEGER NOT NULL, tags TEXT NOT NULL, lat REAL NOT NULL, lng REAL NOT NULL, deleted_at TEXT NULL)"
        ).await?;
        manager.get_connection().execute_unprepared("CREATE TABLE search_eligibility (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, eligible BOOLEAN NOT NULL)").await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        use sea_orm::ConnectionTrait;
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE search_notes")
            .await?;
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE search_eligibility")
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl MigratorTrait for SearchMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(SearchSchema)]
    }
}

pub(crate) async fn database() -> TestDatabase {
    TestDatabase::fresh::<SearchMigrator>().await.unwrap()
}

pub(crate) async fn note(title: &str, enabled: bool, owner: i64) -> SearchNote {
    suprnova::search::indexing::without_syncing_to_search(|| async {
        SearchNote::create(suprnova::attrs! {
            title: title, body: "Search body", category: "notes", rank: 7,
            enabled: enabled, owner_id: owner, tags: vec!["rust", "search"], lat: 40.0, lng: -73.0,
        })
        .await
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn derive_returns_only_declared_typed_attributes_and_embedding_source() {
    let _db = database().await;
    let model = note("Rust search", true, 0).await;
    let document = model.to_search_document().unwrap();
    assert_eq!(document.index, "test_search_notes");
    assert_eq!(document.key, model.id.to_string());
    assert_eq!(document.source_version, 0);
    assert_eq!(document.title.as_deref(), Some("Rust search"));
    assert_eq!(
        document.embed_text.as_deref(),
        Some("Rust search\nSearch body")
    );
    assert!(!document.attributes.contains_key("id"));
    assert!(!document.attributes.contains_key("lat"));
    assert!(!document.attributes.contains_key("deleted_at"));
    assert_eq!(document.attributes["owner_id"], AttributeValue::Integer(0));
    assert_eq!(document.attributes["enabled"], AttributeValue::Bool(true));
    assert_eq!(
        document.attributes["tags"],
        AttributeValue::List(vec![
            AttributeValue::Text("rust".into()),
            AttributeValue::Text("search".into())
        ])
    );
    assert!(matches!(
        document.attributes["location"],
        AttributeValue::Geo(_)
    ));
    assert_eq!(
        document.attributes["__soft_deleted"],
        AttributeValue::Bool(false)
    );
    let json = serde_json::to_string(&document).unwrap();
    assert_eq!(
        serde_json::from_str::<suprnova::SearchDocument>(&json).unwrap(),
        document
    );
}

#[tokio::test]
async fn registry_exposes_order_weights_scope_and_predicate_through_the_derive() {
    let _db = database().await;
    let model = note("Scope", true, 0).await;
    let descriptor = SearchNote::descriptor();
    assert_eq!(descriptor.model_table, "search_notes");
    assert_eq!(descriptor.key_column, "id");
    assert!(descriptor.search_touches.is_empty());
    assert_eq!(
        descriptor.attributes[0].searchable.as_ref().unwrap().order,
        0
    );
    assert_eq!(
        descriptor.attributes[0].searchable.as_ref().unwrap().weight,
        2.0
    );
    assert_eq!(
        descriptor.attribute("tags").unwrap().ty,
        AttributeType::TextList
    );
    assert!(descriptor.attribute("tags").unwrap().filterable);
    let entry = suprnova::SearchRegistry::by_index(descriptor.name).unwrap();
    assert_eq!((entry.type_id)(), std::any::TypeId::of::<SearchNote>());
    assert!((entry.should_index)(&model).unwrap());
    assert!((entry.evaluate_scope)(
        &Viewer::Guest,
        &model.to_search_document().unwrap()
    ));
    let private = note("Private", true, 9).await;
    assert!(!(entry.evaluate_scope)(
        &Viewer::Guest,
        &private.to_search_document().unwrap()
    ));
    let disabled = note("Hidden", false, 0).await;
    assert!(
        (entry.load_document)(&disabled.id.to_string())
            .await
            .unwrap()
            .is_none()
    );
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let documents = CHUNKS
        .scope(
            chunks.clone(),
            (entry.load_documents)(vec![model.id.to_string(), disabled.id.to_string()]),
        )
        .await
        .unwrap();
    assert_eq!(*chunks.lock().unwrap(), vec![2]);
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].key, model.id.to_string());
}

#[tokio::test]
async fn timestamp_free_projection_does_not_assign_a_write_version() {
    let _db = database().await;
    let mut model = note("Version", true, 0).await;
    let first = model.to_search_document().unwrap();
    assert_eq!(
        model.to_search_document().unwrap().source_version,
        first.source_version
    );
    model.title = "Changed version".into();
    assert_eq!(model.to_search_document().unwrap().source_version, 0);
}

#[tokio::test]
async fn loader_includes_soft_deleted_models_only_for_opted_in_indexes() {
    let _db = database().await;
    let model = note("Deleted", true, 0).await;
    let key = model.id.to_string();
    suprnova::without_syncing_to_search(|| async { model.delete().await })
        .await
        .unwrap();
    let entry = suprnova::SearchRegistry::by_index(SearchNote::descriptor().name).unwrap();
    let document = (entry.load_document)(&key).await.unwrap().unwrap();
    assert_eq!(
        document.attributes["__soft_deleted"],
        AttributeValue::Bool(true)
    );
}

#[suprnova::model(table = "search_unsigned", timestamps = false)]
#[derive(Searchable)]
pub(crate) struct UnsignedDocument {
    pub id: i64,
    #[searchable(filterable)]
    pub count: u64,
}

#[test]
fn unsigned_overflow_is_a_field_named_error() {
    let model = UnsignedDocument {
        id: 1,
        count: u64::MAX,
        ..Default::default()
    };
    assert!(
        model
            .to_search_document()
            .unwrap_err()
            .to_string()
            .contains("count")
    );
}

#[suprnova::model(table = "search_floats", timestamps = false)]
#[derive(Searchable)]
pub(crate) struct FloatDocument {
    pub id: i64,
    #[searchable(filterable)]
    pub score: f64,
    #[searchable(filterable)]
    pub optional_score: Option<f32>,
}

#[test]
fn nonfinite_floats_cannot_silently_become_null() {
    let mut model = FloatDocument {
        id: 1,
        score: f64::NAN,
        ..Default::default()
    };
    assert!(
        model
            .to_search_document()
            .unwrap_err()
            .to_string()
            .contains("score")
    );
    model.score = 1.0;
    model.optional_score = Some(f32::INFINITY);
    assert!(
        model
            .to_search_document()
            .unwrap_err()
            .to_string()
            .contains("optional_score")
    );
}

#[test]
fn invalid_updated_at_names_the_index_and_valid_timestamps_use_microseconds() {
    let document = suprnova::SearchDocument {
        index: "timestamp_test".into(),
        key: "1".into(),
        source_version: 0,
        attributes: Default::default(),
        embed_text: None,
        title: None,
    };
    let value = serde_json::json!("2026-10-14T12:34:56.000123Z");
    let actual = suprnova::search::indexing::source_version(&document, Some(value)).unwrap();
    let expected = DateTime::parse_from_rfc3339("2026-10-14T12:34:56.000123Z")
        .unwrap()
        .timestamp_micros();
    assert_eq!(actual, expected);
    assert!(
        suprnova::search::indexing::source_version(&document, Some(serde_json::json!("bad")))
            .unwrap_err()
            .to_string()
            .contains("timestamp_test")
    );
}

#[test]
fn key_binding_preserves_string_keys_and_validates_integer_key_ranges() {
    use suprnova::search::indexing::model_key_value;
    assert_eq!(
        model_key_value::<String>("007", "uuid").unwrap(),
        serde_json::json!("007")
    );
    assert_eq!(
        model_key_value::<i64>("7", "id").unwrap(),
        serde_json::json!(7)
    );
    assert!(
        model_key_value::<i8>("128", "id")
            .unwrap_err()
            .to_string()
            .contains("id")
    );
    assert!(model_key_value::<i64>("garbage", "id").is_err());
}
