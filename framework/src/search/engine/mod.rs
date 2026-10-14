//! Drivers own relevance while preserving one versioned, scoped query contract.
pub mod collection;
pub mod null;
use super::{
    document::SearchDocument,
    embedder::SparseEmbedding,
    expansion::Vocabulary,
    query::{EngineQuery, Filter},
    registry::IndexDescriptor,
    results::{FacetDistribution, FacetStats, Total},
    settings::{FacetOrder, IndexSettings},
};
use crate::FrameworkError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
/// Identifiers keep generation names safe for drivers that create physical indexes.
///
/// ```
/// use suprnova::GenerationId;
/// # fn main() -> Result<(), suprnova::FrameworkError> {
/// let generation = GenerationId::new("catalog_v2")?;
/// assert_eq!(generation.as_str(), "catalog_v2");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GenerationId(
    /// Stable storage identifier, validated again at each driver boundary.
    pub String,
);
impl GenerationId {
    /// Validate before a generation can be used in a physical storage name.
    pub fn new(name: impl Into<String>) -> Result<Self, FrameworkError> {
        let name = name.into();
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(FrameworkError::param(format!(
                "search generation {name}: use 1 to 64 letters, digits or underscores"
            )));
        }
        Ok(Self(name))
    }
    /// Drivers can read the stable generation identifier without copying.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl Default for GenerationId {
    fn default() -> Self {
        Self("default".into())
    }
}
/// The content identity and embeddings travel with the source version for retry safety.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexedDocument {
    /// Typed declared attributes and authoritative source version.
    pub document: SearchDocument,
    /// Hash includes index-time settings so reindexing cannot be skipped.
    pub content_hash: String,
    /// Optional dense embedding in the configured profile's dimensions.
    pub dense: Option<Vec<f32>>,
    /// Optional sparse embedding for drivers that support sparse retrieval.
    pub sparse: Option<SparseEmbedding>,
}
/// Counters explain idempotent writes without treating skips as failures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteOutcome {
    /// Number of documents or removals applied.
    pub written: usize,
    /// Attempts behind the authoritative source version.
    pub skipped_older: usize,
    /// Attempts blocked by a current removal tombstone.
    pub skipped_tombstoned: usize,
    /// Equal content identities already stored.
    pub skipped_unchanged: usize,
}
/// One engine-ranked candidate retains the document for formatting and scope checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineCandidate {
    /// The indexed projection, never an arbitrary model serialization.
    pub document: SearchDocument,
    /// Native relevance before framework normalization or fusion.
    pub score: f64,
    /// Safe native formatted fields when supported by the driver.
    pub highlights: Option<BTreeMap<String, String>>,
}
/// Candidate depth is separate from the complete lexical total.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineResult {
    /// Native order with document-key ties.
    pub candidates: Vec<EngineCandidate>,
    /// Exact lexical or explicitly bounded semantic total.
    pub total: Total,
}
/// Capability reporting makes unsupported operations fail visibly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCapabilities {
    /// Native highlights are HTML escaped before tags are inserted.
    pub native_highlight: bool,
    /// Sparse vectors can be indexed and queried.
    pub sparse: bool,
    /// Phrase adjacency is enforced.
    pub phrase: bool,
    /// Last-word prefixes can be rendered.
    pub prefix: bool,
    /// Dense cosine retrieval is available.
    pub semantic: bool,
    /// Exact lexical facet distributions are available.
    pub facets: bool,
    /// Geo filtering and sorting are available.
    pub geo: bool,
    /// Writes honor source versions and tombstones.
    pub versioned: bool,
    /// Generation switching is atomic.
    pub generations: bool,
}
/// The object-safe driver boundary keeps ranking native and filters bound.
#[async_trait]
pub trait SearchEngine: Send + Sync {
    /// Identify results so callers can understand engine-specific ranking.
    fn name(&self) -> &str;
    /// Report supported behavior without silently ignoring requested features.
    fn capabilities(&self) -> EngineCapabilities;
    /// Resolve the generation readers currently share atomically.
    async fn current_generation(&self, index: &str)
    -> Result<Option<GenerationId>, FrameworkError>;
    /// Create an empty isolated generation with its exact settings.
    async fn create_generation(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError>;
    /// Compare the current generation before switching to prevent lost updates.
    async fn switch_generation(
        &self,
        index: &IndexDescriptor,
        from: Option<GenerationId>,
        to: GenerationId,
    ) -> Result<(), FrameworkError>;
    /// Drop only an inactive generation after outstanding jobs have drained.
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError>;
    /// Query-time settings can change without changing a generation's indexed identity.
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError>;
    /// Read settings back to verify driver synchronization.
    async fn read_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<IndexSettings, FrameworkError>;
    /// Apply validated documents while rejecting stale or tombstoned writes.
    async fn index(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        docs: &[IndexedDocument],
    ) -> Result<WriteOutcome, FrameworkError>;
    /// Record source-version tombstones before acknowledging removals.
    async fn remove(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError>;
    /// Rank the complete lexical predicate, retaining only the requested candidate depth.
    async fn query(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<EngineResult, FrameworkError>;
    /// Retrieve cosine neighbors under exactly the same scope-inclusive filter.
    async fn semantic(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        vector: &[f32],
        filter: Option<&Filter>,
        depth: usize,
    ) -> Result<EngineResult, FrameworkError>;
    /// Count facet values over the full filtered lexical universe before truncation.
    async fn facets(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
        cap: usize,
        order: FacetOrder,
    ) -> Result<FacetDistribution, FrameworkError>;
    /// Ordered numeric or datetime ranges use the same full lexical universe.
    async fn facet_stats(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
    ) -> Result<FacetStats, FrameworkError>;
    /// Type-ahead facets preserve counts for matching values of one declaration.
    async fn facet_search(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attr: &str,
        prefix: &str,
        cap: usize,
    ) -> Result<FacetDistribution, FrameworkError>;
    /// Document frequencies come from the generation actually serving readers.
    async fn vocabulary(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<Arc<Vocabulary>, FrameworkError>;
    /// Count before candidate depth and pagination limit the ranked list.
    async fn count(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<Total, FrameworkError>;
}
