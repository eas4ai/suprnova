//! The default store: the graph in the application's own database, so the graph engine
//! needs no service beyond it. One hop is one query per batch of sources.
use super::{
    GraphEdge, GraphNode, GraphProvenance, GraphScope, GraphStore, Revocation, digest_facts,
    document_node_key,
};
use crate::{
    FrameworkError,
    recommendations::{RecommendationTarget, storage},
};
use async_trait::async_trait;
use sea_orm::Value;

/// Rows bound per statement: up to twelve binds each stay below SQLite's limit.
const ROWS_PER_STATEMENT: usize = 50;
/// Keys bound per `IN` list.
const KEYS_PER_STATEMENT: usize = 200;

/// The graph in the application's database (SQLite, Postgres, MySQL or MariaDB), in the
/// tables the recommendation migrations create.
#[derive(Debug, Clone, Default)]
pub struct DatabaseGraphStore;

fn source_column(target: &RecommendationTarget) -> String {
    format!("{}\n{}", target.index, target.key)
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

/// The provenance columns revocation selects by: the event, or the document and the
/// version a relation was read from.
fn provenance_columns(provenance: &GraphProvenance) -> [Value; 3] {
    let version = match provenance {
        GraphProvenance::Relation { source_version, .. } => Some(*source_version),
        GraphProvenance::Event(_) => None,
    };
    [
        provenance.event_id().into(),
        provenance.source().map(source_column).into(),
        version.into(),
    ]
}

fn conflict(keys: &str, columns: &[&str]) -> Result<String, FrameworkError> {
    Ok(if storage::backend()? == sea_orm::DbBackend::MySql {
        let set = columns
            .iter()
            .map(|column| format!("{column} = VALUES({column})"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("ON DUPLICATE KEY UPDATE {set}")
    } else {
        let set = columns
            .iter()
            .map(|column| format!("{column} = excluded.{column}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("ON CONFLICT ({keys}) DO UPDATE SET {set}")
    })
}

fn node(row: &crate::DynamicRow) -> Result<GraphNode, FrameworkError> {
    let target = match (
        row.get_optional_string("target_index")?,
        row.get_optional_string("target_key")?,
    ) {
        (Some(index), Some(key)) => Some(RecommendationTarget { index, key }),
        _ => None,
    };
    Ok(GraphNode {
        key: row.get_string("node_key")?,
        kind: row.get_string("kind")?,
        target,
        scope: storage::decode::<GraphScope>(row, "scope_json")?,
        source_version: row.get_int("source_version")?,
        provenance: storage::decode(row, "provenance_json")?,
    })
}

fn edge(row: &crate::DynamicRow) -> Result<GraphEdge, FrameworkError> {
    Ok(GraphEdge {
        key: row.get_string("edge_key")?,
        source: row.get_string("source_key")?,
        target: row.get_string("target_key")?,
        kind: row.get_string("kind")?,
        confidence_bps: i32::try_from(row.get_int("confidence_bps")?)
            .map_err(|_| FrameworkError::database("graph edge confidence is out of range"))?,
        scope: storage::decode(row, "scope_json")?,
        source_version: row.get_int("source_version")?,
        provenance: storage::decode(row, "provenance_json")?,
        created_at: row.get_int("created_at")?,
        updated_at: row.get_int("updated_at")?,
    })
}

#[async_trait]
impl GraphStore for DatabaseGraphStore {
    fn name(&self) -> &'static str {
        "database"
    }

    async fn current_generation(&self) -> Result<Option<String>, FrameworkError> {
        Ok(storage::rows(
            "SELECT id FROM recommendation_graph_generations WHERE state = 'current'",
            vec![],
        )
        .await?
        .first()
        .map(|row| row.get_string("id"))
        .transpose()?)
    }

    async fn begin_generation(&self) -> Result<String, FrameworkError> {
        let id = uuid::Uuid::new_v4().to_string();
        storage::execute(
            "INSERT INTO recommendation_graph_generations (id, state, created_at) VALUES (?, 'building', ?)",
            vec![id.clone().into(), crate::clock::now().timestamp_micros().into()],
        )
        .await?;
        Ok(id)
    }

    async fn publish(&self, generation: &str) -> Result<(), FrameworkError> {
        let known = storage::rows(
            "SELECT state FROM recommendation_graph_generations WHERE id = ?",
            vec![generation.to_owned().into()],
        )
        .await?;
        match known
            .first()
            .map(|row| row.get_string("state"))
            .transpose()?
            .as_deref()
        {
            None | Some("retired") => {
                return Err(FrameworkError::param(format!(
                    "graph generation {generation} does not exist or was retired"
                )));
            }
            Some("current") => return Ok(()),
            Some(_) => {}
        }
        // One statement switches readers, so no reader sees two current generations.
        storage::execute(
            "UPDATE recommendation_graph_generations SET state = CASE WHEN id = ? THEN 'current' ELSE 'retired' END WHERE state = 'current' OR id = ?",
            vec![generation.to_owned().into(), generation.to_owned().into()],
        )
        .await?;
        // A reader that started on the previous generation finishes on it; anything
        // older than that has no reader left.
        let retired = storage::rows(
            "SELECT id FROM recommendation_graph_generations WHERE state = 'retired' ORDER BY created_at DESC, id DESC",
            vec![],
        )
        .await?;
        for row in retired.iter().skip(1) {
            self.discard(&row.get_string("id")?).await?;
        }
        Ok(())
    }

    async fn discard(&self, generation: &str) -> Result<(), FrameworkError> {
        for table in ["recommendation_graph_edges", "recommendation_graph_nodes"] {
            storage::execute(
                &format!("DELETE FROM {table} WHERE generation_id = ?"),
                vec![generation.to_owned().into()],
            )
            .await?;
        }
        storage::execute(
            "DELETE FROM recommendation_graph_generations WHERE id = ? AND state <> 'current'",
            vec![generation.to_owned().into()],
        )
        .await?;
        Ok(())
    }

    async fn put_nodes(&self, generation: &str, nodes: &[GraphNode]) -> Result<(), FrameworkError> {
        let columns = [
            "kind",
            "target_index",
            "target_key",
            "scope_json",
            "source_version",
            "provenance_json",
            "provenance_event",
            "provenance_source",
            "provenance_version",
        ];
        let suffix = conflict("generation_id, node_key", &columns)?;
        for chunk in nodes.chunks(ROWS_PER_STATEMENT) {
            let mut values = Vec::with_capacity(chunk.len() * 11);
            for node in chunk {
                let [event, source, version] = provenance_columns(&node.provenance);
                values.extend([
                    generation.to_owned().into(),
                    node.key.clone().into(),
                    node.kind.clone().into(),
                    node.target
                        .as_ref()
                        .map(|target| target.index.clone())
                        .into(),
                    node.target.as_ref().map(|target| target.key.clone()).into(),
                    storage::encode(&node.scope)?.into(),
                    node.source_version.into(),
                    storage::encode(&node.provenance)?.into(),
                    event,
                    source,
                    version,
                ]);
            }
            let rows = vec!["(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"; chunk.len()].join(", ");
            storage::execute(
                &format!("INSERT INTO recommendation_graph_nodes (generation_id, node_key, kind, target_index, target_key, scope_json, source_version, provenance_json, provenance_event, provenance_source, provenance_version) VALUES {rows} {suffix}"),
                values,
            )
            .await?;
        }
        Ok(())
    }

    async fn put_edges(&self, generation: &str, edges: &[GraphEdge]) -> Result<(), FrameworkError> {
        let columns = [
            "source_key",
            "target_key",
            "kind",
            "confidence_bps",
            "scope_json",
            "source_version",
            "provenance_json",
            "provenance_event",
            "provenance_source",
            "provenance_version",
            "updated_at",
        ];
        let suffix = conflict("generation_id, edge_key", &columns)?;
        for chunk in edges.chunks(ROWS_PER_STATEMENT) {
            let mut values = Vec::with_capacity(chunk.len() * 14);
            for edge in chunk {
                if !(0..=10_000).contains(&edge.confidence_bps) {
                    return Err(FrameworkError::param(format!(
                        "graph edge {} confidence must be 0 to 10000 basis points",
                        edge.key
                    )));
                }
                let [event, source, version] = provenance_columns(&edge.provenance);
                values.extend([
                    generation.to_owned().into(),
                    edge.key.clone().into(),
                    edge.source.clone().into(),
                    edge.target.clone().into(),
                    edge.kind.clone().into(),
                    edge.confidence_bps.into(),
                    storage::encode(&edge.scope)?.into(),
                    edge.source_version.into(),
                    storage::encode(&edge.provenance)?.into(),
                    event,
                    source,
                    version,
                    edge.created_at.into(),
                    edge.updated_at.into(),
                ]);
            }
            let rows = vec!["(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"; chunk.len()].join(", ");
            storage::execute(
                &format!("INSERT INTO recommendation_graph_edges (generation_id, edge_key, source_key, target_key, kind, confidence_bps, scope_json, source_version, provenance_json, provenance_event, provenance_source, provenance_version, created_at, updated_at) VALUES {rows} {suffix}"),
                values,
            )
            .await?;
        }
        Ok(())
    }

    async fn revoke(
        &self,
        generation: &str,
        revocation: &Revocation,
    ) -> Result<u64, FrameworkError> {
        let generation_value = || Value::from(generation.to_owned());
        let mut removed = 0;
        match revocation {
            Revocation::Document(target) => {
                let key = document_node_key(target);
                let source = source_column(target);
                removed += storage::execute(
                    "DELETE FROM recommendation_graph_edges WHERE generation_id = ? AND (source_key = ? OR target_key = ? OR provenance_source = ?)",
                    vec![generation_value(), key.clone().into(), key.clone().into(), source.clone().into()],
                )
                .await?;
                removed += storage::execute(
                    "DELETE FROM recommendation_graph_nodes WHERE generation_id = ? AND (node_key = ? OR provenance_source = ?)",
                    vec![generation_value(), key.into(), source.into()],
                )
                .await?;
            }
            Revocation::DocumentBefore(target, version) => {
                let key = document_node_key(target);
                let source = source_column(target);
                removed += storage::execute(
                    "DELETE FROM recommendation_graph_edges WHERE generation_id = ? AND provenance_source = ? AND provenance_version < ?",
                    vec![generation_value(), source.clone().into(), (*version).into()],
                )
                .await?;
                removed += storage::execute(
                    "DELETE FROM recommendation_graph_nodes WHERE generation_id = ? AND ((node_key = ? AND source_version < ?) OR (provenance_source = ? AND provenance_version < ?))",
                    vec![generation_value(), key.into(), (*version).into(), source.into(), (*version).into()],
                )
                .await?;
            }
            Revocation::Events(ids) => {
                for chunk in ids.chunks(KEYS_PER_STATEMENT) {
                    let list = placeholders(chunk.len());
                    for table in ["recommendation_graph_edges", "recommendation_graph_nodes"] {
                        let mut values = vec![generation_value()];
                        values.extend(chunk.iter().map(|id| Value::from(*id)));
                        removed += storage::execute(
                            &format!("DELETE FROM {table} WHERE generation_id = ? AND provenance_event IN ({list})"),
                            values,
                        )
                        .await?;
                    }
                }
            }
        }
        // An edge whose node left goes with it, so nothing points at a removed fact.
        removed += storage::execute(
            "DELETE FROM recommendation_graph_edges WHERE generation_id = ? AND (NOT EXISTS (SELECT 1 FROM recommendation_graph_nodes n WHERE n.generation_id = recommendation_graph_edges.generation_id AND n.node_key = recommendation_graph_edges.source_key) OR NOT EXISTS (SELECT 1 FROM recommendation_graph_nodes n WHERE n.generation_id = recommendation_graph_edges.generation_id AND n.node_key = recommendation_graph_edges.target_key))",
            vec![generation_value()],
        )
        .await?;
        Ok(removed)
    }

    async fn nodes(
        &self,
        generation: &str,
        keys: &[String],
    ) -> Result<Vec<GraphNode>, FrameworkError> {
        let mut nodes = Vec::with_capacity(keys.len());
        for chunk in keys.chunks(KEYS_PER_STATEMENT) {
            let mut values = vec![Value::from(generation.to_owned())];
            values.extend(chunk.iter().map(|key| Value::from(key.clone())));
            for row in storage::rows(
                &format!("SELECT * FROM recommendation_graph_nodes WHERE generation_id = ? AND node_key IN ({}) ORDER BY node_key", placeholders(chunk.len())),
                values,
            )
            .await?
            {
                nodes.push(node(&row)?);
            }
        }
        Ok(nodes)
    }

    async fn outgoing(
        &self,
        generation: &str,
        sources: &[String],
        kinds: &[String],
        per_source: usize,
    ) -> Result<Vec<GraphEdge>, FrameworkError> {
        let per_source = i64::try_from(per_source.max(1))
            .map_err(|_| FrameworkError::param("graph fan-out exceeds the signed range"))?;
        let mut edges = Vec::new();
        for chunk in sources.chunks(KEYS_PER_STATEMENT) {
            let mut values = vec![Value::from(generation.to_owned())];
            values.extend(chunk.iter().map(|key| Value::from(key.clone())));
            let kind_filter = if kinds.is_empty() {
                String::new()
            } else {
                values.extend(kinds.iter().map(|kind| Value::from(kind.clone())));
                format!(" AND kind IN ({})", placeholders(kinds.len()))
            };
            values.push(per_source.into());
            for row in storage::rows(
                &format!("SELECT * FROM (SELECT e.*, ROW_NUMBER() OVER (PARTITION BY source_key ORDER BY confidence_bps DESC, edge_key) AS hop_rank FROM recommendation_graph_edges e WHERE generation_id = ? AND source_key IN ({}){kind_filter}) ranked WHERE hop_rank <= ? ORDER BY source_key, confidence_bps DESC, edge_key", placeholders(chunk.len())),
                values,
            )
            .await?
            {
                edges.push(edge(&row)?);
            }
        }
        Ok(edges)
    }

    async fn document_nodes(
        &self,
        generation: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GraphNode>, FrameworkError> {
        let limit = i64::try_from(limit.max(1))
            .map_err(|_| FrameworkError::param("graph sweep limit exceeds the signed range"))?;
        storage::rows(
            "SELECT * FROM recommendation_graph_nodes WHERE generation_id = ? AND target_index IS NOT NULL AND node_key > ? ORDER BY node_key LIMIT ?",
            vec![generation.to_owned().into(), after.unwrap_or("").to_owned().into(), limit.into()],
        )
        .await?
        .iter()
        .map(node)
        .collect()
    }

    async fn fact_digest(&self, generation: &str) -> Result<String, FrameworkError> {
        let nodes = storage::rows(
            "SELECT * FROM recommendation_graph_nodes WHERE generation_id = ? ORDER BY node_key",
            vec![generation.to_owned().into()],
        )
        .await?
        .iter()
        .map(node)
        .collect::<Result<Vec<_>, _>>()?;
        let edges = storage::rows(
            "SELECT * FROM recommendation_graph_edges WHERE generation_id = ? ORDER BY edge_key",
            vec![generation.to_owned().into()],
        )
        .await?
        .iter()
        .map(edge)
        .collect::<Result<Vec<_>, _>>()?;
        digest_facts(&nodes, &edges)
    }
}
