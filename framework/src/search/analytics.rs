//! Opt-in retained query facts and session-bound shown lists support trustworthy clicks.
use super::{SearchConfig, SearchQuery, SearchRegistry, Viewer};
use crate::{Actor, DB, DynamicRow, EventType, FrameworkError, Recommend, RecommendationTarget};
use sea_orm::{DbBackend, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
struct ShownKey {
    key: String,
    position: usize,
}

fn sql(query: &str) -> Result<String, FrameworkError> {
    let backend = DB::connection()?.inner().get_database_backend();
    let quote = if backend == DbBackend::MySql {
        '`'
    } else {
        '"'
    };
    let query = query
        .replace("{index}", &format!("{quote}index{quote}"))
        .replace("{key}", &format!("{quote}key{quote}"))
        .replace("{keys}", &format!("{quote}keys{quote}"));
    if backend != DbBackend::Postgres {
        return Ok(query);
    }
    let mut ordinal = 0;
    Ok(query
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
async fn rows(query: &str, values: Vec<Value>) -> Result<Vec<DynamicRow>, FrameworkError> {
    DB::select(&sql(query)?, values).await
}
async fn execute(query: &str, values: Vec<Value>) -> Result<u64, FrameworkError> {
    DB::affecting_statement(&sql(query)?, values).await
}
fn cutoff(days: u64) -> Result<i64, FrameworkError> {
    let days = i64::try_from(days)
        .map_err(|_| FrameworkError::param("analytics days exceeds supported retention"))?;
    let duration = chrono::Duration::try_days(days)
        .ok_or_else(|| FrameworkError::param("analytics days exceeds supported retention"))?;
    crate::clock::now()
        .checked_sub_signed(duration)
        .map(|time| time.timestamp_micros())
        .ok_or_else(|| FrameworkError::param("analytics days exceeds supported retention"))
}
fn json<T: Serialize>(value: &T) -> Result<String, FrameworkError> {
    serde_json::to_string(value).map_err(|error| {
        FrameworkError::internal(format!("search analytics serialization: {error}"))
    })
}
fn actor(viewer: &Viewer, session: &str) -> Result<Actor, FrameworkError> {
    match viewer {
        Viewer::Authenticated { id, .. } => Ok(Actor::User(id.clone())),
        Viewer::Guest => Actor::guest(session),
    }
}

pub(crate) async fn record(
    index: &str,
    query: &SearchQuery,
    viewer: &Viewer,
    hits: u64,
    took_ms: u64,
    keys: Vec<String>,
) -> Result<Option<String>, FrameworkError> {
    let config = SearchConfig::current()?;
    if !config.analytics {
        return Ok(None);
    }
    let query_id = uuid::Uuid::new_v4().to_string();
    let index = index.to_string();
    let text = query.text.original.clone();
    let folded = icu_casemap::CaseMapper::new()
        .fold_string(&text)
        .into_owned();
    let filters = json(&query.filter)?;
    let hits = i64::try_from(hits)
        .map_err(|_| FrameworkError::param("search analytics hits exceeds supported count"))?;
    let took_ms = i64::try_from(took_ms).map_err(|_| {
        FrameworkError::param("search analytics took_ms exceeds supported duration")
    })?;
    let viewer_id = if config.analytics_viewer {
        viewer.id().map(str::to_owned)
    } else {
        None
    };
    let session = crate::session::session().map(|session| session.id);
    let shown: Vec<_> = keys
        .into_iter()
        .enumerate()
        .map(|(position, key)| ShownKey { key, position })
        .collect();
    let viewer = viewer.clone();
    DB::transaction(move |_| Box::pin(async move {
        let now = crate::clock::now().timestamp_micros();
        execute("INSERT INTO search_queries (id,{index},query,query_fold,filters,hits,took_ms,viewer_id,created_at) VALUES (?,?,?,?,?,?,?,?,?)", vec![query_id.clone().into(), index.clone().into(), text.into(), folded.into(), filters.into(), hits.into(), took_ms.into(), viewer_id.into(), now.into()]).await?;
        let Some(session) = session.filter(|session| !session.is_empty()) else { return Ok(None); };
        let impression = uuid::Uuid::new_v4().to_string();
        let session_hash = hex::encode(Sha256::digest(session.as_bytes()));
        execute("INSERT INTO search_impressions (id,query_id,session_hash,{keys},created_at) VALUES (?,?,?,?,?)", vec![impression.clone().into(), query_id.into(), session_hash.into(), json(&shown)?.into(), now.into()]).await?;
        for shown in shown {
            Recommend::record_search_event(EventType::Impression, RecommendationTarget { index: index.clone(), key: shown.key }, actor(&viewer, &session)?, "search", &format!("search:{impression}:impression:{}", shown.position)).await?;
        }
        Ok(Some(impression))
    })).await
}

/// Session and shown-position validation prevents clients from inventing recommendation facts.
pub async fn click(impression: &str, key: &str, position: usize) -> Result<(), FrameworkError> {
    if !SearchConfig::current()?.analytics {
        return Err(FrameworkError::validation(
            "impression",
            "search analytics is disabled",
        ));
    }
    let session = crate::session::session()
        .filter(|session| !session.id.is_empty())
        .ok_or_else(|| {
            FrameworkError::validation("impression", "click requires the current server session")
        })?;
    let viewer = Viewer::current().await?;
    let impression = impression.to_string();
    let key = key.to_string();
    let oldest = cutoff(SearchConfig::current()?.analytics_retention_days)?;
    DB::transaction(move |_| Box::pin(async move {
        let row = rows("SELECT i.session_hash,i.{keys},q.{index} FROM search_impressions i JOIN search_queries q ON q.id=i.query_id WHERE i.id=? AND i.created_at>=? AND q.created_at>=?", vec![impression.clone().into(), oldest.into(), oldest.into()]).await?.into_iter().next().ok_or_else(|| FrameworkError::validation("impression", "impression is unknown or expired"))?;
        let current_hash = hex::encode(Sha256::digest(session.id.as_bytes()));
        if row.get_string("session_hash")? != current_hash { return Err(FrameworkError::validation("impression", "impression belongs to another session")); }
        let shown: Vec<ShownKey> = serde_json::from_str(&row.get_string("keys")?).map_err(|error| FrameworkError::database(format!("search impression keys: {error}")))?;
        if !shown.iter().any(|shown| shown.key == key && shown.position == position) { return Err(FrameworkError::validation("position", format!("key '{key}' was not shown at position {position}"))); }
        let position_value = i64::try_from(position).map_err(|_| FrameworkError::validation("position", "position exceeds supported range"))?;
        execute("INSERT INTO search_clicks (id,impression_id,{key},position,created_at) VALUES (?,?,?,?,?)", vec![uuid::Uuid::new_v4().to_string().into(), impression.clone().into(), key.clone().into(), position_value.into(), crate::clock::now().timestamp_micros().into()]).await?;
        Recommend::record_search_event(EventType::Click, RecommendationTarget { index: row.get_string("index")?, key }, actor(&viewer, &session.id)?, "search", &format!("search:{impression}:click:{position}")).await?;
        Ok(())
    })).await
}

/// Suggestions expose case-folded retained queries, ordered by frequency with stable ties.
pub async fn suggest(
    index: &str,
    prefix: &str,
    limit: usize,
) -> Result<Vec<String>, FrameworkError> {
    SearchRegistry::by_index(index)?;
    if limit > 1000 {
        return Err(FrameworkError::param("suggest limit must be at most 1000"));
    }
    if !SearchConfig::current()?.analytics || limit == 0 {
        return Ok(Vec::new());
    }
    let prefix = icu_casemap::CaseMapper::new().fold_string(prefix);
    let pattern = format!(
        "{}%",
        prefix
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_")
    );
    rows("SELECT query_fold,COUNT(*) AS frequency FROM search_queries WHERE {index}=? AND created_at>=? AND query_fold LIKE ? ESCAPE '!' GROUP BY query_fold ORDER BY frequency DESC,query_fold ASC LIMIT ?", vec![index.to_string().into(), cutoff(SearchConfig::current()?.analytics_retention_days)?.into(), pattern.into(), (limit as i64).into()]).await?.into_iter().map(|row| row.get_string("query_fold")).collect()
}

/// Retention removes linked child facts first so partial pruning cannot leave dangling clicks.
pub async fn prune() -> Result<u64, FrameworkError> {
    let oldest = cutoff(SearchConfig::current()?.analytics_retention_days)?;
    let removed = DB::transaction(move |_| Box::pin(async move {
        let clicks = execute("DELETE FROM search_clicks WHERE created_at<? OR impression_id IN (SELECT id FROM search_impressions WHERE created_at<? OR query_id IN (SELECT id FROM search_queries WHERE created_at<?))", vec![oldest.into(), oldest.into(), oldest.into()]).await?;
        let impressions = execute("DELETE FROM search_impressions WHERE created_at<? OR query_id IN (SELECT id FROM search_queries WHERE created_at<?)", vec![oldest.into(), oldest.into()]).await?;
        let queries = execute("DELETE FROM search_queries WHERE created_at<?", vec![oldest.into()]).await?;
        Ok(clicks + impressions + queries)
    })).await?;
    super::generations::prune_tombstones().await?;
    Ok(removed)
}

/// Query frequency is grouped by index so different search surfaces remain distinct.
#[derive(Debug, Serialize)]
pub struct QueryFrequency {
    /// Index identity avoids combining unrelated queries from different models.
    pub index: String,
    /// Unicode-folded text combines typing case without changing the original retained query.
    pub query: String,
    /// Search count captures demand independently of the number of returned hits.
    pub count: i64,
}
/// Click position counts show which shown ranks users select on each index.
#[derive(Debug, Serialize)]
pub struct ClickPosition {
    /// Index identity keeps rank distributions comparable within one surface.
    pub index: String,
    /// Zero-based positions match the exact list that was shown.
    pub position: i64,
    /// Every accepted click passed the session and shown-key checks.
    pub count: i64,
}
/// A bounded report exposes retained demand, misses and validated click ranks.
#[derive(Debug, Serialize)]
pub struct AnalyticsReport {
    /// Frequent queries identify the searches that matter most for tuning.
    pub top_queries: Vec<QueryFrequency>,
    /// Missing queries identify gaps in the retained index corpus.
    pub zero_result_queries: Vec<QueryFrequency>,
    /// Rank counts measure where accepted clicks concentrate.
    pub click_positions: Vec<ClickPosition>,
}

/// Reporting bounds the time window and validates index names before querying retained facts.
pub async fn report(index: Option<&str>, days: u32) -> Result<AnalyticsReport, FrameworkError> {
    if days == 0 {
        return Err(FrameworkError::param("analytics --days must be positive"));
    }
    if let Some(index) = index {
        SearchRegistry::by_index(index)?;
    }
    let days = u64::from(days).min(SearchConfig::current()?.analytics_retention_days);
    let index_clause = if index.is_some() {
        " AND {index}=?"
    } else {
        ""
    };
    let mut values: Vec<Value> = vec![cutoff(days)?.into()];
    if let Some(index) = index {
        values.push(index.to_string().into());
    }
    let frequencies = |rows: Vec<DynamicRow>| -> Result<Vec<QueryFrequency>, FrameworkError> {
        rows.into_iter()
            .map(|row| {
                Ok(QueryFrequency {
                    index: row.get_string("index")?,
                    query: row.get_string("query_fold")?,
                    count: row.get_int("frequency")?,
                })
            })
            .collect()
    };
    let base = format!(
        "SELECT {{index}},query_fold,COUNT(*) AS frequency FROM search_queries WHERE created_at>=?{index_clause}"
    );
    let top_queries = frequencies(rows(&format!("{base} GROUP BY {{index}},query_fold ORDER BY frequency DESC,{{index}},query_fold LIMIT 100"), values.clone()).await?)?;
    let zero_result_queries = frequencies(rows(&format!("{base} AND hits=0 GROUP BY {{index}},query_fold ORDER BY frequency DESC,{{index}},query_fold LIMIT 100"), values.clone()).await?)?;
    let click_index_clause = if index.is_some() {
        " AND q.{index}=?"
    } else {
        ""
    };
    let click_positions = rows(&format!("SELECT q.{{index}},c.position,COUNT(*) AS frequency FROM search_clicks c JOIN search_impressions i ON i.id=c.impression_id JOIN search_queries q ON q.id=i.query_id WHERE c.created_at>=?{click_index_clause} GROUP BY q.{{index}},c.position ORDER BY q.{{index}},c.position LIMIT 100"), values).await?.into_iter().map(|row| Ok(ClickPosition { index: row.get_string("index")?, position: row.get_int("position")?, count: row.get_int("frequency")? })).collect::<Result<Vec<_>, FrameworkError>>()?;
    Ok(AnalyticsReport {
        top_queries,
        zero_result_queries,
        click_positions,
    })
}
