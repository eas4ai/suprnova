//! HydraDB's native graph operations use auto-commit queries and automatic indexes.
mod statements;
mod transport;

use super::{
    GraphEdge, GraphNode, GraphProvenance, GraphStore, Revocation, digest_facts, document_node_key,
};
use crate::FrameworkError;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};
use tokio::sync::Mutex;
use transport::{Record, Transport};

/// Persists recommendation facts in HydraDB's object-store-backed graph.
/// Current HydraDB requires HTTPS because neo4rs 0.8 cannot negotiate Bolt 5.
/// Bolt addresses use the same native statements when the endpoint accepts neo4rs.
#[derive(Clone)]
pub struct HydraDbGraphStore {
    transport: Arc<Transport>,
}

enum IdentityKind {
    Node,
    Edge,
}

type Pools = Mutex<HashMap<String, Arc<Transport>>>;
static POOLS: OnceLock<Pools> = OnceLock::new();

fn error(message: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::database(format!("HydraDB: {message}"))
}

fn identity(generation: &str, key: &str) -> i64 {
    let mut digest = Sha256::new();
    for part in ["suprnova", generation, key] {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    let digest = digest.finalize();
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes) & i64::MAX
}

fn text<'a>(record: &'a Record, key: &str) -> Result<Option<&'a str>, FrameworkError> {
    match record.get(key) {
        Some(Value::String(value)) => Ok(Some(value)),
        Some(Value::Null) => Ok(None),
        _ => Err(error(format!("invalid {key} projection"))),
    }
}

fn decode<T: DeserializeOwned>(records: &[Record]) -> Result<Vec<T>, FrameworkError> {
    records
        .iter()
        .filter_map(|row| match text(row, "json") {
            Ok(None) => None,
            Ok(Some(value)) => Some(serde_json::from_str(value).map_err(error)),
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn provenance(fact: &GraphProvenance) -> Value {
    match fact {
        GraphProvenance::Event(event) => {
            json!({"event":event.id.to_string(), "provenance_source":"", "provenance_version":0})
        }
        GraphProvenance::Relation {
            source,
            source_version,
            ..
        } => {
            json!({"event":"", "provenance_source":document_node_key(source), "provenance_version":source_version})
        }
    }
}

fn fact(generation: &str, key: &str, from: &GraphProvenance) -> Value {
    let mut value = provenance(from);
    value["vertex"] = json!(identity(generation, key));
    value["generation"] = json!(generation);
    value["key"] = json!(key);
    value
}

fn revocation_predicate(revocation: &Revocation, binding: &str, parameters: &mut Value) -> String {
    match revocation {
        Revocation::Events(events) => {
            let terms: Vec<_> = events
                .iter()
                .enumerate()
                .map(|(i, event)| {
                    parameters[format!("event{i}")] = json!(event.to_string());
                    format!("{binding}.event = $event{i}")
                })
                .collect();
            if terms.is_empty() {
                format!("{binding}.key <> {binding}.key")
            } else {
                terms.join(" OR ")
            }
        }
        Revocation::Document(target) => {
            parameters["key"] = json!(document_node_key(target));
            if binding == "n" {
                "n.key = $key OR n.provenance_source = $key"
            } else {
                "e.source = $key OR e.target = $key OR e.provenance_source = $key"
            }
            .into()
        }
        Revocation::DocumentBefore(target, version) => {
            parameters["key"] = json!(document_node_key(target));
            parameters["version"] = json!(version);
            if binding == "n" { "(n.key = $key AND n.source_version < $version) OR (n.provenance_source = $key AND n.provenance_version < $version)" } else { "e.provenance_source = $key AND e.provenance_version < $version" }.into()
        }
        Revocation::DocumentRelationsBefore(target, version) => {
            parameters["key"] = json!(document_node_key(target));
            parameters["version"] = json!(version);
            if binding == "n" {
                "n.key <> $key AND n.provenance_source = $key AND n.provenance_version < $version"
            } else {
                "e.provenance_source = $key AND e.provenance_version < $version"
            }
            .into()
        }
    }
}

impl HydraDbGraphStore {
    /// Verifies the native query endpoint without issuing unsupported index DDL.
    /// HTTPS uses the username as namespace (default `default`) and password as bearer token.
    /// Add `?cell_id=<cell>` to the query URL when the cell is not `cell-0`.
    ///
    /// # Errors
    /// Rejects invalid URLs, credentials, transport failures or an incompatible dialect.
    pub async fn connect(
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        crate::RecommendConfig {
            graph_store: "hydradb".into(),
            graph_url: Some(url.into()),
            ..Default::default()
        }
        .validate()?;
        let mut identity = Sha256::new();
        for part in [url, username, password] {
            identity.update(part.as_bytes());
            identity.update([0]);
        }
        let key = hex::encode(identity.finalize());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut pools = POOLS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .await;
            if let Some(transport) = pools.get(&key) {
                return Ok(Self {
                    transport: transport.clone(),
                });
            }
            let transport = Arc::new(Transport::connect(url, username, password).await?);
            let store = Self {
                transport: transport.clone(),
            };
            for statement in statements::INDEX_DDL {
                store.run(statement, json!({})).await?;
            }
            // A label-only MERGE never resets a published generation on reconnect.
            store
                .run(
                    statements::HEAD,
                    json!({"facts":[{"vertex":self::identity("head", "head")}]}),
                )
                .await?;
            pools.insert(key, transport);
            Ok(store)
        })
        .await
        .map_err(|_| error("connection timed out"))?
    }

    async fn run(&self, statement: &str, parameters: Value) -> Result<(), FrameworkError> {
        self.transport.execute(statement, parameters, &[]).await?;
        Ok(())
    }

    async fn head(&self) -> Result<(Option<String>, Option<String>), FrameworkError> {
        let records = self
            .transport
            .execute(
                statements::READ_HEAD,
                json!({}),
                &["generation", "previous"],
            )
            .await?;
        let row = records
            .first()
            .ok_or_else(|| error("publication head is missing"))?;
        let pointer = |key| -> Result<Option<String>, FrameworkError> {
            Ok(text(row, key)?.filter(|s| !s.is_empty()).map(str::to_owned))
        };
        Ok((pointer("generation")?, pointer("previous")?))
    }

    async fn scan<T: DeserializeOwned>(
        &self,
        statement: &str,
        mut parameters: Value,
    ) -> Result<Vec<T>, FrameworkError> {
        let mut result = Vec::new();
        let mut after = String::new();
        loop {
            parameters["after"] = json!(after);
            parameters["limit"] = json!(500);
            let records = self
                .transport
                .execute(statement, parameters.clone(), &["json", "key"])
                .await?;
            if records.is_empty() {
                return Ok(result);
            }
            let next = text(records.last().ok_or_else(|| error("missing page"))?, "key")?
                .ok_or_else(|| error("missing fact key"))?;
            if next <= after.as_str() {
                return Err(error("fact page did not advance"));
            }
            after = next.into();
            result.extend(decode::<T>(&records)?);
        }
    }

    async fn check_identities(
        &self,
        generation: &str,
        keys: &[&str],
        kind: IdentityKind,
    ) -> Result<(), FrameworkError> {
        let mut identities = HashMap::new();
        for key in keys {
            if identities
                .insert(identity(generation, key), *key)
                .is_some_and(|old| old != *key)
            {
                return Err(error(
                    "numeric graph identity collision; rebuild in a new generation",
                ));
            }
        }
        if identities.is_empty() {
            return Ok(());
        }
        let mut parameters = json!({});
        let terms: Vec<_> = identities
            .keys()
            .enumerate()
            .map(|(i, vertex)| {
                parameters[format!("vertex{i}")] = json!(vertex);
                format!("n.id = $vertex{i}")
            })
            .collect();
        // Unlabelled numeric lookups in HydraDB can project an absent implicit vertex.
        // Labels distinguish persisted records from that virtual, property-less result.
        let statement = match kind {
            IdentityKind::Node => [
            statements::IDENTITY,
            "MATCH (n:SuprnovaGraphGeneration)",
            "MATCH (n:SuprnovaGraphHead)",
        ]
        .iter()
        .map(|pattern| {
            format!(
                "{pattern} WHERE {} RETURN n.key AS key, n.generation AS generation",
                terms.join(" OR ")
            )
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL "),
            IdentityKind::Edge => identities.keys().enumerate().map(|(i, _)| format!("MATCH (s)-[e:SUPRNOVA_GRAPH_EDGE {{id: $vertex{i}}}]->(t) RETURN e.key AS key, e.generation AS generation")).collect::<Vec<_>>().join(" UNION ALL "),
        };
        let records = self
            .transport
            .execute(&statement, parameters, &["key", "generation"])
            .await?;
        for row in records {
            let key = text(&row, "key")?
                .ok_or_else(|| error("numeric identity belongs to another record"))?;
            if identities.get(&identity(generation, key)).copied() != Some(key)
                || text(&row, "generation")? != Some(generation)
            {
                return Err(error(
                    "numeric graph identity collision; rebuild in a new generation",
                ));
            }
        }
        Ok(())
    }

    async fn discard_facts(&self, generation: &str) -> Result<(), FrameworkError> {
        self.run(statements::DISCARD_NODES, json!({"generation":generation}))
            .await?;
        self.run(
            statements::DISCARD_GENERATION,
            json!({"generation":generation}),
        )
        .await
    }
}

#[async_trait]
impl GraphStore for HydraDbGraphStore {
    fn name(&self) -> &'static str {
        "hydradb"
    }

    async fn current_generation(&self) -> Result<Option<String>, FrameworkError> {
        Ok(self.head().await?.0)
    }

    async fn begin_generation(&self) -> Result<String, FrameworkError> {
        let generation = uuid::Uuid::new_v4().to_string();
        self.run(statements::BEGIN, json!({"facts":[{"vertex":identity("generation", &generation), "generation":generation, "state":"building"}]})).await?;
        Ok(generation)
    }

    async fn publish(&self, generation: &str) -> Result<(), FrameworkError> {
        let (current, previous) = self.head().await?;
        if current.as_deref() == Some(generation) {
            return Ok(());
        }
        let records = self
            .transport
            .execute(
                statements::GENERATION,
                json!({"generation":generation}),
                &["generation", "state"],
            )
            .await?;
        if records
            .first()
            .map(|row| text(row, "state"))
            .transpose()?
            .flatten()
            != Some("building")
        {
            return Err(FrameworkError::param(
                "graph generation does not exist or was retired",
            ));
        }
        // Clean the older retired generation first. A cleanup failure then leaves
        // the current pointer intact and the new build retryable.
        if let Some(previous) = previous {
            self.discard_facts(&previous).await?;
        }
        self.run(statements::PUBLISH, json!({"generation":generation, "previous":current.unwrap_or_default(),"state":"published"})).await?;
        Ok(())
    }

    async fn discard(&self, generation: &str) -> Result<(), FrameworkError> {
        if self.current_generation().await?.as_deref() == Some(generation) {
            return Err(FrameworkError::param(
                "cannot discard the current graph generation",
            ));
        }
        self.discard_facts(generation).await
    }

    async fn put_nodes(&self, generation: &str, nodes: &[GraphNode]) -> Result<(), FrameworkError> {
        for chunk in nodes.chunks(50) {
            let mut facts = Vec::new();
            self.check_identities(
                generation,
                &chunk.iter().map(|n| n.key.as_str()).collect::<Vec<_>>(),
                IdentityKind::Node,
            )
            .await?;
            for node in chunk {
                let mut value = fact(generation, &node.key, &node.provenance);
                value["materialized"] = json!(true);
                value["document"] = json!(node.target.is_some());
                value["source_version"] = json!(node.source_version);
                value["json"] = json!(serde_json::to_string(node).map_err(error)?);
                facts.push(value);
            }
            self.run(statements::PUT_NODES, json!({"facts":facts}))
                .await?;
        }
        Ok(())
    }

    async fn put_edges(&self, generation: &str, edges: &[GraphEdge]) -> Result<(), FrameworkError> {
        for chunk in edges.chunks(50) {
            if chunk
                .iter()
                .any(|edge| !(0..=10_000).contains(&edge.confidence_bps))
            {
                return Err(FrameworkError::param(
                    "graph edge confidence must be 0 to 10000 basis points",
                ));
            }
            self.check_identities(
                generation,
                &chunk
                    .iter()
                    .map(|edge| edge.key.as_str())
                    .collect::<Vec<_>>(),
                IdentityKind::Edge,
            )
            .await?;
            let mut parameters = json!({"generation":generation});
            let terms: Vec<_> = chunk
                .iter()
                .enumerate()
                .map(|(i, edge)| {
                    parameters[format!("key{i}")] = json!(edge.key);
                    format!("e.key = $key{i}")
                })
                .collect();
            let statement = format!(
                "{} WHERE {} RETURN e.json AS json",
                statements::EDGES_BY_KEY,
                terms.join(" OR ")
            );
            let records = self
                .transport
                .execute(&statement, parameters, &["json"])
                .await?;
            let previous: HashMap<_, _> = decode::<GraphEdge>(&records)?
                .into_iter()
                .map(|e| (e.key.clone(), e))
                .collect();
            let keys: Vec<_> = chunk
                .iter()
                .flat_map(|e| [e.source.as_str(), e.target.as_str()])
                .collect();
            self.check_identities(generation, &keys, IdentityKind::Node)
                .await?;
            self.run(statements::PLACEHOLDERS, json!({"facts":keys.iter().map(|key| json!({"vertex":identity(generation,key),"generation":generation,"key":key})).collect::<Vec<_>>()})).await?;
            let mut facts = Vec::new();
            let mut moved = Vec::new();
            for original in chunk {
                let mut edge = original.clone();
                if let Some(old) = previous.get(&edge.key) {
                    edge.created_at = old.created_at;
                    if old.source != edge.source || old.target != edge.target {
                        moved.push(json!({"vertex":identity(generation,&edge.key)}));
                    }
                }
                let mut value = fact(generation, &edge.key, &edge.provenance);
                value["source_vertex"] = json!(identity(generation, &edge.source));
                value["target_vertex"] = json!(identity(generation, &edge.target));
                value["source"] = json!(edge.source);
                value["target"] = json!(edge.target);
                value["kind"] = json!(edge.kind);
                value["confidence"] = json!(edge.confidence_bps);
                value["json"] = json!(serde_json::to_string(&edge).map_err(error)?);
                facts.push(value);
            }
            if !moved.is_empty() {
                self.run(statements::DELETE_EDGES, json!({"facts":moved}))
                    .await?;
            }
            self.run(statements::PUT_EDGES, json!({"facts":facts}))
                .await?;
        }
        Ok(())
    }

    async fn revoke(
        &self,
        generation: &str,
        revocation: &Revocation,
    ) -> Result<u64, FrameworkError> {
        let mut parameters = json!({"generation":generation});
        let node_predicate = revocation_predicate(revocation, "n", &mut parameters);
        let statement = statements::ALL_NODES.replace(
            "n.materialized = true",
            &format!("n.materialized = true AND ({node_predicate})"),
        );
        let nodes: Vec<GraphNode> = self.scan(&statement, parameters.clone()).await?;
        let mut edge_predicate = revocation_predicate(revocation, "e", &mut parameters);
        for (i, node) in nodes.iter().enumerate() {
            parameters[format!("node{i}")] = json!(node.key);
            edge_predicate.push_str(&format!(" OR e.source = $node{i} OR e.target = $node{i}"));
        }
        let statement = statements::ALL_EDGES.replace(
            "WHERE e.key > $after",
            &format!("WHERE ({edge_predicate}) AND e.key > $after"),
        );
        let edges: Vec<GraphEdge> = self.scan(&statement, parameters).await?;
        for chunk in edges.chunks(50) {
            self.run(statements::DELETE_EDGES, json!({"facts":chunk.iter().map(|e| json!({"vertex":identity(generation,&e.key)})).collect::<Vec<_>>()})).await?;
        }
        for chunk in nodes.chunks(50) {
            self.run(statements::DELETE_NODES, json!({"facts":chunk.iter().map(|n| json!({"vertex":identity(generation,&n.key)})).collect::<Vec<_>>()})).await?;
        }
        u64::try_from(nodes.len())
            .ok()
            .and_then(|n| n.checked_add(edges.len() as u64))
            .ok_or_else(|| error("revocation count exceeds unsigned range"))
    }

    async fn nodes(
        &self,
        generation: &str,
        keys: &[String],
    ) -> Result<Vec<GraphNode>, FrameworkError> {
        let mut nodes = Vec::new();
        for chunk in keys.chunks(200) {
            let mut parameters = json!({"generation":generation});
            let terms: Vec<_> = chunk
                .iter()
                .enumerate()
                .map(|(i, key)| {
                    parameters[format!("key{i}")] = json!(key);
                    format!("n.key = $key{i}")
                })
                .collect();
            let statement = format!(
                "{} WHERE {} RETURN n.json AS json, n.key AS key",
                statements::NODES,
                terms.join(" OR ")
            );
            let records = self
                .transport
                .execute(&statement, parameters, &["json", "key"])
                .await?;
            nodes.extend(decode::<GraphNode>(&records)?);
        }
        nodes.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(nodes)
    }

    async fn outgoing(
        &self,
        generation: &str,
        sources: &[String],
        kinds: &[String],
        per_source: usize,
    ) -> Result<Vec<GraphEdge>, FrameworkError> {
        let limit = i64::try_from(per_source.max(1))
            .map_err(|_| FrameworkError::param("graph fan-out exceeds signed range"))?;
        let mut result = Vec::new();
        for source in sources {
            let mut parameters = json!({"generation":generation,"source":identity(generation,source),"source_key":source,"limit":limit});
            let mut statement = statements::OUTGOING.to_owned();
            let terms: Vec<_> = kinds
                .iter()
                .enumerate()
                .map(|(i, kind)| {
                    parameters[format!("kind{i}")] = json!(kind);
                    format!("e.kind = $kind{i}")
                })
                .collect();
            if !terms.is_empty() {
                statement.push_str(&format!(" WHERE {}", terms.join(" OR ")));
            }
            statement.push_str(" RETURN e.json AS json, e.key AS key, e.confidence AS confidence ORDER BY confidence DESC, key LIMIT $limit");
            let records = self
                .transport
                .execute(&statement, parameters, &["json", "key", "confidence"])
                .await?;
            result.extend(decode::<GraphEdge>(&records)?);
        }
        result.sort_by(|a: &GraphEdge, b| {
            a.source
                .cmp(&b.source)
                .then_with(|| b.confidence_bps.cmp(&a.confidence_bps))
                .then_with(|| a.key.cmp(&b.key))
        });
        Ok(result)
    }

    async fn document_nodes(
        &self,
        generation: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GraphNode>, FrameworkError> {
        let limit = i64::try_from(limit.max(1))
            .map_err(|_| FrameworkError::param("graph sweep limit exceeds signed range"))?;
        decode(
            &self
                .transport
                .execute(
                    statements::DOCUMENTS,
                    json!({"generation":generation,"after":after.unwrap_or(""),"limit":limit}),
                    &["json", "key"],
                )
                .await?,
        )
    }

    async fn fact_digest(&self, generation: &str) -> Result<String, FrameworkError> {
        let nodes = self
            .scan(statements::ALL_NODES, json!({"generation":generation}))
            .await?;
        let edges = self
            .scan(statements::ALL_EDGES, json!({"generation":generation}))
            .await?;
        digest_facts(&nodes, &edges)
    }
}
