//! Postgres vector search through direct `sqlx`, with application-owned schema.
//!
//! Each store is a validated, quoted table name containing `id VARCHAR(254)`,
//! `embedding vector(N)` and nullable `metadata JSONB`. The application runs
//! [`PgVectorDriver::ensure_table_sql_for`] in a migration; runtime methods
//! never create tables or extensions. First use checks `pg_extension` once
//! successfully, while failed checks remain retryable.
//!
//! Results are best first. Cosine scores are `1 - distance`, Euclidean scores
//! are `1 / (1 + distance)`, and inner-product scores are `-distance` because
//! pgvector's `<#>` operator returns the negative inner product. Cosine keeps
//! negative similarity, unlike MariaDB's normalization to `[0, 1]`.

use super::driver::{VectorDriver, VectorItem, VectorMatch};
use crate::FrameworkError;
use async_trait::async_trait;
use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::collections::HashSet;
use std::fmt::Write;
use tokio::sync::OnceCell;

// Bound parameter counts stay well below Postgres's per-statement limit.
const UPSERT_BATCH_SIZE: usize = 500;
const DELETE_BATCH_SIZE: usize = 1000;

/// Keep the search operator and HNSW opclass aligned with the embedding model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PgVectorDistance {
    /// Compare direction rather than magnitude, as most embedding models do.
    #[default]
    Cosine,
    /// Rank by L2 distance when magnitude is part of the model's geometry.
    Euclidean,
    /// Rank by dot product when the model uses unnormalized vector magnitude.
    InnerProduct,
}

impl PgVectorDistance {
    /// The operator orders raw distances ascending to support index scans.
    pub fn operator(self) -> &'static str {
        match self {
            Self::Cosine => "<=>",
            Self::Euclidean => "<->",
            Self::InnerProduct => "<#>",
        }
    }

    /// The HNSW opclass must match the operator for indexed searches.
    pub fn index_opclass(self) -> &'static str {
        match self {
            Self::Cosine => "vector_cosine_ops",
            Self::Euclidean => "vector_l2_ops",
            Self::InnerProduct => "vector_ip_ops",
        }
    }
}

/// Store embeddings in the application's Postgres without another service.
///
/// Owns a pool separate from SeaORM. Use [`Self::from_pool`] for custom pool
/// limits and [`Self::pool`] for SQL beyond the small [`VectorDriver`] trait.
pub struct PgVectorDriver {
    pool: PgPool,
    distance: PgVectorDistance,
    extension_check: OnceCell<()>,
}

impl PgVectorDriver {
    /// Reuse a configured pool when connection limits or timeouts matter.
    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            pool,
            distance: PgVectorDistance::default(),
            extension_check: OnceCell::new(),
        }
    }

    /// Validate a Postgres URL without connecting, so bootstrap can register
    /// the driver before the server is reachable. Accepts `postgres` and
    /// `postgresql` schemes; errors never include the credential-bearing URL.
    pub fn from_url(url: &str) -> Result<Self, FrameworkError> {
        if !names_postgres(url) {
            return Err(FrameworkError::param(
                "pgvector URL must use the postgres or postgresql scheme",
            ));
        }
        let pool = PgPoolOptions::new().connect_lazy(url).map_err(|_| {
            FrameworkError::param("pgvector URL is invalid; check the Postgres connection syntax")
        })?;
        Ok(Self::from_pool(pool))
    }

    /// Prefer `PGVECTOR_URL`, falling back to a Postgres `DATABASE_URL` when
    /// it is unset or blank. This lets rows and embeddings share one database.
    /// Invalid explicit URLs fail without falling back; errors name the
    /// variable and never print its credential-bearing value.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_variables(|name| std::env::var(name).ok())
    }

    fn from_variables(variable: impl Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let set = |name: &str| {
            variable(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        if let Some(url) = set("PGVECTOR_URL") {
            return Self::from_url(&url).map_err(|e| {
                FrameworkError::param(format!(
                    "PGVECTOR_URL cannot initialize the pgvector pool: {e}"
                ))
            });
        }
        match set("DATABASE_URL") {
            Some(url) if names_postgres(&url) => Self::from_url(&url).map_err(|e| {
                FrameworkError::param(format!(
                    "DATABASE_URL cannot initialize the pgvector pool: {e}"
                ))
            }),
            Some(_) => Err(FrameworkError::param(
                "PGVECTOR_URL is not set, and DATABASE_URL is not Postgres; \
                 set PGVECTOR_URL for the pgvector driver",
            )),
            None => Err(FrameworkError::param(
                "PGVECTOR_URL is not set; the pgvector driver needs it or a Postgres DATABASE_URL",
            )),
        }
    }

    /// Select the model's metric; configure the migration with the same value
    /// so the query operator can use its HNSW index. Defaults to cosine.
    pub fn with_distance(mut self, distance: PgVectorDistance) -> Self {
        self.distance = distance;
        self
    }

    /// Borrow the pool for joins or raw queries beyond the vector trait.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Inspect the metric when coordinating searches with application schema.
    pub fn distance(&self) -> PgVectorDistance {
        self.distance
    }

    /// Restrict identifiers to `[A-Za-z_][A-Za-z0-9_]*` and 63 bytes to prevent
    /// SQL injection and Postgres's silent truncation. Returns the name unchanged.
    pub fn validate_store_name(name: &str) -> Result<&str, FrameworkError> {
        if name.is_empty() {
            return Err(FrameworkError::param("pgvector store name cannot be empty"));
        }
        if name.len() > 63 {
            return Err(FrameworkError::param(format!(
                "pgvector store name '{name}' exceeds 63 bytes"
            )));
        }
        if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            return Err(FrameworkError::param(format!(
                "pgvector store name '{name}' must start with a letter or '_'"
            )));
        }
        if !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            return Err(FrameworkError::param(format!(
                "pgvector store name '{name}' contains invalid characters (allowed: A-Z a-z 0-9 _)"
            )));
        }
        Ok(name)
    }

    /// Emit migration text using the configured distance to keep the HNSW
    /// opclass aligned with searches. The driver never executes this SQL.
    pub fn ensure_table_sql_for(&self, store: &str, dim: usize) -> Result<String, FrameworkError> {
        Self::ensure_table_sql(store, dim, self.distance)
    }

    /// Emit extension, table and HNSW index SQL for application migrations.
    /// Use this form when no driver exists yet; pass the same distance to
    /// [`Self::with_distance`]. Rejects invalid stores and zero dimensions.
    /// The unnamed index statement creates a new index on every execution,
    /// so the application should run its migration once.
    pub fn ensure_table_sql(
        store: &str,
        dim: usize,
        distance: PgVectorDistance,
    ) -> Result<String, FrameworkError> {
        Self::validate_store_name(store)?;
        if dim == 0 {
            return Err(FrameworkError::param(format!(
                "pgvector dim for store '{store}' must be greater than 0"
            )));
        }
        Ok(format!(
            "CREATE EXTENSION IF NOT EXISTS vector;\n\
             CREATE TABLE IF NOT EXISTS \"{store}\" (\n  \
             id VARCHAR(254) PRIMARY KEY,\n  \
             embedding vector({dim}) NOT NULL,\n  \
             metadata JSONB NULL\n\
             );\n\
             CREATE INDEX ON \"{store}\" USING hnsw (embedding {opclass});",
            opclass = distance.index_opclass()
        ))
    }

    /// Bind pgvector's text form without another SQLx type crate. Rejects
    /// empty and non-finite embeddings; finite floats keep f32 precision.
    pub fn embedding_to_vec_text(embedding: &[f32]) -> Result<String, FrameworkError> {
        validate_embedding(embedding)?;
        let mut text = String::with_capacity(embedding.len() * 8);
        text.push('[');
        for (i, f) in embedding.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            write!(&mut text, "{f}").map_err(|e| {
                FrameworkError::internal(format!("pgvector embedding text formatting failed: {e}"))
            })?;
        }
        text.push(']');
        Ok(text)
    }

    /// Map raw distance to higher-is-better scores. Cosine uses `1 - d`,
    /// Euclidean `1 / (1 + d)`, inner product `-d` because pgvector negates
    /// the dot product. Scores keep their signs and are not clamped.
    pub fn score_from_distance(distance: PgVectorDistance, raw: f32) -> f32 {
        match distance {
            PgVectorDistance::Cosine => 1.0 - raw,
            PgVectorDistance::Euclidean => 1.0 / (1.0 + raw),
            PgVectorDistance::InnerProduct => -raw,
        }
    }

    // Cache success only, so installing the extension or recovering a
    // connection permits the next operation to retry the check.
    async fn ensure_extension(&self) -> Result<(), FrameworkError> {
        self.extension_check.get_or_try_init(|| async {
            let installed: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = $1)",
            )
            .bind("vector")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| FrameworkError::internal(format!("pgvector extension check failed: {e}")))?;
            if !installed {
                return Err(FrameworkError::internal(
                    "pgvector extension 'vector' is not installed; install the pgvector package \
                     on Postgres and run CREATE EXTENSION vector in the application's database",
                ));
            }
            Ok(())
        }).await?;
        Ok(())
    }
}

fn names_postgres(url: &str) -> bool {
    matches!(
        url.split_once("://")
            .map(|(scheme, _)| scheme.to_ascii_lowercase())
            .as_deref(),
        Some("postgres" | "postgresql")
    )
}

fn validate_embedding(embedding: &[f32]) -> Result<(), FrameworkError> {
    if embedding.is_empty() {
        return Err(FrameworkError::param("pgvector embedding cannot be empty"));
    }
    if let Some(index) = embedding.iter().position(|f| !f.is_finite()) {
        return Err(FrameworkError::param(format!(
            "pgvector embedding contains a non-finite value at index {index}"
        )));
    }
    Ok(())
}

fn upsert_sql(table: &str, rows: usize) -> String {
    let values = (0..rows)
        .map(|i| {
            let n = i * 3;
            format!("(${}, ${}::vector, ${})", n + 1, n + 2, n + 3)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO \"{table}\" (id, embedding, metadata) VALUES {values} \
         ON CONFLICT (id) DO UPDATE SET embedding = EXCLUDED.embedding, metadata = EXCLUDED.metadata"
    )
}

fn similar_sql(table: &str, distance: PgVectorDistance) -> String {
    format!(
        "SELECT id, metadata, embedding {op} $1::vector AS dist FROM \"{table}\" \
         ORDER BY dist ASC LIMIT $2",
        op = distance.operator()
    )
}

#[async_trait]
impl VectorDriver for PgVectorDriver {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError> {
        if items.is_empty() {
            return Ok(());
        }
        let table = Self::validate_store_name(store)?;
        for item in &items {
            validate_embedding(&item.embedding).map_err(|e| {
                FrameworkError::param(format!(
                    "pgvector upsert invalid item '{}' in store '{table}': {e}",
                    item.id
                ))
            })?;
            if !matches!(
                item.metadata,
                serde_json::Value::Object(_) | serde_json::Value::Null
            ) {
                return Err(FrameworkError::param(format!(
                    "pgvector item '{}' in store '{table}' metadata must be a JSON object or null",
                    item.id
                )));
            }
        }
        self.ensure_extension().await?;
        // Postgres rejects the same conflict key twice in one INSERT.
        // Keep the last occurrence to preserve the trait's merge semantics.
        let mut seen = HashSet::new();
        let mut items: Vec<_> = items
            .into_iter()
            .rev()
            .filter(|item| seen.insert(item.id.clone()))
            .collect();
        items.reverse();
        let mut tx = self.pool.begin().await.map_err(|e| {
            FrameworkError::internal(format!(
                "pgvector upsert begin transaction for store '{table}': {e}"
            ))
        })?;
        let mut iter = items.into_iter();
        loop {
            let chunk: Vec<_> = iter.by_ref().take(UPSERT_BATCH_SIZE).collect();
            if chunk.is_empty() {
                break;
            }
            let row_count = chunk.len();
            let mut q = sqlx::query(sqlx::AssertSqlSafe(upsert_sql(table, row_count)));
            for item in chunk {
                let text = Self::embedding_to_vec_text(&item.embedding)?;
                let metadata = if item.metadata.is_null() {
                    None
                } else {
                    Some(item.metadata)
                };
                q = q.bind(item.id).bind(text).bind(metadata);
            }
            q.execute(&mut *tx).await.map_err(|e| {
                FrameworkError::internal(format!(
                    "pgvector upsert chunk ({row_count} rows) failed for store '{table}': {e}"
                ))
            })?;
        }
        tx.commit().await.map_err(|e| {
            FrameworkError::internal(format!("pgvector upsert commit for store '{table}': {e}"))
        })?;
        Ok(())
    }

    async fn similar(
        &self,
        store: &str,
        query: Vec<f32>,
        k: usize,
    ) -> Result<Vec<VectorMatch>, FrameworkError> {
        if k == 0 || query.is_empty() {
            return Ok(Vec::new());
        }
        let table = Self::validate_store_name(store)?;
        if self.distance == PgVectorDistance::Cosine && query.iter().all(|&f| f == 0.0) {
            return Err(FrameworkError::param(format!(
                "pgvector similar query for store '{table}' is zero-vector under cosine"
            )));
        }
        let text = Self::embedding_to_vec_text(&query).map_err(|e| {
            FrameworkError::param(format!("pgvector similar query for store '{table}': {e}"))
        })?;
        let limit = i64::try_from(k).map_err(|_| {
            FrameworkError::param(format!(
                "pgvector similar k for store '{table}' exceeds the Postgres BIGINT limit"
            ))
        })?;
        self.ensure_extension().await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(similar_sql(table, self.distance)))
            .bind(text)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "pgvector similar failed for store '{table}': {e}"
                ))
            })?;
        rows.into_iter()
            .map(|row| {
                let decode = |e| {
                    FrameworkError::internal(format!(
                        "pgvector similar decode for store '{table}': {e}"
                    ))
                };
                let id: String = row.try_get("id").map_err(decode)?;
                let metadata: Option<serde_json::Value> =
                    row.try_get("metadata").map_err(decode)?;
                // pgvector distance operators return Postgres float8, not float4.
                let dist: f64 = row.try_get("dist").map_err(decode)?;
                Ok(VectorMatch {
                    id,
                    score: Self::score_from_distance(self.distance, dist as f32),
                    metadata: metadata.unwrap_or(serde_json::Value::Null),
                })
            })
            .collect()
    }

    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError> {
        if ids.is_empty() {
            return Ok(());
        }
        let table = Self::validate_store_name(store)?;
        self.ensure_extension().await?;
        let mut tx = self.pool.begin().await.map_err(|e| {
            FrameworkError::internal(format!(
                "pgvector delete begin transaction for store '{table}': {e}"
            ))
        })?;
        for chunk in ids.chunks(DELETE_BATCH_SIZE) {
            let placeholders = (1..=chunk.len())
                .map(|n| format!("${n}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("DELETE FROM \"{table}\" WHERE id IN ({placeholders})");
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
            for id in chunk {
                q = q.bind(id);
            }
            q.execute(&mut *tx).await.map_err(|e| {
                FrameworkError::internal(format!("pgvector delete failed for store '{table}': {e}"))
            })?;
        }
        tx.commit().await.map_err(|e| {
            FrameworkError::internal(format!("pgvector delete commit for store '{table}': {e}"))
        })?;
        Ok(())
    }

    async fn count(&self, store: &str) -> Result<usize, FrameworkError> {
        let table = Self::validate_store_name(store)?;
        self.ensure_extension().await?;
        let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!("pgvector count failed for store '{table}': {e}"))
            })?;
        usize::try_from(count).map_err(|e| {
            FrameworkError::internal(format!(
                "pgvector count for store '{table}' cannot fit usize: {e}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_upsert_sql_numbers_every_bound_value() {
        assert_eq!(
            upsert_sql("docs", 2),
            "INSERT INTO \"docs\" (id, embedding, metadata) VALUES ($1, $2::vector, $3), ($4, $5::vector, $6) ON CONFLICT (id) DO UPDATE SET embedding = EXCLUDED.embedding, metadata = EXCLUDED.metadata"
        );
    }

    #[test]
    fn postgres_similar_sql_uses_each_distance_operator() {
        for (distance, op) in [
            (PgVectorDistance::Cosine, "<=>"),
            (PgVectorDistance::Euclidean, "<->"),
            (PgVectorDistance::InnerProduct, "<#>"),
        ] {
            assert_eq!(
                similar_sql("docs", distance),
                format!(
                    "SELECT id, metadata, embedding {op} $1::vector AS dist FROM \"docs\" ORDER BY dist ASC LIMIT $2"
                )
            );
        }
    }
}
