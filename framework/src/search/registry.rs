//! Inventory supplies model metadata; runtime registrations supply replaceable drivers.

use super::{
    config::SearchConfig,
    document::{AttributeType, SearchDocument},
    embedder::Embedder,
    engine::{
        GenerationId, SearchEngine, collection::CollectionEngine, null::NullEngine,
        qdrant::QdrantSearchEngine,
    },
    query::Filter,
    scope::Viewer,
    settings::{EmbedSource, IndexSettings, WeightedAttribute},
};
use crate::FrameworkError;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, RwLock};

/// Searchable metadata retains declaration order without prescribing engine tokenization.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchableDecl {
    /// The engine applies this positive finite column weight to native relevance.
    pub weight: f32,
    /// Declaration order resolves otherwise equal attribute precedence.
    pub order: usize,
}

/// Typed roles ensure filters, sorts and facets validate against the same schema.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeDecl {
    /// Column names are code-owned and are never parsed from raw SQL.
    pub name: String,
    /// The type binds filter values and validates index writes.
    pub ty: AttributeType,
    /// Text attributes can participate in native relevance with a weight.
    pub searchable: Option<SearchableDecl>,
    /// Only filterable attributes may appear in caller or scope predicates.
    pub filterable: bool,
    /// Sortable attributes may determine the primary result order.
    pub sortable: bool,
    /// Facets imply filterability so a count can be verified by the same filter.
    pub facet: bool,
}

/// A model's index contract is immutable metadata emitted by Searchable.
pub struct IndexDescriptor {
    /// Index identity remains stable across physical engine generations.
    pub name: &'static str,
    /// The model table makes hydration independent of the engine.
    pub model_table: &'static str,
    /// The actual primary key column supports custom model keys.
    pub key_column: &'static str,
    /// Declared attributes are the complete permitted document projection.
    pub attributes: Vec<AttributeDecl>,
    /// Optional semantic sources preserve the declared field order and title.
    pub embed_source: Option<EmbedSource>,
    /// Soft-deleted rows require this index opt-in and an explicit query opt-in.
    pub soft_delete: bool,
    /// Named relations tell the write integration which parents need reindexing.
    pub search_touches: Vec<&'static str>,
    /// Code-owned settings can change without mutating the descriptor's schema.
    pub settings: fn() -> IndexSettings,
    /// A scope is always evaluated with a named viewer, including guests.
    pub scope: Option<fn(&Viewer) -> Filter>,
}

impl IndexDescriptor {
    /// Resolve one declared attribute and name unknown attributes in the error.
    pub fn attribute(&self, name: &str) -> Result<&AttributeDecl, FrameworkError> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .ok_or_else(|| {
                FrameworkError::param(format!(
                    "attribute '{name}' is not declared in index '{}'",
                    self.name
                ))
            })
    }

    /// Merge declared roles into settings so engines cannot miss weights or typed columns.
    pub fn index_settings(&self) -> IndexSettings {
        let mut settings = (self.settings)();
        if settings.index_time.searchable_attributes.is_empty() {
            let mut attributes: Vec<_> = self
                .attributes
                .iter()
                .filter_map(|attribute| {
                    attribute.searchable.as_ref().map(|searchable| {
                        (
                            searchable.order,
                            WeightedAttribute {
                                name: attribute.name.clone(),
                                weight: searchable.weight,
                            },
                        )
                    })
                })
                .collect();
            attributes.sort_by_key(|(order, _)| *order);
            settings.index_time.searchable_attributes = attributes
                .into_iter()
                .map(|(_, attribute)| attribute)
                .collect();
        }
        if settings.index_time.embed_source.is_none() {
            settings.index_time.embed_source = self.embed_source.clone();
        }
        for attribute in &self.attributes {
            if attribute.filterable || attribute.facet {
                settings
                    .index_time
                    .filterable_attributes
                    .entry(attribute.name.clone())
                    .or_insert(attribute.ty);
            }
            if attribute.sortable {
                settings
                    .index_time
                    .sortable_attributes
                    .entry(attribute.name.clone())
                    .or_insert(attribute.ty);
            }
            if attribute.facet {
                settings
                    .index_time
                    .facet_attributes
                    .entry(attribute.name.clone())
                    .or_insert(attribute.ty);
            }
        }
        settings
    }

    /// Validate metadata at the driver boundary, including code-owned identifiers.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        fn identifier(name: &str) -> bool {
            !name.is_empty()
                && name.len() <= 128
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }
        if !identifier(self.name) {
            return Err(FrameworkError::param(format!(
                "index '{}' requires an identifier of at most 128 letters, digits or underscores",
                self.name
            )));
        }
        let mut names = std::collections::BTreeSet::new();
        for attribute in &self.attributes {
            if !identifier(&attribute.name) || !names.insert(&attribute.name) {
                return Err(FrameworkError::param(format!(
                    "attribute '{}' is invalid or repeated in index '{}'",
                    attribute.name, self.name
                )));
            }
            if attribute.facet && !attribute.filterable {
                return Err(FrameworkError::param(format!(
                    "facet attribute '{}' must be filterable",
                    attribute.name
                )));
            }
            if attribute.searchable.is_some()
                && !matches!(attribute.ty, AttributeType::Text | AttributeType::TextList)
            {
                return Err(FrameworkError::param(format!(
                    "searchable attribute '{}' must be text",
                    attribute.name
                )));
            }
        }
        let settings = self.index_settings();
        settings.validate()?;
        for attribute in &settings.index_time.searchable_attributes {
            let declaration = self.attribute(&attribute.name)?;
            if declaration.searchable.is_none() {
                return Err(FrameworkError::param(format!(
                    "attribute '{}' is not searchable",
                    attribute.name
                )));
            }
        }
        for declarations in [
            &settings.index_time.filterable_attributes,
            &settings.index_time.sortable_attributes,
            &settings.index_time.facet_attributes,
        ] {
            for (name, ty) in declarations {
                if self.attribute(name)?.ty != *ty {
                    return Err(FrameworkError::param(format!(
                        "attribute '{name}' settings type differs from its declaration"
                    )));
                }
            }
        }
        if let Some(source) = &settings.index_time.embed_source {
            for name in source.attributes.iter().chain(source.title.iter()) {
                self.attribute(name)?;
            }
        }
        if let Some(name) = &settings.query.distinct_attribute {
            self.attribute(name)?;
        }
        Ok(())
    }
}

/// Type-erased loaders allow queue workers to reload a model without its Rust type.
pub type DocumentLoader =
    fn(
        &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SearchDocument>, FrameworkError>> + Send>>;

/// Batch hydration avoids per-hit database reads during heterogeneous retrieval.
pub type DocumentBatchLoader =
    fn(
        Vec<String>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchDocument>, FrameworkError>> + Send>>;

/// One inventory entry connects model write hooks, jobs and scoped reads.
pub struct SearchableEntry {
    /// Runtime type identity makes non-searchable write hooks a cheap no-op.
    pub type_id: fn() -> TypeId,
    /// The index name supports jobs whose payload carries no model type.
    pub index: &'static str,
    /// Static metadata remains available for the process lifetime.
    pub descriptor: fn() -> &'static IndexDescriptor,
    /// The loader rereads the row so a reordered job uses current content.
    pub load_document: DocumentLoader,
    /// Batch reloads keep federated live-row checks to one query per index.
    pub load_documents: DocumentBatchLoader,
    /// Live document evaluation closes stale index authorization gaps.
    pub evaluate_scope: fn(&Viewer, &SearchDocument) -> bool,
    /// A checked downcast converts a model write to its declared projection.
    pub document: fn(&dyn Any) -> Result<SearchDocument, FrameworkError>,
    /// Predicates run before enqueueing and again when a job reloads the row.
    pub should_index: fn(&dyn Any) -> Result<bool, FrameworkError>,
}

inventory::collect!(SearchableEntry);

/// Rerankers score a bounded leading slice without coupling the builder to a provider.
#[async_trait::async_trait]
pub trait Reranker: Send + Sync {
    /// Scores correspond positionally to documents; invalid lengths or scores are errors.
    async fn rerank(
        &self,
        query: &str,
        documents: &[SearchDocument],
    ) -> Result<Vec<f64>, FrameworkError>;
}

#[derive(Default)]
struct Services {
    engines: RwLock<HashMap<String, Arc<dyn SearchEngine>>>,
    index_drivers: RwLock<HashMap<String, String>>,
    embedders: RwLock<HashMap<String, Arc<dyn Embedder>>>,
    rerankers: RwLock<HashMap<String, Arc<dyn Reranker>>>,
    initializations: tokio::sync::Mutex<()>,
}

static SERVICES: LazyLock<Arc<Services>> = LazyLock::new(|| Arc::new(Services::default()));
tokio::task_local! { static ISOLATED_SERVICES: Arc<Services>; }

fn services() -> Arc<Services> {
    ISOLATED_SERVICES
        .try_with(Arc::clone)
        .unwrap_or_else(|_| Arc::clone(&SERVICES))
}
fn poison() -> FrameworkError {
    FrameworkError::internal("search registry lock poisoned")
}

struct InventoryIndex {
    names: std::collections::BTreeMap<&'static str, Vec<&'static SearchableEntry>>,
    types: HashMap<TypeId, Vec<&'static SearchableEntry>>,
}
static INVENTORY: LazyLock<InventoryIndex> = LazyLock::new(|| {
    let mut names = std::collections::BTreeMap::<_, Vec<_>>::new();
    let mut types = HashMap::<_, Vec<_>>::new();
    for entry in inventory::iter::<SearchableEntry> {
        names.entry(entry.index).or_default().push(entry);
        types.entry((entry.type_id)()).or_default().push(entry);
    }
    InventoryIndex { names, types }
});

pub(crate) struct SearchExecutionContext {
    state: Arc<Services>,
    config: SearchConfig,
}

pub(crate) fn capture_context(config: SearchConfig) -> SearchExecutionContext {
    SearchExecutionContext {
        state: services(),
        config,
    }
}

pub(crate) async fn in_context<T>(
    context: SearchExecutionContext,
    future: impl Future<Output = Result<T, FrameworkError>>,
) -> Result<T, FrameworkError> {
    SearchConfig::scope(
        context.config,
        ISOLATED_SERVICES.scope(context.state, future),
    )
    .await?
}

/// The registry resolves immutable model entries and configurable engine handles.
pub struct SearchRegistry;

impl SearchRegistry {
    /// Bootstrap configured Qdrant search before requests or jobs resolve a generation.
    /// Explicitly registered engines win over environment defaults.
    pub fn bootstrap() -> Result<(), FrameworkError> {
        let configured = SearchConfig::current()?.driver.as_str() == "qdrant"
            || std::env::var("QDRANT_URL").is_ok_and(|v| !v.trim().is_empty());
        if configured
            && !services()
                .engines
                .read()
                .map_err(|_| poison())?
                .contains_key("qdrant")
        {
            Self::register_engine("qdrant", Arc::new(QdrantSearchEngine::from_env()?))?;
        }
        Ok(())
    }

    /// Duplicate index names fail instead of choosing inventory's unspecified order.
    pub fn by_index(index: &str) -> Result<&'static SearchableEntry, FrameworkError> {
        let entries = INVENTORY.names.get(index).ok_or_else(|| {
            FrameworkError::param(format!("search index '{index}' is not registered"))
        })?;
        if entries.len() != 1 {
            return Err(FrameworkError::param(format!(
                "search index '{index}' is registered more than once"
            )));
        }
        let entry = entries[0];
        (entry.descriptor)().validate()?;
        Ok(entry)
    }

    /// Models absent from inventory return None so normal ORM writes stay unchanged.
    pub fn by_type(type_id: TypeId) -> Result<Option<&'static SearchableEntry>, FrameworkError> {
        match INVENTORY.types.get(&type_id) {
            None => Ok(None),
            Some(entries) if entries.len() == 1 => Ok(Some(entries[0])),
            Some(_) => Err(FrameworkError::param(
                "search model type is registered more than once",
            )),
        }
    }

    /// Names are sorted for stable command output and test diagnostics.
    pub fn indexes() -> Result<Vec<&'static str>, FrameworkError> {
        let mut indexes = Vec::new();
        for entry in inventory::iter::<SearchableEntry> {
            Self::by_index(entry.index)?;
            indexes.push(entry.index);
        }
        indexes.sort_unstable();
        Ok(indexes)
    }

    /// Register database, Qdrant or custom drivers at application bootstrap.
    pub fn register_engine(
        name: impl Into<String>,
        engine: Arc<dyn SearchEngine>,
    ) -> Result<(), FrameworkError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(FrameworkError::param(
                "SEARCH_DRIVER requires a nonempty registered name",
            ));
        }
        services()
            .engines
            .write()
            .map_err(|_| poison())?
            .insert(name, engine);
        Ok(())
    }

    /// Per-index selection changes the driver without changing any search call site.
    pub fn set_index_driver(
        index: impl Into<String>,
        driver: impl Into<String>,
    ) -> Result<(), FrameworkError> {
        let index = index.into();
        Self::by_index(&index)?;
        let driver = driver.into();
        if driver.trim().is_empty() {
            return Err(FrameworkError::param(format!(
                "SEARCH_DRIVER for index '{index}' is empty"
            )));
        }
        services()
            .index_drivers
            .write()
            .map_err(|_| poison())?
            .insert(index, driver);
        Ok(())
    }

    /// Built-in database and test engines resolve lazily; applications register other drivers.
    pub fn engine_for(index: &str) -> Result<Arc<dyn SearchEngine>, FrameworkError> {
        Self::by_index(index)?;
        let config = SearchConfig::current()?;
        let state = services();
        let driver = state
            .index_drivers
            .read()
            .map_err(|_| poison())?
            .get(index)
            .cloned()
            .unwrap_or_else(|| config.driver.as_str().into());
        if let Some(engine) = state
            .engines
            .read()
            .map_err(|_| poison())?
            .get(&driver)
            .cloned()
        {
            return Ok(engine);
        }
        let engine: Arc<dyn SearchEngine> = match driver.as_str() {
            "collection" => Arc::new(CollectionEngine::with_tombstone_hours(
                config.tombstone_hours,
            )?),
            "null" => Arc::new(NullEngine::new()),
            "database" => {
                let engine: Arc<dyn SearchEngine> =
                    Arc::new(super::engine::database::DatabaseSearchEngine::new());
                Self::register_engine("database", Arc::clone(&engine))?;
                return Ok(engine);
            }
            "qdrant" => Arc::new(QdrantSearchEngine::from_env()?),
            _ => {
                return Err(FrameworkError::param(format!(
                    "SEARCH_DRIVER '{driver}' for index '{index}' is not registered"
                )));
            }
        };
        let mut engines = state.engines.write().map_err(|_| poison())?;
        Ok(Arc::clone(engines.entry(driver).or_insert(engine)))
    }

    /// In-memory drivers get a complete default generation before their first write.
    /// Persistent drivers require an explicitly initialized generation to avoid hidden DDL.
    pub async fn generation_for(index: &str) -> Result<GenerationId, FrameworkError> {
        let engine = Self::engine_for(index)?;
        if let Some(generation) = engine.current_generation(index).await? {
            return Ok(generation);
        }
        if !matches!(engine.name(), "collection" | "null") {
            return Err(FrameworkError::param(format!(
                "search index '{index}' has no current generation"
            )));
        }
        let state = services();
        let _lock = state.initializations.lock().await;
        if let Some(generation) = engine.current_generation(index).await? {
            return Ok(generation);
        }
        let descriptor = (Self::by_index(index)?.descriptor)();
        let mut settings = descriptor.index_settings();
        let config = SearchConfig::current()?;
        let defaults = super::settings::QuerySettings::default();
        if settings.query.hits_cap == defaults.hits_cap {
            settings.query.hits_cap = config.hits_cap;
        }
        if settings.query.fusion_depth == defaults.fusion_depth {
            settings.query.fusion_depth = config.fusion_depth;
        }
        let generation = GenerationId::default();
        engine
            .create_generation(descriptor, &settings, &generation)
            .await?;
        engine
            .switch_generation(descriptor, None, generation.clone())
            .await?;
        Ok(generation)
    }

    /// Providers register behind one trait so model code never embeds itself.
    pub fn register_embedder(
        name: impl Into<String>,
        embedder: Arc<dyn Embedder>,
    ) -> Result<(), FrameworkError> {
        let name = name.into();
        embedder.profile().validate()?;
        if name.trim().is_empty() || name == "none" {
            return Err(FrameworkError::param(
                "SEARCH_EMBEDDER requires a nonempty provider name other than none",
            ));
        }
        services()
            .embedders
            .write()
            .map_err(|_| poison())?
            .insert(name, embedder);
        Ok(())
    }

    /// Missing providers fail visibly when a query requests semantic retrieval.
    pub fn embedder_for(index: &str) -> Result<Arc<dyn Embedder>, FrameworkError> {
        let descriptor = (Self::by_index(index)?.descriptor)();
        let config = SearchConfig::current()?;
        let name = descriptor
            .index_settings()
            .index_time
            .embedder
            .unwrap_or_else(|| config.embedder.as_str().into());
        Self::embedder_named(index, &name)
    }

    /// Generation settings can resolve their pinned provider independently of code defaults.
    pub fn embedder_named(index: &str, name: &str) -> Result<Arc<dyn Embedder>, FrameworkError> {
        services()
            .embedders
            .read()
            .map_err(|_| poison())?
            .get(name)
            .cloned()
            .ok_or_else(|| {
                FrameworkError::param(format!(
                    "SEARCH_EMBEDDER '{name}' for index '{index}' is not registered"
                ))
            })
    }

    /// Cross-encoder providers can be installed without making the core depend on ONNX.
    pub fn register_reranker(
        name: impl Into<String>,
        reranker: Arc<dyn Reranker>,
    ) -> Result<(), FrameworkError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(FrameworkError::param("search reranker name is empty"));
        }
        services()
            .rerankers
            .write()
            .map_err(|_| poison())?
            .insert(name, reranker);
        Ok(())
    }

    /// Builtin lazy providers fill gaps while preserving application bootstrap registrations.
    #[cfg(feature = "search-fastembed")]
    pub(in crate::search) fn register_reranker_default(
        name: &str,
        reranker: Arc<dyn Reranker>,
    ) -> Result<(), FrameworkError> {
        services()
            .rerankers
            .write()
            .map_err(|_| poison())?
            .entry(name.into())
            .or_insert(reranker);
        Ok(())
    }

    /// An unregistered reranker is an error rather than a silently ignored request.
    pub fn reranker(name: &str) -> Result<Arc<dyn Reranker>, FrameworkError> {
        services()
            .rerankers
            .read()
            .map_err(|_| poison())?
            .get(name)
            .cloned()
            .ok_or_else(|| {
                FrameworkError::param(format!("search reranker '{name}' is not registered"))
            })
    }

    /// Tests can isolate mutable services without affecting other async tests.
    #[cfg(feature = "testing")]
    pub async fn isolated<F: Future>(future: F) -> F::Output {
        ISOLATED_SERVICES
            .scope(
                Arc::new(Services::default()),
                super::indexing::isolated_versions(future),
            )
            .await
    }
}
