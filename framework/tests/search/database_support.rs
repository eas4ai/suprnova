//! Shared fixtures keep each native database corpus isolated.

use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};
use std::{collections::BTreeMap, sync::Arc};
use suprnova::{
    DB, DbConnection, FrameworkError,
    search::{
        AttributeDecl, AttributeType, AttributeValue, EngineQuery, ExpandedQuery, ExpandedTerm,
        GenerationId, IndexDescriptor, IndexSettings, IndexedDocument, SearchDocument,
        SearchableDecl,
        engine::{SearchEngine, database::DatabaseSearchEngine},
    },
    testing::{TestContainer, TestContainerGuard},
    vector::{Vector, memory::MemoryVectorDriver},
};

pub(super) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        suprnova::search::migrations()
    }
}

pub(super) fn descriptor(name: &'static str) -> IndexDescriptor {
    IndexDescriptor {
        name,
        model_table: "search_database_source",
        key_column: "id",
        attributes: vec![
            AttributeDecl {
                name: "title".into(),
                ty: AttributeType::Text,
                searchable: Some(SearchableDecl {
                    weight: 8.0,
                    order: 0,
                }),
                filterable: true,
                sortable: true,
                facet: true,
            },
            AttributeDecl {
                name: "body".into(),
                ty: AttributeType::Text,
                searchable: Some(SearchableDecl {
                    weight: 1.0,
                    order: 1,
                }),
                filterable: false,
                sortable: false,
                facet: false,
            },
            AttributeDecl {
                name: "category".into(),
                ty: AttributeType::Text,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
        ],
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings: IndexSettings::default,
        scope: None,
        scope_fields: vec![],
    }
}

pub(super) fn document(
    index: &IndexDescriptor,
    key: &str,
    title: &str,
    body: &str,
) -> IndexedDocument {
    let document = SearchDocument {
        index: index.name.into(),
        key: key.into(),
        source_version: 1,
        attributes: BTreeMap::from([
            ("title".into(), AttributeValue::Text(title.into())),
            ("body".into(), AttributeValue::Text(body.into())),
            ("category".into(), AttributeValue::Text("fruit".into())),
        ]),
        embed_text: None,
        title: None,
    };
    IndexedDocument {
        content_hash: document.content_hash(&index.index_settings().index_time.digest()),
        document,
        dense: None,
        sparse: None,
    }
}

pub(super) fn query(text: &str) -> EngineQuery {
    EngineQuery {
        expanded: ExpandedQuery {
            terms: vec![ExpandedTerm {
                original: text.into(),
                alternatives: vec![text.into()],
                prefix: false,
                required: false,
                typo_alternatives: vec![],
            }],
            ..ExpandedQuery::default()
        },
        filter: None,
        sort: vec![],
        depth: 100,
        highlights: false,
        crop: Default::default(),
    }
}

pub(super) fn engine() -> DatabaseSearchEngine {
    DatabaseSearchEngine::new()
}

pub(super) fn vectors(index: &str) {
    Vector::register(index, Arc::new(MemoryVectorDriver::new()));
}

pub(super) async fn create(index: &IndexDescriptor) -> Result<GenerationId, FrameworkError> {
    let generation = GenerationId::new("native")?;
    engine()
        .create_generation(index, &index.index_settings(), &generation)
        .await?;
    Ok(generation)
}

pub(super) async fn live_database(variable: &str) -> Result<TestContainerGuard, FrameworkError> {
    let url = std::env::var(variable)
        .map_err(|_| FrameworkError::param(format!("set {variable} to a disposable database")))?;
    let config = suprnova::DatabaseConfig::builder()
        .url(url)
        .max_connections(2)
        .min_connections(0)
        .logging(false)
        .build();
    let connection = DbConnection::connect(&config).await?;
    let guard = TestContainer::fake();
    TestContainer::singleton(connection);
    Migrator::up(DB::connection()?.inner(), None)
        .await
        .map_err(|error| FrameworkError::database(format!("search test migrations: {error}")))?;
    Ok(guard)
}

/// Native driver writes advance the index's RenderCache identity in their own transaction.
pub(super) async fn writes_advance_the_render_cache_index(
    name: &'static str,
) -> Result<(), FrameworkError> {
    suprnova::render_cache::migration::Migration
        .up(&SchemaManager::new(DB::connection()?.inner()))
        .await
        .map_err(|error| FrameworkError::database(format!("render cache migration: {error}")))?;
    // Probe again now that the ledger exists in this database.
    suprnova::render_cache::RenderCache::uninstall_for_test();
    let index = descriptor(name);
    let generation = create(&index).await?;
    let driver = engine();
    let created = render_generation(name).await?;
    assert!(created > 0, "creating a generation advances the index");
    driver
        .index(
            &index,
            &generation,
            &[document(&index, "a", "orchard", "garden")],
        )
        .await?;
    let indexed = render_generation(name).await?;
    assert!(indexed > created, "indexing advances the index");
    driver
        .remove(&index, &generation, &[("a".into(), 2)])
        .await?;
    let removed = render_generation(name).await?;
    assert!(removed > indexed, "removal advances the index");
    let mut settings = index.index_settings();
    settings.query.hits_cap = 50;
    driver
        .write_settings(&index, &generation, &settings)
        .await?;
    assert!(
        render_generation(name).await? > removed,
        "a settings write advances the index"
    );
    driver.drop_generation(&index, &generation).await
}

async fn render_generation(name: &str) -> Result<i64, FrameworkError> {
    // A digest is lowercase hex, never caller input, so it is safe to inline.
    let identity = hex::encode(
        suprnova::render_cache::DependencyIdentity::table(&format!("search:{name}")).digest(),
    );
    DB::scalar(
        &format!(
            "SELECT COALESCE(MAX(generation), 0) AS generation FROM suprnova_render_generations WHERE identity = '{identity}'"
        ),
        vec![],
    )
    .await
}
