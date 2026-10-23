//! A provider name can be replaced without changing the stored generation's vector space.
use super::derive::{SearchNote, database, note};
use sea_orm_migration::MigratorTrait;
use std::sync::Arc;
use suprnova::{
    FakeEmbedder, SearchConfig, SearchDriver, SearchEmbedder, SearchRegistry, Searchable,
    queue::Job,
    search::{Embedder, IndexOperation, SearchIndexJob, generations},
};

async fn provider_replacement_is_refused(query: bool) {
    let _db = database().await;
    let connection = suprnova::DB::connection().unwrap();
    super::database_support::Migrator::up(connection.inner(), None)
        .await
        .unwrap();
    SearchConfig::scope(
        SearchConfig {
            driver: SearchDriver::Collection,
            embedder: SearchEmbedder::Http,
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let original = Arc::new(FakeEmbedder::new(8).unwrap());
            SearchRegistry::register_embedder("http", original.clone()).unwrap();
            let descriptor = SearchNote::descriptor();
            generations::create_index(descriptor.name).await.unwrap();
            let generation = SearchRegistry::generation_for(descriptor.name)
                .await
                .unwrap();
            let settings = SearchRegistry::engine_for(descriptor.name)
                .unwrap()
                .read_settings(descriptor, &generation)
                .await
                .unwrap();
            assert_eq!(
                settings.index_time.model.as_deref(),
                Some("fake-hash@sha256-v1")
            );

            for (id, revision, identity) in [
                ("replacement", Some("sha256-v1"), "replacement@sha256-v1"),
                ("fake-hash", Some("v2"), "fake-hash@v2"),
                ("fake-hash", None, "fake-hash"),
            ] {
                let model = note("Rust provider identity", true, 0).await;
                let document = model.to_search_document().unwrap();
                let job = SearchIndexJob {
                    index: document.index,
                    key: document.key,
                    source_version: document.source_version,
                    op: IndexOperation::Index,
                    generation: None,
                };
                job.clone().handle().await.unwrap();
                assert!(
                    !SearchNote::search("Rust")
                        .hybrid(1.0)
                        .get()
                        .await
                        .unwrap()
                        .hits
                        .is_empty()
                );

                let mut profile = original.profile().clone();
                profile.id = id.into();
                profile.revision = revision.map(str::to_owned);
                SearchRegistry::register_embedder(
                    "http",
                    Arc::new(FakeEmbedder::with_profile(profile).unwrap()),
                )
                .unwrap();
                let result = if query {
                    SearchNote::search("Rust")
                        .hybrid(1.0)
                        .get()
                        .await
                        .map(|_| ())
                } else {
                    // A new row prevents the content-hash shortcut from skipping inference.
                    let fresh = note("New provider document", true, 0).await;
                    let document = fresh.to_search_document().unwrap();
                    SearchIndexJob {
                        index: document.index,
                        key: document.key,
                        source_version: document.source_version,
                        op: IndexOperation::Index,
                        generation: None,
                    }
                    .handle()
                    .await
                };
                let error = result
                    .expect_err("a replacement must not mix vector spaces")
                    .to_string();
                for context in [
                    descriptor.name,
                    "fake-hash@sha256-v1",
                    identity,
                    "must be rebuilt",
                    "search:reindex",
                ] {
                    assert!(error.contains(context), "missing {context}: {error}");
                }
                SearchRegistry::register_embedder("http", original.clone()).unwrap();
                job.handle().await.unwrap();
                assert!(
                    !SearchNote::search("Rust")
                        .hybrid(1.0)
                        .get()
                        .await
                        .unwrap()
                        .hits
                        .is_empty()
                );
            }
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn provider_identity_refuses_indexing_after_equal_dimension_replacement() {
    provider_replacement_is_refused(false).await;
}

#[tokio::test]
async fn provider_identity_refuses_queries_after_equal_dimension_replacement() {
    provider_replacement_is_refused(true).await;
}
