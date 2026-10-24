//! Durable claims keep external vector I/O out of the SQL connection's lifetime.

use super::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const CLAIM_KEY: &str = "_database_vector_claim";
const CLAIM_SECONDS: i64 = 60;
const DISPATCH_SECONDS: u64 = 45;
const BATCH_SIZE: i64 = 256;

#[derive(Serialize, Deserialize)]
struct Claim {
    token: String,
    expires_at: i64,
    ids: Vec<String>,
}

pub(super) fn ensure_unclaimed(metadata: &QueryResult) -> Result<(), FrameworkError> {
    if let Some(value) = generation_settings(metadata)?.get(CLAIM_KEY) {
        let claim: Claim = serde_json::from_value(value.clone()).map_err(db_error)?;
        if claim.expires_at > now() {
            return Err(db_error(
                "vector delivery is in flight; retry after the claim completes or expires",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Operation {
    Upsert,
    Delete,
}

struct Batch {
    claim: Claim,
    versions: Vec<(String, i64)>,
    points: Vec<VectorItem>,
    deletions: Vec<String>,
}

struct Delivery<'a> {
    index: &'a IndexDescriptor,
    generation: &'a GenerationId,
    table: &'a str,
    dialect: Dialect,
}

impl Delivery<'_> {
    async fn claim(&self, operation: Operation) -> Result<Option<Batch>, FrameworkError> {
        let Self {
            index,
            generation,
            table,
            dialect,
        } = *self;
        let conn = DB::connection()?;
        crate::render_cache::write_side_open(false).await?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        DatabaseSearchEngine::lock(&tx, dialect, index.name, generation.as_str()).await?;
        if matches!(operation, Operation::Upsert) {
            ensure_ready(&tx, dialect, index.name, generation).await?;
        }
        let metadata =
            DatabaseSearchEngine::metadata(&tx, dialect, index.name, generation.as_str()).await?;
        let mut batch = Batch {
            claim: Claim {
                token: uuid::Uuid::new_v4().to_string(),
                expires_at: now() + CLAIM_SECONDS,
                ids: Vec::new(),
            },
            versions: Vec::new(),
            points: Vec::new(),
            deletions: Vec::new(),
        };
        match operation {
            Operation::Upsert => {
                let mut sql = Sql::new(dialect);
                let limit = sql.bind(BATCH_SIZE);
                let pending = rows(&tx, format!("SELECT document_json,source_version,content_hash,vector_id,vector_dense_json FROM {} WHERE vector_pending=TRUE LIMIT {limit}", sql.ident(table)), sql).await?;
                for row in pending {
                    let document = decode(&row)?;
                    let id: String = get(&row, "vector_id")?;
                    let version = get(&row, "source_version")?;
                    let embedding: Vec<f32> =
                        serde_json::from_str(&get::<String>(&row, "vector_dense_json")?)
                            .map_err(db_error)?;
                    let hash: String = get(&row, "content_hash")?;
                    batch.points.push(VectorItem::new(id.clone(), embedding, serde_json::json!({"index":index.name,"generation":generation.as_str(),"key":document.key,"content_hash":hash})));
                    batch.versions.push((id.clone(), version));
                    batch.claim.ids.push(id);
                }
            }
            Operation::Delete => {
                let pending: BTreeSet<String> =
                    serde_json::from_str(&get::<String>(&metadata, "pending_vectors_json")?)
                        .map_err(db_error)?;
                batch.claim.ids = pending.into_iter().take(BATCH_SIZE as usize).collect();
                if !batch.claim.ids.is_empty() {
                    let exists = sea_orm_migration::SchemaManager::new(&tx)
                        .has_table(table)
                        .await
                        .map_err(db_error)?;
                    for id in &batch.claim.ids {
                        if !exists || !references_vector(&tx, dialect, table, id).await? {
                            batch.deletions.push(id.clone());
                        }
                    }
                }
            }
        }
        if batch.claim.ids.is_empty() {
            tx.commit().await.map_err(db_error)?;
            return Ok(None);
        }
        // One generation claim serializes upserts with deletions, even across processes.
        // An interrupted worker leaves pending work intact until its bounded lease expires.
        ensure_unclaimed(&metadata)?;
        batch.claim.expires_at = now() + CLAIM_SECONDS;
        let mut settings = generation_settings(&metadata)?;
        settings.insert(
            CLAIM_KEY.into(),
            serde_json::to_value(&batch.claim).map_err(db_error)?,
        );
        update_generation_settings(&tx, dialect, index.name, generation, &settings).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(Some(batch))
    }

    /// Commit the claim before vector I/O because pgvector may need this pool's only
    /// connection. Bound the call below the lease so a failed worker can be retried.
    async fn dispatch(
        &self,
        batch: &mut Batch,
        operation: Operation,
    ) -> Result<(), FrameworkError> {
        let call = async {
            match operation {
                Operation::Upsert => {
                    vector_store(self.index.name)?
                        .upsert(std::mem::take(&mut batch.points))
                        .await
                }
                Operation::Delete if !batch.deletions.is_empty() => {
                    vector_store(self.index.name)?
                        .delete(batch.deletions.clone())
                        .await
                }
                Operation::Delete => Ok(()),
            }
        };
        tokio::time::timeout(Duration::from_secs(DISPATCH_SECONDS), call)
            .await
            .map_err(|_| db_error("vector delivery timed out; pending work can be retried"))?
    }

    async fn acknowledge(
        &self,
        batch: &Batch,
        operation: Operation,
        succeeded: bool,
    ) -> Result<(), FrameworkError> {
        let Self {
            index,
            generation,
            table,
            dialect,
        } = *self;
        let conn = DB::connection()?;
        crate::render_cache::write_side_open(false).await?;
        let tx = conn.inner().begin().await.map_err(db_error)?;
        DatabaseSearchEngine::lock(&tx, dialect, index.name, generation.as_str()).await?;
        let metadata =
            DatabaseSearchEngine::metadata(&tx, dialect, index.name, generation.as_str()).await?;
        let mut settings = generation_settings(&metadata)?;
        let claim: Claim = serde_json::from_value(
            settings
                .get(CLAIM_KEY)
                .cloned()
                .ok_or_else(|| db_error("vector delivery claim disappeared; retry"))?,
        )
        .map_err(db_error)?;
        if claim.token != batch.claim.token {
            return Err(db_error("vector delivery claim was replaced; retry"));
        }
        if succeeded {
            match operation {
                Operation::Upsert => {
                    ensure_ready(&tx, dialect, index.name, generation).await?;
                    let mut sql = Sql::new(dialect);
                    let mut fences = Vec::with_capacity(batch.versions.len());
                    for (id, version) in &batch.versions {
                        let id = sql.bind(id.clone());
                        let version = sql.bind(*version);
                        fences.push(format!("(vector_id={id} AND source_version={version})"));
                    }
                    // A newer SQL write must retain its pending marker, including when
                    // unchanged content reused the same immutable vector identity.
                    execute(
                        &tx,
                        format!(
                            "UPDATE {} SET vector_pending=FALSE WHERE {}",
                            sql.ident(table),
                            fences.join(" OR ")
                        ),
                        sql,
                    )
                    .await?;
                }
                Operation::Delete => {
                    let mut pending: BTreeSet<String> =
                        serde_json::from_str(&get::<String>(&metadata, "pending_vectors_json")?)
                            .map_err(db_error)?;
                    for id in &batch.claim.ids {
                        // A skipped, referenced ID may have been removed by a SQL
                        // writer during dispatch. Keep that new cleanup obligation.
                        if batch.deletions.contains(id)
                            || references_vector(&tx, dialect, table, id).await?
                        {
                            pending.remove(id);
                        }
                    }
                    let mut sql = Sql::new(dialect);
                    let pending = sql.bind(serde_json::to_string(&pending).map_err(db_error)?);
                    let i = sql.bind(index.name.to_owned());
                    let g = sql.bind(generation.as_str().to_owned());
                    execute(&tx, format!("UPDATE search_generations SET pending_vectors_json={pending} WHERE index_name={i} AND generation={g}"), sql).await?;
                }
            }
        }
        // Failure releases only the claim. Pending rows remain available to the retry.
        settings.remove(CLAIM_KEY);
        update_generation_settings(&tx, dialect, index.name, generation, &settings).await?;
        advance_transaction(&tx, index.name).await?;
        tx.commit().await.map_err(db_error)
    }

    async fn run(&self, operation: Operation) -> Result<(), FrameworkError> {
        while let Some(mut batch) = self.claim(operation).await? {
            let result = self.dispatch(&mut batch, operation).await;
            let acknowledgement = self.acknowledge(&batch, operation, result.is_ok()).await;
            match (result, acknowledgement) {
                (Ok(()), Ok(())) => {}
                (Err(error), Ok(())) | (Ok(()), Err(error)) => return Err(error),
                (Err(error), Err(acknowledgement)) => {
                    return Err(db_error(format!(
                        "{error}; releasing vector claim: {acknowledgement}"
                    )));
                }
            }
        }
        Ok(())
    }
}

async fn references_vector<C: ConnectionTrait>(
    conn: &C,
    dialect: Dialect,
    table: &str,
    id: &str,
) -> Result<bool, FrameworkError> {
    let mut sql = Sql::new(dialect);
    let point = sql.bind(id.to_owned());
    let text = format!(
        "SELECT vector_id FROM {} WHERE vector_id={point} LIMIT 1",
        sql.ident(table)
    );
    Ok(!rows(conn, text, sql).await?.is_empty())
}

impl DatabaseSearchEngine {
    /// Release the SQL connection before deleting vectors so a shared pool can progress.
    pub(super) async fn drain_vectors(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        table: &str,
        dialect: Dialect,
    ) -> Result<(), FrameworkError> {
        Delivery {
            index,
            generation,
            table,
            dialect,
        }
        .run(Operation::Delete)
        .await
    }

    /// Claim and acknowledge SQL versions around external I/O so shared pools cannot deadlock.
    pub(super) async fn flush_vectors(
        &self,
        index: &IndexDescriptor,
        generation: &GenerationId,
        table: &str,
        dialect: Dialect,
    ) -> Result<(), FrameworkError> {
        Delivery {
            index,
            generation,
            table,
            dialect,
        }
        .run(Operation::Upsert)
        .await
    }
}
