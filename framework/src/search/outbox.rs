//! Transactional intents bridge the model commit and generation-target reservation.

use sea_orm::{DbBackend, Statement, Value};
use sha2::{Digest, Sha256};

use super::{
    SearchConfig, SearchRegistry,
    indexing::{SearchFlushJob, SearchIndexBatchJob, SearchIndexJob, enqueue_job},
};
use crate::{FrameworkError, Transaction, database::transaction::ExecutorChoice};

fn error(index: &str, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::database(format!("search index `{index}` intent outbox: {error}"))
}

pub(crate) fn statement(
    backend: DbBackend,
    sql: &str,
    values: Vec<Value>,
) -> Result<Statement, FrameworkError> {
    let mut position = 0;
    let mut text = String::new();
    for character in sql.chars() {
        if character == '?' {
            position += 1;
            text.push_str(&crate::database::__macro_support::placeholder(
                backend, position,
            )?);
        } else {
            text.push(character);
        }
    }
    Ok(Statement::from_sql_and_values(backend, text, values))
}

async fn available(index: &str, exec: &ExecutorChoice) -> Result<bool, FrameworkError> {
    persistent_table(index, exec, "search_job_intents").await
}

/// Whether a search metadata table exists for this index's writes. The database engine
/// requires the search migrations, so it never probes; collection-only fixtures have no
/// persistent metadata. The probe runs on the model's own transaction executor so a
/// one-connection pool cannot deadlock here.
pub(crate) async fn persistent_table(
    index: &str,
    exec: &ExecutorChoice,
    table: &str,
) -> Result<bool, FrameworkError> {
    if SearchRegistry::engine_for(index)?.name() == "database" {
        return Ok(true);
    }
    let sql = match exec.backend() {
        DbBackend::Sqlite => "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
        DbBackend::Postgres => {
            "SELECT table_name FROM information_schema.tables WHERE table_schema=current_schema() AND table_name=?"
        }
        DbBackend::MySql => {
            "SELECT table_name FROM information_schema.tables WHERE table_schema=DATABASE() AND table_name=?"
        }
        _ => {
            return Err(error(
                index,
                "backend does not support persistent search metadata discovery",
            ));
        }
    };
    Ok(!exec
        .query_all(statement(
            exec.backend(),
            sql,
            vec![table.to_owned().into()],
        )?)
        .await
        .map_err(|failure| error(index, failure))?
        .is_empty())
}

/// Stage identity-only work on the same executor as the model write.
pub(crate) async fn stage_job(
    index: &str,
    name: &str,
    payload: &serde_json::Value,
    tx: Option<&Transaction>,
) -> Result<Option<String>, FrameworkError> {
    let exec = match tx {
        Some(tx) => ExecutorChoice::from_tx(tx),
        None => ExecutorChoice::resolve_write(None, None, None).await?,
    };
    if !available(index, &exec).await? {
        return Ok(None);
    }
    // The publication fence uses this control row. Holding its write lock until
    // model commit makes an intent visible before a concurrent switch can finish.
    exec.run(statement(exec.backend(), "UPDATE search_generations SET generation=generation WHERE index_name=? AND generation='__control'", vec![index.to_owned().into()])?).await.map_err(|failure| error(index, failure))?;
    let payload = serde_json::to_string(payload).map_err(|failure| error(index, failure))?;
    let id = hex::encode(Sha256::digest(
        format!("{index}\0{name}\0{payload}").as_bytes(),
    ));
    let conflict = if exec.backend() == DbBackend::MySql {
        "ON DUPLICATE KEY UPDATE intent_id=intent_id"
    } else {
        "ON CONFLICT (intent_id) DO NOTHING"
    };
    let sql = format!(
        "INSERT INTO search_job_intents(intent_id,index_name,job_name,payload_json,created_at) VALUES (?,?,?,?,?) {conflict}"
    );
    exec.run(statement(
        exec.backend(),
        &sql,
        vec![
            id.clone().into(),
            index.to_owned().into(),
            name.to_owned().into(),
            payload.into(),
            crate::clock::now().timestamp_micros().into(),
        ],
    )?)
    .await
    .map_err(|failure| error(index, failure))?;
    Ok(Some(id))
}

/// Retire an intent only once every pinned target is represented by durable pending work.
pub(crate) async fn reserved(index: &str, id: Option<&str>) -> Result<(), FrameworkError> {
    let Some(id) = id else { return Ok(()) };
    let exec = ExecutorChoice::resolve_write(None, None, None).await?;
    exec.run(statement(
        exec.backend(),
        "DELETE FROM search_job_intents WHERE index_name=? AND intent_id=?",
        vec![index.to_owned().into(), id.to_owned().into()],
    )?)
    .await
    .map_err(|failure| error(index, failure))?;
    Ok(())
}

/// Recover committed intents before maintenance waits on generation reservations.
pub(crate) async fn recover(index: &str) -> Result<(), FrameworkError> {
    if !super::generations::persistent(index).await? {
        return Ok(());
    }
    let mut config = SearchConfig::current()?;
    config.queue = false;
    SearchConfig::scope(config, async {
        let exec = ExecutorChoice::resolve_read(None, None, None).await?;
        let limit = i64::try_from(SearchConfig::current()?.chunk).map_err(|failure| error(index, failure))?;
        loop {
            let rows = exec.query_all(statement(exec.backend(), "SELECT intent_id,job_name,payload_json FROM search_job_intents WHERE index_name=? ORDER BY created_at,intent_id LIMIT ?", vec![index.to_owned().into(), limit.into()])?).await.map_err(|failure| error(index, failure))?;
            if rows.is_empty() { return Ok(()) }
            for row in rows {
                let id: String = row.try_get("", "intent_id").map_err(|failure| error(index, failure))?;
                let name: String = row.try_get("", "job_name").map_err(|failure| error(index, failure))?;
                let payload: String = row.try_get("", "payload_json").map_err(|failure| error(index, failure))?;
                let payload: serde_json::Value = serde_json::from_str(&payload).map_err(|failure| error(index, failure))?;
                let connection = payload.get("_search_connection").and_then(serde_json::Value::as_str).map(str::to_owned);
                let dispatch = async {
                match name.as_str() {
                    "Suprnova::SearchIndex" => enqueue_job(serde_json::from_value::<SearchIndexJob>(payload.clone()).map_err(|failure| error(index, failure))?, None).await?,
                    "Suprnova::SearchIndexBatch" => enqueue_job(serde_json::from_value::<SearchIndexBatchJob>(payload.clone()).map_err(|failure| error(index, failure))?, None).await?,
                    "Suprnova::SearchFlush" => enqueue_job(serde_json::from_value::<SearchFlushJob>(payload.clone()).map_err(|failure| error(index, failure))?, None).await?,
                    _ => return Err(error(index, format!("unrecognized staged job `{name}`"))),
                }
                reserved(index, Some(&id)).await?;
                    Ok(())
                };
                match connection { Some(name) => super::registry::in_connection(name, dispatch).await?, None => dispatch.await? }

            }
        }
    }).await?
}
