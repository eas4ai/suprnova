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

pub(crate) fn syncing_disabled() -> bool {
    SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
}

/// Search intents require a model transaction even when render caching is disabled.
#[doc(hidden)]
pub async fn atomic<M: 'static, T, F, Fut>(
    connection: Option<&str>,
    write: F,
) -> Result<T, FrameworkError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, FrameworkError>>,
    T: Send,
{
    let mut dependent = SearchRegistry::by_type(TypeId::of::<M>())?.is_some();
    for relation in crate::eloquent::relations::relations().filter(|relation| {
        (relation.target_type)() == TypeId::of::<M>()
            || relation
                .morph_targets
                .iter()
                .any(|target| (target.type_id)() == TypeId::of::<M>())
    }) {
        if let Some(entry) = SearchRegistry::by_type((relation.parent_type)())? {
            dependent |= (entry.descriptor)().search_touches.contains(&relation.name);
        }
    }
    if !dependent || syncing_disabled() || crate::database::after_commit::in_transaction() {
        return crate::render_cache::orm::atomic(connection, write).await;
    }
    crate::render_cache::orm::atomic(connection, || async {
        // The cache wrapper probes and, when needed, opens its transaction first.
        // Only a closed cache write side needs the independent search transaction.
        if crate::database::after_commit::in_transaction() {
            return write().await;
        }
        let run = || DB::transaction_ambient(write);
        match connection {
            Some(name) => DB::with_default_connection(name, run()).await,
            None => run().await,
        }
    })
    .await
}

/// Generated batch loaders share the source connection restored by deferred dispatch.
#[doc(hidden)]
pub fn job_connection() -> Option<String> {
    super::registry::job_connection()
}

type VersionKey = (String, String);
#[derive(Default)]
struct VersionState {
    events: HashMap<VersionKey, i64>,
    indexes: HashMap<String, i64>,
    kept: KeptVersions,
}

/// Rows a generation of kept versions holds before the older generation is dropped.
const KEPT_GENERATION: usize = 65_536;

/// The versions this process last saved or loaded for rows of models without
/// `updated_at`. A projection is synchronous and cannot query, so it reads them here.
/// Two generations bound the memory: when the newer one fills, the older is dropped,
/// and a row neither saved nor loaded since then reads 0 again.
#[derive(Default)]
struct KeptVersions {
    current: HashMap<VersionKey, i64>,
    previous: HashMap<VersionKey, i64>,
}

impl KeptVersions {
    fn get(&mut self, key: &VersionKey) -> Option<i64> {
        if let Some(version) = self.current.get(key) {
            return Some(*version);
        }
        let version = self.previous.remove(key)?;
        self.insert(key.clone(), version);
        Some(version)
    }

    /// Versions only rise, so an older value read late never replaces a newer one.
    fn insert(&mut self, key: VersionKey, version: i64) {
        let known = self
            .current
            .get(&key)
            .or_else(|| self.previous.get(&key))
            .copied()
            .unwrap_or(0);
        if self.current.len() >= KEPT_GENERATION && !self.current.contains_key(&key) {
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.insert(key, version.max(known));
    }
}

/// The kept version a projection reads for a row of a model without `updated_at`: the
/// value this process last saved or loaded for the row, 0 when it has done neither.
/// Eloquent loads these values in one query beside the rows themselves.
#[doc(hidden)]
pub fn kept_version(index: &str, key: &str) -> i64 {
    versions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .kept
        .get(&(index.to_owned(), key.to_owned()))
        .unwrap_or(0)
}

fn remember_kept(cache: &VersionCache, index: &str, versions: &[(String, i64)]) {
    let mut state = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (key, version) in versions {
        state.kept.insert((index.to_owned(), key.clone()), *version);
    }
}

/// Rows bound per kept-version statement: three binds each stay below SQLite's limit.
const KEPT_ROWS_PER_STATEMENT: usize = 200;

/// Raise the versions kept for rows of a model without `updated_at`, on the save's own
/// executor so they commit or roll back with it, and return the value now kept for each.
/// A kept version rises past both its stored value and the version this process minted,
/// so every save of a row stores a newer version, whichever process makes it.
async fn keep_versions(
    index: &str,
    minted: Vec<(String, i64)>,
    tx: Option<&Transaction>,
) -> Result<Vec<(String, i64)>, FrameworkError> {
    let exec = match tx {
        Some(tx) => crate::database::transaction::ExecutorChoice::from_tx(tx),
        None => {
            crate::database::transaction::ExecutorChoice::resolve_write(
                None,
                super::registry::job_connection().as_deref(),
                (SearchRegistry::by_index(index)?.connection)(),
            )
            .await?
        }
    };
    let failure = |error: sea_orm::DbErr| {
        FrameworkError::database(format!("search index `{index}` kept versions: {error}"))
    };
    if minted.is_empty()
        || !super::outbox::persistent_table(index, &exec, "search_source_versions").await?
    {
        return Ok(minted);
    }
    let backend = exec.backend();
    let mut kept = HashMap::with_capacity(minted.len());
    for chunk in minted.chunks(KEPT_ROWS_PER_STATEMENT) {
        let rows = vec!["(?, ?, ?)"; chunk.len()].join(", ");
        let values = chunk
            .iter()
            .flat_map(|(key, version)| {
                [
                    sea_orm::Value::from(index.to_owned()),
                    sea_orm::Value::from(key.clone()),
                    sea_orm::Value::from(*version),
                ]
            })
            .collect::<Vec<_>>();
        let results = if backend == sea_orm::DbBackend::MySql {
            exec.run(super::outbox::statement(
                backend,
                &format!("INSERT INTO search_source_versions (index_name, model_key, version) VALUES {rows} ON DUPLICATE KEY UPDATE version = IF(VALUES(version) > version, VALUES(version), version + 1)"),
                values,
            )?)
            .await
            .map_err(failure)?;
            let keys = vec!["?"; chunk.len()].join(", ");
            let mut values = vec![sea_orm::Value::from(index.to_owned())];
            values.extend(
                chunk
                    .iter()
                    .map(|(key, _)| sea_orm::Value::from(key.clone())),
            );
            exec.query_all(super::outbox::statement(
                backend,
                &format!("SELECT model_key, version FROM search_source_versions WHERE index_name = ? AND model_key IN ({keys})"),
                values,
            )?)
            .await
            .map_err(failure)?
        } else {
            exec.query_all(super::outbox::statement(
                backend,
                &format!("INSERT INTO search_source_versions (index_name, model_key, version) VALUES {rows} ON CONFLICT (index_name, model_key) DO UPDATE SET version = CASE WHEN excluded.version > search_source_versions.version THEN excluded.version ELSE search_source_versions.version + 1 END RETURNING model_key, version"),
                values,
            )?)
            .await
            .map_err(failure)?
        };
        for row in results {
            kept.insert(
                row.try_get::<String>("", "model_key").map_err(failure)?,
                row.try_get::<i64>("", "version").map_err(failure)?,
            );
        }
    }
    let kept = minted
        .into_iter()
        .map(|(key, _)| {
            let version = kept.remove(&key).ok_or_else(|| {
                FrameworkError::database(format!(
                    "search index `{index}` kept no version for key `{key}`"
                ))
            })?;
            Ok((key, version))
        })
        .collect::<Result<Vec<_>, FrameworkError>>()?;
    // Later mints in this process start above the kept value at once; the projection
    // sees it only after the save commits, so a rolled-back save leaves no trace there.
    let cache = versions();
    {
        let mut state = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (key, version) in &kept {
            let slot = state
                .events
                .entry((index.to_owned(), key.clone()))
                .or_insert(0);
            *slot = (*slot).max(*version);
        }
    }
    let committed = kept.clone();
    let index_name = index.to_owned();
    let remember = move || async move {
        remember_kept(&cache, &index_name, &committed);
        Ok(())
    };
    match tx {
        Some(tx) => tx.after_commit(remember),
        None => DB::after_commit(remember).await?,
    }
    Ok(kept)
}

/// Load the kept versions of freshly read rows in one query per batch, so a projection
/// of any row Eloquent returns reads the version its database holds. Runs only for a
/// searchable model without `updated_at`.
#[doc(hidden)]
pub async fn prefetch_kept_versions<M>(
    rows: &[M],
    exec: &crate::database::transaction::ExecutorChoice,
) -> Result<(), FrameworkError>
where
    M: crate::eloquent::Model + From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    if M::HAS_TIMESTAMPS || rows.is_empty() {
        return Ok(());
    }
    let Some(entry) = SearchRegistry::by_type(TypeId::of::<M>())? else {
        return Ok(());
    };
    if !super::outbox::persistent_table(entry.index, exec, "search_source_versions").await? {
        return Ok(());
    }
    let failure = |error: sea_orm::DbErr| {
        FrameworkError::database(format!(
            "search index `{}` kept versions: {error}",
            entry.index
        ))
    };
    let keys = rows
        .iter()
        .map(|row| document_key(&row.primary_key_value_json()))
        .collect::<Result<Vec<_>, _>>()?;
    let backend = exec.backend();
    let mut loaded = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(KEPT_ROWS_PER_STATEMENT) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut values = vec![sea_orm::Value::from(entry.index.to_owned())];
        values.extend(chunk.iter().map(|key| sea_orm::Value::from(key.clone())));
        for row in exec
            .query_all(super::outbox::statement(
                backend,
                &format!("SELECT model_key, version FROM search_source_versions WHERE index_name = ? AND model_key IN ({placeholders})"),
                values,
            )?)
            .await
            .map_err(failure)?
        {
            loaded.push((
                row.try_get::<String>("", "model_key").map_err(failure)?,
                row.try_get::<i64>("", "version").map_err(failure)?,
            ));
        }
    }
    remember_kept(&versions(), entry.index, &loaded);
    Ok(())
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
pub(crate) fn isolated_versions<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let future = Box::pin(future);
    async move {
        ISOLATED_VERSIONS
            .scope(
                std::sync::Arc::new(Mutex::new(VersionState::default())),
                future,
            )
            .await
    }
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

/// Timestamped projections retain their storage version; a timestamp-free projection reads
/// the version the framework keeps for its row (see [`kept_version`]). Reading never mints:
/// a version minted when a row is read could pass a subsequent removal's tombstone.
#[doc(hidden)]
pub fn source_version(
    document: &SearchDocument,
    updated_at: Option<serde_json::Value>,
) -> Result<i64, FrameworkError> {
    match updated_at.filter(|value| !value.is_null()) {
        Some(value) => timestamp_version(&document.index, &value),
        None => Ok(kept_version(&document.index, &document.key)),
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
// in this index even when source timestamps do not change. Fixed flush cutoffs
// must precede every later key's write, including writes under a frozen clock.
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
        .max(document_version)
        .max(
            state
                .indexes
                .get(index)
                .copied()
                .unwrap_or(document_version),
        );
    let next = previous.checked_add(1).ok_or_else(|| {
        FrameworkError::param(format!(
            "search index `{index}` key `{key}` source version overflow"
        ))
    })?;
    let next = next.max(crate::clock::now().timestamp_micros());
    state.events.insert(identity, next);
    state.indexes.insert(index.to_owned(), next);
    Ok(next)
}

fn flush_version(index: &str) -> Result<i64, FrameworkError> {
    event_version(index, "__flush__", 0)
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
        let index = self.index.clone();
        super::registry::in_model_connection(&index, async move {
            if super::generations::already_completed(
                &self.index,
                self.generation.as_ref(),
                &[(self.key.clone(), self.source_version)],
            )
            .await?
            {
                return Ok(());
            }
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
        })
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
        let index = self.index.clone();
        super::registry::in_model_connection(&index, async move {
            if super::generations::already_completed(
                &self.index,
                self.generation.as_ref(),
                &self.keys,
            )
            .await?
            {
                return Ok(());
            }
            let entry = SearchRegistry::by_index(&self.index)?;
            let documents = if self.op == IndexOperation::Index {
                (entry.load_documents)(self.keys.iter().map(|(key, _)| key.clone()).collect())
                    .await?
            } else {
                Vec::new()
            };
            write_documents(&self.index, self.generation, &self.keys, documents).await
        })
        .await
    }
}

// Index-wide removal walks engine documents, including keys whose model row is gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SearchFlushJob {
    index: String,
    generation: Option<GenerationId>,
    #[serde(default)]
    source_version: i64,
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
        let index = self.index.clone();
        super::registry::in_model_connection(&index, async move {
            if super::generations::already_completed(
                &self.index,
                self.generation.as_ref(),
                &[("__flush__".into(), self.source_version)],
            )
            .await?
            {
                return Ok(());
            }
            let descriptor = (SearchRegistry::by_index(&self.index)?.descriptor)();
            let engine = SearchRegistry::engine_for(&self.index)?;
            let generation = match self.generation {
                Some(generation) => generation,
                None => SearchRegistry::generation_for(&self.index).await?,
            };
            engine
                .record_flush(descriptor, &generation, self.source_version)
                .await?;
            loop {
                let keys = engine
                    .keys_before_version(
                        descriptor,
                        &generation,
                        self.source_version,
                        SearchConfig::current()?.chunk,
                    )
                    .await?;
                if keys.is_empty() {
                    break;
                }
                let keys: Vec<_> = keys
                    .into_iter()
                    .map(|(key, _)| (key, self.source_version))
                    .collect();
                let outcome = engine.remove(descriptor, &generation, &keys).await?;
                if outcome.written == 0
                    && outcome.skipped_older == 0
                    && outcome.skipped_tombstoned == 0
                {
                    return Err(FrameworkError::param(format!(
                        "search index `{}` removal made no progress",
                        self.index
                    )));
                }
            }
            super::generations::complete(
                &self.index,
                &generation,
                &[("__flush__".into(), self.source_version)],
            )
            .await?;
            Ok(())
        })
        .await
    }
}

pub(crate) trait DeferredSearchJob: Job + Clone {
    fn index(&self) -> &str;
    fn generation(&self) -> Option<GenerationId>;
    fn pin(&mut self, generation: GenerationId);
    fn keys(&self) -> Vec<(String, i64)>;
}
impl DeferredSearchJob for SearchIndexJob {
    fn index(&self) -> &str {
        &self.index
    }
    fn generation(&self) -> Option<GenerationId> {
        self.generation.clone()
    }
    fn pin(&mut self, generation: GenerationId) {
        self.generation = Some(generation);
    }
    fn keys(&self) -> Vec<(String, i64)> {
        vec![(self.key.clone(), self.source_version)]
    }
}
impl DeferredSearchJob for SearchIndexBatchJob {
    fn index(&self) -> &str {
        &self.index
    }
    fn generation(&self) -> Option<GenerationId> {
        self.generation.clone()
    }
    fn pin(&mut self, generation: GenerationId) {
        self.generation = Some(generation);
    }
    fn keys(&self) -> Vec<(String, i64)> {
        self.keys.clone()
    }
}
impl DeferredSearchJob for SearchFlushJob {
    fn index(&self) -> &str {
        &self.index
    }
    fn generation(&self) -> Option<GenerationId> {
        self.generation.clone()
    }
    fn pin(&mut self, generation: GenerationId) {
        self.generation = Some(generation);
    }
    fn keys(&self) -> Vec<(String, i64)> {
        vec![("__flush__".into(), self.source_version)]
    }
}

pub(crate) fn enqueue_job<J: DeferredSearchJob>(
    job: J,
    tx: Option<&Transaction>,
) -> impl Future<Output = Result<(), FrameworkError>> {
    // Keep the durable dispatch future behind one pointer before ORM transaction
    // and model-hook scopes retain it in their own async state machines.
    Box::pin(async move {
        let config = SearchConfig::current()?;
        let connection_override = super::registry::job_connection();
        let declared = (SearchRegistry::by_index(job.index())?.connection)();
        let connection = match tx {
            Some(tx) => tx.connection_name.to_string(),
            None => crate::database::transaction::ExecutorChoice::resolve_write(
                None,
                connection_override.as_deref(),
                declared,
            )
            .await?
            .connection_name()
            .to_owned(),
        };
        let mut payload = serde_json::to_value(&job).map_err(|failure| {
            FrameworkError::param(format!("search job serialization: {failure}"))
        })?;
        payload
            .as_object_mut()
            .ok_or_else(|| FrameworkError::param("search job payload must be an object"))?
            .insert(
                "_search_connection".into(),
                serde_json::Value::String(connection.clone()),
            );
        let queue_context = crate::context::Context::dehydrate();
        let intent = super::registry::in_connection(
            connection.clone(),
            super::outbox::stage_job(job.index(), J::job_name(), &payload, tx),
        )
        .await?;
        let context = super::registry::capture_context(config.clone());
        let cache = versions();
        let callback = move || {
            super::registry::in_connection(
                connection.clone(),
                crate::context::Context::scope(
                    crate::context::Context::hydrate(queue_context.as_ref()),
                    async move {
                        crate::context::Context::hidden_add(
                            "suprnova.search.connection",
                            &connection,
                        );
                        with_versions(
                            cache,
                            super::registry::in_context(context, async move {
                                let explicit = job.generation();
                                let mut generations = super::generations::reserve_job(
                                    job.index(),
                                    &job.keys(),
                                    explicit.as_ref(),
                                    J::job_name(),
                                    payload.clone(),
                                )
                                .await?;
                                super::outbox::reserved(job.index(), intent.as_deref()).await?;
                                // Scope functions can inspect any filterable attribute. Inline the
                                // full projection after commit so queued work cannot delay visibility.
                                let scoped = J::job_name() == SearchIndexJob::job_name()
                                    && (SearchRegistry::by_index(job.index())?.descriptor)()
                                        .scope
                                        .is_some();
                                if scoped && config.queue {
                                    for generation in &generations {
                                        let mut inline = job.clone();
                                        inline.pin(generation.clone());
                                        inline.handle().await?;
                                    }
                                    // A concurrent publication can retire the initial targets. The
                                    // synchronous write already reached its building generation.
                                    generations = super::generations::reserve_job(
                                        job.index(),
                                        &job.keys(),
                                        explicit.as_ref(),
                                        J::job_name(),
                                        payload.clone(),
                                    )
                                    .await?;
                                }
                                for generation in generations {
                                    let mut pinned = job.clone();
                                    pinned.pin(generation.clone());
                                    let keys = pinned.keys();
                                    let index = pinned.index().to_owned();
                                    if config.queue
                                        && super::generations::import_generation().is_none()
                                    {
                                        Queue::push_with(
                                            pinned,
                                            EnvelopeOverrides {
                                                queue: config.queue_name.clone(),
                                                connection: config.queue_connection.clone(),
                                                after_commit: Some(false),
                                                ..Default::default()
                                            },
                                        )
                                        .await?;
                                        super::generations::published(&index, &generation, &keys)
                                            .await?;
                                    } else {
                                        pinned.handle().await?;
                                    }
                                }
                                Ok(())
                            }),
                        )
                        .await
                    },
                ),
            )
        };
        if let Some(tx) = tx {
            tx.after_commit(callback);
            Ok(())
        } else {
            DB::after_commit(callback).await
        }
    })
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

fn sparse_source(
    document: &SearchDocument,
    attr: &super::WeightedAttribute,
) -> Result<String, FrameworkError> {
    let value = document.attributes.get(&attr.name).ok_or_else(|| {
        FrameworkError::param(format!(
            "search index {} attribute {}: missing sparse source",
            document.index, attr.name
        ))
    })?;
    match value {
        super::AttributeValue::Text(text) => Ok(text.clone()),
        super::AttributeValue::List(values) => Ok(values
            .iter()
            .filter_map(super::AttributeValue::as_text)
            .collect::<Vec<_>>()
            .join("\n")),
        super::AttributeValue::Null => Ok(String::new()),
        _ => Err(FrameworkError::param(format!(
            "search index {} attribute {}: sparse source requires text",
            document.index, attr.name
        ))),
    }
}
fn add_sparse(
    combined: &mut std::collections::BTreeMap<u32, f64>,
    vector: super::SparseEmbedding,
    weight: f32,
) -> Result<(), FrameworkError> {
    vector.validate()?;
    for (dimension, value) in vector.indices.into_iter().zip(vector.values) {
        *combined.entry(dimension).or_default() += f64::from(value) * f64::from(weight);
    }
    Ok(())
}
pub(super) async fn embed_sparse_documents(
    index: &str,
    settings: &super::IndexSettings,
    indexed: &mut [IndexedDocument],
    driver: &str,
) -> Result<(), FrameworkError> {
    let provider = settings
        .index_time
        .sparse_model
        .as_deref()
        .or_else(|| (driver == "qdrant").then_some("bm25"));
    let Some(provider) = provider else {
        return Ok(());
    };
    // The default is engine-native. Only registered sparse models need client vectors.
    if provider == "bm25" || indexed.is_empty() {
        return Ok(());
    }
    let embedder = SearchRegistry::sparse_embedder(index, settings)?;
    let mut inputs = Vec::new();
    let mut weights = Vec::new();
    for (position, doc) in indexed.iter().enumerate() {
        for attr in &settings.index_time.searchable_attributes {
            inputs.push(sparse_source(&doc.document, attr)?);
            weights.push((position, attr.weight));
        }
    }
    let vectors = embedder.embed_sparse(&inputs).await?;
    if vectors.len() != inputs.len() {
        return Err(FrameworkError::param(format!(
            "search index {index} sparse model {provider}: wrong embedding count"
        )));
    }
    let mut combined = vec![std::collections::BTreeMap::new(); indexed.len()];
    for (vector, (position, weight)) in vectors.into_iter().zip(weights) {
        add_sparse(&mut combined[position], vector, weight)?;
    }
    for (doc, values) in indexed.iter_mut().zip(combined) {
        let sparse = super::SparseEmbedding {
            indices: values.keys().copied().collect(),
            values: values.values().map(|v| *v as f32).collect(),
        };
        sparse.validate()?;
        doc.sparse = Some(sparse);
    }
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
    embed_sparse_documents(index, &settings, &mut indexed, engine.name()).await?;
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
    super::generations::complete(index, &generation, keys).await?;
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
    if !M::HAS_TIMESTAMPS {
        keys = keep_versions(M::descriptor().name, keys, None).await?;
    }
    if !keys.is_empty() {
        enqueue_job(
            SearchIndexBatchJob {
                index: M::descriptor().name.into(),
                keys,
                op,
                generation: super::generations::import_generation(),
            },
            None,
        )
        .await?;
    }
    Ok(())
}

async fn enqueue_touched_parents<M: Serialize + 'static>(
    model: &M,
    tx: Option<&Transaction>,
    visited: &mut std::collections::HashSet<(&'static str, String)>,
) -> Result<(), FrameworkError> {
    for job in touched_parent_jobs(model, tx, visited).await? {
        job.enqueue(tx).await?;
    }
    Ok(())
}

/// Capture reverse joins before a cascading deletion can erase its pivot rows.
#[doc(hidden)]
pub async fn plan_model_delete<M: Serialize + 'static>(
    model: &M,
    tx: Option<&Transaction>,
) -> Result<Vec<SearchIndexJob>, FrameworkError> {
    if SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
    {
        return Ok(Vec::new());
    }
    touched_parent_jobs(model, tx, &mut std::collections::HashSet::new()).await
}

/// Register the captured parents only after the deletion statement succeeds.
#[doc(hidden)]
pub async fn after_model_delete<M: 'static>(
    model: &M,
    kind: WriteKind,
    tx: Option<&Transaction>,
    jobs: Vec<SearchIndexJob>,
) -> Result<(), FrameworkError> {
    if SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
    {
        return Ok(());
    }
    for job in jobs {
        job.enqueue(tx).await?;
    }
    enqueue_model(model, kind, tx).await
}

async fn touched_parent_jobs<M: Serialize + 'static>(
    model: &M,
    tx: Option<&Transaction>,
    visited: &mut std::collections::HashSet<(&'static str, String)>,
) -> Result<Vec<SearchIndexJob>, FrameworkError> {
    use crate::eloquent::relations::RelationKind;
    let mut jobs = Vec::new();
    let mut fields = None;
    for relation in crate::eloquent::relations::relations().filter(|relation| {
        (relation.target_type)() == TypeId::of::<M>()
            || relation
                .morph_targets
                .iter()
                .any(|target| (target.type_id)() == TypeId::of::<M>())
    }) {
        let Some(entry) = SearchRegistry::by_type((relation.parent_type)())? else {
            continue;
        };
        let descriptor = (entry.descriptor)();
        if !descriptor.search_touches.contains(&relation.name) {
            continue;
        }
        let fields = fields
            .get_or_insert(serde_json::to_value(model).map_err(|error| {
                FrameworkError::param(format!("search touches model: {error}"))
            })?);
        let read = |column: &str| fields.get(column).filter(|value| !value.is_null()).cloned();
        let (from, predicate, value) = match relation.kind {
            RelationKind::HasOne
            | RelationKind::HasMany
            | RelationKind::MorphOne
            | RelationKind::MorphMany => {
                if !relation.morph_type_column.is_empty()
                    && read(relation.morph_type_column)
                        .as_ref()
                        .and_then(serde_json::Value::as_str)
                        != Some(relation.morph_type_value)
                {
                    continue;
                }
                (
                    descriptor.model_table.to_owned(),
                    format!("p.{}", relation.parent_key),
                    read(relation.foreign_key),
                )
            }
            RelationKind::BelongsTo => (
                descriptor.model_table.to_owned(),
                format!("p.{}", relation.foreign_key),
                read(relation.parent_key),
            ),
            RelationKind::BelongsToMany
            | RelationKind::MorphToMany
            | RelationKind::MorphedByMany
            | RelationKind::HasOneThrough
            | RelationKind::HasManyThrough => {
                let column = if matches!(
                    relation.kind,
                    RelationKind::HasOneThrough | RelationKind::HasManyThrough
                ) {
                    relation.target_join_key
                } else {
                    relation.target_primary_key
                };
                (
                    format!(
                        "{} p JOIN {} r ON p.{} = r.{}",
                        descriptor.model_table,
                        relation.pivot_table,
                        relation.parent_key,
                        relation.pivot_parent_key
                    ),
                    format!("r.{}", relation.pivot_related_key),
                    read(column),
                )
            }
            RelationKind::MorphTo => {
                let target = relation
                    .morph_targets
                    .iter()
                    .find(|target| (target.type_id)() == TypeId::of::<M>())
                    .ok_or_else(|| {
                        FrameworkError::param(format!(
                            "search touches `{}` has no matching morph target",
                            relation.name
                        ))
                    })?;
                (
                    descriptor.model_table.to_owned(),
                    format!("p.{}", relation.foreign_key),
                    read(target.primary_key),
                )
            }
        };
        let Some(value) = value else { continue };
        for identifier in [
            descriptor.model_table,
            descriptor.key_column,
            relation.parent_key,
            relation.foreign_key,
            relation.pivot_table,
            relation.pivot_parent_key,
            relation.pivot_related_key,
        ]
        .into_iter()
        .filter(|identifier| !identifier.is_empty())
        {
            crate::database::validate_identifier(identifier)?;
        }
        let exec = match tx {
            Some(tx) => crate::database::transaction::ExecutorChoice::from_tx(tx),
            None => {
                crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?
            }
        };
        let bind_column = match relation.kind {
            RelationKind::HasOne
            | RelationKind::HasMany
            | RelationKind::MorphOne
            | RelationKind::MorphMany => relation.foreign_key,
            RelationKind::BelongsTo => relation.parent_key,
            RelationKind::HasOneThrough | RelationKind::HasManyThrough => relation.target_join_key,
            RelationKind::MorphTo => relation
                .morph_targets
                .iter()
                .find(|target| (target.type_id)() == TypeId::of::<M>())
                .map(|target| target.primary_key)
                .ok_or_else(|| FrameworkError::param("search touches morph target disappeared"))?,
            RelationKind::BelongsToMany
            | RelationKind::MorphToMany
            | RelationKind::MorphedByMany => relation.target_primary_key,
        };
        let binder = if relation.kind == RelationKind::MorphTo {
            relation
                .morph_targets
                .iter()
                .find(|target| (target.type_id)() == TypeId::of::<M>())
                .map(|target| (target.query_constraints)().binder)
                .ok_or_else(|| FrameworkError::param("search touches morph binder disappeared"))?
        } else {
            relation.related_bind_column
        };
        let bound_key = binder(bind_column, &value)
            .unwrap_or_else(|| crate::eloquent::model::json_value_to_sea_value(&value));
        let key_placeholder =
            crate::database::placeholder::typed_placeholder(exec.backend(), 1, &bound_key)?;
        let mut values = vec![bound_key];
        let key_type = if exec.backend() == sea_orm::DbBackend::MySql {
            "CHAR"
        } else {
            "TEXT"
        };
        let mut sql = if from == descriptor.model_table {
            format!(
                "SELECT CAST(p.{} AS {key_type}) AS search_key FROM {from} p WHERE {predicate} = {}",
                descriptor.key_column, key_placeholder
            )
        } else {
            format!(
                "SELECT CAST(p.{} AS {key_type}) AS search_key FROM {from} WHERE {predicate} = {}",
                descriptor.key_column, key_placeholder
            )
        };
        if relation.kind == RelationKind::MorphTo {
            let target = relation
                .morph_targets
                .iter()
                .find(|target| (target.type_id)() == TypeId::of::<M>())
                .ok_or_else(|| FrameworkError::param("search touches morph target disappeared"))?;
            crate::database::validate_identifier(relation.morph_type_column)?;
            let mut placeholders = Vec::new();
            for name in target.morph_type_names() {
                values.push(name.into());
                placeholders.push(crate::database::__macro_support::placeholder(
                    exec.backend(),
                    values.len(),
                )?);
            }
            sql.push_str(&format!(
                " AND p.{} IN ({})",
                relation.morph_type_column,
                placeholders.join(", ")
            ));
        }
        if !relation.pivot_table.is_empty() && !relation.morph_type_column.is_empty() {
            crate::database::validate_identifier(relation.morph_type_column)?;
            values.push(relation.morph_type_value.into());
            sql.push_str(&format!(
                " AND r.{} = {}",
                relation.morph_type_column,
                crate::database::__macro_support::placeholder(exec.backend(), 2)?
            ));
        }
        let rows = exec
            .query_all(sea_orm::Statement::from_sql_and_values(
                exec.backend(),
                sql,
                values,
            ))
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?;
        for row in rows {
            let key = row
                .try_get::<String>("", "search_key")
                .or_else(|_| {
                    row.try_get::<i64>("", "search_key")
                        .map(|key| key.to_string())
                })
                .or_else(|_| {
                    row.try_get::<u64>("", "search_key")
                        .map(|key| key.to_string())
                })
                .map_err(|error| {
                    FrameworkError::database(format!("search touches key: {error}"))
                })?;
            if visited.insert((entry.index, key.clone())) {
                jobs.push(SearchIndexJob {
                    index: entry.index.into(),
                    source_version: event_version(entry.index, &key, 0)?,
                    key,
                    op: IndexOperation::Index,
                    generation: None,
                });
            }
        }
    }
    Ok(jobs)
}

/// The persistence bridge accepts all model types, making nonsearchable writes a cheap no-op.
pub async fn after_model_write<M: Serialize + 'static>(
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
    enqueue_touched_parents(model, tx, &mut std::collections::HashSet::new()).await?;
    enqueue_model(model, kind, tx).await
}

/// Retain both owners when an update changes a related row's foreign key.
#[doc(hidden)]
pub async fn after_model_update<M: Serialize + 'static>(
    previous: &M,
    current: &M,
    tx: Option<&Transaction>,
) -> Result<(), FrameworkError> {
    if SYNC_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let mut visited = std::collections::HashSet::new();
    enqueue_touched_parents(previous, tx, &mut visited).await?;
    enqueue_touched_parents(current, tx, &mut visited).await?;
    enqueue_model(current, WriteKind::Updated, tx).await
}

async fn enqueue_model<M: 'static>(
    model: &M,
    kind: WriteKind,
    tx: Option<&Transaction>,
) -> Result<(), FrameworkError> {
    let Some(entry) = SearchRegistry::by_type(TypeId::of::<M>())? else {
        return Ok(());
    };
    let document = (entry.document)(model)?;
    let descriptor = (entry.descriptor)();
    let deleted =
        kind == WriteKind::ForceDeleted || (kind == WriteKind::Deleted && !descriptor.soft_delete);
    // Restore receives the pre-restore snapshot from the model macro. Its old
    // deletion flag cannot decide eligibility; the worker evaluates the live row.
    let op = if deleted || (kind != WriteKind::Restored && !(entry.should_index)(model)?) {
        IndexOperation::Remove
    } else {
        IndexOperation::Index
    };
    let mut source_version =
        event_version(&document.index, &document.key, document.source_version)?;
    if entry.keeps_version {
        source_version = keep_versions(
            &document.index,
            vec![(document.key.clone(), source_version)],
            tx,
        )
        .await?
        .remove(0)
        .1;
    }
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
            let count = models.len();
            enqueue_models::<M>(models.into_vec(), IndexOperation::Index).await?;
            crate::console::line(format!(
                "search index `{}` submitted {count} documents",
                M::descriptor().name
            ));
            Ok(())
        })
        .await
}

pub(crate) async fn flush_index(index: &str) -> Result<(), FrameworkError> {
    enqueue_job(
        SearchFlushJob {
            index: index.to_owned(),
            generation: None,
            source_version: flush_version(index)?,
        },
        None,
    )
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
            generation: None,
            source_version: flush_version(M::descriptor().name)?,
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

#[cfg(test)]
mod sparse_tests {
    use super::*;
    use crate::search::{
        AttributeValue, Embedder, Embedding, FakeEmbedder, IndexSettings, LanguageConfig,
        ModelProfile, SparseEmbedding, WeightedAttribute,
    };
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn qdrant_native_bm25_leaves_attribute_text_for_the_server() {
        let mut settings = IndexSettings::default();
        settings.index_time.searchable_attributes = vec![
            WeightedAttribute {
                name: "title".into(),
                weight: 1.0,
            },
            WeightedAttribute {
                name: "body".into(),
                weight: 1.0,
            },
        ];
        settings.query.stop_words.insert("the".into());
        settings.index_time.language = LanguageConfig::Qdrant {
            tokenizer: "word".into(),
            stemmer: None,
            stop_words: vec!["and".into()],
            folding: true,
        };
        let document = SearchDocument {
            index: "test".into(),
            key: "1".into(),
            source_version: 1,
            attributes: BTreeMap::from([
                ("title".into(), AttributeValue::Text("the apple".into())),
                ("body".into(), AttributeValue::Text("and apple pie".into())),
                ("other".into(), AttributeValue::Text("banana".into())),
            ]),
            embed_text: None,
            title: None,
        };
        let mut docs = vec![IndexedDocument {
            document,
            content_hash: "test".into(),
            dense: None,
            sparse: None,
        }];
        let original = docs[0].document.clone();
        for model in [None, Some("bm25".into())] {
            settings.index_time.sparse_model = model;
            embed_sparse_documents("test", &settings, &mut docs, "qdrant")
                .await
                .unwrap();
            assert!(docs[0].sparse.is_none());
            assert_eq!(docs[0].document, original);
        }
        settings.index_time.sparse_model = None;
        for driver in ["database", "collection"] {
            embed_sparse_documents("test", &settings, &mut docs, driver)
                .await
                .unwrap();
            assert!(docs[0].sparse.is_none());
        }
    }

    struct WeightedSparse(FakeEmbedder);

    #[async_trait::async_trait]
    impl Embedder for WeightedSparse {
        fn profile(&self) -> &ModelProfile {
            self.0.profile()
        }
        async fn embed_documents(
            &self,
            inputs: &[EmbedInput],
        ) -> Result<Vec<Embedding>, FrameworkError> {
            self.0.embed_documents(inputs).await
        }
        async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
            self.0.embed_query(text).await
        }
        async fn embed_sparse(
            &self,
            texts: &[String],
        ) -> Result<Vec<SparseEmbedding>, FrameworkError> {
            Ok(texts
                .iter()
                .map(|text| SparseEmbedding {
                    indices: vec![7, 11],
                    values: vec![text.len() as f32, 1.0],
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn named_sparse_models_keep_weighted_attribute_merging() {
        SearchRegistry::isolated(async {
            SearchRegistry::register_embedder(
                "weighted-test",
                std::sync::Arc::new(WeightedSparse(FakeEmbedder::new(3).unwrap())),
            )
            .unwrap();
            let mut settings = IndexSettings::default();
            settings.index_time.sparse_model = Some("weighted-test".into());
            settings.index_time.searchable_attributes = vec![
                WeightedAttribute {
                    name: "title".into(),
                    weight: 2.0,
                },
                WeightedAttribute {
                    name: "body".into(),
                    weight: 3.0,
                },
            ];
            let document = SearchDocument {
                index: "test".into(),
                key: "1".into(),
                source_version: 1,
                attributes: BTreeMap::from([
                    ("title".into(), AttributeValue::Text("apple".into())),
                    ("body".into(), AttributeValue::Text("apple pie".into())),
                ]),
                embed_text: None,
                title: None,
            };
            let doc = IndexedDocument {
                document,
                content_hash: "test".into(),
                dense: None,
                sparse: None,
            };
            let mut docs = vec![doc.clone(), doc];
            embed_sparse_documents("test", &settings, &mut docs, "collection")
                .await
                .unwrap();
            for doc in docs {
                let vector = doc.sparse.unwrap();
                assert_eq!(vector.indices, [7, 11]);
                assert_eq!(vector.values, [37.0, 5.0]);
            }
        })
        .await;
    }
}
