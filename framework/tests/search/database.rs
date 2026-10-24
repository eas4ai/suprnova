//! Native database search runs against the application's ordinary connection.

use suprnova::{
    FrameworkError,
    search::{SearchConfig, SearchRegistry},
    testing::TestDatabase,
};

struct BuilderMigrator;

#[async_trait::async_trait]
impl sea_orm_migration::MigratorTrait for BuilderMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        let mut migrations =
            <super::derive::SearchMigrator as sea_orm_migration::MigratorTrait>::migrations();
        migrations.extend(suprnova::search::migrations());
        migrations
    }
}

#[tokio::test]
async fn database_driver_resolves_by_default() -> Result<(), FrameworkError> {
    let _database = TestDatabase::sqlite_memory().await?;
    SearchRegistry::isolated(async {
        SearchConfig::scope(SearchConfig::default(), async {
            let engine = SearchRegistry::engine_for("test_search_notes")?;
            assert_eq!(engine.name(), "database");
            Ok::<_, FrameworkError>(())
        })
        .await?
    })
    .await
}

#[tokio::test]
async fn sqlite_builder_crop_preserves_native_stemming_highlights() -> Result<(), FrameworkError> {
    use super::derive::{SearchNote, note};
    use std::sync::Arc;
    use suprnova::{
        Searchable,
        search::{GenerationId, IndexedDocument, LanguageConfig, SearchDriver, SearchEngine},
    };

    let _database = TestDatabase::fresh::<BuilderMigrator>().await?;
    SearchRegistry::isolated(async {
        SearchConfig::scope(
            SearchConfig {
                driver: SearchDriver::Null,
                queue: false,
                ..Default::default()
            },
            async {
                let model = note("one two three running four five six seven eight", true, 0).await;
                let descriptor = SearchNote::descriptor();
                let driver = Arc::new(super::database_support::engine());
                SearchRegistry::register_engine("database", driver.clone())?;
                SearchRegistry::set_index_driver(descriptor.name, "database")?;
                let generation = GenerationId::new("porter")?;
                let mut settings = descriptor.index_settings();
                settings.index_time.language = LanguageConfig::Sqlite {
                    tokenizer: "porter unicode61".into(),
                };
                driver
                    .create_generation(descriptor, &settings, &generation)
                    .await?;
                driver
                    .switch_generation(descriptor, None, generation.clone())
                    .await?;
                let document = model.to_search_document()?;
                let indexed = IndexedDocument {
                    content_hash: document.content_hash(&settings.index_time.digest()),
                    document,
                    dense: None,
                    sparse: None,
                };
                driver.index(descriptor, &generation, &[indexed]).await?;
                let result = SearchNote::search("\"run\"")
                    .crop("title", 16)
                    .get()
                    .await?;
                assert_eq!(result.hits.len(), 1);
                let fragment = &result.hits[0].formatted["title"];
                assert!(fragment.contains("<em>running</em>"));
                assert!(fragment.contains("..."));
                assert!(!fragment.contains("one two three running four five six seven eight"));
                Ok::<_, FrameworkError>(())
            },
        )
        .await?
    })
    .await
}
