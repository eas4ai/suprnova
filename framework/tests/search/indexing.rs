//! Index jobs reload rows and honor commit boundaries under both execution modes.

use super::derive::{CHUNKS, EligibilityNote, SearchNote, database, note};
use std::sync::{Arc, Mutex};
use suprnova::{
    DB, FrameworkError, Model, Queue, SearchConfig, SearchDriver, SearchRegistry, Searchable,
    queue::Job,
    search::{
        EngineQuery, SearchEngine, Total,
        engine::collection::CollectionEngine,
        expansion::ExpandedQuery,
        indexing::{
            IndexOperation, SearchIndexJob, WriteKind, after_model_write, make_all_searchable,
            searchable, unsearchable, without_syncing_to_search,
        },
    },
};

fn config(queue: bool) -> SearchConfig {
    SearchConfig {
        driver: SearchDriver::Collection,
        queue,
        chunk: 2,
        ..Default::default()
    }
}

fn job(model: &SearchNote, op: IndexOperation) -> SearchIndexJob {
    let document = model.to_search_document().unwrap();
    SearchIndexJob {
        index: document.index,
        key: document.key,
        source_version: document.source_version,
        op,
        generation: None,
    }
}

async fn count(engine: &dyn SearchEngine) -> u64 {
    let descriptor = SearchNote::descriptor();
    let generation = SearchRegistry::generation_for(descriptor.name)
        .await
        .unwrap();
    engine
        .count(
            descriptor,
            &generation,
            &EngineQuery {
                expanded: ExpandedQuery::default(),
                filter: None,
                sort: vec![],
                depth: 100,
                highlights: false,
                crop: Default::default(),
            },
        )
        .await
        .unwrap()
        .value()
}

#[tokio::test]
async fn job_reloads_latest_rows_and_removes_missing_or_ineligible_rows() {
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let engine = Arc::new(CollectionEngine::new());
            SearchRegistry::register_engine("collection", engine.clone()).unwrap();
            let model = note("First", true, 0).await;
            let old_job = job(&model, IndexOperation::Index);
            model
                .clone()
                .update(suprnova::attrs! { title: "Current" })
                .await
                .unwrap();
            old_job.handle().await.unwrap();
            let generation = SearchRegistry::generation_for(SearchNote::descriptor().name)
                .await
                .unwrap();
            let result = engine
                .query(
                    SearchNote::descriptor(),
                    &generation,
                    &EngineQuery {
                        expanded: ExpandedQuery::default(),
                        filter: None,
                        sort: vec![],
                        depth: 100,
                        highlights: false,
                        crop: Default::default(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.total, Total::Exact(1));
            assert_eq!(
                result.candidates[0].document.attributes["title"],
                suprnova::AttributeValue::Text("Current".into())
            );
            let current = SearchNote::query()
                .where_in("id", vec![model.id])
                .get()
                .await
                .unwrap()
                .into_vec()
                .remove(0);
            current
                .clone()
                .update(suprnova::attrs! { enabled: false })
                .await
                .unwrap();
            job(&current, IndexOperation::Index).handle().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            let other = note("Disappeared", true, 0).await;
            job(&other, IndexOperation::Index).handle().await.unwrap();
            let stale = job(&other, IndexOperation::Index);
            other.force_delete().await.unwrap();
            stale.handle().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_indexing_waits_for_manual_commit_and_rollback_discards_it() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(config(true), async {
        let model = note("Commit", true, 0).await;
        let tx = DB::begin_transaction().await.unwrap();
        after_model_write(&model, WriteKind::Created, Some(&tx))
            .await
            .unwrap();
        assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
        tx.commit().await.unwrap();
        assert_eq!(
            suprnova::queue::testing::pushed::<SearchIndexJob>().len(),
            1
        );
        let tx = DB::begin_transaction().await.unwrap();
        after_model_write(&model, WriteKind::Updated, Some(&tx))
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(
            suprnova::queue::testing::pushed::<SearchIndexJob>().len(),
            1
        );
        let disabled = note("Disabled", false, 0).await;
        after_model_write(&disabled, WriteKind::Created, None)
            .await
            .unwrap();
        let captured = suprnova::queue::testing::pushed::<SearchIndexJob>();
        assert_eq!(captured.last().unwrap().op, IndexOperation::Remove);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn inline_indexing_waits_for_manual_commit_and_never_runs_on_rollback() {
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let engine = Arc::new(CollectionEngine::new());
            SearchRegistry::register_engine("collection", engine.clone()).unwrap();
            let model = note("Inline", true, 0).await;
            let tx = DB::begin_transaction().await.unwrap();
            after_model_write(&model, WriteKind::Created, Some(&tx))
                .await
                .unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            tx.rollback().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            let tx = DB::begin_transaction().await.unwrap();
            after_model_write(&model, WriteKind::Created, Some(&tx))
                .await
                .unwrap();
            tx.commit().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 1);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn ambient_savepoints_discard_only_their_search_jobs_and_outer_commit_releases_the_rest() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(config(true), async {
        let model = note("Savepoint", true, 0).await;
        DB::transaction(|_| {
            Box::pin(async move {
                after_model_write(&model, WriteKind::Created, None).await?;
                assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
                let inner = model.clone();
                let rollback: Result<(), FrameworkError> = DB::transaction(|_| {
                    Box::pin(async move {
                        after_model_write(&inner, WriteKind::Updated, None).await?;
                        Err(FrameworkError::param("roll back savepoint"))
                    })
                })
                .await;
                assert!(rollback.is_err());
                assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
                Ok(())
            })
        })
        .await
        .unwrap();
        assert_eq!(
            suprnova::queue::testing::pushed::<SearchIndexJob>().len(),
            1
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_chunks_reload_once_and_run_the_eager_hook_before_projection() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            for title in ["One", "Two", "Three", "Four", "Five"] {
                note(title, true, 0).await;
            }
            let chunks = Arc::new(Mutex::new(Vec::new()));
            CHUNKS
                .scope(chunks.clone(), async {
                    make_all_searchable::<SearchNote>().await.unwrap();
                    assert!(chunks.lock().unwrap().is_empty());
                    let jobs = suprnova::queue::testing::pending_jobs();
                    assert_eq!(jobs.len(), 3);
                    assert!(
                        jobs.iter()
                            .all(|job| job.name == "Suprnova::SearchIndexBatch")
                    );
                    for job in jobs {
                        suprnova::queue::worker::dispatch_by_name(&job.name, job.payload)
                            .await
                            .unwrap();
                    }
                    assert_eq!(*chunks.lock().unwrap(), vec![2, 2, 1]);
                })
                .await;
            searchable::<SearchNote>(&["1".into(), "3".into()])
                .await
                .unwrap();
            assert_eq!(suprnova::queue::testing::pending_jobs().len(), 4);
            unsearchable::<SearchNote>(&["999".into()]).await.unwrap();
            let jobs = suprnova::queue::testing::pending_jobs();
            assert_eq!(jobs.len(), 5);
            assert_eq!(jobs.last().unwrap().payload["keys"][0][0], "999");
            assert_eq!(jobs.last().unwrap().payload["op"], "Remove");
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn syncing_suppression_is_scoped_and_nonsearchable_writes_are_noops() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(config(true), async {
        let model = note("Suppress", true, 0).await;
        without_syncing_to_search(|| async {
            after_model_write(&model, WriteKind::Created, None).await
        })
        .await
        .unwrap();
        assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
        after_model_write(&123_u64, WriteKind::Created, None)
            .await
            .unwrap();
        assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
        after_model_write(&model, WriteKind::Created, None)
            .await
            .unwrap();
        assert_eq!(
            suprnova::queue::testing::pushed::<SearchIndexJob>().len(),
            1
        );
        let result: Result<(), FrameworkError> =
            without_syncing_to_search(|| async { Err(FrameworkError::param("test")) }).await;
        assert!(result.is_err());
        after_model_write(&model, WriteKind::Updated, None)
            .await
            .unwrap();
        assert_eq!(
            suprnova::queue::testing::pushed::<SearchIndexJob>().len(),
            2
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_removals_are_registered_framework_jobs_and_prevent_stale_reinsertion() {
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let engine = Arc::new(CollectionEngine::new());
            SearchRegistry::register_engine("collection", engine.clone()).unwrap();
            let model = note("Removed", true, 0).await;
            let stale = job(&model, IndexOperation::Index);
            stale.clone().handle().await.unwrap();
            let removal = job(&model, IndexOperation::Remove);
            suprnova::queue::worker::dispatch_by_name(
                SearchIndexJob::job_name(),
                serde_json::to_value(&removal).unwrap(),
            )
            .await
            .unwrap();
            stale.handle().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            let bad = SearchIndexJob {
                index: "not_registered".into(),
                key: "1".into(),
                source_version: 1,
                op: IndexOperation::Index,
                generation: None,
            };
            assert!(
                bad.handle()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("not_registered")
            );
            assert_eq!(SearchIndexJob::job_name(), "Suprnova::SearchIndex");
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn indexing_uses_the_generation_embedder_dimensions_and_document_template() {
    use suprnova::search::{EmbedInput, Embedder, GenerationId, embedder::fake::FakeEmbedder};
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let engine = Arc::new(CollectionEngine::new());
            SearchRegistry::register_engine("collection", engine.clone()).unwrap();
            let embedder = Arc::new(FakeEmbedder::new(16).unwrap());
            SearchRegistry::register_embedder("pinned_fake", embedder.clone()).unwrap();
            let descriptor = SearchNote::descriptor();
            let mut settings = descriptor.index_settings();
            settings.index_time.embedder = Some("pinned_fake".into());
            settings.index_time.dimensions = Some(16);
            settings.index_time.embed_template = "custom {title}: {text}".into();
            settings.index_time.embed_source = Some(suprnova::search::EmbedSource {
                attributes: vec!["category".into(), "tags".into()],
                title: Some("body".into()),
            });
            let generation = GenerationId::new("embedded").unwrap();
            engine
                .create_generation(descriptor, &settings, &generation)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, None, generation.clone())
                .await
                .unwrap();
            let model = note("Embedded", true, 0).await;
            job(&model, IndexOperation::Index).handle().await.unwrap();
            let expected = embedder
                .embed_documents(&[EmbedInput {
                    text: "notes\nrust search".into(),
                    title: Some("Search body".into()),
                    media: vec![],
                    document_template: Some(settings.index_time.embed_template),
                }])
                .await
                .unwrap();
            let hits = engine
                .semantic(descriptor, &generation, &expected[0].dense, None, 10)
                .await
                .unwrap();
            assert_eq!(hits.candidates.len(), 1);
            assert!((hits.candidates[0].score - 1.0).abs() < 0.000001);
            assert_eq!(
                hits.candidates[0].document.embed_text.as_deref(),
                Some("notes\nrust search")
            );
            assert_eq!(
                hits.candidates[0].document.title.as_deref(),
                Some("Search body")
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_explicit_reimport_is_newer_than_manual_removal_and_stale_jobs() {
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let engine = Arc::new(CollectionEngine::new());
            SearchRegistry::register_engine("collection", engine.clone()).unwrap();
            let model = note("Reimport", true, 0).await;
            let keys = vec![model.id.to_string()];
            searchable::<SearchNote>(&keys).await.unwrap();
            let stale = job(&model, IndexOperation::Index);
            assert_eq!(count(engine.as_ref()).await, 1);
            unsearchable::<SearchNote>(&keys).await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            stale.clone().handle().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            searchable::<SearchNote>(&keys).await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 1);
            stale.handle().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 1);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn eligibility_restoration_advances_a_version_without_indexed_attribute_changes() {
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let engine = Arc::new(CollectionEngine::new());
            SearchRegistry::register_engine("collection", engine.clone()).unwrap();
            let model =
                EligibilityNote::create(suprnova::attrs! {title: "Eligibility", eligible: true})
                    .await
                    .unwrap();
            let document = model.to_search_document().unwrap();
            after_model_write(&model, WriteKind::Created, None)
                .await
                .unwrap();
            let disabled = model
                .update(suprnova::attrs! {eligible: false})
                .await
                .unwrap();
            assert_eq!(
                disabled.to_search_document().unwrap().attributes,
                document.attributes
            );
            after_model_write(&disabled, WriteKind::Updated, None)
                .await
                .unwrap();
            let descriptor = EligibilityNote::descriptor();
            let generation = SearchRegistry::generation_for(descriptor.name)
                .await
                .unwrap();
            let query = EngineQuery {
                expanded: ExpandedQuery::default(),
                filter: None,
                sort: vec![],
                depth: 10,
                highlights: false,
                crop: Default::default(),
            };
            assert_eq!(
                engine.count(descriptor, &generation, &query).await.unwrap(),
                Total::Exact(0)
            );
            let enabled = disabled
                .update(suprnova::attrs! {eligible: true})
                .await
                .unwrap();
            after_model_write(&enabled, WriteKind::Updated, None)
                .await
                .unwrap();
            assert_eq!(
                engine.count(descriptor, &generation, &query).await.unwrap(),
                Total::Exact(1)
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn removing_all_search_documents_clears_orphaned_keys_across_bounded_chunks() {
    use suprnova::search::indexing::remove_all_from_search;
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let engine = Arc::new(CollectionEngine::new());
            SearchRegistry::register_engine("collection", engine.clone()).unwrap();
            let mut models = Vec::new();
            for title in ["One", "Two", "Three", "Four", "Five"] {
                models.push(note(title, true, 0).await);
            }
            make_all_searchable::<SearchNote>().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 5);
            let stale = job(&models[0], IndexOperation::Index);
            models[0].clone().force_delete().await.unwrap();
            remove_all_from_search::<SearchNote>().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            stale.handle().await.unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn reordered_timestamp_free_import_cannot_resurrect_a_newer_removal() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let model = note("Tombstone", true, 0).await;
            let keys = vec![model.id.to_string()];
            searchable::<SearchNote>(&keys).await.unwrap();
            unsearchable::<SearchNote>(&keys).await.unwrap();
            let jobs = suprnova::queue::testing::pending_jobs();
            assert_eq!(jobs.len(), 2);
            let indexed_version = jobs[0].payload["keys"][0][1].as_i64().unwrap();
            let removed_version = jobs[1].payload["keys"][0][1].as_i64().unwrap();
            assert!(indexed_version > 0);
            assert!(removed_version > indexed_version);
            suprnova::queue::worker::dispatch_by_name(&jobs[1].name, jobs[1].payload.clone())
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            suprnova::queue::worker::dispatch_by_name(&jobs[0].name, jobs[0].payload.clone())
                .await
                .unwrap();
            let engine = SearchRegistry::engine_for(SearchNote::descriptor().name).unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
        }),
    )
    .await
    .unwrap();
}
