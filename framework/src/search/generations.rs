//! Publication and pending references share a database fence across queue drivers.
use super::{GenerationId, IndexSettings, SearchConfig, SearchRegistry, SettingsChange};
use crate::{DB, FrameworkError, console};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbBackend, QueryResult, Statement, TransactionTrait,
    Value,
};
use sha2::{Digest, Sha256};
use std::time::Duration;

tokio::task_local! { static IMPORT_GENERATION: GenerationId; }

pub(crate) fn import_generation() -> Option<GenerationId> {
    IMPORT_GENERATION.try_with(Clone::clone).ok()
}
fn error(index: &str, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::database(format!("search index '{index}': {error}"))
}
fn statement(backend: DbBackend, text: &str, values: Vec<Value>) -> Statement {
    let text = if backend == DbBackend::Postgres {
        let mut n = 0;
        text.chars()
            .map(|c| {
                if c == '?' {
                    n += 1;
                    format!("${n}")
                } else {
                    c.to_string()
                }
            })
            .collect::<String>()
    } else {
        text.to_owned()
    };
    Statement::from_sql_and_values(backend, text, values)
}
async fn query<C: ConnectionTrait>(
    conn: &C,
    index: &str,
    text: &str,
    values: Vec<Value>,
) -> Result<Vec<QueryResult>, FrameworkError> {
    conn.query_all_raw(statement(conn.get_database_backend(), text, values))
        .await
        .map_err(|e| error(index, e))
}
async fn execute<C: ConnectionTrait>(
    conn: &C,
    index: &str,
    text: &str,
    values: Vec<Value>,
) -> Result<u64, FrameworkError> {
    conn.execute_raw(statement(conn.get_database_backend(), text, values))
        .await
        .map(|r| r.rows_affected())
        .map_err(|e| error(index, e))
}
fn field<T: sea_orm::TryGetable>(
    row: &QueryResult,
    index: &str,
    name: &str,
) -> Result<T, FrameworkError> {
    row.try_get("", name).map_err(|e| error(index, e))
}
pub(crate) async fn persistent(index: &str) -> Result<bool, FrameworkError> {
    if SearchRegistry::engine_for(index)?.name() == "database" {
        return Ok(true);
    }
    let Ok(conn) = DB::connection() else {
        return Ok(false);
    };
    if !sea_orm_migration::SchemaManager::new(conn.inner())
        .has_table("search_jobs_pending")
        .await
        .map_err(|e| error(index, e))?
    {
        return Ok(false);
    }
    Ok(!query(
        conn.inner(),
        index,
        "SELECT generation FROM search_generations WHERE index_name=? AND generation='__control'",
        vec![index.to_owned().into()],
    )
    .await?
    .is_empty())
}
async fn fence(index: &str) -> Result<DatabaseTransaction, FrameworkError> {
    let conn = DB::connection()?;
    let tx = conn.inner().begin().await.map_err(|e| error(index, e))?;
    execute(&tx,index,"UPDATE search_generations SET write_count=write_count+0 WHERE index_name=? AND generation='__control'",vec![index.to_owned().into()]).await?;
    if query(
        &tx,
        index,
        "SELECT generation FROM search_generations WHERE index_name=? AND generation='__control'",
        vec![index.to_owned().into()],
    )
    .await?
    .is_empty()
    {
        return Err(FrameworkError::param(format!(
            "search index '{index}' has no generation - run search:index {index}"
        )));
    }
    Ok(tx)
}
fn job_id(index: &str, generation: &GenerationId, key: &str, version: i64) -> String {
    hex::encode(Sha256::digest(
        serde_json::json!([index, generation.as_str(), key, version])
            .to_string()
            .as_bytes(),
    ))
}

/// Reserve pinned targets and pending references under the same fence as publication.
/// The ledger is independent of queue storage and survives a worker or command restart.
pub async fn reserve(
    index: &str,
    keys: &[(String, i64)],
    explicit: Option<&GenerationId>,
) -> Result<Vec<GenerationId>, FrameworkError> {
    reserve_inner(index, keys, explicit, None).await
}

/// Durable payloads close the gap between committing a reservation and publishing its queue message.
pub async fn reserve_job(
    index: &str,
    keys: &[(String, i64)],
    explicit: Option<&GenerationId>,
    name: &str,
    payload: serde_json::Value,
) -> Result<Vec<GenerationId>, FrameworkError> {
    reserve_inner(index, keys, explicit, Some((name, payload))).await
}
async fn reserve_inner(
    index: &str,
    keys: &[(String, i64)],
    explicit: Option<&GenerationId>,
    job: Option<(&str, serde_json::Value)>,
) -> Result<Vec<GenerationId>, FrameworkError> {
    if !persistent(index).await? {
        let engine = SearchRegistry::engine_for(index)?;
        if let Some(id) = explicit {
            return Ok(vec![id.clone()]);
        }
        let current = SearchRegistry::generation_for(index).await?;
        let mut targets = vec![current.clone()];
        targets.extend(
            engine
                .generation_states(index)
                .await?
                .into_iter()
                .filter(|(id, state)| id != &current && state == "building")
                .map(|(id, _)| id),
        );
        return Ok(targets);
    }
    let tx = fence(index).await?;
    let targets = if let Some(id) = explicit {
        let rows = query(
            &tx,
            index,
            "SELECT state FROM search_generations WHERE index_name=? AND generation=?",
            vec![index.to_owned().into(), id.as_str().to_owned().into()],
        )
        .await?;
        let state: String = rows
            .first()
            .map(|r| field(r, index, "state"))
            .transpose()?
            .ok_or_else(|| {
                error(
                    index,
                    format!("generation '{}' does not exist", id.as_str()),
                )
            })?;
        if !matches!(
            state.as_str(),
            "current" | "building" | "retired" | "active" | "inactive"
        ) {
            return Err(error(
                index,
                format!("generation '{}' is {state}", id.as_str()),
            ));
        }
        vec![id.clone()]
    } else {
        query(&tx,index,"SELECT generation FROM search_generations WHERE index_name=? AND state IN ('current','building','active') ORDER BY generation",vec![index.to_owned().into()]).await?.iter().map(|r| GenerationId::new(field::<String>(r,index,"generation")?)).collect::<Result<Vec<_>,_>>()?
    };
    if targets.is_empty() {
        return Err(error(index, "no current or building generation"));
    }
    execute(&tx,index,"UPDATE search_generations SET write_count=write_count+1 WHERE index_name=? AND generation='__control'",vec![index.to_owned().into()]).await?;
    let rows = query(
        &tx,
        index,
        "SELECT write_count FROM search_generations WHERE index_name=? AND generation='__control'",
        vec![index.to_owned().into()],
    )
    .await?;
    let sequence: i64 = field(
        rows.first()
            .ok_or_else(|| error(index, "control row disappeared"))?,
        index,
        "write_count",
    )?;
    let conflict = if tx.get_database_backend() == DbBackend::MySql {
        "ON DUPLICATE KEY UPDATE completed=FALSE,published=FALSE,completed_at=NULL"
    } else {
        "ON CONFLICT (job_id) DO UPDATE SET completed=FALSE,published=FALSE,completed_at=NULL"
    };
    for generation in &targets {
        for (key, version) in keys {
            let (name,mut payload) = job.clone().unwrap_or_else(|| ("Suprnova::SearchIndex",serde_json::json!({"index":index,"key":key,"source_version":version,"op":"Index"})));
            payload
                .as_object_mut()
                .ok_or_else(|| error(index, "pending job payload must be an object"))?
                .insert("generation".into(), serde_json::json!(generation));
            execute(&tx,index,&format!("INSERT INTO search_jobs_pending(job_id,index_name,generation,sequence,job_name,payload_json) VALUES (?,?,?,?,?,?) {conflict}"),vec![job_id(index,generation,key,*version).into(),index.to_owned().into(),generation.as_str().to_owned().into(),sequence.into(),name.to_owned().into(),serde_json::to_string(&payload).map_err(|e|error(index,e))?.into()]).await?;
        }
    }
    tx.commit().await.map_err(|e| error(index, e))?;
    Ok(targets)
}

/// Success releases references; a failed or retried job must continue to block publication.
pub async fn complete(
    index: &str,
    generation: &GenerationId,
    keys: &[(String, i64)],
) -> Result<(), FrameworkError> {
    if !persistent(index).await? {
        return Ok(());
    }
    let conn = DB::connection()?;
    for (key, version) in keys {
        execute(
            conn.inner(),
            index,
            "UPDATE search_jobs_pending SET completed=TRUE,completed_at=? WHERE job_id=?",
            vec![
                crate::clock::now().timestamp().into(),
                job_id(index, generation, key, *version).into(),
            ],
        )
        .await?;
    }
    Ok(())
}

/// Dispatch acknowledgement distinguishes queued work from interrupted pre-publication work.
pub async fn published(
    index: &str,
    generation: &GenerationId,
    keys: &[(String, i64)],
) -> Result<(), FrameworkError> {
    if !persistent(index).await? {
        return Ok(());
    }
    let conn = DB::connection()?;
    for (key, version) in keys {
        execute(
            conn.inner(),
            index,
            "UPDATE search_jobs_pending SET published=TRUE WHERE job_id=?",
            vec![job_id(index, generation, key, *version).into()],
        )
        .await?;
    }
    Ok(())
}
/// Logical completion receipts make ambiguous queue-publish retries harmless.
pub async fn already_completed(
    index: &str,
    generation: Option<&GenerationId>,
    keys: &[(String, i64)],
) -> Result<bool, FrameworkError> {
    let Some(generation) = generation else {
        return Ok(false);
    };
    if keys.is_empty() || !persistent(index).await? {
        return Ok(false);
    }
    let conn = DB::connection()?;
    for (key, version) in keys {
        let rows = query(
            conn.inner(),
            index,
            "SELECT completed FROM search_jobs_pending WHERE job_id=?",
            vec![job_id(index, generation, key, *version).into()],
        )
        .await?;
        if !rows
            .first()
            .map(|row| field::<bool>(row, index, "completed"))
            .transpose()?
            .unwrap_or(false)
        {
            return Ok(false);
        }
    }
    Ok(true)
}
async fn recover_dispatches(index: &str) -> Result<(), FrameworkError> {
    super::outbox::recover(index).await?;
    use crate::queue::Job;
    let conn = DB::connection()?;
    let limit = i64::try_from(SearchConfig::current()?.chunk).map_err(|e| error(index, e))?;
    loop {
        let rows = query(conn.inner(),index,"SELECT DISTINCT job_name,payload_json FROM search_jobs_pending WHERE index_name=? AND published=FALSE AND completed=FALSE LIMIT ?",vec![index.to_owned().into(),limit.into()]).await?;
        if rows.is_empty() {
            return Ok(());
        }
        for row in rows {
            let payload: String = field(&row, index, "payload_json")?;
            let name: String = field(&row, index, "job_name")?;
            let data: serde_json::Value =
                serde_json::from_str(&payload).map_err(|e| error(index, e))?;
            let connection = data
                .get("_search_connection")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let execute = async {
                match name.as_str() {
                    "Suprnova::SearchIndex" => {
                        serde_json::from_str::<super::SearchIndexJob>(&payload)
                            .map_err(|e| error(index, e))?
                            .handle()
                            .await?
                    }
                    "Suprnova::SearchIndexBatch" => {
                        serde_json::from_str::<super::indexing::SearchIndexBatchJob>(&payload)
                            .map_err(|e| error(index, e))?
                            .handle()
                            .await?
                    }
                    "Suprnova::SearchFlush" => {
                        serde_json::from_str::<super::indexing::SearchFlushJob>(&payload)
                            .map_err(|e| error(index, e))?
                            .handle()
                            .await?
                    }
                    _ => return Err(error(index, format!("unrecognized pending job '{name}'"))),
                }
                Ok(())
            };
            if let Some(connection) = connection {
                super::registry::in_connection(connection, execute).await?;
            } else {
                execute.await?;
            }
        }
    }
}

async fn effective_settings(index: &str) -> Result<IndexSettings, FrameworkError> {
    let mut settings = (SearchRegistry::by_index(index)?.descriptor)().index_settings();
    let config = SearchConfig::current()?;
    let defaults = super::QuerySettings::default();
    if settings.query.hits_cap == defaults.hits_cap {
        settings.query.hits_cap = config.hits_cap;
    }
    if settings.query.fusion_depth == defaults.fusion_depth {
        settings.query.fusion_depth = config.fusion_depth;
    }
    if settings.index_time.embedder.is_none() && config.embedder != super::SearchEmbedder::None {
        settings.index_time.embedder = Some(config.embedder.as_str().into());
    }
    if settings
        .index_time
        .embedder
        .as_deref()
        .is_some_and(|name| name != "none")
    {
        let embedder = SearchRegistry::embedder_for(index)?;
        if settings.index_time.model.is_none() {
            settings.index_time.model = Some(match &embedder.profile().revision {
                Some(revision) => format!("{}@{revision}", embedder.profile().id),
                None => embedder.profile().id.clone(),
            });
        }
        if settings.index_time.dimensions.is_none() {
            settings.index_time.dimensions = Some(embedder.profile().dimensions);
        }
    }
    settings.validate()?;
    Ok(settings)
}
fn new_id() -> Result<GenerationId, FrameworkError> {
    GenerationId::new(format!("g{:016x}", uuid::Uuid::new_v4().as_u128() >> 64))
}
async fn create(
    index: &str,
    settings: &IndexSettings,
    generation: &GenerationId,
) -> Result<(), FrameworkError> {
    let engine = SearchRegistry::engine_for(index)?;
    let descriptor = (SearchRegistry::by_index(index)?.descriptor)();
    engine
        .create_generation(descriptor, settings, generation)
        .await?;
    if engine.name() != "database" {
        let conn = DB::connection()?;
        let conflict = if conn.inner().get_database_backend() == DbBackend::MySql {
            "ON DUPLICATE KEY UPDATE generation=generation"
        } else {
            "ON CONFLICT (index_name,generation) DO NOTHING"
        };
        execute(conn.inner(),index,&format!("INSERT INTO search_generations(index_name,generation,state,settings_json,digest) VALUES (?,'__control','control','{{}}','') {conflict}"),vec![index.to_owned().into()]).await?;
        execute(conn.inner(),index,"INSERT INTO search_generations(index_name,generation,state,settings_json,digest,model,dimensions) VALUES (?,?,'building',?,?,?,?)",vec![index.to_owned().into(),generation.as_str().to_owned().into(),serde_json::to_string(settings).map_err(|e|error(index,e))?.into(),settings.index_time.digest().into(),settings.index_time.model.clone().into(),settings.index_time.dimensions.map(i64::try_from).transpose().map_err(|e|error(index,e))?.into()]).await?;
    }
    Ok(())
}
async fn publish(
    index: &str,
    from: Option<GenerationId>,
    to: GenerationId,
) -> Result<(), FrameworkError> {
    let engine = SearchRegistry::engine_for(index)?;
    if engine.name() == "database" {
        engine
            .switch_generation((SearchRegistry::by_index(index)?.descriptor)(), from, to)
            .await
    } else {
        let tx = fence(index).await?;
        let rows = query(
            &tx,
            index,
            "SELECT COUNT(*) AS n FROM search_jobs_pending WHERE index_name=? AND completed=FALSE",
            vec![index.to_owned().into()],
        )
        .await?;
        if field::<i64>(
            rows.first()
                .ok_or_else(|| error(index, "pending count returned no row"))?,
            index,
            "n",
        )? != 0
        {
            return Err(error(index, "pending jobs prevent generation publication"));
        }
        let intents = query(
            &tx,
            index,
            "SELECT COUNT(*) AS n FROM search_job_intents WHERE index_name=?",
            vec![index.to_owned().into()],
        )
        .await?;
        if field::<i64>(
            intents
                .first()
                .ok_or_else(|| error(index, "intent count returned no row"))?,
            index,
            "n",
        )? != 0
        {
            return Err(error(index, "pending jobs prevent generation publication"));
        }
        engine
            .switch_generation(
                (SearchRegistry::by_index(index)?.descriptor)(),
                from,
                to.clone(),
            )
            .await?;
        execute(&tx,index,"UPDATE search_generations SET state=CASE WHEN generation=? THEN 'current' ELSE 'retired' END WHERE index_name=? AND (state='current' OR generation=?)",vec![to.as_str().to_owned().into(),index.to_owned().into(),to.as_str().to_owned().into()]).await?;
        execute(&tx,index,"UPDATE search_generations SET building_generation=NULL,building_digest=NULL WHERE index_name=? AND generation='__control'",vec![index.to_owned().into()]).await?;
        tx.commit().await.map_err(|e| error(index, e))
    }
}
/// Initialize settings explicitly so a normal model write never performs hidden DDL.
pub async fn create_index(index: &str) -> Result<(), FrameworkError> {
    super::registry::in_index_connection(index, create_index_inner(index)).await
}
async fn create_index_inner(index: &str) -> Result<(), FrameworkError> {
    let engine = SearchRegistry::engine_for(index)?;
    if engine.current_generation(index).await?.is_some() {
        return Err(FrameworkError::param(format!(
            "search index '{index}' already has a current generation"
        )));
    }
    let generation = new_id()?;
    create(index, &effective_settings(index).await?, &generation).await?;
    publish(index, None, generation.clone()).await?;
    console::line(format!(
        "search:index {index}: current generation {}",
        generation.as_str()
    ));
    Ok(())
}
/// The model registry preserves chunk preparation and the configured queue policy.
pub async fn import_index(index: &str) -> Result<(), FrameworkError> {
    super::registry::in_index_connection(index, import_index_inner(index)).await
}
async fn import_index_inner(index: &str) -> Result<(), FrameworkError> {
    SearchRegistry::generation_for(index).await?;
    console::line(format!(
        "search:import {index}: importing chunks of {}",
        SearchConfig::current()?.chunk
    ));
    (SearchRegistry::by_index(index)?.import_all)().await?;
    console::line(format!("search:import {index}: import dispatched"));
    Ok(())
}
/// Removals use the same version and tombstone pipeline as ordinary model deletes.
pub async fn flush_index(index: &str) -> Result<(), FrameworkError> {
    super::registry::in_index_connection(index, flush_index_inner(index)).await
}
async fn flush_index_inner(index: &str) -> Result<(), FrameworkError> {
    super::indexing::flush_index(index).await?;
    console::line(format!("search:flush {index}: removal dispatched"));
    Ok(())
}
async fn pending(
    index: &str,
    generation: Option<&GenerationId>,
    watermark: Option<i64>,
) -> Result<u64, FrameworkError> {
    let conn = DB::connection()?;
    let mut text = String::from(
        "SELECT COUNT(*) AS n FROM search_jobs_pending WHERE index_name=? AND completed=FALSE",
    );
    let mut values = vec![index.to_owned().into()];
    if let Some(generation) = generation {
        text.push_str(" AND generation=?");
        values.push(generation.as_str().to_owned().into());
    }
    if let Some(watermark) = watermark {
        text.push_str(" AND sequence<=?");
        values.push(watermark.into());
    }
    let rows = query(conn.inner(), index, &text, values).await?;
    let count: i64 = field(
        rows.first()
            .ok_or_else(|| error(index, "pending count returned no row"))?,
        index,
        "n",
    )?;
    u64::try_from(count).map_err(|e| error(index, e))
}
async fn watermark(index: &str, generation: &GenerationId) -> Result<i64, FrameworkError> {
    let tx = fence(index).await?;
    let rows = query(
        &tx,
        index,
        "SELECT write_count FROM search_generations WHERE index_name=? AND generation='__control'",
        vec![index.to_owned().into()],
    )
    .await?;
    let value: i64 = field(
        rows.first()
            .ok_or_else(|| error(index, "control row disappeared"))?,
        index,
        "write_count",
    )?;
    execute(
        &tx,
        index,
        "UPDATE search_generations SET switch_watermark=? WHERE index_name=? AND generation=?",
        vec![
            value.into(),
            index.to_owned().into(),
            generation.as_str().to_owned().into(),
        ],
    )
    .await?;
    tx.commit().await.map_err(|e| error(index, e))?;
    Ok(value)
}
async fn drain(
    index: &str,
    watermark: i64,
    deadline: tokio::time::Instant,
) -> Result<(), FrameworkError> {
    loop {
        recover_dispatches(index).await?;
        let count = pending(index, None, Some(watermark)).await?;
        if count == 0 {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(error(
                index,
                format!(
                    "{count} jobs at or before watermark {watermark} remain pending - run the search queue worker and retry search:reindex {index}"
                ),
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn choose_build(
    index: &str,
    settings: &IndexSettings,
) -> Result<(GenerationId, bool), FrameworkError> {
    let conn = DB::connection()?;
    let conflict = if conn.inner().get_database_backend() == DbBackend::MySql {
        "ON DUPLICATE KEY UPDATE generation=generation"
    } else {
        "ON CONFLICT (index_name,generation) DO NOTHING"
    };
    execute(conn.inner(),index,&format!("INSERT INTO search_generations(index_name,generation,state,settings_json,digest) VALUES (?,'__control','control','{{}}','') {conflict}"),vec![index.to_owned().into()]).await?;
    let tx = fence(index).await?;
    let rows = query(&tx,index,"SELECT building_generation,building_digest FROM search_generations WHERE index_name=? AND generation='__control'",vec![index.to_owned().into()]).await?;
    let control = rows
        .first()
        .ok_or_else(|| error(index, "control row disappeared"))?;
    let owner: Option<String> = field(control, index, "building_generation")?;
    let digest = settings.index_time.digest();
    let generation = if let Some(owner) = owner {
        let saved: Option<String> = field(control, index, "building_digest")?;
        if saved.as_deref() != Some(digest.as_str()) {
            return Err(error(
                index,
                "an unfinished build has different index-time settings",
            ));
        }
        GenerationId::new(owner)?
    } else {
        let builds = query(&tx,index,"SELECT generation,digest FROM search_generations WHERE index_name=? AND state IN ('building','creating') ORDER BY generation",vec![index.to_owned().into()]).await?;
        if builds.len() > 1 {
            return Err(error(
                index,
                "multiple generation builds require recovery before reindexing",
            ));
        }
        let generation = if let Some(row) = builds.first() {
            if field::<String>(row, index, "digest")? != digest {
                return Err(error(
                    index,
                    "an unfinished build has different index-time settings",
                ));
            }
            GenerationId::new(field::<String>(row, index, "generation")?)?
        } else {
            new_id()?
        };
        execute(&tx,index,"UPDATE search_generations SET building_generation=?,building_digest=? WHERE index_name=? AND generation='__control'",vec![generation.as_str().to_owned().into(),digest.into(),index.to_owned().into()]).await?;
        generation
    };
    let rows = query(
        &tx,
        index,
        "SELECT state FROM search_generations WHERE index_name=? AND generation=?",
        vec![
            index.to_owned().into(),
            generation.as_str().to_owned().into(),
        ],
    )
    .await?;
    let mut create = rows.is_empty();
    if let Some(row) = rows.first() {
        let state: String = field(row, index, "state")?;
        if state == "creating" {
            create = true;
        } else if state != "building" {
            return Err(error(
                index,
                format!(
                    "generation '{}' is {state}; its build has not finished creating storage",
                    generation.as_str()
                ),
            ));
        }
    }
    tx.commit().await.map_err(|e| error(index, e))?;
    Ok((generation, create))
}
async fn rebuild(index: &str, settings: IndexSettings) -> Result<(), FrameworkError> {
    let engine = SearchRegistry::engine_for(index)?;
    let current = engine.current_generation(index).await?;
    let (generation, needs_create) = choose_build(index, &settings).await?;
    if needs_create {
        create(index, &settings, &generation).await?;
    }
    recover_dispatches(index).await?;
    console::line(format!(
        "search:reindex {index}: building {}",
        generation.as_str()
    ));
    // Import inline into the hidden generation. Live writes still honor SEARCH_QUEUE.
    let mut config = SearchConfig::current()?;
    config.queue = false;
    let import = (SearchRegistry::by_index(index)?.import_all)();
    SearchConfig::scope(config, IMPORT_GENERATION.scope(generation.clone(), import)).await??;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let watermark = watermark(index, &generation).await?;
        console::line(format!(
            "search:reindex {index}: draining through watermark {watermark}"
        ));
        drain(index, watermark, deadline).await?;
        match publish(index, current.clone(), generation.clone()).await {
            Ok(()) => break,
            Err(error)
                if error
                    .to_string()
                    .contains("pending jobs prevent generation publication") =>
            {
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    console::line(format!(
        "search:reindex {index}: current {}",
        generation.as_str()
    ));
    prune_retired(index).await?;
    Ok(())
}
/// Resume an interrupted build or import a fresh hidden generation before publishing it.
pub async fn reindex(index: &str) -> Result<(), FrameworkError> {
    super::registry::in_index_connection(index, reindex_inner(index)).await
}
async fn reindex_inner(index: &str) -> Result<(), FrameworkError> {
    rebuild(index, effective_settings(index).await?).await
}
/// Print every changed setting's class and rebuild only when indexed identity changes.
pub async fn sync_settings(index: &str) -> Result<(), FrameworkError> {
    super::registry::in_index_connection(index, sync_settings_inner(index)).await
}
async fn sync_settings_inner(index: &str) -> Result<(), FrameworkError> {
    let engine = SearchRegistry::engine_for(index)?;
    let descriptor = (SearchRegistry::by_index(index)?.descriptor)();
    let Some(current) = engine.current_generation(index).await? else {
        return create_index(index).await;
    };
    let old = engine.read_settings(descriptor, &current).await?;
    let new = effective_settings(index).await?;
    let changes = SettingsChange::between(&old, &new);
    for path in &changes.query_time {
        console::line(format!("{index}: query-time {path}"));
    }
    for path in &changes.index_time {
        console::line(format!("{index}: index-time {path}"));
    }
    if changes.requires_generation() {
        let mut query_only = old;
        query_only.query = new.query.clone();
        engine
            .write_settings(descriptor, &current, &query_only)
            .await?;
        console::line(format!("{index}: generation build started"));
        rebuild(index, new).await
    } else {
        engine.write_settings(descriptor, &current, &new).await?;
        console::line(format!(
            "{index}: query-time settings applied; no generation build"
        ));
        Ok(())
    }
}
async fn prune_retired(index: &str) -> Result<(), FrameworkError> {
    let engine = SearchRegistry::engine_for(index)?;
    let conn = DB::connection()?;
    let rows = query(conn.inner(),index,"SELECT generation FROM search_generations WHERE index_name=? AND state IN ('retired','dropping','inactive') ORDER BY generation",vec![index.to_owned().into()]).await?;
    for row in rows {
        let id = GenerationId::new(field::<String>(&row, index, "generation")?)?;
        if pending(index, Some(&id), None).await? == 0 {
            if engine.name() != "database" {
                let tx = fence(index).await?;
                let rows = query(&tx,index,"SELECT COUNT(*) AS n FROM search_jobs_pending WHERE index_name=? AND generation=? AND completed=FALSE",vec![index.to_owned().into(),id.as_str().to_owned().into()]).await?;
                if field::<i64>(
                    rows.first()
                        .ok_or_else(|| error(index, "pending count returned no row"))?,
                    index,
                    "n",
                )? != 0
                {
                    continue;
                }
                execute(&tx,index,"UPDATE search_generations SET state='dropping' WHERE index_name=? AND generation=?",vec![index.to_owned().into(),id.as_str().to_owned().into()]).await?;
                tx.commit().await.map_err(|e| error(index, e))?;
            }
            engine
                .drop_generation((SearchRegistry::by_index(index)?.descriptor)(), &id)
                .await?;
            if engine.name() != "database" {
                execute(
                    conn.inner(),
                    index,
                    "DELETE FROM search_generations WHERE index_name=? AND generation=?",
                    vec![index.to_owned().into(), id.as_str().to_owned().into()],
                )
                .await?;
            }
            console::line(format!(
                "{index}: dropped retired generation {}",
                id.as_str()
            ));
        }
    }
    Ok(())
}
/// Delete only after pending references drain, so delayed jobs never write missing tables.
pub async fn delete_index(index: &str) -> Result<(), FrameworkError> {
    super::registry::in_index_connection(index, delete_index_inner(index)).await
}
async fn delete_index_inner(index: &str) -> Result<(), FrameworkError> {
    let tx = fence(index).await?;
    let rows = query(
        &tx,
        index,
        "SELECT COUNT(*) AS n FROM search_jobs_pending WHERE index_name=? AND completed=FALSE",
        vec![index.to_owned().into()],
    )
    .await?;
    if field::<i64>(
        rows.first()
            .ok_or_else(|| error(index, "pending count returned no row"))?,
        index,
        "n",
    )? != 0
    {
        return Err(error(index, "pending search jobs prevent index deletion"));
    }
    let intents = query(
        &tx,
        index,
        "SELECT COUNT(*) AS n FROM search_job_intents WHERE index_name=?",
        vec![index.to_owned().into()],
    )
    .await?;
    if field::<i64>(
        intents
            .first()
            .ok_or_else(|| error(index, "intent count returned no row"))?,
        index,
        "n",
    )? != 0
    {
        return Err(error(index, "staged search jobs prevent index deletion"));
    }
    let builds = query(&tx,index,"SELECT generation FROM search_generations WHERE index_name=? AND (state IN ('building','creating') OR building_generation IS NOT NULL)",vec![index.to_owned().into()]).await?;
    if !builds.is_empty() {
        return Err(error(index, "a generation build prevents index deletion"));
    }
    let engine = SearchRegistry::engine_for(index)?;
    if engine.name() != "database" {
        engine.retire_index(index).await?;
    }
    execute(&tx,index,"UPDATE search_generations SET state='retired' WHERE index_name=? AND generation!='__control'",vec![index.to_owned().into()]).await?;
    tx.commit().await.map_err(|e| error(index, e))?;
    prune_retired(index).await?;
    console::line(format!("search:delete-index {index}: deleted"));
    Ok(())
}
/// Retention expires persistent removal fences independently of queue storage.
pub async fn prune_tombstones() -> Result<(), FrameworkError> {
    let count = super::DatabaseSearchEngine::new()
        .prune_tombstones(SearchConfig::current()?.tombstone_hours)
        .await?;
    let seconds = SearchConfig::current()?
        .tombstone_hours
        .checked_mul(3600)
        .and_then(|n| i64::try_from(n).ok())
        .ok_or_else(|| FrameworkError::param("SEARCH_TOMBSTONE_HOURS exceeds timestamp range"))?;
    let cutoff = crate::clock::now()
        .timestamp()
        .checked_sub(seconds)
        .ok_or_else(|| FrameworkError::param("SEARCH_TOMBSTONE_HOURS exceeds timestamp range"))?;
    execute(
        DB::connection()?.inner(),
        "maintenance",
        "DELETE FROM search_jobs_pending WHERE completed=TRUE AND completed_at<?",
        vec![cutoff.into()],
    )
    .await?;
    console::line(format!("search:status: pruned {count} tombstones"));
    Ok(())
}
/// Report hidden generations and pending references so an interrupted build is actionable.
pub async fn status(index: &str, prune: bool) -> Result<(), FrameworkError> {
    super::registry::in_index_connection(index, status_inner(index, prune)).await
}
async fn status_inner(index: &str, prune: bool) -> Result<(), FrameworkError> {
    if prune {
        recover_dispatches(index).await?;
        prune_retired(index).await?;
        prune_tombstones().await?;
    }
    let engine = SearchRegistry::engine_for(index)?;
    let descriptor = (SearchRegistry::by_index(index)?.descriptor)();
    console::line(format!("search:status {index}: engine {}", engine.name()));
    let conn = DB::connection()?;
    let rows = query(conn.inner(),index,"SELECT generation,state FROM search_generations WHERE index_name=? AND generation!='__control' ORDER BY generation",vec![index.to_owned().into()]).await?;
    let query_all = super::EngineQuery {
        expanded: Default::default(),
        filter: None,
        sort: vec![],
        depth: 1,
        highlights: false,
        crop: Default::default(),
    };
    for row in rows {
        let generation = GenerationId::new(field::<String>(&row, index, "generation")?)?;
        let state: String = field(&row, index, "state")?;
        let count = if matches!(state.as_str(), "dropping" | "creating") {
            0
        } else {
            engine
                .count(descriptor, &generation, &query_all)
                .await?
                .value()
        };
        console::line(format!(
            "{index}: generation {} {state} documents {count} pending {}",
            generation.as_str(),
            pending(index, Some(&generation), None).await?
        ));
    }
    let rows = query(
        conn.inner(),
        index,
        "SELECT COUNT(*) AS n FROM search_tombstones WHERE index_name=?",
        vec![index.to_owned().into()],
    )
    .await?;
    console::line(format!(
        "{index}: pending jobs {} tombstones {}",
        pending(index, None, None).await?,
        field::<i64>(
            rows.first()
                .ok_or_else(|| error(index, "pending count returned no row"))?,
            index,
            "n"
        )?
    ));
    if engine.name() == "database" {
        for (name, value) in super::DatabaseSearchEngine::new().server_limits().await? {
            console::line(format!("{index}: server_limits {name}={value}"));
        }
    }
    Ok(())
}
