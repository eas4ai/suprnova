//! The null driver acknowledges indexing while retaining only generation settings.
use super::collection::CollectionEngine;
use super::*;
/// Applications can disable storage without changing searchable model declarations.
#[derive(Default)]
pub struct NullEngine {
    metadata: CollectionEngine,
}
impl NullEngine {
    /// No document or vector storage is allocated by this driver.
    pub fn new() -> Self {
        Self::default()
    }
}
#[async_trait]
impl SearchEngine for NullEngine {
    fn name(&self) -> &str {
        "null"
    }
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            generations: true,
            ..EngineCapabilities::default()
        }
    }
    async fn keys_before_version(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
        limit: usize,
    ) -> Result<Vec<(String, i64)>, FrameworkError> {
        self.metadata
            .keys_before_version(index, generation, cutoff, limit)
            .await
    }
    async fn record_flush(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
    ) -> Result<(), FrameworkError> {
        self.metadata.record_flush(index, generation, cutoff).await
    }
    async fn current_generation(
        &self,
        index: &str,
    ) -> Result<Option<GenerationId>, FrameworkError> {
        self.metadata.current_generation(index).await
    }
    async fn retire_index(&self, index: &str) -> Result<(), FrameworkError> {
        self.metadata.retire_index(index).await
    }
    async fn generation_states(
        &self,
        index: &str,
    ) -> Result<Vec<(GenerationId, String)>, FrameworkError> {
        self.metadata.generation_states(index).await
    }
    async fn create_generation(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        self.metadata
            .create_generation(index, settings, generation)
            .await
    }
    async fn switch_generation(
        &self,
        index: &IndexDescriptor,
        from: Option<GenerationId>,
        to: GenerationId,
    ) -> Result<(), FrameworkError> {
        self.metadata.switch_generation(index, from, to).await
    }
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        self.metadata.drop_generation(index, generation).await
    }
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        self.metadata
            .write_settings(index, generation, settings)
            .await
    }
    async fn read_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<IndexSettings, FrameworkError> {
        self.metadata.read_settings(index, generation).await
    }
    async fn index(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        docs: &[IndexedDocument],
    ) -> Result<WriteOutcome, FrameworkError> {
        self.metadata.read_settings(index, generation).await?;
        for doc in docs {
            doc.document.validate(index)?;
        }
        Ok(WriteOutcome {
            written: docs.len(),
            ..WriteOutcome::default()
        })
    }
    async fn remove(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError> {
        self.metadata.read_settings(index, generation).await?;
        Ok(WriteOutcome {
            written: keys.len(),
            ..WriteOutcome::default()
        })
    }
    async fn query(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<EngineResult, FrameworkError> {
        self.metadata.query(index, generation, q).await
    }
    async fn semantic(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        _vector: &[f32],
        _filter: Option<&Filter>,
        _depth: usize,
    ) -> Result<EngineResult, FrameworkError> {
        index.validate()?;
        GenerationId::new(generation.0.clone())?;
        Err(FrameworkError::param(format!(
            "search index {}: null engine does not support semantic retrieval",
            index.name
        )))
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
        self.metadata
            .facets(index, generation, q, attrs, cap, order)
            .await
    }
    async fn facet_stats(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
    ) -> Result<FacetStats, FrameworkError> {
        self.metadata.facet_stats(index, generation, q, attrs).await
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
        self.metadata
            .facet_search(index, generation, q, attr, prefix, cap)
            .await
    }
    async fn vocabulary(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<Arc<Vocabulary>, FrameworkError> {
        self.metadata.vocabulary(index, generation).await
    }
    async fn count(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<Total, FrameworkError> {
        self.metadata.count(index, generation, q).await
    }
}
