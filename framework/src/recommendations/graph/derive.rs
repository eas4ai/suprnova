//! Derivation turns the event stream and the declared relations into one generation of
//! facts. It is deterministic: the same stream, documents and relations give the same
//! facts, so a rebuild reproduces the graph it replaces (REC-009). Between rebuilds, the
//! revocation hooks keep facts from outliving their source (REC-005).
use super::{
    GraphEdge, GraphNode, GraphProvenance, GraphScope, GraphStore, Revocation, actor_node_key,
    document_node_key, edge_key,
};
use crate::{
    Cache, FrameworkError, SearchDocument, SearchRegistry,
    recommendations::{
        EventProvenance, RecommendConfig, RecommendationTarget, event::validate_text,
        generation::under_lock, history::Histories, provenance, storage,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, LazyLock, PoisonError, RwLock},
    time::Duration,
};

/// Keys bound per `IN` list.
const KEYS_PER_STATEMENT: usize = 200;

type Related =
    Arc<dyn Fn(&SearchDocument) -> Result<Vec<RecommendationTarget>, FrameworkError> + Send + Sync>;

/// A relation between documents the application declares. It reads the related
/// documents from a document's search projection, so the graph follows it without
/// loading models.
#[derive(Clone)]
pub struct GraphRelation {
    name: String,
    index: String,
    confidence_bps: i32,
    related: Related,
}

impl GraphRelation {
    /// Declare that a document of `index` points at the documents `related` returns,
    /// with `confidence_bps` (1 to 10000) as the edge's strength.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty, longer than 64 bytes or holds a colon,
    /// or the confidence is outside 1 to 10000.
    pub fn new(
        name: impl Into<String>,
        index: impl Into<String>,
        confidence_bps: i32,
        related: impl Fn(&SearchDocument) -> Result<Vec<RecommendationTarget>, FrameworkError>
        + Send
        + Sync
        + 'static,
    ) -> Result<Self, FrameworkError> {
        let name = name.into();
        let index = index.into();
        validate_text(&name, 64, "graph relation name")?;
        if name.contains(':') {
            return Err(FrameworkError::param(
                "graph relation name must not contain a colon",
            ));
        }
        validate_text(&index, 128, "graph relation index")?;
        if !(1..=10_000).contains(&confidence_bps) {
            return Err(FrameworkError::param(
                "graph relation confidence must be 1 to 10000 basis points",
            ));
        }
        Ok(Self {
            name,
            index,
            confidence_bps,
            related: Arc::new(related),
        })
    }

    /// The edge kind this relation's edges carry.
    fn kind(&self) -> String {
        format!("relation:{}", self.name)
    }
}

static RELATIONS: LazyLock<RwLock<BTreeMap<(String, String), GraphRelation>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// Register a relation the graph engine derives edges from. Declaring the same index and
/// name again replaces the earlier declaration, so a boot that runs twice is harmless.
///
/// # Errors
///
/// Returns an error when the relation's index is not a registered search index.
pub fn declare(relation: GraphRelation) -> Result<(), FrameworkError> {
    SearchRegistry::by_index(&relation.index)?;
    RELATIONS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert((relation.index.clone(), relation.name.clone()), relation);
    Ok(())
}

/// The node keys the relation `name` of `document`'s index finds in this revision of the
/// document, or `None` when that relation is no longer declared. A read uses it to tell
/// whether a relation fact read from an older revision still holds (REC-005).
///
/// # Errors
///
/// Returns an error when the relation fails.
pub(crate) fn related_keys(
    name: &str,
    document: &SearchDocument,
) -> Result<Option<std::collections::HashSet<String>>, FrameworkError> {
    let relation = RELATIONS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&(document.index.clone(), name.to_owned()))
        .cloned();
    let Some(relation) = relation else {
        return Ok(None);
    };
    Ok(Some(
        (relation.related)(document)?
            .iter()
            .map(document_node_key)
            .collect(),
    ))
}

fn relations() -> Vec<GraphRelation> {
    RELATIONS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

/// What a rebuild produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphBuild {
    /// The generation readers now use.
    pub generation: String,
    /// Nodes derived.
    pub nodes: usize,
    /// Edges derived.
    pub edges: usize,
    /// Events read from the stream, within each actor's history bound.
    pub events_read: i64,
}

/// Rebuild the graph from the event stream and the declared relations into a new
/// generation, and switch readers to it only when it is complete (REC-009). Returns
/// `None` when another rebuild holds the lock.
///
/// # Errors
///
/// Returns an error when the store, the stream or a relation fails; the half-built
/// generation is discarded and readers keep the previous one.
pub async fn rebuild() -> Result<Option<GraphBuild>, FrameworkError> {
    let config = RecommendConfig::current()?;
    let ttl = Duration::from_secs(config.lock_ttl_secs);
    let Some(lock) = Cache::lock("recommendations:graph", ttl).await? else {
        return Ok(None);
    };
    let built = match super::store().await {
        Ok(store) => {
            under_lock(
                &lock,
                ttl,
                build(store.as_ref(), &config),
                "recommendations graph rebuild",
            )
            .await
        }
        Err(error) => Err(error),
    };
    let release = lock.release().await;
    let built = built?;
    if !release? {
        return Err(FrameworkError::internal(
            "recommendations graph rebuild lost its cache lock before release",
        ));
    }
    Ok(Some(built))
}

async fn build(
    store: &dyn GraphStore,
    config: &RecommendConfig,
) -> Result<GraphBuild, FrameworkError> {
    let generation = store.begin_generation().await?;
    match derive_into(store, &generation, config).await {
        Ok(built) => {
            store.publish(&generation).await?;
            Ok(built)
        }
        Err(error) => match store.discard(&generation).await {
            Ok(()) => Err(error),
            Err(cleanup) => Err(FrameworkError::database(format!(
                "{error}; discarding the unfinished graph generation also failed: {cleanup}"
            ))),
        },
    }
}

/// How many actors share a pair of targets, and the events their sharing rests on.
#[derive(Clone, Copy)]
struct Shared {
    actors: u64,
    /// The oldest event, as time and id.
    oldest: (i64, i64),
    /// The latest event id.
    latest: i64,
}

/// One fact read from the stream before its documents are loaded.
struct Engaged {
    user: String,
    target: RecommendationTarget,
    event: EventProvenance,
}

async fn derive_into(
    store: &dyn GraphStore,
    generation: &str,
    config: &RecommendConfig,
) -> Result<GraphBuild, FrameworkError> {
    let high_water = storage::scalar(
        "SELECT COALESCE(MAX(id), 0) FROM recommendation_events",
        vec![],
    )
    .await?;
    let histories = Histories::load(high_water, config.history).await?;
    let now = crate::clock::now().timestamp_micros();

    // Shared engagement: how many actors hold both targets among their most recent
    // positives. Every event class counts, but only pairs at least RECOMMEND_MIN_PEERS
    // actors share become edges, so no edge describes one person.
    // Each pair keeps its count, the oldest of its actors' events and the latest. An
    // aggregate stops holding once one actor's event behind it goes, and retention deletes
    // the oldest events first, so the oldest is the edge's provenance; a node holds while
    // any support remains, so the latest is the node's (REC-005).
    let mut holders: BTreeMap<RecommendationTarget, u64> = BTreeMap::new();
    let mut pairs: HashMap<(RecommendationTarget, RecommendationTarget), Shared> = HashMap::new();
    for history in histories.actors.values() {
        let mut recent: Vec<_> = history.positive.iter().collect();
        recent.sort_by_key(|(_, positive)| std::cmp::Reverse((positive.time, positive.event_id)));
        recent.truncate(config.graph_fanout);
        for (target, _) in &recent {
            *holders.entry((*target).clone()).or_default() += 1;
        }
        for (i, (first, first_positive)) in recent.iter().enumerate() {
            for (second, second_positive) in recent.iter().skip(i + 1) {
                let key = if first < second {
                    ((*first).clone(), (*second).clone())
                } else {
                    ((*second).clone(), (*first).clone())
                };
                let events = [
                    (first_positive.time, first_positive.event_id),
                    (second_positive.time, second_positive.event_id),
                ];
                let entry = pairs.entry(key).or_insert(Shared {
                    actors: 0,
                    oldest: (i64::MAX, i64::MAX),
                    latest: 0,
                });
                entry.actors += 1;
                for event in events {
                    entry.oldest = entry.oldest.min(event);
                    entry.latest = entry.latest.max(event.1);
                }
            }
        }
    }
    let mut shared: Vec<(RecommendationTarget, RecommendationTarget, i32, Shared)> = Vec::new();
    for ((x, y), pair) in pairs {
        if pair.actors < config.min_peers as u64 {
            continue;
        }
        let both =
            (holders.get(&x).copied().unwrap_or(1) * holders.get(&y).copied().unwrap_or(1)) as f64;
        let confidence = ((pair.actors as f64) / both.sqrt() * 10_000.0)
            .round()
            .clamp(1.0, 10_000.0) as i32;
        shared.push((y.clone(), x.clone(), confidence, pair));
        shared.push((x, y, confidence, pair));
    }
    shared.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.1.cmp(&b.1))
    });
    let mut kept: Vec<(RecommendationTarget, RecommendationTarget, i32, Shared)> = Vec::new();
    let mut per_source = 0;
    for edge in shared {
        if kept.last().is_none_or(|last| last.0 != edge.0) {
            per_source = 0;
        }
        if per_source < config.graph_fanout {
            per_source += 1;
            kept.push(edge);
        }
    }

    let engaged = public_engagement(high_water, config).await?;

    // Every target named so far, loaded live: a target that left its index drops out.
    let mut targets: BTreeSet<RecommendationTarget> = BTreeSet::new();
    for (from, to, _, _) in &kept {
        targets.insert(from.clone());
        targets.insert(to.clone());
    }
    targets.extend(engaged.iter().map(|fact| fact.target.clone()));
    let mut documents = load(&targets).await?;
    let mut provenance_of: BTreeMap<RecommendationTarget, i64> = BTreeMap::new();
    for (from, to, _, pair) in &kept {
        for target in [from, to] {
            let slot = provenance_of.entry(target.clone()).or_insert(0);
            *slot = (*slot).max(pair.latest);
        }
    }
    for fact in &engaged {
        let slot = provenance_of.entry(fact.target.clone()).or_insert(0);
        *slot = (*slot).max(fact.event.id);
    }

    // Declared relations, read once from every document the stream named.
    let mut relation_edges: Vec<(
        GraphRelation,
        RecommendationTarget,
        RecommendationTarget,
        i64,
    )> = Vec::new();
    for relation in relations() {
        let sources: Vec<(RecommendationTarget, i64)> = documents
            .iter()
            .filter(|(target, _)| target.index == relation.index)
            .map(|(target, document)| (target.clone(), document.source_version))
            .collect();
        for (source, version) in sources {
            let Some(document) = documents.get(&source) else {
                continue;
            };
            let mut related = (relation.related)(document)?;
            related.sort();
            related.dedup();
            for target in related {
                target.validate()?;
                if target != source {
                    relation_edges.push((relation.clone(), source.clone(), target, version));
                }
            }
        }
    }
    let missing: BTreeSet<RecommendationTarget> = relation_edges
        .iter()
        .map(|(_, _, target, _)| target.clone())
        .filter(|target| !documents.contains_key(target))
        .collect();
    documents.extend(load(&missing).await?);

    let mut evidence: BTreeMap<i64, EventProvenance> = histories.evidence.clone();
    for fact in &engaged {
        evidence.insert(fact.event.id, fact.event.clone());
    }
    let event = |id: i64| -> Result<GraphProvenance, FrameworkError> {
        evidence
            .get(&id)
            .cloned()
            .map(GraphProvenance::Event)
            .ok_or_else(|| {
                FrameworkError::internal(format!("graph derivation lost evidence for event {id}"))
            })
    };

    let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
    for (target, document) in &documents {
        let provenance = match provenance_of.get(target) {
            Some(id) => event(*id)?,
            None => {
                // Reached only through relations: the first relation in name and source
                // order names it, so every rebuild picks the same one.
                let Some((relation, source, _, version)) = relation_edges
                    .iter()
                    .filter(|(_, _, related, _)| related == target)
                    .min_by(|a, b| (&a.0.name, &a.1).cmp(&(&b.0.name, &b.1)))
                else {
                    continue;
                };
                GraphProvenance::Relation {
                    name: relation.name.clone(),
                    source: source.clone(),
                    source_version: *version,
                }
            }
        };
        let key = document_node_key(target);
        nodes.insert(
            key.clone(),
            GraphNode {
                key,
                kind: target.index.clone(),
                target: Some(target.clone()),
                scope: GraphScope::of(document)?,
                source_version: document.source_version,
                provenance,
            },
        );
    }
    let mut latest_by_user: BTreeMap<&str, i64> = BTreeMap::new();
    for fact in &engaged {
        if documents.contains_key(&fact.target) {
            let slot = latest_by_user.entry(fact.user.as_str()).or_insert(0);
            *slot = (*slot).max(fact.event.id);
        }
    }
    for (user, id) in &latest_by_user {
        let key = actor_node_key(user);
        nodes.insert(
            key.clone(),
            GraphNode {
                key,
                kind: "actor".into(),
                target: None,
                scope: GraphScope::default(),
                source_version: 0,
                provenance: event(*id)?,
            },
        );
    }

    let mut edges: BTreeMap<String, GraphEdge> = BTreeMap::new();
    let mut add = |kind: String,
                   source: String,
                   target: &RecommendationTarget,
                   confidence_bps: i32,
                   source_version: i64,
                   provenance: GraphProvenance|
     -> Result<(), FrameworkError> {
        let Some(document) = documents.get(target) else {
            return Ok(());
        };
        let target_key = document_node_key(target);
        if !nodes.contains_key(&source) || !nodes.contains_key(&target_key) {
            return Ok(());
        }
        let key = edge_key(&kind, &source, &target_key);
        edges.insert(
            key.clone(),
            GraphEdge {
                key,
                source,
                target: target_key,
                kind,
                confidence_bps,
                scope: GraphScope::of(document)?,
                source_version,
                provenance,
                created_at: now,
                updated_at: now,
            },
        );
        Ok(())
    };
    for (from, to, confidence, pair) in &kept {
        let version = documents
            .get(to)
            .map_or(0, |document| document.source_version);
        add(
            "co_engaged".into(),
            document_node_key(from),
            to,
            *confidence,
            version,
            event(pair.oldest.1)?,
        )?;
    }
    for fact in &engaged {
        let version = documents
            .get(&fact.target)
            .map_or(0, |document| document.source_version);
        add(
            "engaged".into(),
            actor_node_key(&fact.user),
            &fact.target,
            10_000,
            version,
            GraphProvenance::Event(fact.event.clone()),
        )?;
    }
    for (relation, source, target, version) in &relation_edges {
        let provenance = GraphProvenance::Relation {
            name: relation.name.clone(),
            source: source.clone(),
            source_version: *version,
        };
        add(
            relation.kind(),
            document_node_key(source),
            target,
            relation.confidence_bps,
            *version,
            provenance.clone(),
        )?;
        add(
            format!("{}:inverse", relation.kind()),
            document_node_key(target),
            source,
            relation.confidence_bps,
            *version,
            provenance,
        )?;
    }

    let nodes: Vec<GraphNode> = nodes.into_values().collect();
    let edges: Vec<GraphEdge> = edges.into_values().collect();
    store.put_nodes(generation, &nodes).await?;
    store.put_edges(generation, &edges).await?;
    Ok(GraphBuild {
        generation: generation.to_owned(),
        nodes: nodes.len(),
        edges: edges.len(),
        events_read: histories.events_read,
    })
}

/// Each user's latest public positive event per target, bounded per user by
/// RECOMMEND_HISTORY. Only the types RECOMMEND_PUBLIC_EVENT_TYPES lists qualify, and
/// only signed-in users, whom another viewer can follow.
async fn public_engagement(
    high_water: i64,
    config: &RecommendConfig,
) -> Result<Vec<Engaged>, FrameworkError> {
    if config.public_event_types.is_empty() {
        return Ok(Vec::new());
    }
    let types: Vec<String> = config.public_event_types.iter().cloned().collect();
    let mut values: Vec<sea_orm::Value> = vec![high_water.into()];
    values.extend(types.iter().map(|name| sea_orm::Value::from(name.clone())));
    values.push((config.history as i64).into());
    let list = vec!["?"; types.len()].join(", ");
    let rows = storage::rows(
        &format!("SELECT * FROM (SELECT actor_id, target_index, target_key, MAX(id) AS id, ROW_NUMBER() OVER (PARTITION BY actor_id ORDER BY MAX(id) DESC) AS engaged_rank FROM recommendation_events WHERE id <= ? AND actor_kind = 'user' AND event_class = 'positive' AND event_type IN ({list}) GROUP BY actor_id, target_index, target_key) ranked WHERE engaged_rank <= ? ORDER BY id"),
        values,
    )
    .await?;
    let ids: Vec<i64> = rows
        .iter()
        .map(|row| row.get_int("id"))
        .collect::<Result<_, _>>()?;
    let mut digests: HashMap<i64, String> = HashMap::new();
    for chunk in ids.chunks(KEYS_PER_STATEMENT) {
        let list = vec!["?"; chunk.len()].join(", ");
        for row in storage::rows(
            &format!("SELECT * FROM recommendation_events WHERE id IN ({list})"),
            chunk.iter().map(|id| sea_orm::Value::from(*id)).collect(),
        )
        .await?
        {
            digests.insert(row.get_int("id")?, provenance::row_digest(&row)?);
        }
    }
    rows.iter()
        .map(|row| {
            let id = row.get_int("id")?;
            let digest = digests.remove(&id).ok_or_else(|| {
                FrameworkError::database(format!("graph derivation could not read event {id}"))
            })?;
            Ok(Engaged {
                user: row.get_string("actor_id")?,
                target: storage::target(row)?,
                event: EventProvenance { id, digest },
            })
        })
        .collect()
}

/// The live search documents for `targets`; a target whose index is gone or whose
/// document left its index, including one `unsearchable` removed, is absent (REC-005).
async fn load(
    targets: &BTreeSet<RecommendationTarget>,
) -> Result<BTreeMap<RecommendationTarget, SearchDocument>, FrameworkError> {
    super::indexed_documents(targets).await
}

/// The store and generation a revocation applies to, when the graph engine runs and a
/// generation exists.
async fn current() -> Result<Option<(Arc<dyn GraphStore>, String)>, FrameworkError> {
    if RecommendConfig::current()?.engine != "graph" {
        return Ok(None);
    }
    let store = super::store().await?;
    Ok(store
        .current_generation()
        .await?
        .map(|generation| (store, generation)))
}

/// Revoke the facts resting on events retention removed, in the generation readers use,
/// before the next rebuild would drop them.
///
/// # Errors
///
/// Returns an error when the store fails.
pub(crate) async fn events_removed(ids: &[i64]) -> Result<(), FrameworkError> {
    if ids.is_empty() {
        return Ok(());
    }
    if let Some((store, generation)) = current().await? {
        store
            .revoke(&generation, &Revocation::Events(ids.to_vec()))
            .await?;
    }
    Ok(())
}

/// Keep the graph in step with a search write (REC-005): a document that left its index
/// loses its facts, and a newer revision replaces the node and relation facts read from
/// an older one. `document` is `None` when the document left its index.
///
/// # Errors
///
/// Returns an error when the store or a declared relation fails.
pub async fn document_changed(
    target: &RecommendationTarget,
    document: Option<&SearchDocument>,
) -> Result<(), FrameworkError> {
    let Some((store, generation)) = current().await? else {
        return Ok(());
    };
    let Some(document) = document else {
        store
            .revoke(&generation, &Revocation::Document(target.clone()))
            .await?;
        return Ok(());
    };
    let key = document_node_key(target);
    let Some(previous) = store
        .nodes(&generation, std::slice::from_ref(&key))
        .await?
        .into_iter()
        .next()
    else {
        // The stream never named this document; the next rebuild decides whether it joins.
        return Ok(());
    };
    if previous.source_version >= document.source_version {
        return Ok(());
    }
    store
        .revoke(
            &generation,
            &Revocation::DocumentBefore(target.clone(), document.source_version),
        )
        .await?;
    let node = GraphNode {
        scope: GraphScope::of(document)?,
        source_version: document.source_version,
        ..previous
    };
    store
        .put_nodes(&generation, std::slice::from_ref(&node))
        .await?;
    let now = crate::clock::now().timestamp_micros();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for relation in relations()
        .into_iter()
        .filter(|relation| relation.index == target.index)
    {
        let mut related = (relation.related)(document)?;
        related.sort();
        related.dedup();
        related.retain(|related| related != target);
        let loaded = load(&related.iter().cloned().collect()).await?;
        let provenance = GraphProvenance::Relation {
            name: relation.name.clone(),
            source: target.clone(),
            source_version: document.source_version,
        };
        for (related, related_document) in &loaded {
            let related_key = document_node_key(related);
            let existing = store
                .nodes(&generation, std::slice::from_ref(&related_key))
                .await?;
            if existing.is_empty() {
                nodes.push(GraphNode {
                    key: related_key.clone(),
                    kind: related.index.clone(),
                    target: Some(related.clone()),
                    scope: GraphScope::of(related_document)?,
                    source_version: related_document.source_version,
                    provenance: provenance.clone(),
                });
            }
            for (kind, source, target_key, scope) in [
                (
                    relation.kind(),
                    key.clone(),
                    related_key.clone(),
                    GraphScope::of(related_document)?,
                ),
                (
                    format!("{}:inverse", relation.kind()),
                    related_key.clone(),
                    key.clone(),
                    node.scope.clone(),
                ),
            ] {
                edges.push(GraphEdge {
                    key: edge_key(&kind, &source, &target_key),
                    source,
                    target: target_key,
                    kind,
                    confidence_bps: relation.confidence_bps,
                    scope,
                    source_version: document.source_version,
                    provenance: provenance.clone(),
                    created_at: now,
                    updated_at: now,
                });
            }
        }
    }
    store.put_nodes(&generation, &nodes).await?;
    store.put_edges(&generation, &edges).await?;
    Ok(())
}

/// The queued job a scheduled `recommendations:generate` pushes when the graph engine
/// runs: it rebuilds the graph (REC-005 derives facts by queued jobs).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphDeriveJob;

#[async_trait::async_trait]
impl crate::queue::Job for GraphDeriveJob {
    fn job_name() -> &'static str {
        "Suprnova::RecommendationGraph"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        // A rebuild already running covers this request.
        rebuild().await.map(|_| ())
    }
}

/// Queue the graph's catch-up after a search write, when the graph engine runs. A job of
/// its own keeps a graph store outage from failing the search write; it retries alone.
///
/// # Errors
///
/// Returns an error when the configuration is invalid or the push fails.
pub(crate) async fn documents_changed(
    index: &str,
    generation: &crate::search::engine::GenerationId,
    pinned: bool,
    indexed: Vec<String>,
    removed: Vec<String>,
) -> Result<(), FrameworkError> {
    if (indexed.is_empty() && removed.is_empty()) || RecommendConfig::current()?.engine != "graph" {
        return Ok(());
    }
    // An import fills a building search generation readers do not use yet; the graph
    // follows only the generation searches read.
    if pinned && *generation != SearchRegistry::generation_for(index).await? {
        return Ok(());
    }
    crate::Queue::push(GraphDocumentJob {
        index: index.to_owned(),
        indexed,
        removed,
    })
    .await
}

/// Apply search writes to the graph: revoke what a removed document held, and replace
/// what a newer revision changed (REC-005).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDocumentJob {
    /// The search index written.
    pub index: String,
    /// Keys indexed by the write.
    pub indexed: Vec<String>,
    /// Keys removed by the write.
    pub removed: Vec<String>,
}

#[async_trait::async_trait]
impl crate::queue::Job for GraphDocumentJob {
    fn job_name() -> &'static str {
        "Suprnova::RecommendationGraphDocuments"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let target = |key: &str| RecommendationTarget {
            index: self.index.clone(),
            key: key.to_owned(),
        };
        for key in &self.removed {
            document_changed(&target(key), None).await?;
        }
        let wanted: BTreeSet<RecommendationTarget> =
            self.indexed.iter().map(|key| target(key)).collect();
        let documents = load(&wanted).await?;
        for wanted in &wanted {
            // A document indexed and then made unsearchable before this job ran has left.
            document_changed(wanted, documents.get(wanted)).await?;
        }
        Ok(())
    }
}
