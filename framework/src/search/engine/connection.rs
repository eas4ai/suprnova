//! Declared model databases also route engine reads when no write callback is active.
use super::*;

/// A driver handle retains the index declaration without capturing a request's live pool.
pub(crate) struct ConnectionEngine {
    index: &'static str,
    inner: Arc<dyn SearchEngine>,
}
impl ConnectionEngine {
    pub(crate) fn new(index: &'static str, inner: Arc<dyn SearchEngine>) -> Self {
        Self { index, inner }
    }
}
#[async_trait]
impl SearchEngine for ConnectionEngine {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn capabilities(&self) -> EngineCapabilities {
        self.inner.capabilities()
    }
    async fn current_generation(
        &self,
        index: &str,
    ) -> Result<Option<GenerationId>, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.current_generation(index),
        )
        .await
    }
    async fn generation_states(
        &self,
        index: &str,
    ) -> Result<Vec<(GenerationId, String)>, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.generation_states(index),
        )
        .await
    }
    async fn record_flush(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
    ) -> Result<(), FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.record_flush(index, generation, cutoff),
        )
        .await
    }
    async fn keys_before_version(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
        limit: usize,
    ) -> Result<Vec<(String, i64)>, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner
                .keys_before_version(index, generation, cutoff, limit),
        )
        .await
    }
    async fn retire_index(&self, index: &str) -> Result<(), FrameworkError> {
        crate::search::registry::in_index_connection(self.index, self.inner.retire_index(index))
            .await
    }
    async fn create_generation(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.create_generation(index, settings, generation),
        )
        .await
    }
    async fn switch_generation(
        &self,
        index: &IndexDescriptor,
        from: Option<GenerationId>,
        to: GenerationId,
    ) -> Result<(), FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.switch_generation(index, from, to),
        )
        .await
    }
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.drop_generation(index, generation),
        )
        .await
    }
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.write_settings(index, generation, settings),
        )
        .await
    }
    async fn read_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<IndexSettings, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.read_settings(index, generation),
        )
        .await
    }
    async fn stored_hashes(
        &self,
        _index: &IndexDescriptor,
        _generation: &GenerationId,
        _documents: &[IndexedDocument],
    ) -> Result<BTreeMap<String, String>, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.stored_hashes(_index, _generation, _documents),
        )
        .await
    }
    async fn index(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        docs: &[IndexedDocument],
    ) -> Result<WriteOutcome, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.index(index, generation, docs),
        )
        .await
    }
    async fn remove(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.remove(index, generation, keys),
        )
        .await
    }
    async fn query(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<EngineResult, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.query(index, generation, q),
        )
        .await
    }
    async fn semantic(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        vector: &[f32],
        filter: Option<&Filter>,
        depth: usize,
    ) -> Result<EngineResult, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner
                .semantic(index, generation, vector, filter, depth),
        )
        .await
    }
    async fn facets(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
        cap: usize,
        order: FacetOrder,
    ) -> Result<FacetDistribution, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.facets(index, generation, q, attrs, cap, order),
        )
        .await
    }
    async fn facet_stats(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
    ) -> Result<FacetStats, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.facet_stats(index, generation, q, attrs),
        )
        .await
    }
    async fn facet_search(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attr: &str,
        prefix: &str,
        cap: usize,
    ) -> Result<FacetDistribution, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner
                .facet_search(index, generation, q, attr, prefix, cap),
        )
        .await
    }
    async fn vocabulary(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<Arc<Vocabulary>, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.vocabulary(index, generation),
        )
        .await
    }
    async fn count(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<Total, FrameworkError> {
        crate::search::registry::in_index_connection(
            self.index,
            self.inner.count(index, generation, q),
        )
        .await
    }
}
