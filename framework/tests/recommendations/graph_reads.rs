//! What the graph engine's reads take from the viewer and pass to the vector engine
//! (REC-006, REC-007).

use super::support::*;
use std::sync::{Arc, Mutex};
use suprnova::{
    EngineCapabilities, EngineQuery, EngineResult, EventType, ExplanationEvidence, FacetDistribution,
    FacetOrder, FacetStats, Filter, FrameworkError, GenerationId, IndexDescriptor, IndexSettings,
    IndexedDocument, Recommend, RecommendConfig, Recommendation, SearchEngine, SearchRegistry,
    Total, VectorDiscover, VectorRecommend, Vocabulary, WriteOutcome,
};

/// The collection engine, reporting dense retrieval and recording every recommend and
/// discover request the graph engine makes, which it answers with no candidates.
struct Recording {
    inner: suprnova::CollectionEngine,
    recommends: Mutex<Vec<VectorRecommend>>,
    discovers: Mutex<Vec<VectorDiscover>>,
}

#[async_trait::async_trait]
impl SearchEngine for Recording {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn capabilities(&self) -> EngineCapabilities {
        let mut capabilities = self.inner.capabilities();
        capabilities.semantic = true;
        capabilities
    }
    async fn current_generation(
        &self,
        index: &str,
    ) -> Result<Option<GenerationId>, FrameworkError> {
        self.inner.current_generation(index).await
    }
    async fn generation_states(
        &self,
        index: &str,
    ) -> Result<Vec<(GenerationId, String)>, FrameworkError> {
        self.inner.generation_states(index).await
    }
    async fn record_flush(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
    ) -> Result<(), FrameworkError> {
        self.inner.record_flush(index, generation, cutoff).await
    }
    async fn keys_before_version(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
        limit: usize,
    ) -> Result<Vec<(String, i64)>, FrameworkError> {
        self.inner
            .keys_before_version(index, generation, cutoff, limit)
            .await
    }
    async fn create_generation(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        self.inner
            .create_generation(index, settings, generation)
            .await
    }
    async fn switch_generation(
        &self,
        index: &IndexDescriptor,
        from: Option<GenerationId>,
        to: GenerationId,
    ) -> Result<(), FrameworkError> {
        self.inner.switch_generation(index, from, to).await
    }
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        self.inner.drop_generation(index, generation).await
    }
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        self.inner.write_settings(index, generation, settings).await
    }
    async fn read_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<IndexSettings, FrameworkError> {
        self.inner.read_settings(index, generation).await
    }
    async fn stored_hashes(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        documents: &[IndexedDocument],
    ) -> Result<std::collections::BTreeMap<String, String>, FrameworkError> {
        self.inner.stored_hashes(index, generation, documents).await
    }
    async fn index(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        documents: &[IndexedDocument],
    ) -> Result<WriteOutcome, FrameworkError> {
        self.inner.index(index, generation, documents).await
    }
    async fn remove(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError> {
        self.inner.remove(index, generation, keys).await
    }
    async fn query(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        query: &EngineQuery,
    ) -> Result<EngineResult, FrameworkError> {
        self.inner.query(index, generation, query).await
    }
    async fn semantic(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        vector: &[f32],
        filter: Option<&Filter>,
        depth: usize,
    ) -> Result<EngineResult, FrameworkError> {
        self.inner
            .semantic(index, generation, vector, filter, depth)
            .await
    }
    async fn recommend(
        &self,
        _index: &IndexDescriptor,
        _generation: &GenerationId,
        request: &VectorRecommend,
    ) -> Result<EngineResult, FrameworkError> {
        self.recommends.lock().unwrap().push(request.clone());
        Ok(EngineResult {
            candidates: Vec::new(),
            total: Total::Estimated(0),
        })
    }
    async fn discover(
        &self,
        _index: &IndexDescriptor,
        _generation: &GenerationId,
        request: &VectorDiscover,
    ) -> Result<EngineResult, FrameworkError> {
        self.discovers.lock().unwrap().push(request.clone());
        Ok(EngineResult {
            candidates: Vec::new(),
            total: Total::Estimated(0),
        })
    }
    async fn facets(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        query: &EngineQuery,
        attributes: &[String],
        cap: usize,
        order: FacetOrder,
    ) -> Result<FacetDistribution, FrameworkError> {
        self.inner
            .facets(index, generation, query, attributes, cap, order)
            .await
    }
    async fn facet_stats(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        query: &EngineQuery,
        attributes: &[String],
    ) -> Result<FacetStats, FrameworkError> {
        self.inner
            .facet_stats(index, generation, query, attributes)
            .await
    }
    async fn facet_search(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        query: &EngineQuery,
        attribute: &str,
        prefix: &str,
        cap: usize,
    ) -> Result<FacetDistribution, FrameworkError> {
        self.inner
            .facet_search(index, generation, query, attribute, prefix, cap)
            .await
    }
    async fn vocabulary(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<Arc<Vocabulary>, FrameworkError> {
        self.inner.vocabulary(index, generation).await
    }
    async fn count(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        query: &EngineQuery,
    ) -> Result<Total, FrameworkError> {
        self.inner.count(index, generation, query).await
    }
}

/// Run `future` with the graph engine over a recording engine, which it returns.
async fn recorded<F, Fut>(future: F) -> Arc<Recording>
where
    F: FnOnce(Arc<Recording>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let engine = Arc::new(Recording {
        inner: suprnova::CollectionEngine::new(),
        recommends: Mutex::default(),
        discovers: Mutex::default(),
    });
    SearchRegistry::register_engine("collection", engine.clone()).unwrap();
    let _queue = suprnova::Queue::fake();
    let config = RecommendConfig {
        engine: "graph".into(),
        ..Default::default()
    };
    RecommendConfig::scope(config, future(engine.clone()))
        .await
        .unwrap();
    engine
}

/// Peers who each like every note in `notes`, so every pair becomes a shared edge.
async fn peers_like(peers: &[&str], notes: &[&Note]) {
    for peer in peers {
        for note in notes {
            record(peer, note, EventType::Like, &format!("n{}", note.id)).await;
        }
    }
}

fn targets(rows: &[Recommendation]) -> Vec<String> {
    rows.iter().map(|row| row.target.key.clone()).collect()
}

#[tokio::test]
async fn rec_006_similar_to_leaves_out_and_steers_away_from_the_viewer_hidden_targets() {
    setup(async {
        recorded(|engine| async move {
            let a = note("A", "a", true, "o", 1).await;
            let b = note("B", "b", true, "o", 2).await;
            let c = note("C", "c", true, "o", 3).await;
            peers_like(&["p1", "p2", "p3"], &[&a, &b, &c]).await;
            record("alice", &b, EventType::Hide, "b").await;
            Recommend::rebuild_graph().await.unwrap().unwrap();
            let rows = Recommend::similar_to(key(&a))
                .for_viewer(viewer("alice"))
                .limit(5)
                .get()
                .await
                .unwrap();
            let reached = targets(&rows);
            assert!(reached.contains(&c.id.to_string()), "{reached:?}");
            assert!(!reached.contains(&b.id.to_string()), "{reached:?}");
            let recommends = engine.recommends.lock().unwrap();
            assert!(
                recommends
                    .iter()
                    .any(|request| request.negative == [b.id.to_string()]),
                "{recommends:?}"
            );
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn rec_006_discover_steers_away_from_every_visible_negative_of_the_viewer() {
    setup(async {
        recorded(|engine| async move {
            let target = note("Target", "t", true, "o", 1).await;
            let liked = note("Liked", "l", true, "o", 2).await;
            let first = note("First hidden", "h", true, "o", 3).await;
            let second = note("Second hidden", "h", true, "o", 4).await;
            peers_like(&["p1", "p2", "p3"], &[&target, &liked]).await;
            record("alice", &liked, EventType::Like, "l").await;
            record("alice", &first, EventType::Hide, "f").await;
            record("alice", &second, EventType::Hide, "s").await;
            Recommend::rebuild_graph().await.unwrap().unwrap();
            Recommend::discover(key(&target))
                .for_viewer(viewer("alice"))
                .limit(5)
                .get()
                .await
                .unwrap();
            let discovers = engine.discovers.lock().unwrap();
            let request = discovers.last().expect("a discover request");
            let negatives: std::collections::BTreeSet<&str> = request
                .context
                .iter()
                .map(|(_, negative)| negative.as_str())
                .collect();
            assert_eq!(
                negatives,
                [first.id.to_string(), second.id.to_string()]
                    .iter()
                    .map(String::as_str)
                    .collect(),
                "{request:?}"
            );
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn rec_006_a_read_for_another_person_never_steers_by_or_cites_their_events() {
    setup(async {
        recorded(|engine| async move {
            let a = note("A", "a", true, "o", 1).await;
            let b = note("B", "b", true, "o", 2).await;
            peers_like(&["p1", "p2", "p3"], &[&a, &b]).await;
            record("bob", &a, EventType::Like, "a").await;
            Recommend::rebuild_graph().await.unwrap().unwrap();
            // Bob's own read follows his like through the graph.
            let own = Recommend::for_user("bob")
                .for_viewer(viewer("bob"))
                .get()
                .await
                .unwrap();
            assert!(own.iter().any(|row| row.explanation.engine == "graph"));
            engine.recommends.lock().unwrap().clear();
            // Alice reading Bob's list sees nothing his events steer or cite.
            let rows = Recommend::for_user("bob")
                .for_viewer(viewer("alice"))
                .get()
                .await
                .unwrap();
            for row in &rows {
                assert_ne!(row.explanation.engine, "graph", "{row:?}");
                assert!(
                    !row.explanation
                        .evidence
                        .iter()
                        .any(|item| matches!(item, ExplanationEvidence::Event(_))),
                    "{row:?}"
                );
            }
            assert!(engine.recommends.lock().unwrap().is_empty());
        })
        .await;
    })
    .await;
}
