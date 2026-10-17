//! The graph engine keeps typed facts with provenance, so a recommendation can follow
//! declared relations and shared engagement while every hop stays inside the viewer's
//! scope. A store only persists facts and answers bounded hops; scoring stays here, so
//! every store ranks the same graph the same way.
mod database;
mod derive;

pub use database::DatabaseGraphStore;
pub use derive::{
    GraphBuild, GraphDeriveJob, GraphDocumentJob, GraphRelation, declare, document_changed, rebuild,
};
pub(crate) use derive::{documents_changed, events_removed};

use super::{EventProvenance, RecommendationTarget};
use crate::{
    FrameworkError, SearchRegistry, Viewer,
    search::{AttributeValue, SearchDocument},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

/// The store the graph engine uses: an application's own `GraphStore` binding when it
/// made one, otherwise the driver `RECOMMEND_GRAPH_STORE` names. Unset, that is the
/// application's database, so the graph engine needs no other service.
///
/// # Errors
///
/// Returns an error when the configuration is invalid or the named driver cannot
/// start.
pub async fn store() -> Result<Arc<dyn GraphStore>, FrameworkError> {
    if crate::App::has_binding::<dyn GraphStore>() {
        return crate::App::resolve_make::<dyn GraphStore>();
    }
    let config = super::RecommendConfig::current()?;
    match config.graph_store.as_str() {
        "database" => Ok(Arc::new(DatabaseGraphStore)),
        "memgraph" => Err(FrameworkError::param(
            "RECOMMEND_GRAPH_STORE=memgraph needs the graph-memgraph feature",
        )),
        other => Err(FrameworkError::param(format!(
            "RECOMMEND_GRAPH_STORE: unknown store {other}"
        ))),
    }
}

/// The visibility a fact carries: the scope-field values of the document it describes,
/// recorded when the fact was derived.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphScope {
    /// The search index whose scope function decides visibility. `None` marks an actor
    /// node, which is reachable only through edges that carry a document's scope.
    pub index: Option<String>,
    /// The document's scope-field values.
    pub values: BTreeMap<String, AttributeValue>,
}

impl GraphScope {
    /// The scope of a search document, read from its index's declared scope fields.
    ///
    /// # Errors
    ///
    /// Returns an error when the document's index is not registered.
    pub fn of(document: &SearchDocument) -> Result<Self, FrameworkError> {
        let descriptor = (SearchRegistry::by_index(&document.index)?.descriptor)();
        Ok(Self {
            index: Some(document.index.clone()),
            values: descriptor
                .scope_fields
                .iter()
                .filter_map(|field| {
                    document
                        .attributes
                        .get(*field)
                        .map(|value| ((*field).to_owned(), value.clone()))
                })
                .collect(),
        })
    }

    /// Whether `viewer` may see the fact: the index's scope function applied to the
    /// recorded values. A fact of an index that is no longer registered is hidden.
    pub fn permits(&self, viewer: &Viewer) -> bool {
        let Some(index) = &self.index else {
            return true;
        };
        let Ok(entry) = SearchRegistry::by_index(index) else {
            return false;
        };
        let document = SearchDocument {
            index: index.clone(),
            key: String::new(),
            source_version: 0,
            attributes: self.values.clone(),
            embed_text: None,
            title: None,
        };
        crate::search::evaluate_scope((entry.descriptor)(), viewer, &document)
    }
}

/// Where a fact came from, so a later derivation can trace it and revoke it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphProvenance {
    /// An event in the stream, by id and content digest (REC-004).
    Event(EventProvenance),
    /// A relation the application declared, read from one revision of a document.
    Relation {
        /// The declared relation's name.
        name: String,
        /// The document the relation was read from.
        source: RecommendationTarget,
        /// That document's source version when the relation was read.
        source_version: i64,
    },
}

impl GraphProvenance {
    /// The event a fact rests on, which retention can remove.
    pub fn event_id(&self) -> Option<i64> {
        match self {
            Self::Event(event) => Some(event.id),
            Self::Relation { .. } => None,
        }
    }

    /// The document a relation fact was read from.
    pub fn source(&self) -> Option<&RecommendationTarget> {
        match self {
            Self::Event(_) => None,
            Self::Relation { source, .. } => Some(source),
        }
    }
}

/// A node: a search document, or an actor that social facts pass through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// The fixed-width identity every store keys by: [`document_node_key`] or
    /// [`actor_node_key`].
    pub key: String,
    /// The node type: the document's index, or `actor`.
    pub kind: String,
    /// The search document the node stands for, when it is one.
    pub target: Option<RecommendationTarget>,
    /// Who may see the node.
    pub scope: GraphScope,
    /// The document's source version when the node was derived.
    pub source_version: i64,
    /// The fact the node was derived from.
    pub provenance: GraphProvenance,
}

/// A directed edge between two nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    /// The fixed-width identity from [`edge_key`].
    pub key: String,
    /// The source node's key.
    pub source: String,
    /// The target node's key.
    pub target: String,
    /// The edge type: `co_engaged`, `relation:<name>`, `follows` or `engaged`.
    pub kind: String,
    /// How strongly the source points at the target, in basis points (0 to 10000).
    pub confidence_bps: i32,
    /// Who may see the edge: the scope of the document it leads to.
    pub scope: GraphScope,
    /// The source version of the document the edge was derived from.
    pub source_version: i64,
    /// The fact the edge was derived from.
    pub provenance: GraphProvenance,
    /// When the edge was first derived, in UTC microseconds.
    pub created_at: i64,
    /// When the edge was last derived, in UTC microseconds.
    pub updated_at: i64,
}

/// Which facts to remove, because their source changed or left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revocation {
    /// A document left its index: its node, every edge to or from it, and every fact
    /// read from it.
    Document(RecommendationTarget),
    /// A document's source version advanced: the facts read from an older version.
    DocumentBefore(RecommendationTarget, i64),
    /// Retention removed these events: the facts that rest on them.
    Events(Vec<i64>),
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

/// A document node's key: a digest, so a store's keys have one width whatever the
/// model key holds.
pub fn document_node_key(target: &RecommendationTarget) -> String {
    digest(&["document", &target.index, &target.key])
}

/// An actor node's key. Only signed-in users have actor nodes: a guest has no stable
/// identity another viewer could follow.
pub fn actor_node_key(user: &str) -> String {
    digest(&["actor", user])
}

/// An edge's key, one per type and ordered pair of nodes.
pub fn edge_key(kind: &str, source: &str, target: &str) -> String {
    digest(&["edge", kind, source, target])
}

/// A store for the graph's facts. It persists nodes and edges in generations and
/// answers one hop at a time; [`traverse`] applies the viewer's scope at every hop, so
/// every store returns the same nodes for the same graph.
#[async_trait]
pub trait GraphStore: Send + Sync {
    /// The driver's name, as `RECOMMEND_GRAPH_STORE` spells it.
    fn name(&self) -> &'static str;

    /// The generation readers use, if one was published.
    async fn current_generation(&self) -> Result<Option<String>, FrameworkError>;

    /// Start an empty generation. Its facts stay invisible until [`Self::publish`].
    async fn begin_generation(&self) -> Result<String, FrameworkError>;

    /// Make `generation` the one readers use, in one step, and drop the previous one.
    async fn publish(&self, generation: &str) -> Result<(), FrameworkError>;

    /// Drop an unpublished generation and its facts.
    async fn discard(&self, generation: &str) -> Result<(), FrameworkError>;

    /// Insert or replace nodes by key.
    async fn put_nodes(&self, generation: &str, nodes: &[GraphNode]) -> Result<(), FrameworkError>;

    /// Insert or replace edges by key.
    async fn put_edges(&self, generation: &str, edges: &[GraphEdge]) -> Result<(), FrameworkError>;

    /// Remove the facts a revocation names; returns how many were removed.
    async fn revoke(
        &self,
        generation: &str,
        revocation: &Revocation,
    ) -> Result<u64, FrameworkError>;

    /// The nodes with these keys that exist.
    async fn nodes(
        &self,
        generation: &str,
        keys: &[String],
    ) -> Result<Vec<GraphNode>, FrameworkError>;

    /// Up to `per_source` edges leaving each source, strongest first, of the given kinds
    /// when `kinds` names some.
    async fn outgoing(
        &self,
        generation: &str,
        sources: &[String],
        kinds: &[String],
        per_source: usize,
    ) -> Result<Vec<GraphEdge>, FrameworkError>;

    /// Every document node, for the freshness sweep: up to `limit` keys after `after`.
    async fn document_nodes(
        &self,
        generation: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GraphNode>, FrameworkError>;

    /// A digest of every fact in the generation, in key order, so two stores or two
    /// builds of one graph can be compared. It reads the whole generation.
    async fn fact_digest(&self, generation: &str) -> Result<String, FrameworkError>;
}

/// The digest [`GraphStore::fact_digest`] returns, over facts already in key order.
/// Timestamps are left out: a rebuild derives the same facts at a later time.
pub(crate) fn digest_facts(
    nodes: &[GraphNode],
    edges: &[GraphEdge],
) -> Result<String, FrameworkError> {
    let mut hasher = Sha256::new();
    for node in nodes {
        let value = serde_json::to_value(node)
            .map_err(|e| FrameworkError::internal(format!("graph node digest: {e}")))?;
        hasher.update(crate::search::document::canonical_json(value).as_bytes());
        hasher.update(b"\n");
    }
    for edge in edges {
        let mut edge = edge.clone();
        edge.created_at = 0;
        edge.updated_at = 0;
        let value = serde_json::to_value(&edge)
            .map_err(|e| FrameworkError::internal(format!("graph edge digest: {e}")))?;
        hasher.update(crate::search::document::canonical_json(value).as_bytes());
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

/// A bounded walk from seed nodes.
#[derive(Debug, Clone)]
pub struct Traversal {
    /// The node keys the walk starts from.
    pub seeds: Vec<String>,
    /// The most hops any path takes.
    pub depth: usize,
    /// The most edges followed out of one node per hop.
    pub per_node: usize,
    /// The most nodes the walk reaches in all.
    pub max_nodes: usize,
    /// The edge kinds to follow; empty follows every kind.
    pub kinds: Vec<String>,
}

/// A node a walk reached, with the strongest path to it.
#[derive(Debug, Clone)]
pub struct Reached {
    /// The node.
    pub node: GraphNode,
    /// The seed the path starts from.
    pub seed: String,
    /// The path's edges, from the seed.
    pub path: Vec<GraphEdge>,
    /// The product of the path's confidences, from 0 to 1.
    pub weight: f64,
}

/// Walk the graph from `traversal.seeds` for `viewer`. A seed, edge or node the viewer's
/// scope does not cover is never returned and never walked through (REC-007), and no
/// path is longer than `traversal.depth`.
///
/// # Errors
///
/// Returns an error when the store fails.
pub async fn traverse(
    store: &dyn GraphStore,
    generation: &str,
    traversal: &Traversal,
    viewer: &Viewer,
) -> Result<Vec<Reached>, FrameworkError> {
    let seeds = store.nodes(generation, &traversal.seeds).await?;
    let mut visited: HashSet<String> = HashSet::new();
    let mut best: HashMap<String, Reached> = HashMap::new();
    let mut frontier: Vec<Reached> = Vec::new();
    for seed in seeds {
        if !seed.scope.permits(viewer) || !visited.insert(seed.key.clone()) {
            continue;
        }
        frontier.push(Reached {
            seed: seed.key.clone(),
            node: seed,
            path: Vec::new(),
            weight: 1.0,
        });
    }
    for _ in 0..traversal.depth {
        if frontier.is_empty() || best.len() >= traversal.max_nodes {
            break;
        }
        let sources: Vec<String> = frontier
            .iter()
            .map(|reached| reached.node.key.clone())
            .collect();
        let by_source: HashMap<&str, &Reached> = frontier
            .iter()
            .map(|reached| (reached.node.key.as_str(), reached))
            .collect();
        let edges = store
            .outgoing(generation, &sources, &traversal.kinds, traversal.per_node)
            .await?;
        let edges: Vec<GraphEdge> = edges
            .into_iter()
            .filter(|edge| !visited.contains(&edge.target) && edge.scope.permits(viewer))
            .collect();
        let mut targets: Vec<String> = edges.iter().map(|edge| edge.target.clone()).collect();
        targets.sort();
        targets.dedup();
        let nodes: HashMap<String, GraphNode> = store
            .nodes(generation, &targets)
            .await?
            .into_iter()
            .filter(|node| node.scope.permits(viewer))
            .map(|node| (node.key.clone(), node))
            .collect();
        let mut next: HashMap<String, Reached> = HashMap::new();
        for edge in edges {
            let (Some(node), Some(from)) =
                (nodes.get(&edge.target), by_source.get(edge.source.as_str()))
            else {
                continue;
            };
            let weight = from.weight * f64::from(edge.confidence_bps.clamp(0, 10_000)) / 10_000.0;
            let replace = next
                .get(&edge.target)
                .is_none_or(|current| weight > current.weight);
            if replace {
                let mut path = from.path.clone();
                path.push(edge.clone());
                next.insert(
                    edge.target.clone(),
                    Reached {
                        node: node.clone(),
                        seed: from.seed.clone(),
                        path,
                        weight,
                    },
                );
            }
        }
        let mut reached: Vec<Reached> = next.into_values().collect();
        reached.sort_by(|a, b| {
            b.weight
                .total_cmp(&a.weight)
                .then_with(|| a.node.key.cmp(&b.node.key))
        });
        reached.truncate(traversal.max_nodes.saturating_sub(best.len()));
        frontier = Vec::with_capacity(reached.len());
        for item in reached {
            visited.insert(item.node.key.clone());
            best.insert(item.node.key.clone(), item.clone());
            frontier.push(item);
        }
    }
    let mut reached: Vec<Reached> = best.into_values().collect();
    reached.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then_with(|| a.node.key.cmp(&b.node.key))
    });
    Ok(reached)
}
