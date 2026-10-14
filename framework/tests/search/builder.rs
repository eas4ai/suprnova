//! End-to-end collection retrieval uses real Eloquent hydration and fake queue capture.
use super::derive::{SearchNote, database, note};
use std::sync::Arc;
use suprnova::{
    AttributeValue, CollectionEngine, FakeEmbedder, Model, Queue, Search, SearchConfig,
    SearchDriver, SearchIndexJob, SearchRegistry, Searchable, SortDirection, Total, Viewer,
    queue::Job,
};

fn config() -> SearchConfig {
    SearchConfig {
        driver: SearchDriver::Collection,
        queue: false,
        ..Default::default()
    }
}
async fn index(model: &SearchNote) {
    let document = model.to_search_document().unwrap();
    SearchIndexJob {
        index: document.index,
        key: document.key,
        source_version: document.source_version,
        op: suprnova::IndexOperation::Index,
        generation: None,
    }
    .handle()
    .await
    .unwrap();
}
struct User(i64);
impl suprnova::Authenticatable for User {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
    fn get_auth_identifier(&self) -> String {
        self.0.to_string()
    }
}
fn viewer(id: i64) -> Viewer {
    Viewer::Authenticated {
        id: id.to_string(),
        user: Arc::new(User(id)),
    }
}

#[tokio::test]
async fn hits_are_hydrated_in_engine_order_with_scores_and_escaped_highlights() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let a = note("Rust <script>search</script> guide", true, 0).await;
            let b = note("Rust book", true, 0).await;
            index(&b).await;
            index(&a).await;
            let result = SearchNote::search("searxh")
                .highlight(["title"])
                .get()
                .await
                .unwrap();
            assert_eq!(result.hits.len(), 2);
            assert_eq!(result.hits[0].model.id, a.id);
            assert_eq!(result.total, Total::Exact(2));
            assert_eq!(result.engine, "collection");
            assert!(result.hits[0].formatted["title"].contains("&lt;script&gt;<em>search</em>"));
            assert_eq!(result.hits[0].normalized_score, 1.0);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn filters_facets_stats_keys_and_raw_share_the_guest_candidate_scope() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let public = note("Rust public", true, 0).await;
            let private = note("Rust private", true, 9).await;
            index(&public).await;
            index(&private).await;
            let result = SearchNote::search("Rust")
                .filter("rank >= 7 AND tags IN ['rust']")
                .facets(["category", "tags"])
                .facet_stats(["rank"])
                .get()
                .await
                .unwrap();
            assert_eq!(result.total, Total::Exact(1));
            assert_eq!(result.facets["category"][0].count, 1);
            assert_eq!(result.facet_stats["rank"].min, AttributeValue::Integer(7));
            assert_eq!(
                SearchNote::search("Rust").keys().await.unwrap(),
                vec![public.id.to_string()]
            );
            let raw = SearchNote::search("Rust").raw().await.unwrap();
            assert_eq!(raw.len(), 1);
            assert!(!raw[0].attributes.contains_key("id"));
            assert_eq!(raw[0].attributes["owner_id"], AttributeValue::Integer(0));
            let facets = SearchNote::search("Rust")
                .facet_search("category", "no")
                .await
                .unwrap();
            assert_eq!(facets["category"][0].count, 1);
            let own = SearchNote::search("Rust")
                .for_viewer(viewer(9))
                .get()
                .await
                .unwrap();
            assert_eq!(own.hits[0].model.id, private.id);
            assert_eq!(own.total, Total::Exact(1));
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn live_scope_recheck_drops_stale_public_documents_from_hits_keys_and_raw() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let public = note("Rust changing", true, 0).await;
            index(&public).await;
            public
                .update(suprnova::attrs! { owner_id: 9 })
                .await
                .unwrap();
            assert!(
                SearchNote::search("Rust")
                    .get()
                    .await
                    .unwrap()
                    .hits
                    .is_empty()
            );
            assert!(SearchNote::search("Rust").keys().await.unwrap().is_empty());
            assert!(SearchNote::search("Rust").raw().await.unwrap().is_empty());
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn missing_row_is_dropped_and_its_removal_is_enqueued() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let model = note("Rust disappearing", true, 0).await;
            index(&model).await;
            model.force_delete().await.unwrap();
            let _queue = Queue::fake();
            let config = SearchConfig {
                queue: true,
                ..config()
            };
            SearchConfig::scope(config, async {
                assert!(
                    SearchNote::search("Rust")
                        .get()
                        .await
                        .unwrap()
                        .hits
                        .is_empty()
                );
                let pushed = suprnova::queue::testing::pushed::<SearchIndexJob>();
                assert_eq!(pushed.len(), 1);
                assert_eq!(pushed[0].op, suprnova::IndexOperation::Remove);
            })
            .await
            .unwrap();
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn hydration_callback_filters_do_not_remove_live_index_documents() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let model = note("Rust callback", true, 0).await;
            index(&model).await;
            let _queue = Queue::fake();
            assert!(
                SearchNote::search("Rust")
                    .query(|query| query.r#where("id", -1))
                    .get()
                    .await
                    .unwrap()
                    .hits
                    .is_empty()
            );
            assert!(suprnova::queue::testing::pushed::<SearchIndexJob>().is_empty());
            assert_eq!(
                SearchNote::search("Rust").get().await.unwrap().hits.len(),
                1
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn consecutive_pages_have_no_overlap_and_ties_use_keys() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            for _ in 0..5 {
                index(&note("Rust same", true, 0).await).await;
            }
            let a = SearchNote::search("Rust")
                .sort_by("rank", SortDirection::Asc)
                .paginate(2, 1)
                .await
                .unwrap();
            let b = SearchNote::search("Rust")
                .sort_by("rank", SortDirection::Asc)
                .paginate(2, 2)
                .await
                .unwrap();
            assert_eq!(a.hits.len(), 2);
            assert!(a.pagination.has_more);
            assert!(
                a.hits
                    .iter()
                    .all(|left| b.hits.iter().all(|right| left.key != right.key))
            );
            assert!(a.hits[1].key < b.hits[0].key);
            let c = SearchNote::search("Rust")
                .simple_paginate(2, 3)
                .await
                .unwrap();
            assert_eq!(c.hits.len(), 1);
            assert!(!c.pagination.has_more);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn grouping_and_distinct_keep_the_first_ranked_member() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            for _ in 0..3 {
                index(&note("Rust grouping", true, 0).await).await;
            }
            let grouped = SearchNote::search("Rust")
                .group_by("category", 2)
                .get()
                .await
                .unwrap();
            assert_eq!(grouped.hits.len(), 1);
            assert_eq!(grouped.hits[0].group_members.len(), 1);
            let engine = SearchRegistry::engine_for(SearchNote::descriptor().name).unwrap();
            let generation = SearchRegistry::generation_for(SearchNote::descriptor().name)
                .await
                .unwrap();
            let mut settings = engine
                .read_settings(SearchNote::descriptor(), &generation)
                .await
                .unwrap();
            settings.query.distinct_attribute = Some("category".into());
            engine
                .write_settings(SearchNote::descriptor(), &generation, &settings)
                .await
                .unwrap();
            assert_eq!(
                SearchNote::search("Rust").get().await.unwrap().hits.len(),
                1
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn trashed_hits_require_both_index_and_query_opt_in() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let model = note("Rust trashed", true, 0).await;
            model.clone().delete().await.unwrap();
            let entry = SearchRegistry::by_index(SearchNote::descriptor().name).unwrap();
            let document = (entry.load_document)(&model.id.to_string())
                .await
                .unwrap()
                .unwrap();
            SearchIndexJob {
                index: document.index,
                key: document.key,
                source_version: document.source_version,
                op: suprnova::IndexOperation::Index,
                generation: None,
            }
            .handle()
            .await
            .unwrap();
            assert!(
                SearchNote::search("Rust")
                    .get()
                    .await
                    .unwrap()
                    .hits
                    .is_empty()
            );
            assert_eq!(
                SearchNote::search("Rust")
                    .with_trashed()
                    .get()
                    .await
                    .unwrap()
                    .hits[0]
                    .model
                    .id,
                model.id
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn semantic_queries_validate_providers_and_return_estimated_totals() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let model = note("Rust semantic", true, 0).await;
            index(&model).await;
            assert!(
                SearchNote::search("Rust")
                    .hybrid(0.5)
                    .get()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("SEARCH_EMBEDDER")
            );
            let fake = Arc::new(FakeEmbedder::new(8).unwrap());
            SearchRegistry::register_embedder("fake", fake.clone()).unwrap();
            let engine = SearchRegistry::engine_for(SearchNote::descriptor().name).unwrap();
            let generation = SearchRegistry::generation_for(SearchNote::descriptor().name)
                .await
                .unwrap();
            let mut settings = engine
                .read_settings(SearchNote::descriptor(), &generation)
                .await
                .unwrap();
            // Index-time changes create a new generation instead of mutating a live one.
            settings.index_time.embedder = Some("fake".into());
            settings.index_time.dimensions = Some(8);
            let new = suprnova::GenerationId::new("semantic").unwrap();
            engine
                .create_generation(SearchNote::descriptor(), &settings, &new)
                .await
                .unwrap();
            engine
                .switch_generation(SearchNote::descriptor(), Some(generation), new.clone())
                .await
                .unwrap();
            index(&model).await;
            let result = SearchNote::search("Rust semantic")
                .hybrid(1.0)
                .get()
                .await
                .unwrap();
            assert_eq!(result.hits.len(), 1);
            assert!(matches!(result.total, Total::Estimated(1)));
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn invalid_builder_requests_name_attributes_and_missing_rerankers() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            assert!(
                SearchNote::search("Rust")
                    .filter("secret = 1")
                    .get()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("secret")
            );
            assert!(
                SearchNote::search("Rust")
                    .sort_by("body", SortDirection::Asc)
                    .get()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("body")
            );
            assert!(
                SearchNote::search("Rust")
                    .rerank("missing-cross-encoder", 10)
                    .get()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("missing-cross-encoder")
            );
            assert!(SearchNote::search("Rust").hybrid(1.5).get().await.is_err());
            assert!(SearchNote::search("Rust").paginate(10, 0).await.is_err());
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn federation_projects_declared_fields_and_rechecks_live_scope() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let visible = note("Rust visible", true, 0).await;
            let private = note("Rust private", true, 9).await;
            index(&visible).await;
            index(&private).await;
            let result = Search::federated([SearchNote::descriptor().name])
                .search("Rust")
                .facets(["category"])
                .highlight(["title"])
                .get()
                .await
                .unwrap();
            assert_eq!(result.hits.len(), 1);
            assert_eq!(result.total, Total::Exact(1));
            assert!(!result.hits[0].attributes.contains_key("id"));
            assert_eq!(
                result.facets[SearchNote::descriptor().name]["category"][0].count,
                1
            );
            visible
                .update(suprnova::attrs! { owner_id: 9 })
                .await
                .unwrap();
            assert!(
                Search::federated([SearchNote::descriptor().name])
                    .search("Rust")
                    .get()
                    .await
                    .unwrap()
                    .hits
                    .is_empty()
            );
            assert!(
                Search::federated([SearchNote::descriptor().name, SearchNote::descriptor().name])
                    .get()
                    .await
                    .is_err()
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn registry_errors_name_driver_index_and_provider_without_fallback() {
    SearchConfig::scope(
        SearchConfig::default(),
        SearchRegistry::isolated(async {
            assert!(
                SearchRegistry::engine_for("missing-index")
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("missing-index")
            );
            let error = match SearchRegistry::engine_for(SearchNote::descriptor().name) {
                Ok(_) => unreachable!("unregistered database driver accepted"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("SEARCH_DRIVER"));
            SearchRegistry::register_engine("custom", Arc::new(CollectionEngine::new())).unwrap();
            SearchRegistry::set_index_driver(SearchNote::descriptor().name, "custom").unwrap();
            assert_eq!(
                SearchRegistry::engine_for(SearchNote::descriptor().name)
                    .unwrap()
                    .name(),
                "collection"
            );
        }),
    )
    .await
    .unwrap();
}

#[async_trait::async_trait]
impl suprnova::Reranker for ReverseKeys {
    async fn rerank(
        &self,
        _query: &str,
        documents: &[suprnova::SearchDocument],
    ) -> Result<Vec<f64>, suprnova::FrameworkError> {
        Ok(documents
            .iter()
            .map(|document| document.key.parse::<f64>().unwrap())
            .collect())
    }
}
struct ReverseKeys;

#[tokio::test]
async fn reranking_runs_after_distinct_and_group_representatives_are_selected() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let a = note("Rust identical", true, 0).await;
            let b = note("Rust identical", true, 0).await;
            index(&a).await;
            index(&b).await;
            SearchRegistry::register_reranker("reverse-keys", Arc::new(ReverseKeys)).unwrap();
            let reranked = SearchNote::search("Rust")
                .rerank("reverse-keys", 2)
                .get()
                .await
                .unwrap();
            assert_eq!(reranked.hits[0].model.id, b.id);
            assert_eq!(reranked.hits[0].normalized_score, 1.0);
            let grouped = SearchNote::search("Rust")
                .group_by("category", 2)
                .rerank("reverse-keys", 2)
                .get()
                .await
                .unwrap();
            assert_eq!(grouped.hits[0].model.id, a.id);
            assert_eq!(grouped.hits[0].group_members[0].model.id, b.id);
            let engine = SearchRegistry::engine_for(SearchNote::descriptor().name).unwrap();
            let generation = SearchRegistry::generation_for(SearchNote::descriptor().name)
                .await
                .unwrap();
            let mut settings = engine
                .read_settings(SearchNote::descriptor(), &generation)
                .await
                .unwrap();
            settings.query.distinct_attribute = Some("category".into());
            engine
                .write_settings(SearchNote::descriptor(), &generation, &settings)
                .await
                .unwrap();
            assert_eq!(
                SearchNote::search("Rust")
                    .rerank("reverse-keys", 2)
                    .get()
                    .await
                    .unwrap()
                    .hits[0]
                    .model
                    .id,
                a.id
            );
        }),
    )
    .await
    .unwrap();
}

fn archive_scope(viewer: &Viewer) -> suprnova::Filter {
    suprnova::Filter::Eq(
        "owner_id".into(),
        AttributeValue::Integer(viewer.id().and_then(|id| id.parse().ok()).unwrap_or(0)),
    )
}
#[suprnova::model(table = "search_notes", timestamps = false, soft_deletes)]
#[derive(Searchable)]
#[searchable(index = "test_search_archives", scope = "archive_scope")]
struct SearchArchive {
    pub id: i64,
    #[searchable(text)]
    pub title: String,
    #[searchable(filterable)]
    pub owner_id: i64,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[tokio::test]
async fn federation_keeps_equal_keys_in_distinct_indexes_and_ties_follow_index_order() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let model = note("Rust shared", true, 0).await;
            index(&model).await;
            let entry = SearchRegistry::by_index(SearchArchive::descriptor().name).unwrap();
            let document = (entry.load_document)(&model.id.to_string())
                .await
                .unwrap()
                .unwrap();
            SearchIndexJob {
                index: document.index,
                key: document.key,
                source_version: document.source_version,
                op: suprnova::IndexOperation::Index,
                generation: None,
            }
            .handle()
            .await
            .unwrap();
            let indexes = [
                SearchArchive::descriptor().name,
                SearchNote::descriptor().name,
            ];
            let result = Search::federated(indexes)
                .search("Rust")
                .get()
                .await
                .unwrap();
            assert_eq!(result.total, Total::Exact(2));
            assert_eq!(result.hits.len(), 2);
            assert_eq!(result.hits[0].index, indexes[0]);
            assert_eq!(result.hits[1].index, indexes[1]);
            assert_eq!(result.hits[0].key, result.hits[1].key);
            assert_eq!(
                Search::federated(indexes)
                    .search("Rust")
                    .paginate(1, 2)
                    .await
                    .unwrap()
                    .hits[0]
                    .index,
                indexes[1]
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn hits_cap_is_applied_after_distinct_and_grouping() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let a = note("Rust identical", true, 0).await;
            let b = note("Rust identical", true, 0).await;
            let mut c = note("Rust identical", true, 0).await;
            c.category = "books".into();
            c.clone().save().await.unwrap();
            index(&a).await;
            index(&b).await;
            index(&c).await;
            let grouped = SearchNote::search("Rust")
                .hits_cap(2)
                .group_by("category", 1)
                .get()
                .await
                .unwrap();
            assert_eq!(grouped.hits.len(), 2);
            let engine = SearchRegistry::engine_for(SearchNote::descriptor().name).unwrap();
            let generation = SearchRegistry::generation_for(SearchNote::descriptor().name)
                .await
                .unwrap();
            let mut settings = engine
                .read_settings(SearchNote::descriptor(), &generation)
                .await
                .unwrap();
            settings.query.distinct_attribute = Some("category".into());
            engine
                .write_settings(SearchNote::descriptor(), &generation, &settings)
                .await
                .unwrap();
            let distinct = SearchNote::search("Rust").hits_cap(2).get().await.unwrap();
            assert_eq!(distinct.hits.len(), 2);
            assert_eq!(distinct.total, Total::Exact(3));
            let mut settings = engine
                .read_settings(SearchNote::descriptor(), &generation)
                .await
                .unwrap();
            settings.query.hits_cap = 2;
            engine
                .write_settings(SearchNote::descriptor(), &generation, &settings)
                .await
                .unwrap();
            let federated = Search::federated([SearchNote::descriptor().name])
                .search("Rust")
                .get()
                .await
                .unwrap();
            assert_eq!(federated.hits.len(), 2);
        }),
    )
    .await
    .unwrap();
}

struct DecoratedEngine {
    inner: CollectionEngine,
    inject_attribute: bool,
    unsafe_highlights: bool,
    index_calls: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait::async_trait]
impl suprnova::SearchEngine for DecoratedEngine {
    fn name(&self) -> &str {
        "collection"
    }
    fn capabilities(&self) -> suprnova::EngineCapabilities {
        let mut caps = self.inner.capabilities();
        caps.native_highlight = true;
        caps
    }
    async fn current_generation(
        &self,
        index: &str,
    ) -> Result<Option<suprnova::GenerationId>, suprnova::FrameworkError> {
        self.inner.current_generation(index).await
    }
    async fn create_generation(
        &self,
        index: &suprnova::IndexDescriptor,
        settings: &suprnova::IndexSettings,
        generation: &suprnova::GenerationId,
    ) -> Result<(), suprnova::FrameworkError> {
        self.inner
            .create_generation(index, settings, generation)
            .await
    }
    async fn switch_generation(
        &self,
        index: &suprnova::IndexDescriptor,
        from: Option<suprnova::GenerationId>,
        to: suprnova::GenerationId,
    ) -> Result<(), suprnova::FrameworkError> {
        self.inner.switch_generation(index, from, to).await
    }
    async fn drop_generation(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
    ) -> Result<(), suprnova::FrameworkError> {
        self.inner.drop_generation(index, generation).await
    }
    async fn write_settings(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        settings: &suprnova::IndexSettings,
    ) -> Result<(), suprnova::FrameworkError> {
        self.inner.write_settings(index, generation, settings).await
    }
    async fn read_settings(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
    ) -> Result<suprnova::IndexSettings, suprnova::FrameworkError> {
        self.inner.read_settings(index, generation).await
    }
    async fn stored_hashes(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        documents: &[suprnova::IndexedDocument],
    ) -> Result<std::collections::BTreeMap<String, String>, suprnova::FrameworkError> {
        self.inner.stored_hashes(index, generation, documents).await
    }
    async fn index(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        docs: &[suprnova::IndexedDocument],
    ) -> Result<suprnova::WriteOutcome, suprnova::FrameworkError> {
        self.index_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.index(index, generation, docs).await
    }
    async fn remove(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        keys: &[(String, i64)],
    ) -> Result<suprnova::WriteOutcome, suprnova::FrameworkError> {
        self.inner.remove(index, generation, keys).await
    }
    async fn query(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        q: &suprnova::EngineQuery,
    ) -> Result<suprnova::EngineResult, suprnova::FrameworkError> {
        let mut result = self.inner.query(index, generation, q).await?;
        for candidate in &mut result.candidates {
            if self.inject_attribute {
                candidate.document.attributes.insert(
                    "removed_secret".into(),
                    AttributeValue::Text("private".into()),
                );
            }
            if self.unsafe_highlights {
                candidate.highlights = Some(std::collections::BTreeMap::from([(
                    "title".into(),
                    "<img src=x onerror=alert(1)>".into(),
                )]));
            }
        }
        Ok(result)
    }
    async fn semantic(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        vector: &[f32],
        filter: Option<&suprnova::Filter>,
        depth: usize,
    ) -> Result<suprnova::EngineResult, suprnova::FrameworkError> {
        self.inner
            .semantic(index, generation, vector, filter, depth)
            .await
    }
    async fn facets(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        q: &suprnova::EngineQuery,
        attrs: &[String],
        cap: usize,
        order: suprnova::FacetOrder,
    ) -> Result<suprnova::FacetDistribution, suprnova::FrameworkError> {
        self.inner
            .facets(index, generation, q, attrs, cap, order)
            .await
    }
    async fn facet_stats(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        q: &suprnova::EngineQuery,
        attrs: &[String],
    ) -> Result<suprnova::FacetStats, suprnova::FrameworkError> {
        self.inner.facet_stats(index, generation, q, attrs).await
    }
    async fn facet_search(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        q: &suprnova::EngineQuery,
        attr: &str,
        prefix: &str,
        cap: usize,
    ) -> Result<suprnova::FacetDistribution, suprnova::FrameworkError> {
        self.inner
            .facet_search(index, generation, q, attr, prefix, cap)
            .await
    }
    async fn vocabulary(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
    ) -> Result<Arc<suprnova::Vocabulary>, suprnova::FrameworkError> {
        self.inner.vocabulary(index, generation).await
    }
    async fn count(
        &self,
        index: &suprnova::IndexDescriptor,
        generation: &suprnova::GenerationId,
        q: &suprnova::EngineQuery,
    ) -> Result<suprnova::Total, suprnova::FrameworkError> {
        self.inner.count(index, generation, q).await
    }
}

#[tokio::test]
async fn raw_and_federated_projections_reject_undeclared_fields_returned_by_a_driver() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            SearchRegistry::register_engine(
                "collection",
                Arc::new(DecoratedEngine {
                    inner: CollectionEngine::new(),
                    inject_attribute: true,
                    unsafe_highlights: false,
                    index_calls: Arc::default(),
                }),
            )
            .unwrap();
            let model = note("Rust protected", true, 0).await;
            index(&model).await;
            assert!(
                SearchNote::search("Rust")
                    .raw()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("removed_secret")
            );
            assert!(
                Search::federated([SearchNote::descriptor().name])
                    .search("Rust")
                    .get()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("removed_secret")
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn unsafe_native_highlights_fall_back_to_escaped_framework_formatting() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            SearchRegistry::register_engine(
                "collection",
                Arc::new(DecoratedEngine {
                    inner: CollectionEngine::new(),
                    inject_attribute: false,
                    unsafe_highlights: true,
                    index_calls: Arc::default(),
                }),
            )
            .unwrap();
            let model = note("Rust <script>alert(1)</script>", true, 0).await;
            index(&model).await;
            let result = SearchNote::search("Rust")
                .highlight(["title"])
                .get()
                .await
                .unwrap();
            let formatted = &result.hits[0].formatted["title"];
            assert!(!formatted.contains("<img"));
            assert!(!formatted.contains("<script"));
            assert!(formatted.contains("&lt;script&gt;"));
            assert!(formatted.contains("<em>Rust</em>"));
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn hydration_callback_cannot_restore_deleted_hits_without_index_opt_in() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let model = note("Rust deleted", true, 0).await;
            let entry = SearchRegistry::by_index(SearchArchive::descriptor().name).unwrap();
            let document = (entry.load_document)(&model.id.to_string())
                .await
                .unwrap()
                .unwrap();
            SearchIndexJob {
                index: document.index,
                key: document.key,
                source_version: document.source_version,
                op: suprnova::IndexOperation::Index,
                generation: None,
            }
            .handle()
            .await
            .unwrap();
            suprnova::without_syncing_to_search(|| async { model.delete().await.map(|_| ()) })
                .await
                .unwrap();
            assert!(
                SearchArchive::search("Rust")
                    .query(|query| query.with_trashed())
                    .get()
                    .await
                    .unwrap()
                    .hits
                    .is_empty()
            );
            assert!(
                SearchArchive::search("Rust")
                    .query(|query| query.with_trashed())
                    .keys()
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                SearchArchive::search("Rust")
                    .query(|query| query.with_trashed())
                    .raw()
                    .await
                    .unwrap()
                    .is_empty()
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn deeply_nested_typed_filters_return_an_error_without_overflowing() {
    // A child process isolates stack-abort regressions from the rest of the suite.
    const CHILD: &str = "SUPRNOVA_DEEP_FILTER_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "builder::deeply_nested_typed_filters_return_an_error_without_overflowing",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            for shape in 0..3 {
                let mut filter =
                    suprnova::Filter::Eq("owner_id".into(), AttributeValue::Integer(0));
                for _ in 0..100_000 {
                    filter = match shape {
                        0 => suprnova::Filter::Not(Box::new(filter)),
                        1 => suprnova::Filter::And(vec![filter]),
                        _ => suprnova::Filter::Or(vec![filter]),
                    };
                }
                let error = SearchNote::search("Rust")
                    .filter(filter)
                    .get()
                    .await
                    .unwrap_err();
                assert!(matches!(error, suprnova::FrameworkError::ParamError { .. }));
                assert!(error.to_string().contains("nesting exceeds 64"));
            }
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn semantic_and_hybrid_results_exclude_negative_words_in_searchable_lists() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            SearchRegistry::register_embedder("fake", Arc::new(FakeEmbedder::new(8).unwrap()))
                .unwrap();
            let descriptor = SearchNote::descriptor();
            let engine = SearchRegistry::engine_for(descriptor.name).unwrap();
            let mut settings = descriptor.index_settings();
            settings.index_time.embedder = Some("fake".into());
            settings.index_time.dimensions = Some(8);
            let generation = suprnova::GenerationId::new("negative_semantic").unwrap();
            engine
                .create_generation(descriptor, &settings, &generation)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, None, generation)
                .await
                .unwrap();
            let blocked = note("Rust forbidden", true, 0).await;
            index(&blocked).await;
            let listed = note("Rust listed", true, 0)
                .await
                .update(suprnova::attrs! { tags: vec!["FORBIDDEN"] })
                .await
                .unwrap();
            index(&listed).await;
            let allowed = note("Rust permitted", true, 0).await;
            index(&allowed).await;
            for ratio in [0.5, 1.0] {
                let result = SearchNote::search("Rust -forbidden")
                    .hybrid(ratio)
                    .get()
                    .await
                    .unwrap();
                assert_eq!(
                    result
                        .hits
                        .iter()
                        .map(|hit| hit.key.clone())
                        .collect::<Vec<_>>(),
                    [allowed.id.to_string()]
                );
            }
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn pagination_refills_candidates_dropped_by_live_scope_checks() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let a = note("Rust same", true, 0).await;
            let b = note("Rust same", true, 0).await;
            let c = note("Rust same", true, 0).await;
            index(&a).await;
            index(&b).await;
            index(&c).await;
            a.update(suprnova::attrs! { owner_id: 9 }).await.unwrap();
            let first = SearchNote::search("Rust")
                .hits_cap(2)
                .paginate(1, 1)
                .await
                .unwrap();
            assert_eq!(first.hits[0].key, b.id.to_string());
            let second = SearchNote::search("Rust")
                .hits_cap(2)
                .paginate(1, 2)
                .await
                .unwrap();
            assert_eq!(
                second
                    .hits
                    .iter()
                    .map(|hit| hit.key.clone())
                    .collect::<Vec<_>>(),
                [c.id.to_string()]
            );
            assert_eq!(
                SearchNote::search("Rust")
                    .hits_cap(2)
                    .get()
                    .await
                    .unwrap()
                    .hits
                    .len(),
                2
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn searchable_text_lists_are_joined_for_highlights_and_crops() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let model = note("Unrelated", true, 0)
                .await
                .update(suprnova::attrs! { tags: vec!["before <script>", "Rust", "after after"] })
                .await
                .unwrap();
            index(&model).await;
            let result = SearchNote::search("rust")
                .highlight(["tags"])
                .get()
                .await
                .unwrap();
            assert_eq!(
                result.hits[0].formatted["tags"],
                "before &lt;script&gt; <em>Rust</em> after after"
            );
            let cropped = SearchNote::search("rust")
                .crop("tags", 8)
                .get()
                .await
                .unwrap();
            let text = &cropped.hits[0].formatted["tags"];
            assert!(text.contains("<em>Rust</em>"));
            assert!(text.contains("..."));
            assert!(!text.contains("<script>"));
        }),
    )
    .await
    .unwrap();
}

struct CountingEmbedder {
    inner: FakeEmbedder,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl suprnova::Embedder for CountingEmbedder {
    fn profile(&self) -> &suprnova::ModelProfile {
        suprnova::Embedder::profile(&self.inner)
    }
    async fn embed_documents(
        &self,
        inputs: &[suprnova::EmbedInput],
    ) -> Result<Vec<suprnova::Embedding>, suprnova::FrameworkError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        suprnova::Embedder::embed_documents(&self.inner, inputs).await
    }
    async fn embed_query(
        &self,
        text: &str,
    ) -> Result<suprnova::Embedding, suprnova::FrameworkError> {
        suprnova::Embedder::embed_query(&self.inner, text).await
    }
}

#[tokio::test]
async fn unchanged_writes_skip_embedding_and_indexing_but_preserve_the_version_watermark() {
    use std::sync::atomic::Ordering::SeqCst;
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            SearchRegistry::register_engine(
                "collection",
                Arc::new(DecoratedEngine {
                    inner: CollectionEngine::new(),
                    inject_attribute: false,
                    unsafe_highlights: false,
                    index_calls: calls.clone(),
                }),
            )
            .unwrap();
            let embedder = Arc::new(CountingEmbedder {
                inner: FakeEmbedder::new(8).unwrap(),
                calls: Default::default(),
            });
            SearchRegistry::register_embedder("counting", embedder.clone()).unwrap();
            let descriptor = SearchNote::descriptor();
            let engine = SearchRegistry::engine_for(descriptor.name).unwrap();
            let mut settings = descriptor.index_settings();
            settings.index_time.embedder = Some("counting".into());
            settings.index_time.dimensions = Some(8);
            let generation = suprnova::GenerationId::new("hash_skip").unwrap();
            engine
                .create_generation(descriptor, &settings, &generation)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, None, generation.clone())
                .await
                .unwrap();
            let model = note("Unchanged", true, 0).await;
            for version in [10, 20] {
                SearchIndexJob {
                    index: descriptor.name.into(),
                    key: model.id.to_string(),
                    source_version: version,
                    op: suprnova::IndexOperation::Index,
                    generation: None,
                }
                .handle()
                .await
                .unwrap();
            }
            assert_eq!(embedder.calls.load(SeqCst), 1);
            assert_eq!(calls.load(SeqCst), 1);
            let mut stale = model.to_search_document().unwrap();
            stale.source_version = 15;
            stale
                .attributes
                .insert("title".into(), AttributeValue::Text("Stale".into()));
            let result = engine
                .index(
                    descriptor,
                    &generation,
                    &[suprnova::IndexedDocument {
                        content_hash: stale.content_hash(&settings.index_time.digest()),
                        document: stale,
                        dense: None,
                        sparse: None,
                    }],
                )
                .await
                .unwrap();
            assert_eq!(result.skipped_older, 1);
            assert_eq!(
                SearchNote::search("Unchanged").keys().await.unwrap(),
                [model.id.to_string()]
            );
            // A settings identity change must force fresh embeddings in the new generation.
            settings.index_time.searchable_attributes[0].weight += 1.0;
            let next = suprnova::GenerationId::new("hash_skip_new").unwrap();
            engine
                .create_generation(descriptor, &settings, &next)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, Some(generation), next)
                .await
                .unwrap();
            index(&model).await;
            assert_eq!(embedder.calls.load(SeqCst), 2);
            assert_eq!(calls.load(SeqCst), 3);
            let changed = model
                .update(suprnova::attrs! { title: "Changed" })
                .await
                .unwrap();
            index(&changed).await;
            assert_eq!(embedder.calls.load(SeqCst), 3);
            assert_eq!(calls.load(SeqCst), 4);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn refills_reuse_the_hydration_callback_and_stop_at_the_live_hits_cap() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let a = note("Rust same", true, 0).await;
            let b = note("Rust same", true, 0).await;
            let c = note("Rust same", true, 0).await;
            let d = note("Rust same", true, 0).await;
            for model in [&a, &b, &c, &d] {
                index(model).await;
            }
            let calls = Arc::new(AtomicUsize::new(0));
            let callback_calls = calls.clone();
            let keys = vec![b.id, c.id, d.id];
            let result = SearchNote::search("Rust")
                .hits_cap(2)
                .query(move |query| {
                    callback_calls.fetch_add(1, SeqCst);
                    query.where_in("id", keys)
                })
                .get()
                .await
                .unwrap();
            assert_eq!(calls.load(SeqCst), 1);
            assert_eq!(
                result
                    .hits
                    .iter()
                    .map(|hit| hit.key.clone())
                    .collect::<Vec<_>>(),
                [b.id.to_string(), c.id.to_string()]
            );
            let keys = SearchNote::search("Rust")
                .hits_cap(2)
                .query(|query| query.where_in("id", Vec::<i64>::new()))
                .keys()
                .await
                .unwrap();
            assert!(keys.is_empty());
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn refills_preserve_live_group_members_and_distinct_representatives() {
    let _db = database().await;
    SearchConfig::scope(
        config(),
        SearchRegistry::isolated(async {
            let a = note("Rust same", true, 0).await;
            let b = note("Rust same", true, 0).await;
            let c = note("Rust same", true, 0).await;
            let d = note("Rust same", true, 0)
                .await
                .update(suprnova::attrs! { category: "books" })
                .await
                .unwrap();
            let e = note("Rust same", true, 0)
                .await
                .update(suprnova::attrs! { category: "books" })
                .await
                .unwrap();
            for model in [&a, &b, &c, &d, &e] {
                index(model).await;
            }
            a.update(suprnova::attrs! { owner_id: 9 }).await.unwrap();
            let groups = SearchNote::search("Rust")
                .hits_cap(2)
                .group_by("category", 2)
                .get()
                .await
                .unwrap();
            assert_eq!(groups.hits.len(), 2);
            assert_eq!(groups.hits[0].key, b.id.to_string());
            assert_eq!(groups.hits[0].group_members[0].key, c.id.to_string());
            assert_eq!(groups.hits[1].key, d.id.to_string());
            assert_eq!(groups.hits[1].group_members[0].key, e.id.to_string());
            let descriptor = SearchNote::descriptor();
            let engine = SearchRegistry::engine_for(descriptor.name).unwrap();
            let generation = SearchRegistry::generation_for(descriptor.name)
                .await
                .unwrap();
            let mut settings = engine.read_settings(descriptor, &generation).await.unwrap();
            settings.query.distinct_attribute = Some("category".into());
            engine
                .write_settings(descriptor, &generation, &settings)
                .await
                .unwrap();
            let distinct = SearchNote::search("Rust").hits_cap(2).get().await.unwrap();
            assert_eq!(
                distinct
                    .hits
                    .iter()
                    .map(|hit| hit.key.clone())
                    .collect::<Vec<_>>(),
                [b.id.to_string(), d.id.to_string()]
            );
        }),
    )
    .await
    .unwrap();
}
