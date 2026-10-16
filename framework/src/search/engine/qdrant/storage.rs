//! Bound metadata statements work on SQLite, Postgres and MySQL alike.
use crate::{
    DB, FrameworkError,
    search::{GenerationId, IndexSettings},
};
use sea_orm::sea_query::{Alias, Expr, ExprTrait, OnConflict, Query};
use sea_orm::{ConnectionTrait, QueryResult, TransactionTrait};
use std::collections::BTreeSet;

fn error(index: &str, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::database(format!("Qdrant search index {index} metadata: {error}"))
}
fn a(name: &str) -> Alias {
    Alias::new(name)
}
fn generation_row(index: &str, generation: &GenerationId) -> sea_orm::sea_query::SelectStatement {
    Query::select()
        .columns([a("settings_json"), a("state")])
        .from(a("search_generations"))
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .to_owned()
}
pub(super) async fn settings(
    index: &str,
    generation: &GenerationId,
) -> Result<IndexSettings, FrameworkError> {
    settings_optional(index, generation).await?.ok_or_else(|| {
        FrameworkError::param(format!(
            "Qdrant search index {index} generation {}: not created",
            generation.as_str()
        ))
    })
}
pub(super) async fn settings_optional(
    index: &str,
    generation: &GenerationId,
) -> Result<Option<IndexSettings>, FrameworkError> {
    let exec = crate::database::transaction::ExecutorChoice::resolve_read(
        None,
        Some(&DB::default_connection()),
        None,
    )
    .await?;
    let query = generation_row(index, generation);
    let Some(row) = exec
        .query_one(exec.backend().build(&query))
        .await
        .map_err(|e| error(index, e))?
    else {
        return Ok(None);
    };
    let state: String = row.try_get("", "state").map_err(|e| error(index, e))?;
    if !matches!(
        state.as_str(),
        "current" | "building" | "retired" | "active" | "inactive"
    ) {
        return Err(error(
            index,
            format!(
                "generation {}: unavailable state {state}",
                generation.as_str()
            ),
        ));
    }
    let json: String = row
        .try_get("", "settings_json")
        .map_err(|e| error(index, e))?;
    serde_json::from_str(&json)
        .map(Some)
        .map_err(|e| error(index, e))
}
pub(super) async fn create(
    index: &str,
    generation: &GenerationId,
    settings: &IndexSettings,
) -> Result<(), FrameworkError> {
    let db = DB::get()?;
    crate::render_cache::write_side_open(false).await?;
    let txn = db.inner().begin().await.map_err(|e| error(index, e))?;
    let backend = txn.get_database_backend();
    let control = Query::insert()
        .into_table(a("search_generations"))
        .columns([
            a("index_name"),
            a("generation"),
            a("state"),
            a("settings_json"),
            a("digest"),
        ])
        .values([
            index.into(),
            "__control".into(),
            "control".into(),
            "{}".into(),
            "".into(),
        ])
        .map_err(|e| error(index, e))?
        .on_conflict(
            OnConflict::columns([a("index_name"), a("generation")])
                .value(a("generation"), "__control")
                .to_owned(),
        )
        .to_owned();
    txn.execute_raw(backend.build(&control))
        .await
        .map_err(|e| error(index, e))?;
    let json = serde_json::to_string(settings).map_err(|e| error(index, e))?;
    let query = Query::insert()
        .into_table(a("search_generations"))
        .columns([
            a("index_name"),
            a("generation"),
            a("settings_json"),
            a("digest"),
            a("model"),
            a("dimensions"),
            a("state"),
        ])
        .values([
            index.into(),
            generation.as_str().into(),
            json.into(),
            settings.index_time.digest().into(),
            settings.index_time.model.clone().into(),
            settings
                .index_time
                .dimensions
                .map(i64::try_from)
                .transpose()
                .map_err(|e| error(index, e))?
                .into(),
            // A new generation is building until a switch publishes it, as the database engine records it.
            "building".into(),
        ])
        .map_err(|e| error(index, e))?
        .to_owned();
    txn.execute_raw(backend.build(&query))
        .await
        .map_err(|e| error(index, e))?;
    crate::search::render_cache::advance_transaction(&txn, index).await?;
    txn.commit().await.map_err(|e| error(index, e))
}
pub(super) async fn write_settings(
    index: &str,
    generation: &GenerationId,
    settings: &IndexSettings,
) -> Result<(), FrameworkError> {
    let old = self::settings(index, generation).await?;
    if old.index_time != settings.index_time {
        return Err(FrameworkError::param(format!(
            "Qdrant search index {index} generation {}: index-time settings require a new generation",
            generation.as_str()
        )));
    }
    let db = DB::get()?;
    crate::render_cache::write_side_open(false).await?;
    let txn = db.inner().begin().await.map_err(|e| error(index, e))?;
    let query = Query::update()
        .table(a("search_generations"))
        .value(
            a("settings_json"),
            serde_json::to_string(settings).map_err(|e| error(index, e))?,
        )
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .to_owned();
    txn.execute_raw(txn.get_database_backend().build(&query))
        .await
        .map_err(|e| error(index, e))?;
    crate::search::render_cache::advance_transaction(&txn, index).await?;
    txn.commit().await.map_err(|e| error(index, e))
}
fn state_query(index: &str) -> sea_orm::sea_query::SelectStatement {
    Query::select()
        .column(a("generation"))
        .from(a("search_generations"))
        .and_where(Expr::col(a("state")).is_in(["current", "active"]))
        .and_where(Expr::col(a("index_name")).eq(index))
        .to_owned()
}
fn decode_current(
    index: &str,
    rows: Vec<QueryResult>,
) -> Result<Option<GenerationId>, FrameworkError> {
    if rows.len() > 1 {
        return Err(error(index, "multiple active generations"));
    }
    rows.first()
        .map(|row| {
            let generation: String = row.try_get("", "generation").map_err(|e| error(index, e))?;
            GenerationId::new(generation)
        })
        .transpose()
}
pub(super) async fn current(index: &str) -> Result<Option<GenerationId>, FrameworkError> {
    let exec = crate::database::transaction::ExecutorChoice::resolve_read(
        None,
        Some(&DB::default_connection()),
        None,
    )
    .await?;
    let row = exec
        .query_all(exec.backend().build(&state_query(index)))
        .await
        .map_err(|e| error(index, e))?;
    decode_current(index, row)
}
/// A row lock serializes generation switching and dropping across processes.
pub(super) async fn locked(
    index: &str,
    generation: &GenerationId,
) -> Result<sea_orm::DatabaseTransaction, FrameworkError> {
    let db = DB::get()?;
    crate::render_cache::write_side_open(false).await?;
    let txn = db.inner().begin().await.map_err(|e| error(index, e))?;
    for target in ["__control", generation.as_str()] {
        let query = Query::update()
            .table(a("search_generations"))
            .value(a("write_count"), Expr::col(a("write_count")).add(0))
            .and_where(Expr::col(a("generation")).eq(target))
            .and_where(Expr::col(a("index_name")).eq(index))
            .to_owned();
        txn.execute_raw(txn.get_database_backend().build(&query))
            .await
            .map_err(|e| error(index, e))?;
        let query = Query::select()
            .column(a("state"))
            .from(a("search_generations"))
            .and_where(Expr::col(a("generation")).eq(target))
            .and_where(Expr::col(a("index_name")).eq(index))
            .to_owned();
        let row = txn
            .query_one_raw(txn.get_database_backend().build(&query))
            .await
            .map_err(|e| error(index, e))?
            .ok_or_else(|| error(index, format!("generation {target}: not created")))?;
        let state: String = row.try_get("", "state").map_err(|e| error(index, e))?;
        if target != "__control"
            && !matches!(
                state.as_str(),
                "current" | "building" | "retired" | "active" | "inactive"
            )
        {
            return Err(error(
                index,
                format!("generation {target}: unavailable state {state}"),
            ));
        }
    }
    Ok(txn)
}
pub(super) async fn switch(
    index: &str,
    from: Option<GenerationId>,
    to: &GenerationId,
) -> Result<(), FrameworkError> {
    let txn = locked(index, to).await?;
    let backend = txn.get_database_backend();
    let row = txn
        .query_all_raw(backend.build(&state_query(index)))
        .await
        .map_err(|e| error(index, e))?;
    if decode_current(index, row)? != from {
        return Err(FrameworkError::param(format!(
            "Qdrant search index {index}: current generation changed before switch"
        )));
    }
    let target = txn
        .query_one_raw(backend.build(&generation_row(index, to)))
        .await
        .map_err(|e| error(index, e))?
        .ok_or_else(|| error(index, format!("generation {}: not created", to.as_str())))?;
    let state: String = target.try_get("", "state").map_err(|e| error(index, e))?;
    if !matches!(
        state.as_str(),
        "current" | "building" | "retired" | "active" | "inactive"
    ) {
        return Err(error(
            index,
            format!("generation {}: unavailable state {state}", to.as_str()),
        ));
    }
    let query = Query::update()
        .table(a("search_generations"))
        .value(a("state"), "retired")
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("state")).is_in(["current", "active"]))
        .to_owned();
    txn.execute_raw(backend.build(&query))
        .await
        .map_err(|e| error(index, e))?;
    let query = Query::update()
        .table(a("search_generations"))
        .value(a("state"), "current")
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(to.as_str()))
        .to_owned();
    txn.execute_raw(backend.build(&query))
        .await
        .map_err(|e| error(index, e))?;
    crate::search::render_cache::advance_transaction(&txn, index).await?;
    txn.commit().await.map_err(|e| error(index, e))
}
pub(super) async fn delete_metadata(
    txn: sea_orm::DatabaseTransaction,
    index: &str,
    generation: &GenerationId,
) -> Result<(), FrameworkError> {
    for table in [
        "search_vocabulary",
        "search_tombstones",
        "search_generations",
    ] {
        let query = Query::delete()
            .from_table(a(table))
            .and_where(Expr::col(a("index_name")).eq(index))
            .and_where(Expr::col(a("generation")).eq(generation.as_str()))
            .to_owned();
        txn.execute_raw(txn.get_database_backend().build(&query))
            .await
            .map_err(|e| error(index, e))?;
    }
    crate::search::render_cache::advance_transaction(&txn, index).await?;
    txn.commit().await.map_err(|e| error(index, e))
}
pub(super) async fn current_locked(
    txn: &sea_orm::DatabaseTransaction,
    index: &str,
) -> Result<Option<GenerationId>, FrameworkError> {
    let row = txn
        .query_all_raw(txn.get_database_backend().build(&state_query(index)))
        .await
        .map_err(|e| error(index, e))?;
    decode_current(index, row)
}
pub(super) async fn tombstone_on<C: ConnectionTrait>(
    conn: &C,
    index: &str,
    generation: &GenerationId,
    key: &str,
) -> Result<Option<i64>, FrameworkError> {
    let query = Query::select()
        .column(a("source_version"))
        .from(a("search_tombstones"))
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .and_where(Expr::col(a("document_key")).eq(key))
        .to_owned();
    let row = conn
        .query_one_raw(conn.get_database_backend().build(&query))
        .await
        .map_err(|e| error(index, e))?;
    row.map(|r| r.try_get("", "source_version").map_err(|e| error(index, e)))
        .transpose()
}
pub(super) async fn record_tombstone_on<C: ConnectionTrait>(
    conn: &C,
    index: &str,
    generation: &GenerationId,
    key: &str,
    version: i64,
) -> Result<(), FrameworkError> {
    let query = Query::insert()
        .into_table(a("search_tombstones"))
        .columns([
            a("index_name"),
            a("generation"),
            a("document_key"),
            a("source_version"),
            a("removed_at"),
        ])
        .values([
            index.into(),
            generation.as_str().into(),
            key.into(),
            version.into(),
            crate::clock::now().timestamp().into(),
        ])
        .map_err(|e| error(index, e))?
        .on_conflict(
            OnConflict::columns([a("index_name"), a("generation"), a("document_key")])
                .update_columns([a("source_version"), a("removed_at")])
                .to_owned(),
        )
        .to_owned();
    conn.execute_raw(conn.get_database_backend().build(&query))
        .await
        .map_err(|e| error(index, e))?;
    Ok(())
}
pub(super) async fn commit(
    txn: sea_orm::DatabaseTransaction,
    index: &str,
) -> Result<(), FrameworkError> {
    txn.commit().await.map_err(|e| error(index, e))
}

pub(super) async fn commit_write(
    txn: sea_orm::DatabaseTransaction,
    index: &str,
) -> Result<(), FrameworkError> {
    crate::search::render_cache::advance_transaction(&txn, index).await?;
    commit(txn, index).await
}

/// Maintain the same per-generation document frequencies as the database engine.
pub(super) async fn update_vocabulary<C: ConnectionTrait>(
    conn: &C,
    index: &str,
    generation: &GenerationId,
    old: &BTreeSet<String>,
    new: &BTreeSet<String>,
) -> Result<(), FrameworkError> {
    let backend = conn.get_database_backend();
    for word in old.difference(new) {
        let update = Query::update()
            .table(a("search_vocabulary"))
            .value(
                a("document_frequency"),
                Expr::col(a("document_frequency")).sub(1),
            )
            .and_where(Expr::col(a("index_name")).eq(index))
            .and_where(Expr::col(a("generation")).eq(generation.as_str()))
            .and_where(Expr::col(a("word")).eq(word.clone()))
            .to_owned();
        conn.execute_raw(backend.build(&update))
            .await
            .map_err(|e| error(index, e))?;
    }
    for word in new.difference(old) {
        let insert = Query::insert()
            .into_table(a("search_vocabulary"))
            .columns([
                a("index_name"),
                a("generation"),
                a("word"),
                a("document_frequency"),
            ])
            .values([
                index.into(),
                generation.as_str().into(),
                word.clone().into(),
                1i64.into(),
            ])
            .map_err(|e| error(index, e))?
            .on_conflict(
                OnConflict::columns([a("index_name"), a("generation"), a("word")])
                    .value(
                        a("document_frequency"),
                        Expr::col((a("search_vocabulary"), a("document_frequency"))).add(1),
                    )
                    .to_owned(),
            )
            .to_owned();
        conn.execute_raw(backend.build(&insert))
            .await
            .map_err(|e| error(index, e))?;
    }
    let delete = Query::delete()
        .from_table(a("search_vocabulary"))
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .and_where(Expr::col(a("document_frequency")).lte(0))
        .to_owned();
    conn.execute_raw(backend.build(&delete))
        .await
        .map_err(|e| error(index, e))?;
    let bump = Query::update()
        .table(a("search_generations"))
        .value(a("write_count"), Expr::col(a("write_count")).add(1))
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .to_owned();
    conn.execute_raw(backend.build(&bump))
        .await
        .map_err(|e| error(index, e))?;
    Ok(())
}

/// Reconcile frequencies after an uncertain remote acknowledgement or metadata commit.
pub(super) async fn replace_vocabulary<C: ConnectionTrait>(
    conn: &C,
    index: &str,
    generation: &GenerationId,
    vocabulary: &crate::search::Vocabulary,
) -> Result<(), FrameworkError> {
    let backend = conn.get_database_backend();
    let delete = Query::delete()
        .from_table(a("search_vocabulary"))
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .to_owned();
    conn.execute_raw(backend.build(&delete))
        .await
        .map_err(|e| error(index, e))?;
    for (word, frequency) in &vocabulary.words {
        let frequency = i64::try_from(*frequency).map_err(|e| error(index, e))?;
        let insert = Query::insert()
            .into_table(a("search_vocabulary"))
            .columns([
                a("index_name"),
                a("generation"),
                a("word"),
                a("document_frequency"),
            ])
            .values([
                index.into(),
                generation.as_str().into(),
                word.clone().into(),
                frequency.into(),
            ])
            .map_err(|e| error(index, e))?
            .to_owned();
        conn.execute_raw(backend.build(&insert))
            .await
            .map_err(|e| error(index, e))?;
    }
    let refresh = Query::update()
        .table(a("search_generations"))
        .value(a("refreshed_count"), Expr::col(a("write_count")))
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .to_owned();
    conn.execute_raw(backend.build(&refresh))
        .await
        .map_err(|e| error(index, e))?;
    Ok(())
}

pub(super) async fn vocabulary_on<C: ConnectionTrait>(
    conn: &C,
    index: &str,
    generation: &GenerationId,
) -> Result<crate::search::Vocabulary, FrameworkError> {
    let query = Query::select()
        .columns([a("word"), a("document_frequency")])
        .from(a("search_vocabulary"))
        .and_where(Expr::col(a("index_name")).eq(index))
        .and_where(Expr::col(a("generation")).eq(generation.as_str()))
        .and_where(Expr::col(a("document_frequency")).gt(0))
        .to_owned();
    let rows = conn
        .query_all_raw(conn.get_database_backend().build(&query))
        .await
        .map_err(|e| error(index, e))?;
    let mut vocabulary = crate::search::Vocabulary::default();
    for row in rows {
        let word: String = row.try_get("", "word").map_err(|e| error(index, e))?;
        let count: i64 = row
            .try_get("", "document_frequency")
            .map_err(|e| error(index, e))?;
        vocabulary
            .words
            .insert(word, u64::try_from(count).map_err(|e| error(index, e))?);
    }
    Ok(vocabulary)
}
