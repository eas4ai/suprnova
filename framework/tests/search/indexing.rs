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

#[tokio::test]
async fn persistence_hooks_wait_for_explicit_commit_and_drop_rollback_and_savepoints() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            fn attrs() -> suprnova::Attrs {
                suprnova::attrs! { title: "Hook", eligible: true }
            }
            let tx = DB::begin_transaction().await.unwrap();
            EligibilityNote::create_with_tx(&tx, attrs()).await.unwrap();
            assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
            tx.rollback().await.unwrap();
            assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
            DB::transaction(|_| {
                Box::pin(async {
                    EligibilityNote::create(attrs()).await?;
                    let failed: Result<(), FrameworkError> = DB::transaction(|_| {
                        Box::pin(async {
                            EligibilityNote::create(attrs()).await?;
                            Err(FrameworkError::param("discard nested write"))
                        })
                    })
                    .await;
                    assert!(failed.is_err());
                    assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
                    Ok(())
                })
            })
            .await
            .unwrap();
            let jobs = suprnova::queue::testing::pushed::<SearchIndexJob>();
            assert_eq!(jobs.len(), 1);
            assert!(jobs[0].generation.is_some());
            let tx = DB::begin_transaction().await.unwrap();
            EligibilityNote::create_with_tx(&tx, attrs()).await.unwrap();
            tx.commit().await.unwrap();
            assert_eq!(
                suprnova::queue::testing::pushed::<SearchIndexJob>().len(),
                2
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_scope_changes_refresh_guest_totals_and_facets_before_return() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let model = note("Scope freshness", true, 0).await;
            after_model_write(&model, WriteKind::Created, None)
                .await
                .unwrap();
            let visible = SearchNote::search("Scope")
                .facets(["category"])
                .get()
                .await
                .unwrap();
            assert_eq!(visible.total, Total::Exact(1));
            assert_eq!(visible.facets["category"][0].count, 1);
            model
                .update(suprnova::attrs! { owner_id: 9 })
                .await
                .unwrap();
            let hidden = SearchNote::search("Scope")
                .facets(["category"])
                .get()
                .await
                .unwrap();
            assert_eq!(hidden.total, Total::Exact(0));
            assert!(hidden.facets["category"].is_empty());
            assert_eq!(
                suprnova::queue::testing::pushed::<SearchIndexJob>().len(),
                2
            );
        }),
    )
    .await
    .unwrap();
}

#[suprnova::model(table = "search_touch_parents", timestamps = false, fillable = ["title"], relations = {
    children: HasMany<SearchTouchChild> { fk = "parent_id" },
})]
#[derive(Searchable)]
#[searchable(
    index = "search_touch_parent_index",
    touches("children"),
    with = "load_children"
)]
struct SearchTouchParent {
    id: i64,
    #[searchable(text)]
    title: String,
}

#[suprnova::model(table = "search_touch_children", timestamps = false, fillable = ["parent_id", "title"])]
struct SearchTouchChild {
    id: i64,
    parent_id: i64,
    title: String,
}

impl SearchTouchParent {
    async fn load_children(mut models: Vec<Self>) -> Result<Vec<Self>, FrameworkError> {
        let ids = models.iter().map(|model| model.id).collect::<Vec<_>>();
        let children = SearchTouchChild::query()
            .where_in("parent_id", ids)
            .get()
            .await?;
        for model in &mut models {
            model.title = children
                .iter()
                .filter(|child| child.parent_id == model.id)
                .map(|child| child.title.as_str())
                .collect::<Vec<_>>()
                .join(" ");
        }
        Ok(models)
    }
}

#[tokio::test]
async fn a_nonsearchable_related_write_enqueues_only_its_parent_and_reloads_relations() {
    let _db = database().await;
    DB::unprepared("CREATE TABLE search_touch_parents (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)").await.unwrap();
    DB::unprepared("CREATE TABLE search_touch_children (id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER NOT NULL, title TEXT NOT NULL)").await.unwrap();
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let (parent, other) = without_syncing_to_search(|| async {
                Ok((
                    SearchTouchParent::create(suprnova::attrs! {title: "Parent"}).await?,
                    SearchTouchParent::create(suprnova::attrs! {title: "Other"}).await?,
                ))
            })
            .await
            .unwrap();
            let tx = DB::begin_transaction().await.unwrap();
            SearchTouchChild::create_with_tx(
                &tx,
                suprnova::attrs! {parent_id: parent.id, title: "Discard"},
            )
            .await
            .unwrap();
            tx.rollback().await.unwrap();
            assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
            let child =
                SearchTouchChild::create(suprnova::attrs! {parent_id: parent.id, title: "Related"})
                    .await
                    .unwrap();
            let jobs = suprnova::queue::testing::pushed::<SearchIndexJob>();
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].key, parent.id.to_string());
            jobs[0].clone().handle().await.unwrap();
            assert_eq!(
                SearchTouchParent::search("Related")
                    .get()
                    .await
                    .unwrap()
                    .total,
                Total::Exact(1)
            );
            let mut moved = child
                .update(suprnova::attrs! {parent_id: other.id})
                .await
                .unwrap();
            let jobs = suprnova::queue::testing::pushed::<SearchIndexJob>();
            assert_eq!(jobs.len(), 3);
            let keys = jobs[1..]
                .iter()
                .map(|job| job.key.clone())
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(
                keys,
                [parent.id.to_string(), other.id.to_string()]
                    .into_iter()
                    .collect()
            );
            moved.parent_id = parent.id;
            moved.save().await.unwrap();
            let jobs = suprnova::queue::testing::pushed::<SearchIndexJob>();
            assert_eq!(jobs.len(), 5);
            let saved_keys = jobs[3..]
                .iter()
                .map(|job| job.key.clone())
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(
                saved_keys,
                [parent.id.to_string(), other.id.to_string()]
                    .into_iter()
                    .collect()
            );
            moved.delete().await.unwrap();
            let jobs = suprnova::queue::testing::pushed::<SearchIndexJob>();
            assert_eq!(jobs.len(), 6);
            assert_eq!(jobs[5].key, parent.id.to_string());
            for job in jobs[1..].iter().cloned() {
                job.handle().await.unwrap();
            }
            assert_eq!(
                SearchTouchParent::search("Related")
                    .get()
                    .await
                    .unwrap()
                    .total,
                Total::Exact(0)
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn derived_soft_delete_restore_and_force_delete_update_search_without_manual_hooks() {
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let model = note("Lifecycle", true, 0).await;
            after_model_write(&model, WriteKind::Created, None)
                .await
                .unwrap();
            model.clone().delete().await.unwrap();
            assert_eq!(
                SearchNote::search("Lifecycle").get().await.unwrap().total,
                Total::Exact(0)
            );
            let deleted = SearchNote::search("Lifecycle")
                .with_trashed()
                .get()
                .await
                .unwrap();
            assert_eq!(deleted.total, Total::Exact(1));
            model.clone().restore().await.unwrap();
            assert_eq!(
                SearchNote::search("Lifecycle").get().await.unwrap().total,
                Total::Exact(1)
            );
            model.force_delete().await.unwrap();
            assert_eq!(
                SearchNote::search("Lifecycle")
                    .with_trashed()
                    .get()
                    .await
                    .unwrap()
                    .total,
                Total::Exact(0)
            );
        }),
    )
    .await
    .unwrap();
}

#[suprnova::model(table = "search_pivot_parents", timestamps = false, fillable = ["title"], relations = {
    labels: BelongsToMany<SearchPivotLabel, SearchPivotLink> {
        pivot_table = "search_pivot_links", pivot_foreign_key = "parent_id", pivot_related_key = "label_id"
    },
})]
#[derive(Searchable)]
#[searchable(index = "search_pivot_parent_index", touches("labels"))]
struct SearchPivotParent {
    id: i64,
    #[searchable(text)]
    title: String,
}
#[suprnova::model(table = "search_pivot_labels", timestamps = false, fillable = ["title"])]
struct SearchPivotLabel {
    id: i64,
    title: String,
}
#[suprnova::model(table = "search_pivot_links", timestamps = false, fillable = ["parent_id", "label_id"])]
struct SearchPivotLink {
    id: i64,
    parent_id: i64,
    label_id: i64,
}

#[tokio::test]
async fn cascading_related_deletion_captures_parent_keys_before_pivot_rows_disappear() {
    let _db = database().await;
    for sql in [
        "CREATE TABLE search_pivot_parents (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE search_pivot_labels (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE search_pivot_links (id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER NOT NULL, label_id INTEGER NOT NULL)",
        "CREATE TRIGGER search_pivot_cascade AFTER DELETE ON search_pivot_labels BEGIN DELETE FROM search_pivot_links WHERE label_id = OLD.id; END",
    ] {
        DB::unprepared(sql).await.unwrap();
    }
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let (parent, label) = without_syncing_to_search(|| async {
                let parent = SearchPivotParent::create(suprnova::attrs! {title: "Parent"}).await?;
                let label = SearchPivotLabel::create(suprnova::attrs! {title: "Label"}).await?;
                SearchPivotLink::create(
                    suprnova::attrs! {parent_id: parent.id, label_id: label.id},
                )
                .await?;
                Ok((parent, label))
            })
            .await
            .unwrap();
            let tx = DB::begin_transaction().await.unwrap();
            label.clone().force_delete_with_tx(&tx).await.unwrap();
            tx.rollback().await.unwrap();
            assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
            label.force_delete().await.unwrap();
            let jobs = suprnova::queue::testing::pushed::<SearchIndexJob>();
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].index, "search_pivot_parent_index");
            assert_eq!(jobs[0].key, parent.id.to_string());
            assert_eq!(SearchPivotLink::query().get().await.unwrap().len(), 0);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn automatic_inline_writes_are_invisible_before_commit_and_absent_after_rollback() {
    let _db = database().await;
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            let tx = DB::begin_transaction().await.unwrap();
            EligibilityNote::create_with_tx(
                &tx,
                suprnova::attrs! {title: "Inline hook", eligible: true},
            )
            .await
            .unwrap();
            assert_eq!(
                EligibilityNote::search("Inline").get().await.unwrap().total,
                Total::Exact(0)
            );
            tx.rollback().await.unwrap();
            assert_eq!(
                EligibilityNote::search("Inline").get().await.unwrap().total,
                Total::Exact(0)
            );
            let tx = DB::begin_transaction().await.unwrap();
            EligibilityNote::create_with_tx(
                &tx,
                suprnova::attrs! {title: "Inline hook", eligible: true},
            )
            .await
            .unwrap();
            assert_eq!(
                EligibilityNote::search("Inline").get().await.unwrap().total,
                Total::Exact(0)
            );
            tx.commit().await.unwrap();
            assert_eq!(
                EligibilityNote::search("Inline").get().await.unwrap().total,
                Total::Exact(1)
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn duplicate_flush_cannot_remove_a_later_explicit_reimport() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let model = note("Flush cutoff", true, 0).await;
            let keys = [model.id.to_string()];
            searchable::<SearchNote>(&keys).await.unwrap();
            suprnova::search::indexing::remove_all_from_search::<SearchNote>()
                .await
                .unwrap();
            let jobs = suprnova::queue::testing::pending_jobs();
            for job in &jobs {
                suprnova::queue::worker::dispatch_by_name(&job.name, job.payload.clone())
                    .await
                    .unwrap();
            }
            let engine = SearchRegistry::engine_for(SearchNote::descriptor().name).unwrap();
            assert_eq!(count(engine.as_ref()).await, 0);
            searchable::<SearchNote>(&keys).await.unwrap();
            let latest = suprnova::queue::testing::pending_jobs()
                .into_iter()
                .filter(|job| job.name == "Suprnova::SearchIndexBatch")
                .max_by_key(|job| job.payload["keys"][0][1].as_i64().unwrap())
                .unwrap();
            suprnova::queue::worker::dispatch_by_name(&latest.name, latest.payload)
                .await
                .unwrap();
            assert_eq!(count(engine.as_ref()).await, 1);
            let flush = jobs
                .iter()
                .find(|job| job.name == "Suprnova::SearchFlush")
                .unwrap();
            suprnova::queue::worker::dispatch_by_name(&flush.name, flush.payload.clone())
                .await
                .unwrap();
            assert_eq!(count(engine.as_ref()).await, 1);
        }),
    )
    .await
    .unwrap();
}

struct IntentMigrator;
#[async_trait::async_trait]
impl sea_orm_migration::MigratorTrait for IntentMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        let mut migrations = super::derive::SearchMigrator::migrations();
        migrations.extend(suprnova::search::migrations());
        migrations
    }
}

#[tokio::test]
async fn transactional_intents_roll_back_and_committed_intents_recover_missing_dispatch() {
    let _db = suprnova::testing::TestDatabase::fresh::<IntentMigrator>()
        .await
        .unwrap();
    let _queue = Queue::fake();
    SearchConfig::scope(config(true), SearchRegistry::isolated(async {
        let index = EligibilityNote::descriptor().name;
        suprnova::search::generations::create_index(index).await.unwrap();
        let tx = DB::begin_transaction().await.unwrap();
        EligibilityNote::create_with_tx(&tx, suprnova::attrs! {title: "Rollback intent", eligible: true}).await.unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_job_intents", Vec::<sea_orm::Value>::new()).await.unwrap(), 0);
        assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
        let model = without_syncing_to_search(|| async {
            EligibilityNote::create(suprnova::attrs! {title: "Recover intent", eligible: true}).await
        }).await.unwrap();
        let staged = SearchIndexJob { index: index.into(), key: model.id.to_string(), source_version: 1, op: IndexOperation::Index, generation: None };
        DB::statement("INSERT INTO search_job_intents(intent_id,index_name,job_name,payload_json,created_at) VALUES (?,?,?,?,?)", vec!["committed_without_callback".into(), index.into(), SearchIndexJob::job_name().into(), serde_json::to_string(&staged).unwrap().into(), 1i64.into()]).await.unwrap();
        suprnova::search::generations::status(index, true).await.unwrap();
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_job_intents", Vec::<sea_orm::Value>::new()).await.unwrap(), 0);
        assert_eq!(EligibilityNote::search("Recover").get().await.unwrap().total, Total::Exact(1));
    })).await.unwrap();
}

#[suprnova::model(table = "search_touch_clock", fillable = ["title"])]
#[derive(Searchable)]
struct SearchTouchClock {
    id: i64,
    #[searchable(text)]
    title: String,
    created_at: chrono::DateTime<chrono::Utc>,
    #[searchable(filterable)]
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::test]
async fn timestamp_touch_indexes_after_atomic_commit_and_ambient_rollback_discards_it() {
    use suprnova::Touchable;
    let _db = suprnova::testing::TestDatabase::fresh::<IntentMigrator>()
        .await
        .unwrap();
    DB::unprepared("CREATE TABLE search_touch_clock (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)").await.unwrap();
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            suprnova::search::generations::create_index(SearchTouchClock::descriptor().name)
                .await
                .unwrap();
            let model = without_syncing_to_search(|| async {
                SearchTouchClock::create(suprnova::attrs! {title: "Touch"}).await
            })
            .await
            .unwrap();
            let pending = model.clone();
            let rollback: Result<(), FrameworkError> = DB::transaction(|_| {
                Box::pin(async move {
                    pending.touch().await?;
                    Err(FrameworkError::param("discard touch"))
                })
            })
            .await;
            assert!(rollback.is_err());
            assert_eq!(
                SearchTouchClock::search("Touch").get().await.unwrap().total,
                Total::Exact(0)
            );
            model.touch().await.unwrap();
            let stored = SearchTouchClock::find(model.id).await.unwrap().unwrap();
            let documents = SearchTouchClock::search("Touch").raw().await.unwrap();
            assert_eq!(documents.len(), 1);
            assert_eq!(
                documents[0].attributes,
                stored.to_search_document().unwrap().attributes
            );
            assert_eq!(
                DB::scalar::<i64>(
                    "SELECT COUNT(*) FROM search_job_intents",
                    Vec::<sea_orm::Value>::new()
                )
                .await
                .unwrap(),
                0
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn standalone_source_insert_rolls_back_when_intent_staging_fails_with_cache_disabled() {
    struct CacheDisabled;
    impl Drop for CacheDisabled {
        fn drop(&mut self) {
            suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(None);
        }
    }
    suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(Some(false));
    let _disabled = CacheDisabled;
    let _db = suprnova::testing::TestDatabase::fresh::<IntentMigrator>()
        .await
        .unwrap();
    let config = SearchConfig {
        driver: SearchDriver::Database,
        queue: false,
        ..config(false)
    };
    SearchConfig::scope(config, SearchRegistry::isolated(async {
        suprnova::search::generations::create_index(EligibilityNote::descriptor().name).await.unwrap();
        DB::unprepared("CREATE TRIGGER reject_search_intent BEFORE INSERT ON search_job_intents BEGIN SELECT RAISE(ABORT, 'reject intent staging'); END").await.unwrap();
        let outcome = EligibilityNote::create(suprnova::attrs! {title: "Atomic source", eligible: true}).await;
        assert!(outcome.unwrap_err().to_string().contains("reject intent staging"));
        assert_eq!(EligibilityNote::query().get().await.unwrap().len(), 0);
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_job_intents", Vec::<sea_orm::Value>::new()).await.unwrap(), 0);
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_jobs_pending", Vec::<sea_orm::Value>::new()).await.unwrap(), 0);
    })).await.unwrap();
}

#[tokio::test]
async fn sqlite_queued_scope_write_refreshes_totals_and_facets_before_return() {
    let _db = suprnova::testing::TestDatabase::fresh::<IntentMigrator>()
        .await
        .unwrap();
    let _queue = Queue::fake();
    let config = SearchConfig {
        driver: SearchDriver::Database,
        queue: true,
        ..config(true)
    };
    SearchConfig::scope(config, SearchRegistry::isolated(async {
        suprnova::search::generations::create_index(SearchNote::descriptor().name).await.unwrap();
        let model = SearchNote::create(suprnova::attrs! {title: "Scoped sqlite", body: "Search body", category: "notes", rank: 7, enabled: true, owner_id: 0, tags: vec!["rust"], lat: 40.0, lng: -73.0}).await.unwrap();
        let visible = SearchNote::search("Scoped").facets(["category"]).get().await.unwrap();
        assert_eq!(visible.total, Total::Exact(1));
        assert_eq!(visible.facets["category"][0].count, 1);
        model.update(suprnova::attrs! {owner_id: 9}).await.unwrap();
        let hidden = SearchNote::search("Scoped").facets(["category"]).get().await.unwrap();
        assert_eq!(hidden.total, Total::Exact(0));
        assert!(hidden.facets["category"].is_empty());
        assert_eq!(suprnova::queue::testing::pushed::<SearchIndexJob>().len(), 2);
    })).await.unwrap();
}

#[suprnova::model(table = "search_named_notes", connection = "search_secondary", timestamps = false, fillable = ["title"])]
#[derive(Searchable)]
struct NamedSearchNote {
    id: i64,
    #[searchable(text)]
    title: String,
}

async fn named_databases() -> (suprnova::testing::TestDatabase, suprnova::DbConnection) {
    use sea_orm::ConnectionTrait;
    use sea_orm_migration::MigratorTrait;
    let primary = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    let settings = suprnova::DatabaseConfig::builder()
        .url("sqlite::memory:")
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let secondary = suprnova::DbConnection::connect(&settings).await.unwrap();
    super::database_support::Migrator::up(secondary.inner(), None)
        .await
        .unwrap();
    secondary.inner().execute_unprepared("CREATE TABLE search_named_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)").await.unwrap();
    suprnova::ConnectionRegistry::register_existing("search_secondary", secondary.clone())
        .await
        .unwrap();
    (primary, secondary)
}

#[tokio::test]
async fn named_standalone_source_intent_and_inline_engine_stay_off_unmigrated_primary() {
    let (primary, _secondary) = named_databases().await;
    let config = SearchConfig {
        driver: SearchDriver::Database,
        queue: false,
        ..config(false)
    };
    SearchConfig::scope(config, SearchRegistry::isolated(async {
        suprnova::console::test(["search:index", "search_named_notes"]).run().await.assert_successful();
        let model = NamedSearchNote::create(suprnova::attrs! {title: "Named inline"}).await.unwrap();
        assert_eq!(NamedSearchNote::search("Named").keys().await.unwrap(), vec![model.id.to_string()]);
        DB::with_default_connection("search_secondary", async {
            assert_eq!(NamedSearchNote::search("Named").keys().await?, vec![model.id.to_string()]);
            assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_job_intents", Vec::<sea_orm::Value>::new()).await?, 0);
            Ok(())
        }).await.unwrap();
        suprnova::console::test(["search:import", "NamedSearchNote"]).run().await.assert_successful();
        suprnova::console::test(["search:reindex", "NamedSearchNote"]).run().await.assert_successful();
        suprnova::console::test(["search:status", "search_named_notes"]).run().await.assert_successful();
        assert_eq!(NamedSearchNote::search("Named").keys().await.unwrap(), vec![model.id.to_string()]);
        assert!(primary.fetch_all("SELECT name FROM sqlite_master WHERE name IN ('search_named_notes','search_jobs_pending','search_job_intents')", Vec::<sea_orm::Value>::new()).await.unwrap().is_empty());
    })).await.unwrap();
}

#[tokio::test]
async fn a_named_queued_worker_restores_model_connection_from_primary_context() {
    let (primary, _secondary) = named_databases().await;
    let _queue = Queue::fake();
    let config = SearchConfig {
        driver: SearchDriver::Database,
        queue: true,
        ..config(true)
    };
    SearchConfig::scope(config, SearchRegistry::isolated(async {
        DB::with_default_connection("search_secondary", async {
            suprnova::search::generations::create_index(NamedSearchNote::descriptor().name).await
        }).await.unwrap();
        let model = NamedSearchNote::create(suprnova::attrs! {title: "Named queued"}).await.unwrap();
        let jobs = suprnova::queue::testing::pushed::<SearchIndexJob>();
        assert_eq!(jobs.len(), 1);
        jobs[0].clone().handle().await.unwrap();
        DB::with_default_connection("search_secondary", async {
            assert_eq!(NamedSearchNote::search("Named").keys().await?, vec![model.id.to_string()]);
            assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_jobs_pending WHERE completed=FALSE", Vec::<sea_orm::Value>::new()).await?, 0);
            Ok(())
        }).await.unwrap();
        assert!(primary.fetch_all("SELECT name FROM sqlite_master WHERE name IN ('search_named_notes','search_jobs_pending','search_job_intents')", Vec::<sea_orm::Value>::new()).await.unwrap().is_empty());
    })).await.unwrap();
}

#[tokio::test]
async fn explicit_secondary_transaction_connection_travels_with_queue_context_without_mutating_pusher()
 {
    use sea_orm::ConnectionTrait;
    let (primary, secondary) = named_databases().await;
    secondary.inner().execute_unprepared("CREATE TABLE search_eligibility (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, eligible BOOLEAN NOT NULL)").await.unwrap();
    let queue = Queue::fake();
    let config = SearchConfig {
        driver: SearchDriver::Database,
        queue: true,
        ..config(true)
    };
    suprnova::context::Context::scope(suprnova::context::ContextStore::default(), SearchConfig::scope(config, SearchRegistry::isolated(async {
        suprnova::context::Context::hidden_add("pusher.marker", "unchanged");
        DB::with_default_connection("search_secondary", async {
            suprnova::search::generations::create_index(EligibilityNote::descriptor().name).await
        }).await.unwrap();
        let tx = DB::with_default_connection("search_secondary", async { DB::begin_transaction().await }).await.unwrap();
        let model = EligibilityNote::create_with_tx(&tx, suprnova::attrs! {title: "Explicit secondary", eligible: true}).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(suprnova::context::Context::hidden_get::<String>("pusher.marker").as_deref(), Some("unchanged"));
        assert!(suprnova::context::Context::hidden_get::<String>("suprnova.search.connection").is_none());
        let reserved = queue.driver().pop(std::time::Duration::from_secs(30)).await.unwrap().unwrap();
        let context = reserved.envelope.context.as_ref().unwrap();
        assert_eq!(context.hidden["suprnova.search.connection"], "search_secondary");
        assert_eq!(context.hidden["pusher.marker"], "unchanged");
        assert_eq!(DB::default_connection(), suprnova::database::PRIMARY_CONNECTION_NAME);
        suprnova::queue::worker::run_through_middleware(reserved.envelope).await.unwrap();
        DB::with_default_connection("search_secondary", async {
            assert_eq!(EligibilityNote::search("Explicit").keys().await?, vec![model.id.to_string()]);
            Ok(())
        }).await.unwrap();
        assert!(primary.fetch_all("SELECT name FROM sqlite_master WHERE name IN ('search_eligibility','search_jobs_pending','search_job_intents')", Vec::<sea_orm::Value>::new()).await.unwrap().is_empty());
    }))).await.unwrap();
}

struct CacheIntentMigrator;
#[async_trait::async_trait]
impl sea_orm_migration::MigratorTrait for CacheIntentMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        let mut migrations = <IntentMigrator as sea_orm_migration::MigratorTrait>::migrations();
        migrations.push(Box::new(suprnova::render_cache::migration::Migration));
        migrations
    }
}

#[tokio::test]
async fn first_searchable_write_advances_migrated_render_cache_before_any_prior_write() {
    struct CacheEnabled;
    impl Drop for CacheEnabled {
        fn drop(&mut self) {
            suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(None);
        }
    }
    suprnova::render_cache::RenderCache::set_write_side_enabled_for_test(Some(true));
    let _enabled = CacheEnabled;
    let _db = suprnova::testing::TestDatabase::fresh::<CacheIntentMigrator>()
        .await
        .unwrap();
    let config = SearchConfig {
        driver: SearchDriver::Database,
        queue: false,
        ..config(false)
    };
    SearchConfig::scope(
        config,
        SearchRegistry::isolated(async {
            suprnova::search::generations::create_index(EligibilityNote::descriptor().name)
                .await
                .unwrap();
            // Index creation now advances its own identity. Reset the probe so
            // the first model write still has to discover the migrated ledger.
            suprnova::render_cache::RenderCache::uninstall_for_test();
            let index_identity = hex::encode(
                suprnova::render_cache::DependencyIdentity::table("search:search_eligibility")
                    .digest(),
            );
            assert_eq!(
                DB::scalar::<i64>(
                    "SELECT COUNT(*) FROM suprnova_render_generations WHERE identity <> ?",
                    vec![index_identity.clone().into()]
                )
                .await
                .unwrap(),
                0
            );
            EligibilityNote::create(
                suprnova::attrs! {title: "First cached source", eligible: true},
            )
            .await
            .unwrap();
            assert!(
                DB::scalar::<i64>(
                    "SELECT COUNT(*) FROM suprnova_render_generations WHERE identity <> ?",
                    vec![index_identity.clone().into()]
                )
                .await
                .unwrap()
                    > 0
            );
            assert_eq!(
                EligibilityNote::search("First").get().await.unwrap().total,
                Total::Exact(1)
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_different_key_created_after_flush_survives_a_frozen_clock_cutoff() {
    let _db = database().await;
    let at = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    suprnova::clock::TestClock::scope(at, |_| {
        SearchConfig::scope(
            config(false),
            SearchRegistry::isolated(async {
                EligibilityNote::create(
                    suprnova::attrs! {title: "Before frozen flush", eligible: true},
                )
                .await
                .unwrap();
                suprnova::search::indexing::remove_all_from_search::<EligibilityNote>()
                    .await
                    .unwrap();
                let created = EligibilityNote::create(
                    suprnova::attrs! {title: "After frozen flush", eligible: true},
                )
                .await
                .unwrap();
                assert_eq!(
                    EligibilityNote::search("After").keys().await.unwrap(),
                    vec![created.id.to_string()]
                );
                assert_eq!(
                    EligibilityNote::search("Before").get().await.unwrap().total,
                    Total::Exact(0)
                );
            }),
        )
    })
    .await
    .unwrap();
}

/// The version each row of `SearchNote` (no `updated_at`) keeps in the database.
async fn stored_version(id: i64) -> Option<i64> {
    use sea_orm::ConnectionTrait;
    let row = DB::connection()
        .unwrap()
        .inner()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT version FROM search_source_versions WHERE index_name = ? AND model_key = ?",
            [SearchNote::descriptor().name.into(), id.to_string().into()],
        ))
        .await
        .unwrap()?;
    Some(row.try_get::<i64>("", "version").unwrap())
}

#[tokio::test]
async fn search_001_each_saved_revision_of_a_timestamp_free_model_carries_a_newer_version() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let entry = SearchRegistry::by_index(SearchNote::descriptor().name).unwrap();
            let model = note("First", true, 0).await;
            let key = model.id.to_string();
            let model = model
                .update(suprnova::attrs! { title: "Second" })
                .await
                .unwrap();
            let second = (entry.load_document)(&key).await.unwrap().unwrap();
            assert!(second.source_version > 0);
            assert_eq!(stored_version(model.id).await, Some(second.source_version));
            assert_eq!(
                model.to_search_document().unwrap().source_version,
                second.source_version
            );
            let model = model
                .update(suprnova::attrs! { title: "Third" })
                .await
                .unwrap();
            let third = (entry.load_document)(&key).await.unwrap().unwrap();
            assert!(third.source_version > second.source_version);
            assert_eq!(
                model.to_search_document().unwrap().source_version,
                third.source_version
            );
            let job = suprnova::queue::testing::pending_jobs()
                .into_iter()
                .last()
                .unwrap();
            assert_eq!(
                job.payload["source_version"].as_i64(),
                Some(third.source_version)
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn search_001_deriving_a_document_mints_no_version() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let entry = SearchRegistry::by_index(SearchNote::descriptor().name).unwrap();
            let model = note("Read", true, 0)
                .await
                .update(suprnova::attrs! { title: "Read twice" })
                .await
                .unwrap();
            let key = model.id.to_string();
            let first = (entry.load_document)(&key).await.unwrap().unwrap();
            let again = (entry.load_document)(&key).await.unwrap().unwrap();
            assert_eq!(first.source_version, again.source_version);
            assert_eq!(
                model.to_search_document().unwrap().source_version,
                model.to_search_document().unwrap().source_version
            );
            assert_eq!(stored_version(model.id).await, Some(first.source_version));
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn search_001_a_process_that_never_saved_the_row_reads_its_kept_version_on_load() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(config(true), async {
        let (id, kept) = SearchRegistry::isolated(async {
            let model = note("Elsewhere", true, 0)
                .await
                .update(suprnova::attrs! { title: "Saved elsewhere" })
                .await
                .unwrap();
            (model.id, stored_version(model.id).await.unwrap())
        })
        .await;
        // A fresh version state stands in for another process that only reads the row.
        SearchRegistry::isolated(async move {
            let model = SearchNote::find(id).await.unwrap().unwrap();
            assert_eq!(model.to_search_document().unwrap().source_version, kept);
            let listed = SearchNote::query().get().await.unwrap();
            assert_eq!(listed[0].to_search_document().unwrap().source_version, kept);
        })
        .await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn search_001_a_rolled_back_save_keeps_no_version() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        config(true),
        SearchRegistry::isolated(async {
            let model = note("Kept", true, 0)
                .await
                .update(suprnova::attrs! { title: "Kept once" })
                .await
                .unwrap();
            let kept = stored_version(model.id).await.unwrap();
            let id = model.id;
            let failed: Result<(), FrameworkError> = DB::transaction(|_| {
                Box::pin(async move {
                    SearchNote::find(id)
                        .await?
                        .unwrap()
                        .update(suprnova::attrs! { title: "Rolled back" })
                        .await?;
                    Err(FrameworkError::internal("roll back"))
                })
            })
            .await;
            assert!(failed.is_err());
            assert_eq!(stored_version(model.id).await, Some(kept));
            let reloaded = SearchNote::find(model.id).await.unwrap().unwrap();
            assert_eq!(reloaded.to_search_document().unwrap().source_version, kept);
        }),
    )
    .await
    .unwrap();
}

#[suprnova::model(table = "search_kept_rows", timestamps = false, fillable = ["id", "title"])]
#[derive(Searchable)]
#[searchable(index = "test_search_kept_rows")]
pub(crate) struct KeptRow {
    pub id: i64,
    #[searchable(text)]
    pub title: String,
}

async fn kept(id: i64) -> Option<i64> {
    let rows = DB::select(
        &format!("SELECT version FROM search_source_versions WHERE index_name = 'test_search_kept_rows' AND model_key = '{id}'"),
        vec![],
    )
    .await
    .unwrap();
    rows.first().map(|row| row.get_int("version").unwrap())
}

/// SEARCH-001's kept version on a live database: each save raises it in its own
/// transaction (Postgres through RETURNING, MySQL and MariaDB through ON DUPLICATE KEY),
/// and a rolled-back save leaves it.
async fn kept_versions_follow_saves_on(variable: &str) {
    let _guard = super::database_support::live_database(variable)
        .await
        .unwrap();
    let _ = DB::update("DROP TABLE search_kept_rows", vec![]).await;
    DB::update(
        "CREATE TABLE search_kept_rows (id BIGINT PRIMARY KEY, title VARCHAR(255) NOT NULL)",
        vec![],
    )
    .await
    .unwrap();
    SearchConfig::scope(
        config(false),
        SearchRegistry::isolated(async {
            SearchRegistry::register_engine("collection", Arc::new(CollectionEngine::new()))
                .unwrap();
            // A plain insert keeps the row's key portable; the model's saves below are
            // what the write boundary sees.
            DB::update(
                "INSERT INTO search_kept_rows (id, title) VALUES (7, 'zero')",
                vec![],
            )
            .await
            .unwrap();
            let row = KeptRow::find(7_i64)
                .await
                .unwrap()
                .unwrap()
                .update(suprnova::attrs! { title: "first" })
                .await
                .unwrap();
            let first = kept(7).await.expect("the save kept a version");
            let row = row
                .update(suprnova::attrs! { title: "second" })
                .await
                .unwrap();
            let second = kept(7).await.unwrap();
            assert!(second > first);
            assert_eq!(row.to_search_document().unwrap().source_version, second);
            let failed: Result<(), FrameworkError> = DB::transaction(|_| {
                Box::pin(async move {
                    KeptRow::find(7_i64)
                        .await?
                        .unwrap()
                        .update(suprnova::attrs! { title: "rolled back" })
                        .await?;
                    Err(FrameworkError::internal("roll back"))
                })
            })
            .await;
            assert!(failed.is_err());
            assert_eq!(kept(7).await, Some(second));
        }),
    )
    .await
    .unwrap();
    DB::update("DROP TABLE search_kept_rows", vec![])
        .await
        .unwrap();
    DB::update(
        "DELETE FROM search_source_versions WHERE index_name = 'test_search_kept_rows'",
        vec![],
    )
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(search_database_postgres)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn search_001_kept_versions_follow_saves_on_postgres() {
    kept_versions_follow_saves_on("PG_TEST_URL").await;
}

#[tokio::test]
#[serial_test::serial(search_database_mysql)]
#[ignore = "requires disposable MariaDB or MySQL at MYSQL_TEST_URL"]
async fn search_001_kept_versions_follow_saves_on_mariadb() {
    kept_versions_follow_saves_on("MYSQL_TEST_URL").await;
}
