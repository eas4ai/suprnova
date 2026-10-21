//! Shared fixtures keep each native database corpus isolated.

use sea_orm_migration::prelude::{Alias, DynIden, IntoIden};
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

    /// A ledger of its own: the gate runs every engine test against one
    /// database, and a migrator refuses a ledger naming migrations it does
    /// not have, so the search tests' rows must not sit in the shared one.
    fn migration_table_name() -> DynIden {
        Alias::new("suprnova_search_test_ledger").into_iden()
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
    live_database_with_connections(variable, 2).await
}

pub(super) async fn live_database_with_connections(
    variable: &str,
    max_connections: u32,
) -> Result<TestContainerGuard, FrameworkError> {
    let url = std::env::var(variable)
        .map_err(|_| FrameworkError::param(format!("set {variable} to a disposable database")))?;
    let config = suprnova::DatabaseConfig::builder()
        .url(url)
        .max_connections(max_connections)
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

pub(super) async fn cleanup_removed_lists(name: &'static str) -> Result<(), FrameworkError> {
    use suprnova::search::{AttributeDecl, AttributeType};

    let mut index = descriptor(name);
    index.attributes.push(AttributeDecl {
        name: "tags".into(),
        ty: AttributeType::TextList,
        searchable: None,
        filterable: true,
        sortable: false,
        facet: false,
    });
    let generation = create(&index).await?;
    // A query settings update must preserve the generation's physical ownership record.
    let mut settings = engine().read_settings(&index, &generation).await?;
    settings.query.hits_cap = 42;
    engine()
        .write_settings(&index, &generation, &settings)
        .await?;
    index
        .attributes
        .retain(|attribute| attribute.name != "tags");
    engine().drop_generation(&index, &generation).await?;
    let connection = DB::connection()?;
    let manager = sea_orm_migration::SchemaManager::new(connection.inner());
    assert!(
        !manager
            .has_table(format!("search_{name}_native_tags"))
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?
    );
    assert!(
        !manager
            .has_table(format!("search_{name}_native"))
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?
    );
    Ok(())
}

pub(super) struct ConnectionVectorDriver {
    index_name: &'static str,
    pub(super) advance_version: std::sync::atomic::AtomicBool,
    pub(super) block_upsert: std::sync::atomic::AtomicBool,
    pub(super) entered_upsert: tokio::sync::Notify,
    pub(super) remove_current_on_delete: std::sync::atomic::AtomicBool,
    pub(super) upsert_calls: std::sync::atomic::AtomicUsize,
    memory: suprnova::vector::MemoryVectorDriver,
    fail_upsert: std::sync::atomic::AtomicBool,
    fail_delete: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl suprnova::vector::VectorDriver for ConnectionVectorDriver {
    async fn upsert(
        &self,
        store: &str,
        items: Vec<suprnova::vector::VectorItem>,
    ) -> Result<(), FrameworkError> {
        DB::select("SELECT 1", vec![]).await?;
        self.upsert_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.block_upsert.load(std::sync::atomic::Ordering::SeqCst) {
            self.entered_upsert.notify_one();
            std::future::pending::<()>().await;
        }
        if self
            .advance_version
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            let index = descriptor(self.index_name);
            let mut doc = document(&index, "a", "orchard", "garden");
            doc.document.source_version = 2;
            doc.dense = Some(vec![1.0, 0.0, 0.0]);
            let generation = GenerationId::new("native")?;
            let error = engine()
                .index(&index, &generation, &[doc])
                .await
                .expect_err("a concurrent writer commits SQL but cannot steal this claim");
            assert!(error.to_string().contains("in flight"), "{error}");
        }
        if self
            .fail_upsert
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(FrameworkError::database("injected upsert failure"));
        }
        self.memory.upsert(store, items).await
    }

    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError> {
        DB::select("SELECT 1", vec![]).await?;
        if self
            .remove_current_on_delete
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            let index = descriptor(self.index_name);
            let generation = GenerationId::new("native")?;
            let error = engine()
                .remove(&index, &generation, &[("a".into(), 4)])
                .await
                .expect_err("SQL removal commits while another worker owns vector delivery");
            assert!(error.to_string().contains("in flight"), "{error}");
        }
        if self
            .fail_delete
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(FrameworkError::database("injected delete failure"));
        }
        self.memory.delete(store, ids).await
    }

    async fn similar(
        &self,
        store: &str,
        query: Vec<f32>,
        k: usize,
    ) -> Result<Vec<suprnova::vector::VectorMatch>, FrameworkError> {
        self.memory.similar(store, query, k).await
    }

    async fn count(&self, store: &str) -> Result<usize, FrameworkError> {
        self.memory.count(store).await
    }
}

pub(super) async fn shared_pool_vector_delivery(name: &'static str) -> Result<(), FrameworkError> {
    let index = descriptor(name);
    let generation = create(&index).await?;
    let store = connection_vector_driver(index.name);
    store
        .fail_upsert
        .store(true, std::sync::atomic::Ordering::SeqCst);
    store
        .fail_delete
        .store(true, std::sync::atomic::Ordering::SeqCst);
    suprnova::vector::Vector::register(index.name, store);
    let mut doc = document(&index, "a", "orchard", "garden");
    doc.dense = Some(vec![1.0, 0.0, 0.0]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let error = engine()
            .index(&index, &generation, &[doc.clone()])
            .await
            .expect_err("first upsert fails");
        assert!(error.to_string().contains("injected upsert failure"));
        engine().index(&index, &generation, &[doc]).await?;
        assert_eq!(
            suprnova::vector::Vector::store(index.name)?.count().await?,
            1
        );
        let error = engine()
            .remove(&index, &generation, &[("a".into(), 2)])
            .await
            .expect_err("first deletion fails");
        assert!(error.to_string().contains("injected delete failure"));
        engine()
            .remove(&index, &generation, &[("a".into(), 2)])
            .await?;
        assert_eq!(
            suprnova::vector::Vector::store(index.name)?.count().await?,
            0
        );
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("vector calls must not hold the pool's only connection")?;
    engine().drop_generation(&index, &generation).await
}

pub(super) fn connection_vector_driver(index_name: &'static str) -> Arc<ConnectionVectorDriver> {
    Arc::new(ConnectionVectorDriver {
        index_name,
        memory: suprnova::vector::MemoryVectorDriver::new(),
        fail_upsert: false.into(),
        fail_delete: false.into(),
        advance_version: false.into(),
        block_upsert: false.into(),
        entered_upsert: tokio::sync::Notify::new(),
        remove_current_on_delete: false.into(),
        upsert_calls: 0.into(),
    })
}
