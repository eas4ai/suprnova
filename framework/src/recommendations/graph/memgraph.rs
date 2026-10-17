//! Bolt persistence for the same facts and bounded hops as the database driver.
use super::{GraphEdge, GraphNode, GraphStore, Revocation, digest_facts, document_node_key};
use crate::{FrameworkError, recommendations::storage};
use async_trait::async_trait;
use neo4rs::{BoltType, Graph, Query, Row, Txn, query};
use serde::de::DeserializeOwned;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Mutex;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const BATCH_SIZE: usize = 200;
type Properties = HashMap<String, BoltType>;
type Pools = Mutex<HashMap<String, Arc<Graph>>>;
static POOLS: OnceLock<Pools> = OnceLock::new();

/// Persists graph facts in Memgraph while the framework owns visibility and ranking.
/// Clones and connections to the same URL share one Bolt connection pool.
#[derive(Clone)]
pub struct MemgraphGraphStore {
    graph: Arc<Graph>,
}

fn query_error(error: impl std::fmt::Display) -> FrameworkError {
    // The driver's message names what failed (a refused login, a query the server
    // rejected), which an operator needs. Credentials never reach it: the URL carries
    // none (RecommendConfig refuses one with a user part) and Bolt sends the password
    // only in its handshake, which Memgraph does not echo.
    FrameworkError::database(format!("Memgraph: {error}"))
}

async fn bounded<T>(
    operation: impl Future<Output = Result<T, neo4rs::Error>>,
) -> Result<T, FrameworkError> {
    tokio::time::timeout(QUERY_TIMEOUT, operation)
        .await
        .map_err(|_| FrameworkError::database("Memgraph operation timed out"))?
        .map_err(query_error)
}

async fn rows(txn: &mut Txn, statement: Query) -> Result<Vec<Row>, FrameworkError> {
    let mut stream = bounded(txn.execute(statement)).await?;
    let mut result = Vec::new();
    while let Some(row) = bounded(stream.next(txn.handle())).await? {
        result.push(row);
    }
    Ok(result)
}

fn get<T: DeserializeOwned>(row: &Row, key: &str) -> Result<T, FrameworkError> {
    row.get(key).map_err(query_error)
}

fn decode<T: DeserializeOwned>(json: String) -> Result<T, FrameworkError> {
    serde_json::from_str(&json).map_err(query_error)
}

fn optional_property(n: &neo4rs::Node, key: &str) -> Result<Option<String>, FrameworkError> {
    match n.get(key) {
        Err(neo4rs::DeError::NoSuchProperty) => Ok(None),
        result => result.map_err(query_error),
    }
}

fn node(row: &Row) -> Result<GraphNode, FrameworkError> {
    let n: neo4rs::Node = get(row, "fact")?;
    let target = match (
        optional_property(&n, "target_index")?,
        optional_property(&n, "target_key")?,
    ) {
        (Some(index), Some(key)) => Some(crate::RecommendationTarget { index, key }),
        _ => None,
    };
    Ok(GraphNode {
        key: n.get("key").map_err(query_error)?,
        kind: n.get("kind").map_err(query_error)?,
        target,
        scope: decode(n.get("scope_json").map_err(query_error)?)?,
        source_version: n.get("source_version").map_err(query_error)?,
        provenance: decode(n.get("provenance_json").map_err(query_error)?)?,
    })
}

fn edge(row: &Row) -> Result<GraphEdge, FrameworkError> {
    let e: neo4rs::Relation = get(row, "fact")?;
    Ok(GraphEdge {
        key: e.get("key").map_err(query_error)?,
        source: e.get("source").map_err(query_error)?,
        target: e.get("target").map_err(query_error)?,
        kind: e.get("kind").map_err(query_error)?,
        confidence_bps: e.get("confidence_bps").map_err(query_error)?,
        scope: decode(e.get("scope_json").map_err(query_error)?)?,
        source_version: e.get("source_version").map_err(query_error)?,
        provenance: decode(e.get("provenance_json").map_err(query_error)?)?,
        created_at: e.get("created_at").map_err(query_error)?,
        updated_at: e.get("updated_at").map_err(query_error)?,
    })
}

fn provenance(
    properties: &mut Properties,
    from: &super::GraphProvenance,
) -> Result<(), FrameworkError> {
    properties.insert("provenance_json".into(), storage::encode(from)?.into());
    if let Some(id) = from.event_id() {
        properties.insert("provenance_event".into(), id.into());
    }
    if let super::GraphProvenance::Relation {
        source,
        source_version,
        ..
    } = from
    {
        properties.insert(
            "provenance_source".into(),
            format!("{}\n{}", source.index, source.key).into(),
        );
        properties.insert("provenance_version".into(), (*source_version).into());
    }
    Ok(())
}

fn node_properties(n: &GraphNode) -> Result<Properties, FrameworkError> {
    let mut properties: Properties = [
        ("key".into(), n.key.clone().into()),
        ("kind".into(), n.kind.clone().into()),
        ("source_version".into(), n.source_version.into()),
        ("scope_json".into(), storage::encode(&n.scope)?.into()),
    ]
    .into();
    if let Some(target) = &n.target {
        properties.insert("target_index".into(), target.index.clone().into());
        properties.insert("target_key".into(), target.key.clone().into());
    }
    provenance(&mut properties, &n.provenance)?;
    Ok(properties)
}

fn edge_properties(e: &GraphEdge) -> Result<Properties, FrameworkError> {
    if !(0..=10_000).contains(&e.confidence_bps) {
        return Err(FrameworkError::param(
            "graph edge confidence must be 0 to 10000 basis points",
        ));
    }
    let mut properties: Properties = [
        ("key".into(), e.key.clone().into()),
        ("source".into(), e.source.clone().into()),
        ("target".into(), e.target.clone().into()),
        ("kind".into(), e.kind.clone().into()),
        ("confidence_bps".into(), i64::from(e.confidence_bps).into()),
        ("source_version".into(), e.source_version.into()),
        ("scope_json".into(), storage::encode(&e.scope)?.into()),
        ("created_at".into(), e.created_at.into()),
        ("updated_at".into(), e.updated_at.into()),
    ]
    .into();
    provenance(&mut properties, &e.provenance)?;
    Ok(properties)
}

async fn discard(txn: &mut Txn, generation: &str) -> Result<(), FrameworkError> {
    bounded(
        txn.run(
            query("MATCH (n:SuprnovaGraphNode {generation: $generation}) DETACH DELETE n")
                .param("generation", generation),
        ),
    )
    .await?;
    bounded(txn.run(query("MATCH (g:SuprnovaGraphGeneration {id: $generation}) WHERE g.state <> $current DELETE g").param("generation", generation).param("current", "current"))).await
}

impl MemgraphGraphStore {
    /// Opens and verifies a shared pool with a timeout, then creates lookup indexes.
    /// Credentials are used only by Bolt authentication, never by framework config.
    ///
    /// # Errors
    /// Returns a sanitized error when connecting, authenticating or indexing fails.
    pub async fn connect(
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        tokio::time::timeout(CONNECT_TIMEOUT, async {
            let mut pools = POOLS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .await;
            if let Some(graph) = pools.get(url) {
                return Ok(Self {
                    graph: graph.clone(),
                });
            }
            let config = neo4rs::ConfigBuilder::default()
                .uri(url)
                .user(username)
                .password(password)
                .db("memgraph")
                .build()
                .map_err(query_error)?;
            let graph = Graph::connect(config).await.map_err(query_error)?;
            // Index creation is idempotent in Memgraph and also verifies the lazy pool.
            for statement in [
                "CREATE INDEX ON :SuprnovaGraphNode(generation)",
                "CREATE INDEX ON :SuprnovaGraphNode(key)",
                "CREATE INDEX ON :SuprnovaGraphGeneration(id)",
                "CREATE INDEX ON :SuprnovaGraphGeneration(state)",
                "CREATE EDGE INDEX ON :SUPRNOVA_GRAPH_EDGE(key)",
                "CREATE EDGE INDEX ON :SUPRNOVA_GRAPH_EDGE(generation)",
            ] {
                graph.run(query(statement)).await.map_err(query_error)?;
            }
            let graph = Arc::new(graph);
            pools.insert(url.to_owned(), graph.clone());
            Ok(Self { graph })
        })
        .await
        .map_err(|_| FrameworkError::database("Memgraph connection timed out"))?
    }

    async fn read(&self, statement: Query) -> Result<Vec<Row>, FrameworkError> {
        let mut txn = bounded(self.graph.start_txn()).await?;
        let result = rows(&mut txn, statement).await?;
        bounded(txn.commit()).await?;
        Ok(result)
    }

    async fn write(&self, statement: Query) -> Result<(), FrameworkError> {
        let mut txn = bounded(self.graph.start_txn()).await?;
        bounded(txn.run(statement)).await?;
        bounded(txn.commit()).await
    }
}

#[async_trait]
impl GraphStore for MemgraphGraphStore {
    fn name(&self) -> &'static str {
        "memgraph"
    }

    async fn current_generation(&self) -> Result<Option<String>, FrameworkError> {
        self.read(
            query("MATCH (g:SuprnovaGraphGeneration {state: $state}) RETURN g.id AS id")
                .param("state", "current"),
        )
        .await?
        .first()
        .map(|row| get(row, "id"))
        .transpose()
    }

    async fn begin_generation(&self) -> Result<String, FrameworkError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.write(query("CREATE (:SuprnovaGraphGeneration {id: $id, state: $state, created_at: $created_at})")
            .param("id", id.clone()).param("state", "building").param("created_at", crate::clock::now().timestamp_micros())).await?;
        Ok(id)
    }

    async fn publish(&self, generation: &str) -> Result<(), FrameworkError> {
        let mut txn = bounded(self.graph.start_txn()).await?;
        let known = rows(
            &mut txn,
            query("MATCH (g:SuprnovaGraphGeneration {id: $generation}) RETURN g.state AS state")
                .param("generation", generation),
        )
        .await?;
        let state: Option<String> = known.first().map(|row| get(row, "state")).transpose()?;
        match state.as_deref() {
            None | Some("retired") => {
                bounded(txn.rollback()).await?;
                return Err(FrameworkError::param(
                    "graph generation does not exist or was retired",
                ));
            }
            Some("current") => return bounded(txn.commit()).await,
            _ => {}
        }
        bounded(txn.run(query("MATCH (g:SuprnovaGraphGeneration) WHERE g.state = $current OR g.id = $generation SET g.state = CASE WHEN g.id = $generation THEN $current ELSE $retired END")
            .param("generation", generation).param("current", "current").param("retired", "retired"))).await?;
        let retired = rows(&mut txn, query("MATCH (g:SuprnovaGraphGeneration {state: $state}) RETURN g.id AS id ORDER BY g.created_at DESC, g.id DESC").param("state", "retired")).await?;
        for row in retired.iter().skip(1) {
            discard(&mut txn, &get::<String>(row, "id")?).await?;
        }
        bounded(txn.commit()).await
    }

    async fn discard(&self, generation: &str) -> Result<(), FrameworkError> {
        let mut txn = bounded(self.graph.start_txn()).await?;
        discard(&mut txn, generation).await?;
        bounded(txn.commit()).await
    }

    async fn put_nodes(&self, generation: &str, nodes: &[GraphNode]) -> Result<(), FrameworkError> {
        for chunk in nodes.chunks(50) {
            let facts = chunk
                .iter()
                .map(node_properties)
                .collect::<Result<Vec<_>, _>>()?;
            self.write(query("UNWIND $facts AS fact MERGE (n:SuprnovaGraphNode {generation: $generation, key: fact.key}) SET n = fact, n.generation = $generation")
                .param("generation", generation).param("facts", facts)).await?;
        }
        Ok(())
    }

    async fn put_edges(&self, generation: &str, edges: &[GraphEdge]) -> Result<(), FrameworkError> {
        for chunk in edges.chunks(50) {
            let facts = chunk
                .iter()
                .map(edge_properties)
                .collect::<Result<Vec<_>, _>>()?;
            // Placeholders preserve SQL's ability to accept an edge before its nodes.
            // Only materialized nodes (with scope_json) are returned to readers.
            self.write(query("UNWIND $facts AS fact OPTIONAL MATCH (previous_source:SuprnovaGraphNode {generation: $generation})-[old:SUPRNOVA_GRAPH_EDGE {generation: $generation, key: fact.key}]->() WITH fact, collect(old) AS previous, coalesce(min(old.created_at), fact.created_at) AS created FOREACH (r IN previous | DELETE r) MERGE (s:SuprnovaGraphNode {generation: $generation, key: fact.source}) MERGE (t:SuprnovaGraphNode {generation: $generation, key: fact.target}) CREATE (s)-[e:SUPRNOVA_GRAPH_EDGE]->(t) SET e = fact, e.generation = $generation, e.created_at = created")
                .param("generation", generation).param("facts", facts)).await?;
        }
        Ok(())
    }

    async fn revoke(
        &self,
        generation: &str,
        revocation: &Revocation,
    ) -> Result<u64, FrameworkError> {
        let (mode, key, source, version, events) = match revocation {
            Revocation::Document(target) => (
                "document",
                document_node_key(target),
                format!("{}\n{}", target.index, target.key),
                0,
                vec![],
            ),
            Revocation::DocumentBefore(target, version) => (
                "before",
                document_node_key(target),
                format!("{}\n{}", target.index, target.key),
                *version,
                vec![],
            ),
            Revocation::Events(ids) => ("events", String::new(), String::new(), 0, ids.clone()),
        };
        let bind = |q: Query| {
            q.param("generation", generation)
                .param("mode", mode)
                .param("document_mode", "document")
                .param("before_mode", "before")
                .param("events_mode", "events")
                .param("key", key.clone())
                .param("source", source.clone())
                .param("version", version)
                .param("events", events.clone())
        };
        let mut txn = bounded(self.graph.start_txn()).await?;
        // Select nodes first so every edge removed by DETACH is included in the count.
        let nodes = rows(&mut txn, bind(query("MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.scope_json IS NOT NULL AND (($mode = $document_mode AND (n.key = $key OR n.provenance_source = $source)) OR ($mode = $before_mode AND ((n.key = $key AND n.source_version < $version) OR (n.provenance_source = $source AND n.provenance_version < $version))) OR ($mode = $events_mode AND n.provenance_event IN $events)) RETURN n.key AS key"))).await?;
        let keys = nodes
            .iter()
            .map(|row| get::<String>(row, "key"))
            .collect::<Result<Vec<_>, _>>()?;
        let removed = rows(&mut txn, bind(query("MATCH (s:SuprnovaGraphNode)-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->(t:SuprnovaGraphNode) WHERE s.key IN $removed OR t.key IN $removed OR s.scope_json IS NULL OR t.scope_json IS NULL OR ($mode = $document_mode AND (e.source = $key OR e.target = $key OR e.provenance_source = $source)) OR ($mode = $before_mode AND e.provenance_source = $source AND e.provenance_version < $version) OR ($mode = $events_mode AND e.provenance_event IN $events) DELETE e RETURN count(e) AS removed")).param("removed", keys.clone())).await?;
        let edge_count: i64 = removed
            .first()
            .map(|row| get(row, "removed"))
            .transpose()?
            .unwrap_or(0);
        bounded(txn.run(query("MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.key IN $keys DETACH DELETE n").param("generation", generation).param("keys", keys.clone()))).await?;
        bounded(txn.commit()).await?;
        u64::try_from(edge_count)
            .ok()
            .and_then(|count| count.checked_add(keys.len() as u64))
            .ok_or_else(|| {
                FrameworkError::database("Memgraph revocation count exceeds the unsigned range")
            })
    }

    async fn nodes(
        &self,
        generation: &str,
        keys: &[String],
    ) -> Result<Vec<GraphNode>, FrameworkError> {
        let mut result = Vec::new();
        for chunk in keys.chunks(BATCH_SIZE) {
            result.extend(self.read(query("MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.key IN $keys AND n.scope_json IS NOT NULL RETURN n AS fact ORDER BY n.key")
                .param("generation", generation).param("keys", chunk)).await?.iter().map(node).collect::<Result<Vec<_>, _>>()?);
        }
        Ok(result)
    }

    async fn outgoing(
        &self,
        generation: &str,
        sources: &[String],
        kinds: &[String],
        per_source: usize,
    ) -> Result<Vec<GraphEdge>, FrameworkError> {
        let limit = i64::try_from(per_source.max(1))
            .map_err(|_| FrameworkError::param("graph fan-out exceeds the signed range"))?;
        let mut result = Vec::new();
        for chunk in sources.chunks(BATCH_SIZE) {
            result.extend(self.read(query("MATCH (s:SuprnovaGraphNode {generation: $generation})-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->() WHERE s.key IN $sources AND (size($kinds) = 0 OR e.kind IN $kinds) WITH s, e ORDER BY e.confidence_bps DESC, e.key WITH s, collect(e)[..$limit] AS edges UNWIND edges AS e RETURN e AS fact ORDER BY s.key, e.confidence_bps DESC, e.key")
                .param("generation", generation).param("sources", chunk).param("kinds", kinds).param("limit", limit)).await?.iter().map(edge).collect::<Result<Vec<_>, _>>()?);
        }
        Ok(result)
    }

    async fn document_nodes(
        &self,
        generation: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GraphNode>, FrameworkError> {
        let limit = i64::try_from(limit.max(1))
            .map_err(|_| FrameworkError::param("graph sweep limit exceeds the signed range"))?;
        self.read(query("MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.target_index IS NOT NULL AND n.key > $after RETURN n AS fact ORDER BY n.key LIMIT $limit")
            .param("generation", generation).param("after", after.unwrap_or("")).param("limit", limit)).await?.iter().map(node).collect()
    }

    async fn fact_digest(&self, generation: &str) -> Result<String, FrameworkError> {
        let nodes = self.read(query("MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.scope_json IS NOT NULL RETURN n AS fact ORDER BY n.key").param("generation", generation)).await?
            .iter().map(node).collect::<Result<Vec<_>, _>>()?;
        let edges = self.read(query("MATCH ()-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->() RETURN e AS fact ORDER BY e.key").param("generation", generation)).await?
            .iter().map(edge).collect::<Result<Vec<_>, _>>()?;
        digest_facts(&nodes, &edges)
    }
}
