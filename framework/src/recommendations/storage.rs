//! All values are bound; only static framework SQL passes through dialect conversion.
use super::{Actor, RecommendationTarget};
use crate::{DB, DynamicRow, FrameworkError};
use sea_orm::{DbBackend, Value};

pub(crate) fn backend() -> Result<DbBackend, FrameworkError> {
    Ok(DB::connection()?.inner().get_database_backend())
}
pub(crate) fn sql(query: &str) -> Result<String, FrameworkError> {
    let backend = backend()?;
    let quote = if backend == DbBackend::MySql {
        "`rank`"
    } else {
        "\"rank\""
    };
    let mut quoted = String::new();
    let mut word = String::new();
    for c in query.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            quoted.push_str(if word == "rank" { quote } else { &word });
            word.clear();
            quoted.push(c);
        }
    }
    if backend != DbBackend::Postgres {
        return Ok(quoted);
    }
    // These statements contain no quoted question marks or application-owned SQL.
    let mut ordinal = 0;
    Ok(quoted
        .chars()
        .map(|c| {
            if c == '?' {
                ordinal += 1;
                format!("${ordinal}")
            } else {
                c.to_string()
            }
        })
        .collect())
}
pub(crate) async fn rows(
    query: &str,
    values: Vec<Value>,
) -> Result<Vec<DynamicRow>, FrameworkError> {
    DB::select(&sql(query)?, values).await
}
pub(crate) async fn execute(query: &str, values: Vec<Value>) -> Result<u64, FrameworkError> {
    DB::affecting_statement(&sql(query)?, values).await
}
pub(crate) async fn scalar(query: &str, values: Vec<Value>) -> Result<i64, FrameworkError> {
    DB::scalar(&sql(query)?, values).await
}
pub(crate) fn actor(row: &DynamicRow) -> Result<Actor, FrameworkError> {
    let id = row.get_string("actor_id")?;
    match row.get_string("actor_kind")?.as_str() {
        "user" => Ok(Actor::User(id)),
        "guest" => Ok(Actor::Guest(id)),
        other => Err(FrameworkError::database(format!(
            "invalid recommendation actor kind: {other}"
        ))),
    }
}
pub(crate) fn target(row: &DynamicRow) -> Result<RecommendationTarget, FrameworkError> {
    Ok(RecommendationTarget {
        index: row.get_string("target_index")?,
        key: row.get_string("target_key")?,
    })
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    row: &DynamicRow,
    column: &str,
) -> Result<T, FrameworkError> {
    serde_json::from_str(&row.get_string(column)?)
        .map_err(|e| FrameworkError::database(format!("recommendation {column}: {e}")))
}
pub(crate) fn encode<T: serde::Serialize>(value: &T) -> Result<String, FrameworkError> {
    serde_json::to_string(value)
        .map_err(|e| FrameworkError::internal(format!("recommendation serialization: {e}")))
}
