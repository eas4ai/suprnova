//! Search keeps engine-native relevance behind one typed, scoped model API.
//!
//! The collection driver and deterministic embedder make the same pipeline testable
//! without external services. Database search resolves on the application connection;
//! applications register external drivers at bootstrap.

/// Bootstrap pins model configuration before indexing or retrieval begins.
pub mod boot;
pub mod builder;
pub mod config;
pub mod document;
pub mod embedder;
pub mod engine;
pub mod expansion;
pub mod fusion;
pub mod highlight;
pub mod indexing;
pub mod migrations;
pub mod query;
pub mod registry;
pub mod results;
pub mod scope;
pub mod settings;
#[cfg(feature = "testing")]
pub mod testing;

pub use builder::{FederatedBuilder, SearchBuilder, SearchFilter};
pub use config::{SearchConfig, SearchDriver, SearchEmbedder};
pub use document::{AttributeType, AttributeValue, GeoPoint, SearchDocument};
pub use embedder::fake::FakeEmbedder;
#[cfg(feature = "search-fastembed")]
pub use embedder::fastembed::{
    FastembedConfig, FastembedEmbedder, FastembedReranker, ManifestFile, ModelManifest,
    ModelSource, precision_files,
};
pub use embedder::http::{HttpEmbedder, HttpEmbedderConfig};
pub use embedder::profiles::{
    default_profile, names as embedding_profile_names, profile as embedding_profile,
};
pub use embedder::{
    EmbedInput, Embedder, Embedding, InputKinds, MediaPart, ModelFiles, ModelProfile,
    ModelProvenance, Pooling, Precision, SparseEmbedding, validate_embedding,
};
pub use engine::collection::CollectionEngine;
pub use engine::database::DatabaseSearchEngine;
pub use engine::null::NullEngine;
pub use engine::{
    EngineCandidate, EngineCapabilities, EngineResult, GenerationId, IndexedDocument, SearchEngine,
    WriteOutcome,
};
pub use expansion::{ExpandedQuery, ExpandedTerm, Vocabulary, expand, tokenize};
pub use fusion::{FederatedRank, federated_rrf, normalize, paginate, weighted_rrf};
pub use highlight::{highlight, validate_tags};
pub use indexing::{
    IndexOperation, SearchIndexJob, WriteKind, after_model_write, make_all_searchable,
    remove_all_from_search, searchable, unsearchable, without_syncing_to_search,
};
pub use migrations::migrations;
pub use query::{
    EngineQuery, Filter, GroupBy, QueryText, Rerank, SearchQuery, Sort, SortDirection,
};
pub use registry::{
    AttributeDecl, DocumentBatchLoader, DocumentLoader, IndexDescriptor, Reranker, SearchRegistry,
    SearchableDecl, SearchableEntry,
};
pub use results::{
    FacetDistribution, FacetRange, FacetStats, FacetValue, FederatedHit, FederatedResults, Hit,
    RawHit, SearchPagination, SearchResults, Total,
};
pub use scope::{Viewer, apply_scope, evaluate_scope};
pub use settings::{
    EmbedSource, FacetOrder, IndexSettings, IndexTimeSettings, LanguageConfig, QuerySettings,
    SettingsChange, Synonym, TypoTolerance, WeightedAttribute,
};

use crate::{FrameworkError, eloquent::Model};
use sea_orm::{EntityTrait, IntoActiveModel, PrimaryKeyTrait};
use serde::Serialize;

/// Model declarations implement this trait so search hydration remains ordinary Eloquent.
/// The derive must appear below `#[suprnova::model]` to see its generated metadata.
///
/// ```
/// use suprnova::Searchable;
/// mod example {
///     use suprnova::Searchable;
///     #[suprnova::model(table = "search_example", timestamps = false)]
///     #[derive(Searchable)]
///     pub struct Article {
///         pub id: i64,
///         #[searchable(text)]
///         pub title: String,
///     }
/// }
/// let query = example::Article::search("Rust").limit(10);
/// assert_eq!(query.search_query().limit, 10);
/// ```
#[async_trait::async_trait]
pub trait Searchable: Model + crate::eloquent::EagerLoadDispatch
where
    Self: From<<Self::Entity as EntityTrait>::Model>,
    <Self::Entity as EntityTrait>::Model: From<Self>
        + IntoActiveModel<<Self::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <Self::Entity as EntityTrait>::ActiveModel: Send,
    <<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Begin a typed query without requiring an engine-specific API at the call site.
    fn search(query: impl Into<String>) -> SearchBuilder<Self> {
        SearchBuilder::new(query)
    }

    /// Import eligible rows in bounded chunks, preparing relations before projection.
    async fn make_all_searchable() -> Result<(), FrameworkError> {
        indexing::make_all_searchable::<Self>().await
    }

    /// Remove all model keys while retaining versioned removal tombstones.
    async fn remove_all_from_search() -> Result<(), FrameworkError> {
        indexing::remove_all_from_search::<Self>().await
    }

    /// Suppress write synchronization for one awaited operation without affecting other tasks.
    async fn without_syncing_to_search<F, Fut, T>(callback: F) -> Result<T, FrameworkError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, FrameworkError>> + Send,
        T: Send,
    {
        indexing::without_syncing_to_search(callback).await
    }

    /// Immutable metadata validates all documents and caller query attributes.
    fn descriptor() -> &'static IndexDescriptor;

    /// Project declared fields only, with the source version needed for safe retries.
    fn to_search_document(&self) -> Result<SearchDocument, FrameworkError>;

    /// A false predicate removes a previously indexed row rather than leaving stale hits.
    fn should_be_searchable(&self) -> Result<bool, FrameworkError> {
        Ok(true)
    }

    /// Prepare a whole chunk before documenting any row, avoiding per-row relation loads.
    async fn make_searchable_using(models: Vec<Self>) -> Result<Vec<Self>, FrameworkError> {
        Ok(models)
    }
}

/// The facade exposes heterogeneous retrieval without inventing a shared model type.
pub struct Search;

impl Search {
    /// Install the deployment embedder once so model call sites never construct providers.
    pub async fn boot() -> Result<(), FrameworkError> {
        boot::boot().await
    }

    /// Index names are validated when the terminal method runs, including duplicates.
    pub fn federated(indexes: impl IntoIterator<Item = impl Into<String>>) -> FederatedBuilder {
        FederatedBuilder::new(indexes.into_iter().map(Into::into).collect())
    }
}
