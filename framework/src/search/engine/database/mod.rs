//! Persistent search uses the application's database and its native full-text index.
//!
//! Generation metadata serializes mutations across processes. Dense points carry
//! immutable generation and content identities, so a partial external write cannot
//! replace another generation's vectors or expose content absent from the side table.
mod filter;
mod mysql;
mod postgres;
mod sqlite;

use super::{
    EngineCandidate, EngineCapabilities, EngineResult, GenerationId, IndexedDocument, SearchEngine,
    WriteOutcome,
};
use crate::{
    DB, FrameworkError,
    search::{
        AttributeType, AttributeValue, EngineQuery, FacetDistribution, FacetOrder, FacetRange,
        FacetStats, FacetValue, Filter, IndexDescriptor, IndexSettings, SearchDocument, Total,
        Vocabulary, tokenize,
    },
    vector::{Vector, VectorItem, VectorStore},
};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbBackend, QueryResult, Statement, TransactionTrait,
    Value,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::OnceCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Sqlite,
    Mysql,
    MariaDb,
    Postgres,
}
impl Dialect {
    fn backend(self) -> DbBackend {
        match self {
            Self::Sqlite => DbBackend::Sqlite,
            Self::Postgres => DbBackend::Postgres,
            Self::Mysql | Self::MariaDb => DbBackend::MySql,
        }
    }
    fn mysql(self) -> bool {
        matches!(self, Self::Mysql | Self::MariaDb)
    }
}
struct Sql {
    dialect: Dialect,
    values: Vec<Value>,
}
impl Sql {
    fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            values: vec![],
        }
    }
    fn bind(&mut self, value: impl Into<Value>) -> String {
        self.values.push(value.into());
        if self.dialect == Dialect::Postgres {
            format!("${}", self.values.len())
        } else {
            "?".into()
        }
    }
    fn ident(&self, value: &str) -> String {
        crate::database::clauses::quote_identifier(self.dialect.backend(), value)
    }
    fn statement(self, text: impl Into<String>) -> Statement {
        Statement::from_sql_and_values(self.dialect.backend(), text, self.values)
    }
}
// Field-specific checks use the same word disjunction as each native renderer.
fn word_predicates(
    query: &crate::search::ExpandedQuery,
    mut render: impl FnMut(&crate::search::ExpandedTerm) -> Result<String, FrameworkError>,
) -> Result<Vec<String>, FrameworkError> {
    let mut required = Vec::new();
    let mut optional = Vec::new();
    for term in &query.terms {
        let clause = render(term)?;
        if term.required {
            required.push(clause);
        } else {
            optional.push(clause);
        }
    }
    if !optional.is_empty() {
        required.push(format!("({})", optional.join(" OR ")));
    }
    Ok(required)
}

#[cfg(test)]
fn expansion_fixture() -> crate::search::ExpandedQuery {
    crate::search::ExpandedQuery {
        terms: vec![crate::search::ExpandedTerm {
            original: "rust".into(),
            alternatives: vec!["rust".into(), "trust".into(), "rust language".into()],
            prefix: true,
            required: false,
            typo_alternatives: vec!["trust".into()],
        }],
        phrases: vec![vec!["database".into(), "engine".into()]],
        negatives: vec!["slow".into()],
        ..crate::search::ExpandedQuery::default()
    }
}

fn db_error(error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::database(format!("database search: {error}"))
}
fn identifier(name: &str) -> Result<(), FrameworkError> {
    crate::database::identifier::validate_savepoint_name(name).map(|_|()).map_err(|_|FrameworkError::param(format!("database search identifier '{name}': requires [A-Za-z_][A-Za-z0-9_]* and at most 64 bytes")))
}

fn table(index: &IndexDescriptor, generation: &GenerationId) -> Result<String, FrameworkError> {
    index.validate()?;
    identifier(index.name)?;
    identifier(generation.as_str())?;
    if generation.as_str() == "__control" {
        return Err(FrameworkError::param(format!(
            "search index {}: generation __control is reserved",
            index.name
        )));
    }
    let name = format!("search_{}_{}", index.name, generation.as_str());
    // Postgres truncates physical identifiers at 63 bytes. Refuse aliases on every backend.
    if name.len() + 6 > 63 {
        return Err(FrameworkError::param(format!(
            "search index {} generation {}: physical table name exceeds 57 bytes",
            index.name,
            generation.as_str()
        )));
    }
    let mut names = BTreeSet::from([
        "key".to_owned(),
        "source_version".into(),
        "content_hash".into(),
        "indexed_at".into(),
        "document_json".into(),
        "vector_id".into(),
        "vector_dense_json".into(),
        "vector_pending".into(),
        "search_vector".into(),
        "rowid".into(),
    ]);
    for attr in &index.attributes {
        identifier(&attr.name)?;
        let columns = if attr.ty == AttributeType::Geo {
            vec![format!("{}_lat", attr.name), format!("{}_lng", attr.name)]
        } else {
            vec![attr.name.clone()]
        };
        let mut columns = columns;
        if attr.ty == AttributeType::TextList && attr.searchable.is_some() {
            columns.push(format!("{}_text", attr.name));
        }
        if attr.facet && attr.ty != AttributeType::TextList {
            columns.push(format!("{}_facet", attr.name));
            if attr.ty == AttributeType::Float {
                columns.push(format!("{}_facet_label", attr.name));
            }
        }
        for col in columns {
            if col.len() > 63 || !names.insert(col.clone()) {
                return Err(FrameworkError::param(format!(
                    "search index {} attribute {}: reserved, repeated or too long column {col}",
                    index.name, attr.name
                )));
            }
        }
        if attr.ty == AttributeType::TextList && name.len() + attr.name.len() + 1 > 63 {
            return Err(FrameworkError::param(format!(
                "search index {} attribute {}: list table name exceeds 63 bytes",
                index.name, attr.name
            )));
        }
    }
    Ok(name)
}
// JSON remains the filter projection; native indexes receive unescaped list text.
fn native_attribute(index: &IndexDescriptor, name: &str) -> String {
    if index.attributes.iter().any(|attribute| {
        attribute.name == name
            && attribute.ty == AttributeType::TextList
            && attribute.searchable.is_some()
    }) {
        format!("{name}_text")
    } else {
        name.to_owned()
    }
}
fn original_attribute(index: &IndexDescriptor, name: &str) -> String {
    index
        .attributes
        .iter()
        .find(|attribute| {
            attribute.ty == AttributeType::TextList
                && attribute.searchable.is_some()
                && format!("{}_text", attribute.name) == name
        })
        .map_or_else(|| name.to_owned(), |attribute| attribute.name.clone())
}
fn native_settings(index: &IndexDescriptor, settings: &IndexSettings) -> IndexSettings {
    let mut native = settings.clone();
    for attribute in &mut native.index_time.searchable_attributes {
        attribute.name = native_attribute(index, &attribute.name);
    }
    native
}
fn native_query(
    index: &IndexDescriptor,
    query: &crate::search::ExpandedQuery,
) -> crate::search::ExpandedQuery {
    let mut native = query.clone();
    native.typo_disabled_attributes = query
        .typo_disabled_attributes
        .iter()
        .map(|name| native_attribute(index, name))
        .collect();
    native
}
fn searchable_text(value: &AttributeValue) -> Option<String> {
    match value {
        AttributeValue::Text(text) => Some(text.clone()),
        AttributeValue::List(values) => Some(
            values
                .iter()
                .filter_map(AttributeValue::as_text)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

async fn execute<C: ConnectionTrait>(
    conn: &C,
    text: String,
    sql: Sql,
) -> Result<u64, FrameworkError> {
    Ok(conn
        .execute_raw(sql.statement(text))
        .await
        .map_err(db_error)?
        .rows_affected())
}
async fn rows<C: ConnectionTrait>(
    conn: &C,
    text: String,
    sql: Sql,
) -> Result<Vec<QueryResult>, FrameworkError> {
    crate::render_cache::collector::observe_unobservable_read();
    conn.query_all_raw(sql.statement(text))
        .await
        .map_err(db_error)
}
fn get<T: sea_orm::TryGetable>(row: &QueryResult, col: &str) -> Result<T, FrameworkError> {
    row.try_get("", col).map_err(db_error)
}
fn decode(row: &QueryResult) -> Result<SearchDocument, FrameworkError> {
    let mut doc: SearchDocument =
        serde_json::from_str(&get::<String>(row, "document_json")?).map_err(db_error)?;
    doc.source_version = get(row, "source_version")?;
    Ok(doc)
}
fn now() -> i64 {
    crate::clock::now().timestamp()
}
fn vector_id(index: &str, generation: &GenerationId, doc: &IndexedDocument) -> String {
    let identity = serde_json::json!([
        index,
        generation.as_str(),
        doc.document.key,
        doc.content_hash
    ]);
    hex::encode(Sha256::digest(identity.to_string().as_bytes()))
}
fn vector_store(index: &str) -> Result<VectorStore, FrameworkError> {
    let name = crate::config::env_optional::<String>("SEARCH_VECTOR_STORE")
        .unwrap_or_else(|| index.into());
    Vector::store(&name)
}
fn words(document: &SearchDocument, settings: &IndexSettings) -> BTreeSet<String> {
    settings
        .index_time
        .searchable_attributes
        .iter()
        .filter_map(|a| document.attributes.get(&a.name))
        .flat_map(|value| match value {
            AttributeValue::Text(text) => tokenize(text),
            AttributeValue::List(values) => values
                .iter()
                .filter_map(AttributeValue::as_text)
                .flat_map(tokenize)
                .collect(),
            _ => vec![],
        })
        .collect()
}
fn escape_native(text: &str, settings: &IndexSettings) -> Result<String, FrameworkError> {
    crate::search::highlight::format_native(text, &settings.query, None)
}

/// Search on the configured DB connection without maintaining a second application pool.
#[derive(Default)]
pub struct DatabaseSearchEngine {
    mysql_dialect: OnceCell<Dialect>,
}
impl DatabaseSearchEngine {
    /// Defer backend discovery until the application's connection has been installed.
    pub fn new() -> Self {
        Self::default()
    }
    async fn dialect(&self) -> Result<Dialect, FrameworkError> {
        match DB::connection()?.get_database_backend() {
            DbBackend::Sqlite => Ok(Dialect::Sqlite),
            DbBackend::Postgres => Ok(Dialect::Postgres),
            DbBackend::MySql => Ok(*self
                .mysql_dialect
                .get_or_try_init(|| async {
                    let version: String = DB::scalar("SELECT VERSION()", vec![]).await?;
                    Ok::<_, FrameworkError>(if version.to_lowercase().contains("mariadb") {
                        Dialect::MariaDb
                    } else {
                        Dialect::Mysql
                    })
                })
                .await?),
            backend => Err(FrameworkError::param(format!(
                "database search: unsupported backend {backend:?}"
            ))),
        }
    }
    /// Expose the server's actual indexing limits instead of changing document tokens.
    pub async fn server_limits(&self) -> Result<BTreeMap<String, String>, FrameworkError> {
        let dialect = self.dialect().await?;
        if !dialect.mysql() {
            return Ok(BTreeMap::new());
        }
        let mut sql = Sql::new(dialect);
        let vars = [
            "innodb_ft_min_token_size",
            "ft_min_word_len",
            "innodb_ft_enable_stopword",
            "innodb_ft_server_stopword_table",
            "innodb_ft_user_stopword_table",
        ];
        let placeholders = vars
            .iter()
            .map(|v| sql.bind((*v).to_owned()))
            .collect::<Vec<_>>()
            .join(",");
        let conn = DB::connection()?;
        let result = rows(
            conn.inner(),
            format!("SHOW VARIABLES WHERE Variable_name IN ({placeholders})"),
            sql,
        )
        .await?;
        let mut limits: BTreeMap<String, String> = result
            .iter()
            .map(|r| Ok((get(r, "Variable_name")?, get(r, "Value")?)))
            .collect::<Result<_, FrameworkError>>()?;
        let table = [
            "innodb_ft_user_stopword_table",
            "innodb_ft_server_stopword_table",
        ]
        .iter()
        .find_map(|name| limits.get(*name).filter(|v| !v.is_empty()).cloned())
        .unwrap_or_else(|| "INFORMATION_SCHEMA.INNODB_FT_DEFAULT_STOPWORD".into());
        limits.insert("stopword_table".into(), table);
        Ok(limits)
    }
    /// Indexing maintenance calls this after the configured removal retention window.
    pub async fn prune_tombstones(&self, hours: u64) -> Result<u64, FrameworkError> {
        let seconds = hours
            .checked_mul(3600)
            .and_then(|v| i64::try_from(v).ok())
            .ok_or_else(|| {
                FrameworkError::param("SEARCH_TOMBSTONE_HOURS exceeds timestamp range")
            })?;
        let cutoff = now().checked_sub(seconds).ok_or_else(|| {
            FrameworkError::param("SEARCH_TOMBSTONE_HOURS exceeds timestamp range")
        })?;
        let mut sql = Sql::new(self.dialect().await?);
        let cutoff = sql.bind(cutoff);
        execute(
            DB::connection()?.inner(),
            format!("DELETE FROM search_tombstones WHERE removed_at <= {cutoff}"),
            sql,
        )
        .await
    }
    async fn context(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(Dialect, String, IndexSettings), FrameworkError> {
        let table = table(index, generation)?;
        let dialect = self.dialect().await?;
        let settings = self.read_settings(index, generation).await?;
        Ok((dialect, table, settings))
    }
    async fn lock(
        tx: &DatabaseTransaction,
        dialect: Dialect,
        index: &str,
        generation: &str,
    ) -> Result<(), FrameworkError> {
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.to_owned());
        let g = sql.bind(generation.to_owned());
        let count=execute(tx,format!("UPDATE search_generations SET write_count=write_count+0 WHERE index_name={i} AND generation={g}"),sql).await?;
        // MySQL reports changed rather than matched rows. Check existence under the lock.
        if count == 0 {
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.to_owned());
            let g = sql.bind(generation.to_owned());
            if rows(
                tx,
                format!(
                    "SELECT state FROM search_generations WHERE index_name={i} AND generation={g}"
                ),
                sql,
            )
            .await?
            .is_empty()
            {
                return Err(FrameworkError::param(format!(
                    "search index {index} generation {generation}: does not exist"
                )));
            }
        }
        Ok(())
    }
    async fn metadata<C: ConnectionTrait>(
        conn: &C,
        dialect: Dialect,
        index: &str,
        generation: &str,
    ) -> Result<QueryResult, FrameworkError> {
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.to_owned());
        let g = sql.bind(generation.to_owned());
        rows(
            conn,
            format!("SELECT * FROM search_generations WHERE index_name={i} AND generation={g}"),
            sql,
        )
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            FrameworkError::param(format!(
                "search index {index} generation {generation}: does not exist"
            ))
        })
    }
    fn lexical(
        index: &IndexDescriptor,
        dialect: Dialect,
        table: &str,
        settings: &IndexSettings,
        q: &EngineQuery,
        sql: &mut Sql,
    ) -> Result<String, FrameworkError> {
        let settings = native_settings(index, settings);
        let expanded = native_query(index, &q.expanded);
        match dialect {
            Dialect::Sqlite => sqlite::predicate(table, &settings, &expanded, sql),
            Dialect::Postgres => postgres::predicate(table, &settings, &expanded, sql),
            Dialect::Mysql | Dialect::MariaDb => mysql::predicate(table, &settings, &expanded, sql),
        }
    }
    fn predicate(
        index: &IndexDescriptor,
        dialect: Dialect,
        table: &str,
        settings: &IndexSettings,
        q: &EngineQuery,
        sql: &mut Sql,
    ) -> Result<String, FrameworkError> {
        let lexical = Self::lexical(index, dialect, table, settings, q, sql)?;
        let filter = if let Some(f) = &q.filter {
            filter::predicate(index, table, f, sql)?
        } else {
            "1=1".into()
        };
        Ok(format!("({lexical}) AND ({filter})"))
    }
    async fn count_on<C: ConnectionTrait>(
        conn: &C,
        index: &IndexDescriptor,
        dialect: Dialect,
        table: &str,
        settings: &IndexSettings,
        q: &EngineQuery,
    ) -> Result<Total, FrameworkError> {
        let mut sql = Sql::new(dialect);
        let predicate = Self::predicate(index, dialect, table, settings, q, &mut sql)?;
        let text = format!(
            "SELECT COUNT(*) AS n FROM {} AS s WHERE {predicate}",
            sql.ident(table)
        );
        let result = rows(conn, text, sql).await?;
        let n: i64 = get(
            result
                .first()
                .ok_or_else(|| db_error("count returned no row"))?,
            "n",
        )?;
        Ok(Total::Exact(n as u64))
    }
    async fn bump(
        tx: &DatabaseTransaction,
        dialect: Dialect,
        index: &str,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        execute(tx,format!("UPDATE search_generations SET write_count=write_count+1 WHERE index_name={i} AND generation={g}"),sql).await?;
        Ok(())
    }
    async fn update_words(
        tx: &DatabaseTransaction,
        dialect: Dialect,
        index: &str,
        generation: &GenerationId,
        old: BTreeSet<String>,
        new: BTreeSet<String>,
    ) -> Result<(), FrameworkError> {
        for word in old.difference(&new) {
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            let w = sql.bind(word.clone());
            execute(tx,format!("UPDATE search_vocabulary SET document_frequency=document_frequency-1 WHERE index_name={i} AND generation={g} AND word={w}"),sql).await?;
        }
        for word in new.difference(&old) {
            if word.chars().count() > 255 {
                return Err(FrameworkError::param(format!(
                    "search index {index}: vocabulary word exceeds 255 characters"
                )));
            }
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            let w = sql.bind(word.clone());
            let one = sql.bind(1i64);
            let text = format!(
                "INSERT INTO search_vocabulary (index_name,generation,word,document_frequency) VALUES ({i},{g},{w},{one}) ON DUPLICATE KEY UPDATE document_frequency=document_frequency+1"
            );
            execute(tx, text, sql).await?;
        }
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        execute(tx,format!("DELETE FROM search_vocabulary WHERE index_name={i} AND generation={g} AND document_frequency<=0"),sql).await?;
        Ok(())
    }
    async fn refresh_pg(
        tx: &DatabaseTransaction,
        index: &IndexDescriptor,
        generation: &GenerationId,
        table: &str,
    ) -> Result<(), FrameworkError> {
        let mut sql = Sql::new(Dialect::Postgres);
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        execute(
            tx,
            format!("DELETE FROM search_vocabulary WHERE index_name={i} AND generation={g}"),
            sql,
        )
        .await?;
        let mut sql = Sql::new(Dialect::Postgres);
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        let statement = sql.bind(format!("SELECT search_vector FROM {}", sql.ident(table)));
        execute(tx,format!("INSERT INTO search_vocabulary(index_name,generation,word,document_frequency) SELECT {i},{g},word,ndoc FROM ts_stat({statement})"),sql).await?;
        let mut sql = Sql::new(Dialect::Postgres);
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        execute(tx,format!("UPDATE search_generations SET refreshed_count=write_count WHERE index_name={i} AND generation={g}"),sql).await?;
        Ok(())
    }
    /// Refresh Postgres' materialized native vocabulary at generation build completion.
    pub async fn refresh_vocabulary(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        let (dialect, table, _) = self.context(index, generation).await?;
        if dialect != Dialect::Postgres {
            return Ok(());
        }
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, generation.as_str()).await?;
        ensure_ready(&tx, dialect, index.name, generation).await?;
        Self::refresh_pg(&tx, index, generation, &table).await?;
        tx.commit().await.map_err(db_error)
    }
    /// Retrieve FTS5 token-window snippets while escaping source text before adding tags.
    pub async fn snippets(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attribute: &str,
        tokens: usize,
    ) -> Result<BTreeMap<String, String>, FrameworkError> {
        let (dialect, table, settings) = self.context(index, generation).await?;
        if dialect != Dialect::Sqlite || tokens == 0 {
            return Err(FrameworkError::param(format!(
                "search index {} attribute {attribute}: snippets require SQLite and a positive token window",
                index.name
            )));
        }
        let mut sql = Sql::new(dialect);
        let native = native_settings(index, &settings);
        let expanded = native_query(index, &q.expanded);
        let attribute = native_attribute(index, attribute);
        let snippet = sqlite::snippet(&table, &native, &expanded, &attribute, tokens, &mut sql)?;
        let predicate = Self::predicate(index, dialect, &table, &settings, q, &mut sql)?;
        let limit = sql.bind(i64::try_from(q.depth).map_err(db_error)?);
        let text = format!(
            "SELECT s.{} AS document_key, {snippet} AS fragment FROM {} AS s WHERE {predicate} ORDER BY s.{} LIMIT {limit}",
            sql.ident("key"),
            sql.ident(&table),
            sql.ident("key")
        );
        rows(DB::connection()?.inner(), text, sql)
            .await?
            .iter()
            .map(|r| {
                Ok((
                    get(r, "document_key")?,
                    escape_native(&get::<String>(r, "fragment")?, &settings)?,
                ))
            })
            .collect()
    }
}

fn side_ddl(index: &IndexDescriptor, table: &str, dialect: Dialect) -> Vec<String> {
    let sql = Sql::new(dialect);
    // Query expansion folds case and accents; exact projections retain binary defaults.
    let native_collation = match dialect {
        Dialect::Mysql => " COLLATE utf8mb4_0900_ai_ci",
        Dialect::MariaDb => " COLLATE utf8mb4_unicode_ci",
        _ => "",
    };
    let mut cols = vec![
        format!("{} VARCHAR(255) PRIMARY KEY", sql.ident("key")),
        "source_version BIGINT NOT NULL".into(),
        "content_hash VARCHAR(64) NOT NULL".into(),
        "indexed_at BIGINT NOT NULL".into(),
        format!(
            "document_json {} NOT NULL",
            if dialect.mysql() { "LONGTEXT" } else { "TEXT" }
        ),
        "vector_id VARCHAR(64)".into(),
        format!(
            "vector_dense_json {}",
            if dialect.mysql() { "LONGTEXT" } else { "TEXT" }
        ),
        "vector_pending BOOLEAN NOT NULL DEFAULT FALSE".into(),
    ];
    for attr in &index.attributes {
        if attr.ty == AttributeType::Geo {
            for suffix in ["lat", "lng"] {
                cols.push(format!(
                    "{} DOUBLE PRECISION",
                    sql.ident(&format!("{}_{suffix}", attr.name))
                ));
            }
        } else {
            let ty = match attr.ty {
                AttributeType::Integer => "BIGINT",
                AttributeType::Float => "DOUBLE PRECISION",
                AttributeType::Bool => "BOOLEAN",
                _ => {
                    if dialect.mysql() {
                        "LONGTEXT"
                    } else {
                        "TEXT"
                    }
                }
            };
            let collation = if attr.ty == AttributeType::Text && attr.searchable.is_some() {
                native_collation
            } else {
                ""
            };
            cols.push(format!("{} {ty}{collation}", sql.ident(&attr.name)));
        }
    }
    for attr in index.attributes.iter().filter(|attribute| {
        attribute.ty == AttributeType::TextList && attribute.searchable.is_some()
    }) {
        cols.push(format!(
            "{} {}{native_collation}",
            sql.ident(&native_attribute(index, &attr.name)),
            if dialect.mysql() { "LONGTEXT" } else { "TEXT" }
        ));
    }
    for attr in index
        .attributes
        .iter()
        .filter(|a| a.facet && a.ty != AttributeType::TextList)
    {
        cols.push(format!(
            "{} {}",
            sql.ident(&format!("{}_facet", attr.name)),
            if dialect.mysql() { "LONGTEXT" } else { "TEXT" }
        ));
    }
    for attr in index
        .attributes
        .iter()
        .filter(|a| a.facet && a.ty == AttributeType::Float)
    {
        cols.push(format!(
            "{} {}",
            sql.ident(&format!("{}_facet_label", attr.name)),
            if dialect.mysql() { "LONGTEXT" } else { "TEXT" }
        ));
    }
    let options = match dialect {
        Dialect::Mysql => " CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_bin",
        Dialect::MariaDb => " CHARACTER SET utf8mb4 COLLATE utf8mb4_nopad_bin",
        _ => "",
    };
    let mut ddl = vec![format!(
        "CREATE TABLE {} ({}){options}",
        sql.ident(table),
        cols.join(",")
    )];
    for (n, attr) in index.attributes.iter().enumerate() {
        if attr.ty == AttributeType::TextList {
            let list = format!("{table}_{}", attr.name);
            ddl.push(format!("CREATE TABLE {} (document_key VARCHAR(255) NOT NULL, position BIGINT NOT NULL, value {} NOT NULL, prefix_value {} NOT NULL, PRIMARY KEY(document_key,position)){options}",sql.ident(&list),if dialect.mysql(){"LONGTEXT"}else{"TEXT"},if dialect.mysql(){"LONGTEXT"}else{"TEXT"}));
            let value = if dialect.mysql() {
                "value(191)"
            } else {
                "value"
            };
            let name = hex::encode(Sha256::digest(list.as_bytes()));
            ddl.push(format!(
                "CREATE INDEX {} ON {} ({value}, document_key)",
                sql.ident(&format!("si_{}", &name[..24])),
                sql.ident(&list)
            ));
        } else if attr.filterable || attr.sortable {
            let columns = if attr.ty == AttributeType::Geo {
                vec![format!("{}_lat", attr.name), format!("{}_lng", attr.name)]
            } else {
                vec![attr.name.clone()]
            };
            for (part, col) in columns.iter().enumerate() {
                let hash = hex::encode(Sha256::digest(format!("{table}_{n}_{part}").as_bytes()));
                let suffix = if dialect.mysql()
                    && matches!(attr.ty, AttributeType::Text | AttributeType::DateTime)
                {
                    "(191)"
                } else {
                    ""
                };
                ddl.push(format!(
                    "CREATE INDEX {} ON {} ({}{suffix})",
                    sql.ident(&format!("si_{}", &hash[..24])),
                    sql.ident(table),
                    sql.ident(col)
                ));
            }
        }
    }
    ddl
}
async fn drop_tables<C: ConnectionTrait>(
    conn: &C,
    index: &IndexDescriptor,
    table: &str,
    dialect: Dialect,
) -> Result<(), FrameworkError> {
    let sql = Sql::new(dialect);
    let mut tables = Vec::new();
    if dialect == Dialect::Sqlite {
        tables.extend([format!("{table}_vocab"), format!("{table}_fts")]);
    }
    tables.extend(
        index
            .attributes
            .iter()
            .filter(|a| a.ty == AttributeType::TextList)
            .map(|a| format!("{table}_{}", a.name)),
    );
    tables.push(table.to_owned());
    for name in tables {
        conn.execute_unprepared(&format!("DROP TABLE IF EXISTS {}", sql.ident(&name)))
            .await
            .map_err(db_error)?;
    }
    drop_pinned_configuration(conn, table, dialect).await
}
/// A Postgres generation built with the server's language owns a copy of the
/// server's text search configuration (see `postgres::pinned_configuration`);
/// it goes with the generation's table.
async fn drop_pinned_configuration<C: ConnectionTrait>(
    conn: &C,
    table: &str,
    dialect: Dialect,
) -> Result<(), FrameworkError> {
    if dialect != Dialect::Postgres {
        return Ok(());
    }
    let sql = Sql::new(dialect);
    let name = postgres::pinned_configuration(table)?;
    conn.execute_unprepared(&format!(
        "DROP TEXT SEARCH CONFIGURATION IF EXISTS {}",
        sql.ident(&name)
    ))
    .await
    .map_err(db_error)?;
    Ok(())
}

const CREATION_LEASE_SECONDS: i64 = 60;

async fn renew_creation<C: ConnectionTrait>(
    conn: &C,
    dialect: Dialect,
    index: &str,
    generation: &str,
    owner: &str,
) -> Result<(), FrameworkError> {
    let mut sql = Sql::new(dialect);
    let expires = sql.bind(crate::clock::now().timestamp() + CREATION_LEASE_SECONDS);
    let i = sql.bind(index.to_owned());
    let g = sql.bind(generation.to_owned());
    let token = sql.bind(owner.to_owned());
    execute(conn, format!("UPDATE search_generations SET creation_expires_at={expires} WHERE index_name={i} AND generation={g} AND state='creating' AND creation_owner={token}"), sql).await?;
    let metadata = DatabaseSearchEngine::metadata(conn, dialect, index, generation).await?;
    if get::<String>(&metadata, "state")? != "creating"
        || get::<Option<String>>(&metadata, "creation_owner")?.as_deref() != Some(owner)
    {
        return Err(db_error(format!(
            "index {index} generation {generation}: creation lease was replaced"
        )));
    }
    Ok(())
}

#[async_trait]
impl SearchEngine for DatabaseSearchEngine {
    fn name(&self) -> &str {
        "database"
    }
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            native_highlight: true,
            native_crop: true,
            phrase: true,
            prefix: true,
            semantic: true,
            facets: true,
            geo: true,
            versioned: true,
            generations: true,
            sparse: false,
        }
    }
    async fn current_generation(
        &self,
        index: &str,
    ) -> Result<Option<GenerationId>, FrameworkError> {
        identifier(index)?;
        let mut sql = Sql::new(self.dialect().await?);
        let i = sql.bind(index.to_owned());
        let active = sql.bind("current".to_owned());
        let result = rows(
            DB::connection()?.inner(),
            format!(
                "SELECT generation FROM search_generations WHERE index_name={i} AND state IN ({active},'active')"
            ),
            sql,
        )
        .await?;
        if result.len() > 1 {
            return Err(FrameworkError::database(format!(
                "search index {index}: multiple active generations"
            )));
        }
        result
            .first()
            .map(|r| GenerationId::new(get::<String>(r, "generation")?))
            .transpose()
    }
    async fn generation_states(
        &self,
        index: &str,
    ) -> Result<Vec<(GenerationId, String)>, FrameworkError> {
        identifier(index)?;
        let mut sql = Sql::new(self.dialect().await?);
        let i = sql.bind(index.to_owned());
        rows(DB::connection()?.inner(), format!("SELECT generation,state FROM search_generations WHERE index_name={i} AND generation!='__control' ORDER BY generation"), sql).await?.iter().map(|row| Ok((GenerationId::new(get::<String>(row,"generation")?)?, get(row,"state")?))).collect()
    }
    async fn create_generation(
        &self,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        let table = table(index, generation)?;
        super::collection::validate_settings(index, settings)?;
        let dialect = self.dialect().await?;
        let native_settings = native_settings(index, settings);
        let native = match dialect {
            Dialect::Sqlite => sqlite::ddl(&table, &native_settings)?,
            Dialect::Postgres => postgres::ddl(&table, &native_settings)?,
            _ => mysql::ddl(&table, &native_settings)?,
        };
        let conn = DB::connection()?;
        // The permanent control row provides a lock even before an active generation exists.
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind("__control".to_owned());
        let state = sql.bind("control".to_owned());
        let empty = sql.bind("{}".to_owned());
        let digest = sql.bind(String::new());
        let conflict = if dialect.mysql() {
            "ON DUPLICATE KEY UPDATE generation=generation"
        } else {
            "ON CONFLICT (index_name,generation) DO NOTHING"
        };
        execute(conn.inner(),format!("INSERT INTO search_generations(index_name,generation,state,settings_json,digest) VALUES ({i},{g},{state},{empty},{digest}) {conflict}"),sql).await?;
        let owner = uuid::Uuid::new_v4().to_string();
        let now = crate::clock::now().timestamp();
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, "__control").await?;
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        let existing = rows(&tx, format!("SELECT state,digest,creation_expires_at FROM search_generations WHERE index_name={i} AND generation={g}"), sql).await?;
        let recovering = if let Some(row) = existing.first() {
            if get::<String>(row, "state")? != "creating"
                || get::<String>(row, "digest")? != settings.index_time.digest()
            {
                return Err(db_error(format!(
                    "index {} generation {}: already exists with a different state or settings",
                    index.name,
                    generation.as_str()
                )));
            }
            if get::<Option<i64>>(row, "creation_expires_at")?.is_some_and(|expiry| expiry > now) {
                return Err(db_error(format!(
                    "index {} generation {}: creation lease is still active; retry after its expiry",
                    index.name,
                    generation.as_str()
                )));
            }
            let mut sql = Sql::new(dialect);
            let token = sql.bind(owner.clone());
            let expiry = sql.bind(now + CREATION_LEASE_SECONDS);
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            execute(&tx, format!("UPDATE search_generations SET creation_owner={token},creation_expires_at={expiry} WHERE index_name={i} AND generation={g}"), sql).await?;
            true
        } else {
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            let state = sql.bind("creating".to_owned());
            let settings_json = sql.bind(serde_json::to_string(settings).map_err(db_error)?);
            let digest = sql.bind(settings.index_time.digest());
            let model = sql.bind(settings.index_time.model.clone());
            let dimensions = sql.bind(
                settings
                    .index_time
                    .dimensions
                    .map(i64::try_from)
                    .transpose()
                    .map_err(db_error)?,
            );
            let token = sql.bind(owner.clone());
            let expiry = sql.bind(now + CREATION_LEASE_SECONDS);
            execute(&tx,format!("INSERT INTO search_generations(index_name,generation,state,settings_json,digest,model,dimensions,creation_owner,creation_expires_at) VALUES ({i},{g},{state},{settings_json},{digest},{model},{dimensions},{token},{expiry})"),sql).await?;
            false
        };
        tx.commit().await.map_err(db_error)?;
        let mut created_tables = Vec::new();
        let build=async {
            let ddl = async {
                if recovering {
                    renew_creation(conn.inner(),dialect,index.name,generation.as_str(),&owner).await?;
                    drop_tables(conn.inner(),index,&table,dialect).await?;
                }
                for ddl in side_ddl(index,&table,dialect).into_iter().chain(native) {
                    renew_creation(conn.inner(),dialect,index.name,generation.as_str(),&owner).await?;
                    conn.inner().execute_unprepared(&ddl).await.map_err(db_error)?;
                    if let Some(name)=created_table(&ddl){created_tables.push(name);}
                }
                Ok::<_,FrameworkError>(())
            };
            // Empty-table DDL is normally quick, but a live creator waiting on the
            // database must not lose its lease just because one statement is slow.
            let heartbeat = async || -> Result<(), FrameworkError> {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    renew_creation(conn.inner(),dialect,index.name,generation.as_str(),&owner).await?;
                }
            };
            tokio::select! {
                result = ddl => result?,
                result = heartbeat() => result?,
            }
            let tx = conn.inner().begin().await.map_err(db_error)?;
            Self::lock(&tx,dialect,index.name,"__control").await?;
            renew_creation(&tx,dialect,index.name,generation.as_str(),&owner).await?;
            let mut sql=Sql::new(dialect);let state=sql.bind("building".to_owned());let i=sql.bind(index.name.to_owned());let g=sql.bind(generation.as_str().to_owned());let token=sql.bind(owner.clone());
            if execute(&tx,format!("UPDATE search_generations SET state={state},creation_owner=NULL,creation_expires_at=NULL WHERE index_name={i} AND generation={g} AND state='creating' AND creation_owner={token}"),sql).await? != 1 {return Err(db_error(format!("search index {}: generation creation lease was replaced",index.name)));}
            tx.commit().await.map_err(db_error)?;
            Ok::<_,FrameworkError>(())
        }.await;
        if let Err(error) = build {
            // A superseded creator must never tear down its successor's storage.
            if renew_creation(
                conn.inner(),
                dialect,
                index.name,
                generation.as_str(),
                &owner,
            )
            .await
            .is_err()
            {
                return Err(error);
            }
            async {
                drop_created(conn.inner(), &created_tables, dialect).await?;
                drop_pinned_configuration(conn.inner(), &table, dialect).await
            }
            .await
            .map_err(|cleanup| {
                db_error(format!(
                    "index {} generation {}: {error}; cleanup failed: {cleanup}",
                    index.name,
                    generation.as_str()
                ))
            })?;
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            let token = sql.bind(owner);
            execute(
                conn.inner(),
                format!("DELETE FROM search_generations WHERE index_name={i} AND generation={g} AND state='creating' AND creation_owner={token}"),
                sql,
            )
            .await?;
            return Err(db_error(format!(
                "index {} generation {}: {error}",
                index.name,
                generation.as_str()
            )));
        }
        Ok(())
    }
    async fn switch_generation(
        &self,
        index: &IndexDescriptor,
        from: Option<GenerationId>,
        to: GenerationId,
    ) -> Result<(), FrameworkError> {
        let table = table(index, &to)?;
        let dialect = self.dialect().await?;
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, "__control").await?;
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.name.to_owned());
        let active = sql.bind("current".to_owned());
        let current = rows(
            &tx,
            format!(
                "SELECT generation FROM search_generations WHERE index_name={i} AND state IN ({active},'active')"
            ),
            sql,
        )
        .await?
        .into_iter()
        .next()
        .map(|r| get::<String>(&r, "generation"))
        .transpose()?;
        if current.as_deref() != from.as_ref().map(GenerationId::as_str) {
            return Err(FrameworkError::param(format!(
                "search index {}: current generation changed",
                index.name
            )));
        }
        Self::lock(&tx, dialect, index.name, to.as_str()).await?;
        let state: String = get(
            &Self::metadata(&tx, dialect, index.name, to.as_str()).await?,
            "state",
        )?;
        if !matches!(
            state.as_str(),
            "current" | "building" | "retired" | "active" | "inactive"
        ) {
            return Err(db_error(format!(
                "search index {} generation {}: state {state} cannot serve readers",
                index.name,
                to.as_str()
            )));
        }
        let mut pending_sql = Sql::new(dialect);
        let name = pending_sql.bind(index.name.to_owned());
        let pending = rows(&tx, format!("SELECT COUNT(*) AS n FROM search_jobs_pending WHERE index_name={name} AND completed=FALSE"), pending_sql).await?;
        if pending
            .first()
            .map(|row| get::<i64>(row, "n"))
            .transpose()?
            .unwrap_or(0)
            != 0
        {
            return Err(db_error(format!(
                "search index {}: pending jobs prevent generation publication",
                index.name
            )));
        }
        let mut intent_sql = Sql::new(dialect);
        let name = intent_sql.bind(index.name.to_owned());
        let intents = rows(
            &tx,
            format!("SELECT COUNT(*) AS n FROM search_job_intents WHERE index_name={name}"),
            intent_sql,
        )
        .await?;
        if intents
            .first()
            .map(|row| get::<i64>(row, "n"))
            .transpose()?
            .unwrap_or(0)
            != 0
        {
            return Err(db_error(format!(
                "search index {}: pending jobs prevent generation publication",
                index.name
            )));
        }
        if dialect == Dialect::Postgres {
            Self::refresh_pg(&tx, index, &to, &table).await?;
        }
        let mut sql = Sql::new(dialect);
        let target = sql.bind(to.as_str().to_owned());
        let active = sql.bind("current".to_owned());
        let inactive = sql.bind("retired".to_owned());
        let i = sql.bind(index.name.to_owned());
        let old = sql.bind("current".to_owned());
        let target2 = sql.bind(to.as_str().to_owned());
        execute(&tx,format!("UPDATE search_generations SET state=CASE WHEN generation={target} THEN {active} ELSE {inactive} END WHERE index_name={i} AND (state IN ({old},'active') OR generation={target2})"),sql).await?;
        let mut sql = Sql::new(dialect);
        let i = sql.bind(index.name.to_owned());
        execute(&tx,format!("UPDATE search_generations SET building_generation=NULL,building_digest=NULL WHERE index_name={i} AND generation='__control'"),sql).await?;
        tx.commit().await.map_err(db_error)
    }
    async fn drop_generation(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<(), FrameworkError> {
        let table = table(index, generation)?;
        let dialect = self.dialect().await?;
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, "__control").await?;
        Self::lock(&tx, dialect, index.name, generation.as_str()).await?;
        let state: String = get(
            &Self::metadata(&tx, dialect, index.name, generation.as_str()).await?,
            "state",
        )?;
        if matches!(state.as_str(), "current" | "active" | "creating") {
            return Err(FrameworkError::param(format!(
                "search index {}: cannot drop active or building generation {}",
                index.name,
                generation.as_str()
            )));
        }
        let mut pending_sql = Sql::new(dialect);
        let i = pending_sql.bind(index.name.to_owned());
        let g = pending_sql.bind(generation.as_str().to_owned());
        let pending = rows(&tx,format!("SELECT COUNT(*) AS n FROM search_jobs_pending WHERE index_name={i} AND generation={g} AND completed=FALSE"),pending_sql).await?;
        if pending
            .first()
            .map(|row| get::<i64>(row, "n"))
            .transpose()?
            .unwrap_or(0)
            != 0
        {
            return Err(db_error(format!(
                "search index {} generation {}: pending jobs prevent dropping",
                index.name,
                generation.as_str()
            )));
        }
        let mut sql = Sql::new(dialect);
        let dropping = sql.bind("dropping".to_owned());
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        execute(&tx,format!("UPDATE search_generations SET state={dropping} WHERE index_name={i} AND generation={g}"),sql).await?;
        tx.commit().await.map_err(db_error)?;
        // MySQL DDL commits implicitly. Mark the generation unavailable before issuing it.
        self.drain_vectors(index, generation, &table, dialect)
            .await?;
        let sql = Sql::new(dialect);
        let text = format!(
            "SELECT vector_id FROM {} WHERE vector_id IS NOT NULL",
            sql.ident(&table)
        );
        let exists = sea_orm_migration::SchemaManager::new(conn.inner())
            .has_table(&table)
            .await
            .map_err(db_error)?;
        let existing = if exists {
            rows(conn.inner(), text, sql).await?
        } else {
            vec![]
        };
        if !existing.is_empty() {
            vector_store(index.name)?
                .delete(
                    existing
                        .iter()
                        .map(|r| get::<String>(r, "vector_id"))
                        .collect::<Result<Vec<_>, _>>()?,
                )
                .await?;
        }
        drop_tables(conn.inner(), index, &table, dialect).await?;
        for metadata in [
            "search_vocabulary",
            "search_tombstones",
            "search_generations",
        ] {
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            execute(
                conn.inner(),
                format!("DELETE FROM {metadata} WHERE index_name={i} AND generation={g}"),
                sql,
            )
            .await?;
        }
        Ok(())
    }
    async fn write_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        settings: &IndexSettings,
    ) -> Result<(), FrameworkError> {
        super::collection::validate_settings(index, settings)?;
        let (dialect, _, old) = self.context(index, generation).await?;
        if old.index_time.digest() != settings.index_time.digest() {
            return Err(FrameworkError::param(format!(
                "search index {}: index-time settings require a new generation",
                index.name
            )));
        }
        let mut sql = Sql::new(dialect);
        let value = sql.bind(serde_json::to_string(settings).map_err(db_error)?);
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        execute(DB::connection()?.inner(),format!("UPDATE search_generations SET settings_json={value} WHERE index_name={i} AND generation={g}"),sql).await?;
        Ok(())
    }
    async fn record_flush(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
    ) -> Result<(), FrameworkError> {
        table(index, generation)?;
        let dialect = self.dialect().await?;
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, generation.as_str()).await?;
        ensure_ready(&tx, dialect, index.name, generation).await?;
        let metadata = Self::metadata(&tx, dialect, index.name, generation.as_str()).await?;
        if get::<Option<i64>>(&metadata, "flush_version")?.is_none_or(|previous| cutoff >= previous)
        {
            let mut sql = Sql::new(dialect);
            let version = sql.bind(cutoff);
            let removed = sql.bind(now());
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            execute(&tx, format!("UPDATE search_generations SET flush_version={version},flush_removed_at={removed} WHERE index_name={i} AND generation={g}"), sql).await?;
        }
        tx.commit().await.map_err(db_error)
    }
    async fn keys_before_version(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        cutoff: i64,
        limit: usize,
    ) -> Result<Vec<(String, i64)>, FrameworkError> {
        let (dialect, table, _) = self.context(index, generation).await?;
        let mut sql = Sql::new(dialect);
        let key = sql.ident("key");
        let table = sql.ident(&table);
        let cutoff = sql.bind(cutoff);
        let limit = sql.bind(i64::try_from(limit).map_err(db_error)?);
        rows(DB::connection()?.inner(), format!("SELECT {key},source_version FROM {table} WHERE source_version<={cutoff} ORDER BY {key} LIMIT {limit}"), sql)
            .await?
            .iter()
            .map(|row| Ok((get(row, "key")?, get(row, "source_version")?)))
            .collect()
    }
    async fn read_settings(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<IndexSettings, FrameworkError> {
        table(index, generation)?;
        let dialect = self.dialect().await?;
        let metadata = Self::metadata(
            DB::connection()?.inner(),
            dialect,
            index.name,
            generation.as_str(),
        )
        .await?;
        let state: String = get(&metadata, "state")?;
        if !matches!(
            state.as_str(),
            "current" | "building" | "retired" | "active" | "inactive"
        ) {
            return Err(FrameworkError::database(format!(
                "search index {} generation {}: unavailable state {state}",
                index.name,
                generation.as_str()
            )));
        }
        serde_json::from_str(&get::<String>(&metadata, "settings_json")?).map_err(db_error)
    }
    async fn index(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        docs: &[IndexedDocument],
    ) -> Result<WriteOutcome, FrameworkError> {
        let (dialect, table, settings) = self.context(index, generation).await?;
        for doc in docs {
            validate_document(index, &settings, doc)?;
        }
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, generation.as_str()).await?;
        let mut outcome = WriteOutcome::default();
        ensure_ready(&tx, dialect, index.name, generation).await?;
        let metadata = Self::metadata(&tx, dialect, index.name, generation.as_str()).await?;
        let retention = crate::search::SearchConfig::current()?
            .tombstone_hours
            .checked_mul(3600)
            .and_then(|seconds| i64::try_from(seconds).ok())
            .ok_or_else(|| {
                FrameworkError::param("SEARCH_TOMBSTONE_HOURS exceeds timestamp range")
            })?;
        let expires_before = now().checked_sub(retention).ok_or_else(|| {
            FrameworkError::param("SEARCH_TOMBSTONE_HOURS exceeds timestamp range")
        })?;
        let flush = get::<Option<i64>>(&metadata, "flush_version")?
            .zip(get::<Option<i64>>(&metadata, "flush_removed_at")?)
            .filter(|(_, removed)| *removed > expires_before)
            .map(|(version, _)| version);
        let scope = WriteContext {
            index,
            generation,
            dialect,
            table: &table,
            settings: &settings,
        };
        for doc in docs {
            if flush.is_some_and(|cutoff| doc.document.source_version <= cutoff) {
                outcome.skipped_tombstoned += 1;
                continue;
            }
            write_document(&tx, &scope, doc, &mut outcome).await?;
        }
        if dialect == Dialect::Postgres {
            let metadata = Self::metadata(&tx, dialect, index.name, generation.as_str()).await?;
            if get::<i64>(&metadata, "write_count")? - get::<i64>(&metadata, "refreshed_count")?
                >= vocabulary_step()?
            {
                Self::refresh_pg(&tx, index, generation, &table).await?;
            }
        }
        tx.commit().await.map_err(db_error)?;
        self.flush_vectors(index, generation, &table, dialect)
            .await?;
        self.drain_vectors(index, generation, &table, dialect)
            .await?;
        Ok(outcome)
    }
    async fn remove(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        keys: &[(String, i64)],
    ) -> Result<WriteOutcome, FrameworkError> {
        let (dialect, table, settings) = self.context(index, generation).await?;
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, generation.as_str()).await?;
        ensure_ready(&tx, dialect, index.name, generation).await?;
        let mut outcome = WriteOutcome::default();
        for (key, version) in keys {
            let existing = find_document(&tx, dialect, &table, key).await?;
            let removed = find_tombstone(&tx, dialect, index.name, generation, key).await?;
            if removed.is_some_and(|v| v > *version)
                || existing
                    .as_ref()
                    .map(|r| get::<i64>(r, "source_version"))
                    .transpose()?
                    .is_some_and(|v| v > *version)
            {
                outcome.skipped_older += 1;
                continue;
            }
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            let k = sql.bind(key.clone());
            execute(&tx,format!("DELETE FROM search_tombstones WHERE index_name={i} AND generation={g} AND document_key={k}"),sql).await?;
            let mut sql = Sql::new(dialect);
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            let k = sql.bind(key.clone());
            let v = sql.bind(*version);
            let t = sql.bind(now());
            execute(&tx,format!("INSERT INTO search_tombstones(index_name,generation,document_key,source_version,removed_at) VALUES({i},{g},{k},{v},{t})"),sql).await?;
            clear_lists(&tx, index, dialect, &table, key).await?;
            if let Some(existing) = &existing {
                if dialect.mysql() {
                    Self::update_words(
                        &tx,
                        dialect,
                        index.name,
                        generation,
                        words(&decode(existing)?, &settings),
                        BTreeSet::new(),
                    )
                    .await?;
                }
                if let Some(id) = get::<Option<String>>(existing, "vector_id")? {
                    queue_vector(&tx, dialect, index.name, generation, id).await?;
                }
            }
            let mut sql = Sql::new(dialect);
            let k = sql.bind(key.clone());
            let text = format!(
                "DELETE FROM {} WHERE {}={k}",
                sql.ident(&table),
                sql.ident("key")
            );
            execute(&tx, text, sql).await?;
            Self::bump(&tx, dialect, index.name, generation).await?;
            outcome.written += 1;
        }
        if dialect == Dialect::Postgres {
            Self::refresh_pg(&tx, index, generation, &table).await?;
        }
        tx.commit().await.map_err(db_error)?;
        self.drain_vectors(index, generation, &table, dialect)
            .await?;
        Ok(outcome)
    }
    async fn query(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<EngineResult, FrameworkError> {
        let (dialect, table, settings) = self.context(index, generation).await?;
        super::collection::validate_query(index, q)?;
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        let total = Self::count_on(&tx, index, dialect, &table, &settings, q).await?;
        let native_settings = native_settings(index, &settings);
        let expanded = native_query(index, &q.expanded);
        let crop = q
            .crop
            .iter()
            .map(|(name, length)| (native_attribute(index, name), *length))
            .collect();
        let mut sql = Sql::new(dialect);
        let ranked = if dialect == Dialect::Sqlite {
            sqlite::ranked_matches(&table, &native_settings, &expanded, &mut sql)?
        } else {
            None
        };
        let score = match dialect {
            Dialect::Sqlite if ranked.is_some() => "search_ranked.score".into(),
            Dialect::Sqlite => "0.0".into(),
            Dialect::Postgres => postgres::score(&table, &native_settings, &expanded, &mut sql)?,
            _ => mysql::score(&table, &native_settings, &expanded, &mut sql)?,
        };
        let score = if dialect == Dialect::Postgres {
            format!("CAST(({score}) AS DOUBLE PRECISION)")
        } else {
            score
        };
        let native = if q.highlights || !q.crop.is_empty() {
            match dialect {
                Dialect::Sqlite => {
                    sqlite::highlights(&table, &native_settings, &expanded, &crop, &mut sql)?
                }
                Dialect::Postgres => {
                    postgres::highlights(&table, &native_settings, &expanded, &mut sql)?
                }
                _ => vec![],
            }
        } else {
            vec![]
        };
        let native = native
            .into_iter()
            .map(|(name, expression)| (original_attribute(index, &name), expression))
            .collect::<Vec<_>>();
        let formatted = native
            .iter()
            .enumerate()
            .map(|(n, (_, expression))| format!(", {expression} AS h{n}"))
            .collect::<String>();
        let predicate = Self::predicate(index, dialect, &table, &settings, q, &mut sql)?;
        let mut order = filter::order(index, &q.sort, &mut sql)?;
        let collation = if dialect == Dialect::Postgres {
            " COLLATE \"C\""
        } else {
            ""
        };
        order.extend([
            "score DESC".into(),
            format!("s.{}{collation} ASC", sql.ident("key")),
        ]);
        let limit = sql.bind(i64::try_from(q.depth).map_err(db_error)?);
        let (with, join) = match ranked {
            Some(ranked) => (
                format!("WITH search_ranked AS MATERIALIZED ({ranked}) "),
                " JOIN search_ranked ON search_ranked.rowid=s.rowid",
            ),
            None => (String::new(), ""),
        };
        let text = format!(
            "{with}SELECT s.document_json,s.source_version,{score} AS score{formatted} FROM {} AS s{join} WHERE {predicate} ORDER BY {} LIMIT {limit}",
            sql.ident(&table),
            order.join(",")
        );
        let result = rows(&tx, text, sql).await?;
        let mut candidates = Vec::new();
        for row in result {
            let document = decode(&row)?;
            let mut highlights = BTreeMap::new();
            if q.highlights || !q.crop.is_empty() {
                if dialect.mysql() {
                    for attr in &settings.index_time.searchable_attributes {
                        if let Some(text) = document
                            .attributes
                            .get(&attr.name)
                            .and_then(searchable_text)
                        {
                            highlights.insert(
                                attr.name.clone(),
                                crate::search::highlight(
                                    &text,
                                    &attribute_query(&q.expanded, &attr.name),
                                    &settings.query,
                                    q.crop.get(&attr.name).copied(),
                                )?,
                            );
                        }
                    }
                } else {
                    for (n, (name, _)) in native.iter().enumerate() {
                        if let Some(fragment) = get::<Option<String>>(&row, &format!("h{n}"))? {
                            highlights.insert(
                                name.clone(),
                                crate::search::highlight::format_native(
                                    &fragment,
                                    &settings.query,
                                    q.crop.get(name).copied(),
                                )?,
                            );
                        }
                    }
                }
            }
            candidates.push(EngineCandidate {
                document,
                score: get(&row, "score")?,
                highlights: (q.highlights || !q.crop.is_empty()).then_some(highlights),
            });
        }
        tx.commit().await.map_err(db_error)?;
        Ok(EngineResult { candidates, total })
    }
    async fn semantic(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        vector: &[f32],
        filter: Option<&Filter>,
        depth: usize,
    ) -> Result<EngineResult, FrameworkError> {
        let (dialect, table, settings) = self.context(index, generation).await?;
        validate_dense(index, &settings, vector)?;
        if let Some(f) = filter {
            f.validate(index)?;
        }
        let limit = depth.checked_mul(4).ok_or_else(|| {
            FrameworkError::param(format!(
                "search index {}: semantic depth overflow",
                index.name
            ))
        })?;
        if limit == 0 {
            return Ok(EngineResult {
                candidates: vec![],
                total: Total::Estimated(0),
            });
        }
        let matches = vector_store(index.name)?
            .similar(vector.to_vec(), limit)
            .await?;
        let mut sql = Sql::new(dialect);
        let mut selected = Vec::new();
        for m in &matches {
            if m.metadata.get("index").and_then(serde_json::Value::as_str) == Some(index.name)
                && m.metadata
                    .get("generation")
                    .and_then(serde_json::Value::as_str)
                    == Some(generation.as_str())
                && let Some(key) = m.metadata.get("key").and_then(serde_json::Value::as_str)
            {
                selected.push((m.id.clone(), m.score, key.to_owned()));
            }
        }
        if selected.is_empty() {
            return Ok(EngineResult {
                candidates: vec![],
                total: Total::Estimated(0),
            });
        }
        let ids = selected
            .iter()
            .map(|(id, _, _)| sql.bind(id.clone()))
            .collect::<Vec<_>>()
            .join(",");
        let keys = selected
            .iter()
            .map(|(_, _, key)| sql.bind(key.clone()))
            .collect::<Vec<_>>()
            .join(",");
        let predicate = if let Some(f) = filter {
            filter::predicate(index, &table, f, &mut sql)?
        } else {
            "1=1".into()
        };
        let text = format!(
            "SELECT s.document_json,s.source_version,s.vector_id FROM {} AS s WHERE s.vector_id IN ({ids}) AND s.{} IN ({keys}) AND ({predicate})",
            sql.ident(&table),
            sql.ident("key")
        );
        let documents = rows(DB::connection()?.inner(), text, sql)
            .await?
            .iter()
            .map(|r| Ok((get::<String>(r, "vector_id")?, decode(r)?)))
            .collect::<Result<BTreeMap<_, _>, FrameworkError>>()?;
        let mut candidates = selected
            .into_iter()
            .filter_map(|(id, score, _)| {
                documents.get(&id).map(|doc| EngineCandidate {
                    document: doc.clone(),
                    score: f64::from(score),
                    highlights: None,
                })
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.document.key.cmp(&b.document.key))
        });
        candidates.truncate(depth);
        let total = Total::Estimated(candidates.len() as u64);
        Ok(EngineResult { candidates, total })
    }
    async fn facets(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
        cap: usize,
        order: FacetOrder,
    ) -> Result<FacetDistribution, FrameworkError> {
        facet_query(
            self,
            index,
            generation,
            q,
            FacetRequest {
                attrs,
                cap,
                order,
                prefix: None,
            },
        )
        .await
    }
    async fn facet_stats(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attrs: &[String],
    ) -> Result<FacetStats, FrameworkError> {
        let (dialect, table, settings) = self.context(index, generation).await?;
        let mut result = BTreeMap::new();
        for attr in attrs {
            super::collection::facet_decl(index, attr, true)?;
            let mut sql = Sql::new(dialect);
            let predicate = Self::predicate(index, dialect, &table, &settings, q, &mut sql)?;
            let col = sql.ident(attr);
            let text = format!(
                "SELECT MIN(s.{col}) AS lo,MAX(s.{col}) AS hi FROM {} AS s WHERE {predicate}",
                sql.ident(&table)
            );
            let extrema = rows(DB::connection()?.inner(), text, sql).await?;
            if let Some(row) = extrema.first()
                && let Some(min) = read_attribute(row, "lo", index.attribute(attr)?.ty)?
            {
                let max =
                    read_attribute(row, "hi", index.attribute(attr)?.ty)?.ok_or_else(|| {
                        db_error(format!(
                            "search index {} attribute {attr}: missing max",
                            index.name
                        ))
                    })?;
                result.insert(attr.clone(), FacetRange { min, max });
            }
        }
        Ok(result)
    }
    async fn facet_search(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
        attr: &str,
        prefix: &str,
        cap: usize,
    ) -> Result<FacetDistribution, FrameworkError> {
        facet_query(
            self,
            index,
            generation,
            q,
            FacetRequest {
                attrs: &[attr.into()],
                cap,
                order: FacetOrder::Count,
                prefix: Some(prefix),
            },
        )
        .await
    }
    async fn vocabulary(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
    ) -> Result<Arc<Vocabulary>, FrameworkError> {
        let (dialect, table, _) = self.context(index, generation).await?;
        let conn = DB::connection()?;
        if dialect == Dialect::Postgres {
            let step = vocabulary_step()?;
            let meta =
                Self::metadata(conn.inner(), dialect, index.name, generation.as_str()).await?;
            let writes: i64 = get(&meta, "write_count")?;
            let refreshed: i64 = get(&meta, "refreshed_count")?;
            if writes > 0 && (refreshed == 0 || writes - refreshed >= step) {
                self.refresh_vocabulary(index, generation).await?;
            }
        }
        let mut sql = Sql::new(dialect);
        let text = if dialect == Dialect::Sqlite {
            let settings = self.read_settings(index, generation).await?;
            if settings.index_time.searchable_attributes.is_empty() {
                return Ok(Arc::new(Vocabulary::default()));
            }
            format!(
                "SELECT term AS word,doc AS document_frequency FROM {}",
                sql.ident(&format!("{table}_vocab"))
            )
        } else {
            let i = sql.bind(index.name.to_owned());
            let g = sql.bind(generation.as_str().to_owned());
            format!(
                "SELECT word,document_frequency FROM search_vocabulary WHERE index_name={i} AND generation={g} AND document_frequency>0"
            )
        };
        let words = rows(conn.inner(), text, sql)
            .await?
            .iter()
            .map(|r| Ok((get(r, "word")?, get::<i64>(r, "document_frequency")? as u64)))
            .collect::<Result<BTreeMap<_, _>, FrameworkError>>()?;
        Ok(Arc::new(Vocabulary {
            words: words.into(),
        }))
    }
    async fn count(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        q: &EngineQuery,
    ) -> Result<Total, FrameworkError> {
        let (dialect, table, settings) = self.context(index, generation).await?;
        Self::count_on(
            DB::connection()?.inner(),
            index,
            dialect,
            &table,
            &settings,
            q,
        )
        .await
    }
}

fn validate_dense(
    index: &IndexDescriptor,
    settings: &IndexSettings,
    dense: &[f32],
) -> Result<(), FrameworkError> {
    if dense.is_empty()
        || dense.iter().all(|v| *v == 0.0)
        || dense.iter().any(|v| !v.is_finite())
        || settings
            .index_time
            .dimensions
            .is_some_and(|n| n != dense.len())
    {
        return Err(FrameworkError::param(format!(
            "search index {}: dense embedding must be finite and match configured dimensions",
            index.name
        )));
    }
    Ok(())
}
fn validate_document(
    index: &IndexDescriptor,
    settings: &IndexSettings,
    doc: &IndexedDocument,
) -> Result<(), FrameworkError> {
    doc.document.validate(index)?;
    if doc.document.key.chars().count() > 255 {
        return Err(FrameworkError::param(format!(
            "search index {} key: exceeds 255 characters",
            index.name
        )));
    }
    if doc.sparse.is_some() {
        return Err(FrameworkError::param(format!(
            "search index {}: database does not support sparse embeddings",
            index.name
        )));
    }
    if let Some(dense) = &doc.dense {
        validate_dense(index, settings, dense)?;
    }
    for (attr, value) in &doc.document.attributes {
        let texts = match value {
            AttributeValue::Text(t) => vec![t.as_str()],
            AttributeValue::List(v) => v.iter().filter_map(AttributeValue::as_text).collect(),
            _ => vec![],
        };
        if texts.iter().any(|t| t.contains(['\u{001e}', '\u{001f}'])) {
            return Err(FrameworkError::param(format!(
                "search index {} attribute {attr}: reserved highlight control characters",
                index.name
            )));
        }
    }
    Ok(())
}
async fn find_document<C: ConnectionTrait>(
    conn: &C,
    dialect: Dialect,
    table: &str,
    key: &str,
) -> Result<Option<QueryResult>, FrameworkError> {
    let mut sql = Sql::new(dialect);
    let k = sql.bind(key.to_owned());
    let text = format!(
        "SELECT document_json,source_version,content_hash,vector_id FROM {} WHERE {}={k}",
        sql.ident(table),
        sql.ident("key")
    );
    Ok(rows(conn, text, sql).await?.into_iter().next())
}
async fn find_tombstone<C: ConnectionTrait>(
    conn: &C,
    dialect: Dialect,
    index: &str,
    generation: &GenerationId,
    key: &str,
) -> Result<Option<i64>, FrameworkError> {
    let mut sql = Sql::new(dialect);
    let i = sql.bind(index.to_owned());
    let g = sql.bind(generation.as_str().to_owned());
    let k = sql.bind(key.to_owned());
    rows(conn,format!("SELECT source_version FROM search_tombstones WHERE index_name={i} AND generation={g} AND document_key={k}"),sql).await?.first().map(|r|get(r,"source_version")).transpose()
}
async fn clear_lists<C: ConnectionTrait>(
    conn: &C,
    index: &IndexDescriptor,
    dialect: Dialect,
    table: &str,
    key: &str,
) -> Result<(), FrameworkError> {
    for attr in index
        .attributes
        .iter()
        .filter(|a| a.ty == AttributeType::TextList)
    {
        let mut sql = Sql::new(dialect);
        let k = sql.bind(key.to_owned());
        let text = format!(
            "DELETE FROM {} WHERE document_key={k}",
            sql.ident(&format!("{table}_{}", attr.name))
        );
        execute(conn, text, sql).await?;
    }
    Ok(())
}
#[derive(Clone, Copy)]
struct WriteContext<'a> {
    index: &'a IndexDescriptor,
    generation: &'a GenerationId,
    dialect: Dialect,
    table: &'a str,
    settings: &'a IndexSettings,
}
async fn write_document(
    tx: &DatabaseTransaction,
    scope: &WriteContext<'_>,
    doc: &IndexedDocument,
    outcome: &mut WriteOutcome,
) -> Result<(), FrameworkError> {
    let WriteContext {
        index,
        generation,
        dialect,
        table,
        settings,
    } = *scope;
    let key = &doc.document.key;
    let version = doc.document.source_version;
    if find_tombstone(tx, dialect, index.name, generation, key)
        .await?
        .is_some_and(|v| version <= v)
    {
        outcome.skipped_tombstoned += 1;
        return Ok(());
    }
    let existing = find_document(tx, dialect, table, key).await?;
    if let Some(existing) = &existing {
        if version < get::<i64>(existing, "source_version")? {
            outcome.skipped_older += 1;
            return Ok(());
        }
        if doc.content_hash == get::<String>(existing, "content_hash")? {
            let existing_id = get::<Option<String>>(existing, "vector_id")?;
            let mut sql = Sql::new(dialect);
            let v = sql.bind(version);
            let mut assignments = vec![format!("source_version={v}")];
            if existing_id.is_none() && doc.dense.is_some() {
                let point = sql.bind(Some(vector_id(index.name, generation, doc)));
                let dense = sql.bind(serde_json::to_string(&doc.dense).map_err(db_error)?);
                let pending = sql.bind(true);
                assignments.extend([
                    format!("vector_id={point}"),
                    format!("vector_dense_json={dense}"),
                    format!("vector_pending={pending}"),
                ]);
            }
            let k = sql.bind(key.clone());
            let text = format!(
                "UPDATE {} SET {} WHERE {}={k}",
                sql.ident(table),
                assignments.join(","),
                sql.ident("key")
            );
            execute(tx, text, sql).await?;
            outcome.skipped_unchanged += 1;
            return Ok(());
        }
    }
    let point = doc
        .dense
        .as_ref()
        .map(|_| vector_id(index.name, generation, doc));
    let mut sql = Sql::new(dialect);
    let mut columns = vec![
        "key".into(),
        "source_version".into(),
        "content_hash".into(),
        "indexed_at".into(),
        "document_json".into(),
        "vector_id".into(),
        "vector_dense_json".into(),
        "vector_pending".into(),
    ];
    let mut values = vec![
        sql.bind(key.clone()),
        sql.bind(version),
        sql.bind(doc.content_hash.clone()),
        sql.bind(now()),
        sql.bind(serde_json::to_string(&doc.document).map_err(db_error)?),
        sql.bind(point),
        sql.bind(
            doc.dense
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(db_error)?,
        ),
        sql.bind(doc.dense.is_some()),
    ];
    for attr in &index.attributes {
        let value = doc.document.attributes.get(&attr.name).ok_or_else(|| {
            FrameworkError::param(format!(
                "search index {} attribute {}: missing value",
                index.name, attr.name
            ))
        })?;
        if attr.ty == AttributeType::Geo {
            let coords = if let AttributeValue::Geo(point) = value {
                [Some(point.lat), Some(point.lng)]
            } else {
                [None, None]
            };
            for (suffix, value) in ["lat", "lng"].into_iter().zip(coords) {
                columns.push(format!("{}_{suffix}", attr.name));
                values.push(sql.bind(value));
            }
        } else {
            columns.push(attr.name.clone());
            values.push(sql.bind(typed_value(value, attr.ty)?));
            if attr.ty == AttributeType::TextList && attr.searchable.is_some() {
                columns.push(native_attribute(index, &attr.name));
                values.push(sql.bind(searchable_text(value)));
            }
        }
    }
    for attr in index
        .attributes
        .iter()
        .filter(|a| a.facet && a.ty != AttributeType::TextList)
    {
        columns.push(format!("{}_facet", attr.name));
        let prefix = doc
            .document
            .attributes
            .get(&attr.name)
            .filter(|v| !matches!(v, AttributeValue::Null))
            .map(|v| facet_prefix(&v.facet_key()));
        values.push(sql.bind(prefix));
    }
    for attr in index
        .attributes
        .iter()
        .filter(|a| a.facet && a.ty == AttributeType::Float)
    {
        columns.push(format!("{}_facet_label", attr.name));
        values.push(
            sql.bind(
                doc.document
                    .attributes
                    .get(&attr.name)
                    .filter(|v| !matches!(v, AttributeValue::Null))
                    .map(AttributeValue::facet_key),
            ),
        );
    }
    let quoted = columns.iter().map(|c| sql.ident(c)).collect::<Vec<_>>();
    let text = if existing.is_some() {
        let assignments = quoted
            .iter()
            .zip(&values)
            .skip(1)
            .map(|(c, v)| format!("{c}={v}"))
            .collect::<Vec<_>>();
        // Anonymous placeholders follow textual order, so the key bind belongs last.
        let key_value = sql.values.remove(0);
        if dialect == Dialect::Postgres {
            // Numbered parameters retain their original key ordinal.
            sql.values.insert(0, key_value);
            format!(
                "UPDATE {} SET {} WHERE {}={}",
                sql.ident(table),
                assignments.join(","),
                sql.ident("key"),
                values[0]
            )
        } else {
            sql.values.push(key_value);
            format!(
                "UPDATE {} SET {} WHERE {}=?",
                sql.ident(table),
                assignments.join(","),
                sql.ident("key")
            )
        }
    } else {
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            sql.ident(table),
            quoted.join(","),
            values.join(",")
        )
    };
    execute(tx, text, sql).await?;
    if dialect == Dialect::Postgres {
        let mut sql = Sql::new(dialect);
        let native = native_settings(index, settings);
        let vector = postgres::vector(table, &native, &mut sql)?;
        let k = sql.bind(key.clone());
        let text = format!(
            "UPDATE {} AS s SET search_vector={vector} WHERE s.{}={k}",
            sql.ident(table),
            sql.ident("key")
        );
        execute(tx, text, sql).await?;
    }
    clear_lists(tx, index, dialect, table, key).await?;
    for attr in index
        .attributes
        .iter()
        .filter(|a| a.ty == AttributeType::TextList)
    {
        if let Some(AttributeValue::List(members)) = doc.document.attributes.get(&attr.name) {
            for (position, value) in members.iter().enumerate() {
                let mut sql = Sql::new(dialect);
                let k = sql.bind(key.clone());
                let p = sql.bind(i64::try_from(position).map_err(db_error)?);
                let v = sql.bind(filter::value(value)?);
                let prefix = sql.bind(facet_prefix(&value.facet_key()));
                let text = format!(
                    "INSERT INTO {} (document_key,position,value,prefix_value) VALUES ({k},{p},{v},{prefix})",
                    sql.ident(&format!("{table}_{}", attr.name))
                );
                execute(tx, text, sql).await?;
            }
        }
    }
    if dialect.mysql() {
        let old = existing
            .as_ref()
            .map(decode)
            .transpose()?
            .map(|d| words(&d, settings))
            .unwrap_or_default();
        DatabaseSearchEngine::update_words(
            tx,
            dialect,
            index.name,
            generation,
            old,
            words(&doc.document, settings),
        )
        .await?;
    }
    let mut sql = Sql::new(dialect);
    let i = sql.bind(index.name.to_owned());
    let g = sql.bind(generation.as_str().to_owned());
    let k = sql.bind(key.clone());
    execute(tx,format!("DELETE FROM search_tombstones WHERE index_name={i} AND generation={g} AND document_key={k}"),sql).await?;
    DatabaseSearchEngine::bump(tx, dialect, index.name, generation).await?;
    if let Some(old) = existing
        .as_ref()
        .map(|r| get::<Option<String>>(r, "vector_id"))
        .transpose()?
        .flatten()
        && Some(&old)
            != doc
                .dense
                .as_ref()
                .map(|_| vector_id(index.name, generation, doc))
                .as_ref()
    {
        queue_vector(tx, dialect, index.name, generation, old).await?;
    }
    outcome.written += 1;
    Ok(())
}

fn read_attribute(
    row: &QueryResult,
    col: &str,
    ty: AttributeType,
) -> Result<Option<AttributeValue>, FrameworkError> {
    match ty {
        AttributeType::Integer => Ok(get::<Option<i64>>(row, col)?.map(AttributeValue::Integer)),
        AttributeType::Float => Ok(get::<Option<f64>>(row, col)?.map(AttributeValue::Float)),
        AttributeType::Bool => Ok(get::<Option<bool>>(row, col)?.map(AttributeValue::Bool)),
        AttributeType::Text | AttributeType::TextList => {
            Ok(get::<Option<String>>(row, col)?.map(AttributeValue::Text))
        }
        AttributeType::DateTime => get::<Option<String>>(row, col)?
            .map(|text| AttributeValue::from_json(&text.into(), ty, col))
            .transpose(),
        AttributeType::Geo => get::<Option<String>>(row, col)?
            .map(|label| {
                let (lat, lng) = label.split_once(',').ok_or_else(|| {
                    FrameworkError::param(format!(
                        "search attribute {col}: invalid geo facet label"
                    ))
                })?;
                let point = crate::search::GeoPoint {
                    lat: lat.parse().map_err(|e| {
                        FrameworkError::param(format!("search attribute {col}: {e}"))
                    })?,
                    lng: lng.parse().map_err(|e| {
                        FrameworkError::param(format!("search attribute {col}: {e}"))
                    })?,
                };
                point.validate()?;
                Ok(AttributeValue::Geo(point))
            })
            .transpose(),
    }
}
struct FacetRequest<'a> {
    attrs: &'a [String],
    cap: usize,
    order: FacetOrder,
    prefix: Option<&'a str>,
}
async fn facet_query(
    engine: &DatabaseSearchEngine,
    index: &IndexDescriptor,
    generation: &GenerationId,
    q: &EngineQuery,
    request: FacetRequest<'_>,
) -> Result<FacetDistribution, FrameworkError> {
    let FacetRequest {
        attrs,
        cap,
        order,
        prefix,
    } = request;
    let (dialect, table, settings) = engine.context(index, generation).await?;
    let mut result = BTreeMap::new();
    for attr in attrs {
        super::collection::facet_decl(index, attr, false)?;
        let decl = index.attribute(attr)?;
        let mut sql = Sql::new(dialect);
        let (value, join, ty) = if decl.ty == AttributeType::TextList {
            (
                String::from("f.value"),
                format!(
                    " JOIN {} AS f ON f.document_key=s.{}",
                    sql.ident(&format!("{table}_{attr}")),
                    sql.ident("key")
                ),
                AttributeType::Text,
            )
        } else {
            (
                format!(
                    "s.{}",
                    sql.ident(&if decl.ty == AttributeType::Geo {
                        format!("{attr}_facet")
                    } else {
                        attr.clone()
                    })
                ),
                String::new(),
                decl.ty,
            )
        };
        let predicate =
            DatabaseSearchEngine::predicate(index, dialect, &table, &settings, q, &mut sql)?;
        let prefix = if let Some(prefix) = prefix {
            let escaped = format!(
                "{}%",
                facet_prefix(prefix)
                    .replace('!', "!!")
                    .replace('%', "!%")
                    .replace('_', "!_")
            );
            let p = sql.bind(escaped);
            let prefix_column = if decl.ty == AttributeType::TextList {
                "f.prefix_value".into()
            } else {
                format!("s.{}", sql.ident(&format!("{attr}_facet")))
            };
            format!(" AND {prefix_column} LIKE {p} ESCAPE '!'")
        } else {
            String::new()
        };
        let limit = sql.bind(i64::try_from(cap).map_err(db_error)?);
        let label = if decl.ty == AttributeType::Float {
            format!("MIN(s.{})", sql.ident(&format!("{attr}_facet_label")))
        } else {
            format!(
                "CAST({value} AS {})",
                if dialect.mysql() { "CHAR" } else { "TEXT" }
            )
        };
        let tie = match dialect {
            Dialect::Postgres => format!("{label} COLLATE \"C\""),
            Dialect::Mysql | Dialect::MariaDb => format!("CAST({label} AS BINARY)"),
            Dialect::Sqlite => label,
        };
        // Include the original expression for ONLY_FULL_GROUP_BY while grouping exact bytes.
        let grouping = if dialect.mysql() && ty == AttributeType::Text {
            format!("{value}, CAST({value} AS BINARY)")
        } else {
            value.clone()
        };
        let text = format!(
            "SELECT {value} AS facet_value,COUNT(DISTINCT s.{}) AS n FROM {} AS s{join} WHERE ({predicate}) AND {value} IS NOT NULL{prefix} GROUP BY {grouping} ORDER BY n DESC,{tie} LIMIT {limit}",
            sql.ident("key"),
            sql.ident(&table)
        );
        let result_rows = rows(DB::connection()?.inner(), text, sql).await?;
        let mut values = Vec::new();
        for row in result_rows {
            if let Some(value) = read_attribute(&row, "facet_value", ty)? {
                values.push(FacetValue {
                    value,
                    count: get::<i64>(&row, "n")? as u64,
                });
            }
        }
        values.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.value.facet_key().cmp(&b.value.facet_key()))
        });
        values.truncate(cap);
        if order == FacetOrder::Alphabetical {
            values.sort_by_key(|v| v.value.facet_key());
        }
        result.insert(attr.clone(), values);
    }
    Ok(result)
}

fn typed_value(value: &AttributeValue, ty: AttributeType) -> Result<Value, FrameworkError> {
    if matches!(value, AttributeValue::Null) {
        Ok(match ty {
            AttributeType::Integer => Value::BigInt(None),
            AttributeType::Float => Value::Double(None),
            AttributeType::Bool => Value::Bool(None),
            _ => Value::String(None),
        })
    } else {
        filter::value(value)
    }
}
fn attribute_query(
    query: &crate::search::ExpandedQuery,
    attribute: &str,
) -> crate::search::ExpandedQuery {
    let mut result = query.clone();
    if query.typo_disabled_attributes.contains(attribute) {
        for term in &mut result.terms {
            term.alternatives
                .retain(|word| !term.typo_alternatives.contains(word));
        }
    }
    result
}

async fn ensure_ready(
    tx: &DatabaseTransaction,
    dialect: Dialect,
    index: &str,
    generation: &GenerationId,
) -> Result<(), FrameworkError> {
    let metadata = DatabaseSearchEngine::metadata(tx, dialect, index, generation.as_str()).await?;
    let state: String = get(&metadata, "state")?;
    if !matches!(
        state.as_str(),
        "current" | "building" | "retired" | "active" | "inactive"
    ) {
        return Err(FrameworkError::database(format!(
            "search index {index} generation {}: unavailable state {state}",
            generation.as_str()
        )));
    }
    Ok(())
}
async fn queue_vector(
    tx: &DatabaseTransaction,
    dialect: Dialect,
    index: &str,
    generation: &GenerationId,
    id: String,
) -> Result<(), FrameworkError> {
    let metadata = DatabaseSearchEngine::metadata(tx, dialect, index, generation.as_str()).await?;
    let mut pending: BTreeSet<String> =
        serde_json::from_str(&get::<String>(&metadata, "pending_vectors_json")?)
            .map_err(db_error)?;
    pending.insert(id);
    let mut sql = Sql::new(dialect);
    let pending = sql.bind(serde_json::to_string(&pending).map_err(db_error)?);
    let i = sql.bind(index.to_owned());
    let g = sql.bind(generation.as_str().to_owned());
    execute(tx,format!("UPDATE search_generations SET pending_vectors_json={pending} WHERE index_name={i} AND generation={g}"),sql).await?;
    Ok(())
}
impl DatabaseSearchEngine {
    async fn drain_vectors(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        table: &str,
        dialect: Dialect,
    ) -> Result<(), FrameworkError> {
        let conn = DB::connection()?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        Self::lock(&tx, dialect, index.name, generation.as_str()).await?;
        let metadata = Self::metadata(&tx, dialect, index.name, generation.as_str()).await?;
        let pending: BTreeSet<String> =
            serde_json::from_str(&get::<String>(&metadata, "pending_vectors_json")?)
                .map_err(db_error)?;
        if pending.is_empty() {
            return tx.commit().await.map_err(db_error);
        }
        let mut remaining = Vec::new();
        let exists = sea_orm_migration::SchemaManager::new(&tx)
            .has_table(table)
            .await
            .map_err(db_error)?;
        for id in pending {
            let mut sql = Sql::new(dialect);
            let point = sql.bind(id.clone());
            let text = format!(
                "SELECT vector_id FROM {} WHERE vector_id={point} LIMIT 1",
                sql.ident(table)
            );
            if !exists || rows(&tx, text, sql).await?.is_empty() {
                remaining.push(id);
            }
        }
        if !remaining.is_empty() {
            vector_store(index.name)?.delete(remaining).await?;
        }
        let mut sql = Sql::new(dialect);
        let empty = sql.bind("[]".to_owned());
        let i = sql.bind(index.name.to_owned());
        let g = sql.bind(generation.as_str().to_owned());
        execute(&tx,format!("UPDATE search_generations SET pending_vectors_json={empty} WHERE index_name={i} AND generation={g}"),sql).await?;
        tx.commit().await.map_err(db_error)
    }
}

fn vocabulary_step() -> Result<i64, FrameworkError> {
    crate::config::env_optional::<String>("SEARCH_VOCABULARY_REFRESH")
        .map(|value| {
            value.parse::<i64>().ok().filter(|v| *v > 0).ok_or_else(|| {
                FrameworkError::param("SEARCH_VOCABULARY_REFRESH must be a positive signed integer")
            })
        })
        .unwrap_or(Ok(1000))
}

fn facet_prefix(value: &str) -> String {
    deunicode::deunicode(value).to_lowercase()
}

fn created_table(ddl: &str) -> Option<String> {
    let rest = ddl
        .strip_prefix("CREATE TABLE ")
        .or_else(|| ddl.strip_prefix("CREATE VIRTUAL TABLE "))?;
    rest.split_whitespace()
        .next()
        .map(|name| name.trim_matches(['"', '`']).to_owned())
}
async fn drop_created<C: ConnectionTrait>(
    conn: &C,
    tables: &[String],
    dialect: Dialect,
) -> Result<(), FrameworkError> {
    let sql = Sql::new(dialect);
    for table in tables.iter().rev() {
        identifier(table)?;
        conn.execute_unprepared(&format!("DROP TABLE IF EXISTS {}", sql.ident(table)))
            .await
            .map_err(db_error)?;
    }
    Ok(())
}
impl DatabaseSearchEngine {
    async fn flush_vectors(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        table: &str,
        dialect: Dialect,
    ) -> Result<(), FrameworkError> {
        loop {
            let conn = DB::connection()?;
            let tx = conn.inner().begin().await.map_err(db_error)?;
            Self::lock(&tx, dialect, index.name, generation.as_str()).await?;
            ensure_ready(&tx, dialect, index.name, generation).await?;
            let mut sql = Sql::new(dialect);
            let limit = sql.bind(512i64);
            let text = format!(
                "SELECT document_json,source_version,content_hash,vector_id,vector_dense_json FROM {} WHERE vector_pending=TRUE LIMIT {limit}",
                sql.ident(table)
            );
            let pending = rows(&tx, text, sql).await?;
            if pending.is_empty() {
                return tx.commit().await.map_err(db_error);
            }
            let mut points = Vec::with_capacity(pending.len());
            for row in &pending {
                let document = decode(row)?;
                let embedding: Vec<f32> =
                    serde_json::from_str(&get::<String>(row, "vector_dense_json")?)
                        .map_err(db_error)?;
                let hash: String = get(row, "content_hash")?;
                points.push(VectorItem::new(get::<String>(row,"vector_id")?,embedding,serde_json::json!({"index":index.name,"generation":generation.as_str(),"key":document.key,"content_hash":hash})));
            }
            vector_store(index.name)?.upsert(points).await?;
            let mut sql = Sql::new(dialect);
            let ids = pending
                .iter()
                .map(|r| Ok(sql.bind(get::<String>(r, "vector_id")?)))
                .collect::<Result<Vec<_>, FrameworkError>>()?
                .join(",");
            let text = format!(
                "UPDATE {} SET vector_pending=FALSE WHERE vector_id IN ({ids})",
                sql.ident(table)
            );
            execute(&tx, text, sql).await?;
            tx.commit().await.map_err(db_error)?;
        }
    }
}
