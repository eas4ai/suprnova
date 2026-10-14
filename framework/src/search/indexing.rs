//! After-commit jobs reload authoritative rows so retries do not replay stale projections.

use std::{
    any::TypeId,
    collections::HashMap,
    future::Future,
    sync::{LazyLock, Mutex},
};

use async_trait::async_trait;
use sea_orm::{EntityTrait, IntoActiveModel, PrimaryKeyTrait};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    SearchConfig, SearchDocument, SearchEmbedder, SearchRegistry, Searchable,
    embedder::{EmbedInput, validate_embedding},
    engine::{GenerationId, IndexedDocument},
};
use crate::{
    DB, FrameworkError, Queue, Transaction,
    queue::{EnvelopeOverrides, Job},
};

tokio::task_local! { static SYNC_DISABLED: bool; }

type VersionKey = (String, String);
#[derive(Default)]
struct VersionState {
    events: HashMap<VersionKey, i64>,
}
type VersionCache = std::sync::Arc<Mutex<VersionState>>;
static VERSIONS: LazyLock<VersionCache> =
    LazyLock::new(|| std::sync::Arc::new(Mutex::new(VersionState::default())));
#[cfg(feature = "testing")]
tokio::task_local! { static ISOLATED_VERSIONS: VersionCache; }

fn versions() -> VersionCache {
    #[cfg(feature = "testing")]
    if let Ok(cache) = ISOLATED_VERSIONS.try_with(std::sync::Arc::clone) {
        return cache;
    }
    std::sync::Arc::clone(&VERSIONS)
}

#[cfg(feature = "testing")]
pub(crate) async fn isolated_versions<F: Future>(future: F) -> F::Output {
    ISOLATED_VERSIONS
        .scope(
            std::sync::Arc::new(Mutex::new(VersionState::default())),
            future,
        )
        .await
}

/// Model keys use their scalar representation, preserving strings without JSON quotes.
#[doc(hidden)]
pub fn document_key(value: &serde_json::Value) -> Result<String, FrameworkError> {
    match value {
        serde_json::Value::String(key) if !key.is_empty() => Ok(key.clone()),
        serde_json::Value::Number(key) if key.is_i64() || key.is_u64() => Ok(key.to_string()),
        _ => Err(FrameworkError::param(
            "search document key: expected a nonempty string or integer",
        )),
    }
}

/// Preserve textual keys while validating and binding numeric keys as their declared Rust type.
#[doc(hidden)]
pub fn model_key_value<K: serde::de::DeserializeOwned>(
    key: &str,
    column: &str,
) -> Result<serde_json::Value, FrameworkError> {
    let string = serde_json::Value::String(key.to_owned());
    if serde_json::from_value::<K>(string.clone()).is_ok() {
        return Ok(string);
    }
    let value: serde_json::Value = serde_json::from_str(key).map_err(|_| {
        FrameworkError::param(format!("search key `{column}`: invalid value `{key}`"))
    })?;
    if !value
        .as_number()
        .is_some_and(|number| number.is_i64() || number.is_u64())
    {
        return Err(FrameworkError::param(format!(
            "search key `{column}`: expected a string or integer"
        )));
    }
    serde_json::from_value::<K>(value.clone()).map_err(|_| {
        FrameworkError::param(format!(
            "search key `{column}`: value `{key}` does not match the model key type"
        ))
    })?;
    Ok(value)
}

/// Timestamped projections retain their storage version; timestamp-free projections use zero.
/// The write boundary assigns a version once and serializes it in the job. Reloading a row
/// must never mint a newer version that could pass a subsequent removal's tombstone.
#[doc(hidden)]
pub fn source_version(
    document: &SearchDocument,
    updated_at: Option<serde_json::Value>,
) -> Result<i64, FrameworkError> {
    match updated_at.filter(|value| !value.is_null()) {
        Some(value) => timestamp_version(&document.index, &value),
        None => Ok(0),
    }
}

fn timestamp_version(index: &str, value: &serde_json::Value) -> Result<i64, FrameworkError> {
    let parsed = value
        .as_str()
        .and_then(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|time| time.timestamp_micros())
                .ok()
                .or_else(|| {
                    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
                        .map(|time| time.and_utc().timestamp_micros())
                        .ok()
                })
                .or_else(|| {
                    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                        .map(|time| time.and_utc().timestamp_micros())
                        .ok()
                })
        })
        .or_else(|| value.as_i64());
    parsed.ok_or_else(|| {
        FrameworkError::param(format!(
            "search index `{index}` updated_at: expected an ISO datetime"
        ))
    })
}

// An explicit write/import/removal is newer than all previously enqueued operations
// for this key even when its searchable attributes and source timestamp did not change.
fn event_version(index: &str, key: &str, document_version: i64) -> Result<i64, FrameworkError> {
    let cache = versions();
    let mut state = cache
        .lock()
        .map_err(|_| FrameworkError::internal("search source-version cache lock poisoned"))?;
    let identity = (index.to_owned(), key.to_owned());
    let previous = state
        .events
        .get(&identity)
        .copied()
        .unwrap_or(document_version)
        .max(document_version);
    let next = previous.checked_add(1).ok_or_else(|| {
        FrameworkError::param(format!(
            "search index `{index}` key `{key}` source version overflow"
        ))
    })?;
    let next = next.max(crate::clock::now().timestamp_micros());
    state.events.insert(identity, next);
    Ok(next)
}

/// Write kinds keep deletion policy separate from the model's persistence implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteKind {
    /// New rows become eligible for their declared index.
    Created,
    /// Changed rows replace their current projection.
    Updated,
    /// Restoration clears deletion state in the indexed projection.
    Restored,
    /// Soft deletion can retain a flagged projection when explicitly configured.
    Deleted,
    /// A permanent deletion always produces a removal tombstone.
    ForceDeleted,
}

/// One job shape lets workers dispatch every searchable model by inventory name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexOperation {
    /// Reload and index the current model, removing it if it is no longer eligible.
    Index,
    /// Remove the key with an authoritative source-version tombstone.
    Remove,
}

/// Serialized jobs carry identity and version rather than stale model attributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndexJob {
    /// Inventory name resolves the model loader and engine at execution time.
    pub index: String,
    /// The canonical model key is suitable for a bound Eloquent lookup.
    pub key: String,
    /// Versions prevent a delayed write from resurrecting a removed row.
    pub source_version: i64,
    /// Removal and indexing share the same worker registration.
    pub op: IndexOperation,
    /// An explicit generation supports isolated imports without switching readers.
    pub generation: Option<GenerationId>,
}

#[async_trait]
impl Job for SearchIndexJob {
    fn job_name() -> &'static str {
        "Suprnova::SearchIndex"
    }

    fn after_commit() -> bool {
        true
    }

    fn message_group(&self) -> Option<String> {
        Some(hex::encode(Sha256::digest(
            format!("{}:{}", self.index, self.key).as_bytes(),
        )))
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let entry = SearchRegistry::by_index(&self.index)?;
        let documents = if self.op == IndexOperation::Index {
            (entry.load_document)(&self.key)
                .await?
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        write_documents(
            &self.index,
            self.generation,
            &[(self.key, self.source_version)],
            documents,
        )
        .await
    }
}

impl SearchIndexJob {
    /// Both queue and inline execution wait for the enclosing transaction's actual commit.
    pub async fn enqueue(self, tx: Option<&Transaction>) -> Result<(), FrameworkError> {
        enqueue_job(self, tx).await
    }
}

// Bulk imports keep relation loading, embedding and engine writes at chunk granularity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SearchIndexBatchJob {
    index: String,
    keys: Vec<(String, i64)>,
    op: IndexOperation,
    generation: Option<GenerationId>,
}

#[async_trait]
impl Job for SearchIndexBatchJob {
    fn job_name() -> &'static str {
        "Suprnova::SearchIndexBatch"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        let entry = SearchRegistry::by_index(&self.index)?;
        let documents = if self.op == IndexOperation::Index {
            (entry.load_documents)(self.keys.iter().map(|(key, _)| key.clone()).collect()).await?
        } else {
            Vec::new()
        };
        write_documents(&self.index, self.generation, &self.keys, documents).await
    }
}

// Index-wide removal walks engine documents, including keys whose model row is gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SearchFlushJob {
    index: String,
}

#[async_trait]
impl Job for SearchFlushJob {
    fn job_name() -> &'static str {
        "Suprnova::SearchFlush"
    }
    fn after_commit() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        let descriptor = (SearchRegistry::by_index(&self.index)?.descriptor)();
        let engine = SearchRegistry::engine_for(&self.index)?;
        let generation = SearchRegistry::generation_for(&self.index).await?;
        let query = super::query::EngineQuery {
            expanded: super::expansion::ExpandedQuery::default(),
            filter: None,
            sort: Vec::new(),
            depth: SearchConfig::current()?.chunk,
            highlights: false,
        };
        let total = engine.count(descriptor, &generation, &query).await?;
        let super::results::Total::Exact(total) = total else {
            return Err(FrameworkError::param(format!(
                "search index `{}` removal requires an exact lexical count",
                self.index
            )));
        };
        let mut removed = 0u64;
        while removed < total {
            let page = engine.query(descriptor, &generation, &query).await?;
            if page.candidates.is_empty() {
                if engine.count(descriptor, &generation, &query).await?.value() == 0 {
                    return Ok(());
                }
                return Err(FrameworkError::param(format!(
                    "search index `{}` reported remaining documents but returned no removal candidates",
                    self.index
                )));
            }
            let keys = page
                .candidates
                .into_iter()
                .map(|candidate| {
                    let document = candidate.document;
                    Ok((
                        document.key.clone(),
                        event_version(&self.index, &document.key, document.source_version)?,
                    ))
                })
                .collect::<Result<Vec<_>, FrameworkError>>()?;
            let outcome = engine.remove(descriptor, &generation, &keys).await?;
            if outcome.written == 0 {
                return Err(FrameworkError::param(format!(
                    "search index `{}` removal made no progress because writes raced with the sweep",
                    self.index
                )));
            }
            removed = removed.saturating_add(outcome.written as u64);
        }
        if engine.count(descriptor, &generation, &query).await?.value() != 0 {
            return Err(FrameworkError::param(format!(
                "search index `{}` received concurrent writes during removal",
                self.index
            )));
        }
        Ok(())
    }
}

async fn enqueue_job<J: Job>(job: J, tx: Option<&Transaction>) -> Result<(), FrameworkError> {
    let config = SearchConfig::current()?;
    let context = super::registry::capture_context(config.clone());
    let cache = versions();
    let callback = move || async move {
        with_versions(
            cache,
            super::registry::in_context(context, async move {
                if config.queue {
                    Queue::push_with(
                        job,
                        EnvelopeOverrides {
                            queue: config.queue_name,
                            connection: config.queue_connection,
                            // This callback is already past the transaction's commit boundary.
                            after_commit: Some(false),
                            ..Default::default()
                        },
                    )
                    .await
                } else {
                    job.handle().await
                }
            }),
        )
        .await
    };
    if let Some(tx) = tx {
        tx.after_commit(callback);
        Ok(())
    } else {
        DB::after_commit(callback).await
    }
}

async fn with_versions<F: Future>(cache: VersionCache, future: F) -> F::Output {
    #[cfg(feature = "testing")]
    {
        ISOLATED_VERSIONS.scope(cache, future).await
    }
    #[cfg(not(feature = "testing"))]
    {
        let _ = cache;
        future.await
    }
}

fn apply_embed_source(
    document: &mut SearchDocument,
    source: Option<&super::settings::EmbedSource>,
) -> Result<(), FrameworkError> {
    let Some(source) = source else {
        document.embed_text = None;
        document.title = None;
        return Ok(());
    };
    let mut parts = Vec::new();
    for name in &source.attributes {
        match document.attributes.get(name) {
            Some(super::AttributeValue::Text(text)) => parts.push(text.clone()),
            Some(super::AttributeValue::List(values)) => {
                let texts = values
                    .iter()
                    .map(|value| {
                        value.as_text().ok_or_else(|| {
                            FrameworkError::param(format!(
                                "search embed attribute `{name}` requires a text list"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                parts.push(texts.join(" "));
            }
            Some(super::AttributeValue::Null) => {}
            _ => {
                return Err(FrameworkError::param(format!(
                    "search embed attribute `{name}` requires text"
                )));
            }
        }
    }
    document.embed_text = (!parts.is_empty()).then(|| parts.join("\n"));
    document.title = match source
        .title
        .as_deref()
        .and_then(|name| document.attributes.get(name))
    {
        Some(super::AttributeValue::Text(title)) => Some(title.clone()),
        None | Some(super::AttributeValue::Null) => None,
        _ => {
            return Err(FrameworkError::param(format!(
                "search embed title attribute `{}` requires text",
                source.title.as_deref().unwrap_or("")
            )));
        }
    };
    Ok(())
}

async fn write_documents(
    index: &str,
    generation: Option<GenerationId>,
    keys: &[(String, i64)],
    documents: Vec<SearchDocument>,
) -> Result<(), FrameworkError> {
    let descriptor = (SearchRegistry::by_index(index)?.descriptor)();
    let engine = SearchRegistry::engine_for(index)?;
    let generation = match generation {
        Some(generation) => generation,
        None => SearchRegistry::generation_for(index).await?,
    };
    let settings = engine.read_settings(descriptor, &generation).await?;
    let mut remaining: HashMap<_, _> = keys
        .iter()
        .map(|(key, version)| (key.as_str(), *version))
        .collect();
    let digest = settings.index_time.digest();
    let mut indexed = Vec::with_capacity(documents.len());
    for mut document in documents {
        let version = remaining.remove(document.key.as_str()).ok_or_else(|| {
            FrameworkError::param(format!(
                "search index `{index}` loader returned an unrequested or duplicate key `{}`",
                document.key
            ))
        })?;
        document.source_version = version;
        apply_embed_source(&mut document, settings.index_time.embed_source.as_ref())?;
        document.validate(descriptor)?;
        let content_hash = document.content_hash(&digest);
        indexed.push(IndexedDocument {
            document,
            content_hash,
            dense: None,
            sparse: None,
        });
    }
    if !indexed.is_empty() {
        let hashes = engine
            .stored_hashes(descriptor, &generation, &indexed)
            .await?;
        indexed
            .retain(|document| hashes.get(&document.document.key) != Some(&document.content_hash));
    }
    let embedding_enabled = match settings.index_time.embedder.as_deref() {
        Some("none") => false,
        Some(_) => true,
        None => SearchConfig::current()?.embedder != SearchEmbedder::None,
    };
    if embedding_enabled && !indexed.is_empty() {
        let embedder = match settings.index_time.embedder.as_deref() {
            Some(name) => SearchRegistry::embedder_named(index, name)?,
            None => SearchRegistry::embedder_for(index)?,
        };
        let inputs = indexed
            .iter()
            .map(|indexed| {
                let document = &indexed.document;
                Ok(EmbedInput {
                    text: document.embed_text.clone().ok_or_else(|| {
                        FrameworkError::param(format!(
                            "search index `{index}` embed source is missing"
                        ))
                    })?,
                    title: document.title.clone(),
                    media: Vec::new(),
                    document_template: (!settings.index_time.embed_template.is_empty())
                        .then(|| settings.index_time.embed_template.clone()),
                })
            })
            .collect::<Result<Vec<_>, FrameworkError>>()?;
        let embeddings = embedder.embed_documents(&inputs).await?;
        if embeddings.len() != indexed.len() {
            return Err(FrameworkError::param(format!(
                "search index `{index}` embedder returned {} vectors for {} documents",
                embeddings.len(),
                indexed.len()
            )));
        }
        for (indexed, embedding) in indexed.iter_mut().zip(embeddings) {
            indexed.dense = Some(
                validate_embedding(
                    embedding.dense,
                    embedder.profile(),
                    settings.index_time.dimensions,
                )?
                .dense,
            );
        }
    }
    let removals: Vec<_> = remaining
        .into_iter()
        .map(|(key, version)| (key.to_owned(), version))
        .collect();
    if !removals.is_empty() {
        engine.remove(descriptor, &generation, &removals).await?;
    }
    if !indexed.is_empty() {
        engine.index(descriptor, &generation, &indexed).await?;
    }
    Ok(())
}

async fn enqueue_models<M>(models: Vec<M>, op: IndexOperation) -> Result<(), FrameworkError>
where
    M: Searchable + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    if SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
        || models.is_empty()
    {
        return Ok(());
    }
    let mut keys = Vec::with_capacity(models.len());
    for model in models {
        let key = document_key(&model.primary_key_value_json())?;
        let model_version = if M::HAS_TIMESTAMPS {
            match model
                .field_value(M::UPDATED_AT_COLUMN)
                .filter(|value| !value.is_null())
            {
                Some(value) => timestamp_version(M::descriptor().name, &value)?,
                None => 0,
            }
        } else {
            0
        };
        keys.push((
            key.clone(),
            event_version(M::descriptor().name, &key, model_version)?,
        ));
    }
    if !keys.is_empty() {
        enqueue_job(
            SearchIndexBatchJob {
                index: M::descriptor().name.into(),
                keys,
                op,
                generation: None,
            },
            None,
        )
        .await?;
    }
    Ok(())
}

/// The persistence bridge accepts all model types, making nonsearchable writes a cheap no-op.
pub async fn after_model_write<M: 'static>(
    model: &M,
    kind: WriteKind,
    tx: Option<&Transaction>,
) -> Result<(), FrameworkError> {
    if SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let Some(entry) = SearchRegistry::by_type(TypeId::of::<M>())? else {
        return Ok(());
    };
    let document = (entry.document)(model)?;
    let descriptor = (entry.descriptor)();
    let deleted =
        kind == WriteKind::ForceDeleted || (kind == WriteKind::Deleted && !descriptor.soft_delete);
    let op = if deleted || !(entry.should_index)(model)? {
        IndexOperation::Remove
    } else {
        IndexOperation::Index
    };
    let source_version = event_version(&document.index, &document.key, document.source_version)?;
    SearchIndexJob {
        index: document.index,
        key: document.key,
        source_version,
        op,
        generation: None,
    }
    .enqueue(tx)
    .await
}

/// Suppression follows the current async task and unwinds automatically when the closure ends.
pub async fn without_syncing_to_search<F, Fut, T>(callback: F) -> Result<T, FrameworkError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, FrameworkError>>,
{
    SYNC_DISABLED.scope(true, callback()).await
}

/// Imports use bounded Eloquent chunks and let the model load relations once per chunk.
pub async fn make_all_searchable<M>() -> Result<(), FrameworkError>
where
    M: Searchable + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    let chunk = SearchConfig::current()?.chunk;
    let mut query = M::query();
    if M::descriptor().soft_delete {
        query = query.__disable_named_scope("soft_deletes");
    }
    query
        .chunk_by_id(chunk as u64, |models| async move {
            enqueue_models::<M>(models.into_vec(), IndexOperation::Index).await
        })
        .await
}

/// Removal walks the engine in bounded chunks so orphaned and soft-deleted keys are cleared too.
pub async fn remove_all_from_search<M>() -> Result<(), FrameworkError>
where
    M: Searchable + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    if SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
    {
        return Ok(());
    }
    enqueue_job(
        SearchFlushJob {
            index: M::descriptor().name.into(),
        },
        None,
    )
    .await
}

/// A bound key-list import applies the eager hook before any chunk's projections are built.
pub async fn searchable<M>(keys: &[String]) -> Result<(), FrameworkError>
where
    M: Searchable + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    let chunk = SearchConfig::current()?.chunk;
    for keys in keys.chunks(chunk) {
        let mut query = M::query();
        if M::descriptor().soft_delete {
            query = query.__disable_named_scope("soft_deletes");
        }
        let keys = keys
            .iter()
            .map(|key| model_key_value::<M::Key>(key, M::primary_key_name()))
            .collect::<Result<Vec<_>, _>>()?;
        let models = query.where_in(M::primary_key_name(), keys).get().await?;
        enqueue_models::<M>(models.into_vec(), IndexOperation::Index).await?;
    }
    Ok(())
}

/// Key-list removal works even when the source rows have already disappeared.
pub async fn unsearchable<M>(keys: &[String]) -> Result<(), FrameworkError>
where
    M: Searchable + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    if SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let chunk = SearchConfig::current()?.chunk;
    for keys in keys.chunks(chunk) {
        let keys = keys
            .iter()
            .map(|key| Ok((key.clone(), event_version(M::descriptor().name, key, 0)?)))
            .collect::<Result<Vec<_>, FrameworkError>>()?;
        enqueue_job(
            SearchIndexBatchJob {
                index: M::descriptor().name.into(),
                keys,
                op: IndexOperation::Remove,
                generation: None,
            },
            None,
        )
        .await?;
    }
    Ok(())
}
