//! The graph engine's reads (REC-007): authorize every node, edge and evidence item
//! first, then rank by a blended score per surface with diversity caps, and answer with
//! the starter's rows when the graph's evidence is too thin.
use super::{
    Gate, GraphStore, LiveFacts, Traversal, actor_node_key, document_node_key, traverse_with,
};
use crate::{
    FrameworkError, SearchDocument, SearchRegistry, Viewer,
    recommendations::{
        Actor, EventProvenance, ExplanationEvidence, GenerateRun, GenerationOutcome, GraphSurface,
        RecommendConfig, Recommendation, RecommendationExplanation, RecommendationTarget,
        Recommender, StarterRecommender, documents, history::Histories, ranking, storage,
    },
    search::{VectorDiscover, VectorRecommend},
};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};

/// Keys bound per `IN` list.
const KEYS_PER_STATEMENT: usize = 200;

/// The knowledge-graph engine, which falls back to the starter's rows when its evidence
/// is too thin (REC-005 to REC-007). RECOMMEND_ENGINE=graph selects it.
pub struct GraphRecommender;

#[async_trait]
impl Recommender for GraphRecommender {
    async fn generate(&self, run: GenerateRun) -> Result<GenerationOutcome, FrameworkError> {
        // The starter's rows are the fallback, so they stay current; the graph rebuilds
        // in a queued job of its own (REC-005).
        let outcome = StarterRecommender.generate(run).await?;
        crate::Queue::push(super::GraphDeriveJob).await?;
        Ok(outcome)
    }

    async fn for_user(
        &self,
        actor: &Actor,
        surface: &str,
        viewer: &Viewer,
        limit: usize,
    ) -> Result<Vec<Recommendation>, FrameworkError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        // Only the viewer's own events steer a graph read and stand as its evidence
        // (REC-006, REC-007), so another person's list is the starter's rows for them.
        let own = match (actor, viewer) {
            (Actor::User(user), Viewer::Authenticated { id, .. }) => user == id,
            (Actor::Guest(_), Viewer::Guest) => true,
            _ => false,
        };
        if !own {
            return fallback(
                StarterRecommender
                    .for_user(actor, surface, viewer, limit)
                    .await?,
                "the graph reads only the viewer's own events, and this list is another actor's",
            );
        }
        let config = RecommendConfig::current()?;
        let histories = Histories::for_actor(actor, config.history).await?;
        let history = histories.actors.get(actor);
        let seeds: Vec<Seed> = history
            .map(|history| {
                history
                    .positive
                    .iter()
                    .map(|(target, positive)| Seed {
                        target: target.clone(),
                        weight: positive.score.max(0) as f64,
                        event: histories.evidence.get(&positive.event_id).cloned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let request = Request {
            seeds,
            negative: history.map(|h| h.negative.clone()).unwrap_or_default(),
            excluded: history.map(|h| h.excluded.clone()).unwrap_or_default(),
            actor: Some(actor.clone()),
            surface: surface.to_owned(),
            viewer: viewer.clone(),
            limit,
            discover: None,
        };
        match rank(&request, &config).await? {
            Some(rows) => Ok(rows),
            None => fallback(
                StarterRecommender
                    .for_user(actor, surface, viewer, limit)
                    .await?,
                THIN,
            ),
        }
    }

    async fn similar_to(
        &self,
        target: &RecommendationTarget,
        viewer: &Viewer,
        limit: usize,
    ) -> Result<Vec<Recommendation>, FrameworkError> {
        if LiveFacts::new(viewer)
            .visible(std::slice::from_ref(target))
            .await?
            .is_empty()
        {
            return Err(FrameworkError::param(
                "recommendation seed is missing or outside the viewer's scope",
            ));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let config = RecommendConfig::current()?;
        // The viewer's own negatives steer the vectors away and leave the list, as do the
        // targets they have already seen (REC-006).
        let actor = viewer.id().map(|id| Actor::User(id.to_owned()));
        let histories = match &actor {
            Some(actor) => Histories::for_actor(actor, config.history).await?,
            None => Histories::default(),
        };
        let history = actor.as_ref().and_then(|actor| histories.actors.get(actor));
        let mut excluded = history.map(|h| h.excluded.clone()).unwrap_or_default();
        excluded.insert(target.clone());
        let request = Request {
            seeds: vec![Seed {
                target: target.clone(),
                weight: 1.0,
                event: None,
            }],
            negative: history.map(|h| h.negative.clone()).unwrap_or_default(),
            excluded,
            actor,
            surface: "similar".into(),
            viewer: viewer.clone(),
            limit,
            discover: None,
        };
        match rank(&request, &config).await? {
            Some(rows) => Ok(rows),
            None => fallback(
                StarterRecommender.similar_to(target, viewer, limit).await?,
                THIN,
            ),
        }
    }

    async fn discover(
        &self,
        target: &RecommendationTarget,
        actor: &Actor,
        viewer: &Viewer,
        limit: usize,
    ) -> Result<Vec<Recommendation>, FrameworkError> {
        if LiveFacts::new(viewer)
            .visible(std::slice::from_ref(target))
            .await?
            .is_empty()
        {
            return Err(FrameworkError::param(
                "recommendation seed is missing or outside the viewer's scope",
            ));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let config = RecommendConfig::current()?;
        let histories = if viewer.id().is_some() {
            Histories::for_actor(actor, config.history).await?
        } else {
            Histories::default()
        };
        let history = histories.actors.get(actor);
        let positive: Vec<RecommendationTarget> = history
            .map(|history| history.positive.keys().cloned().collect())
            .unwrap_or_default();
        let negative: BTreeSet<RecommendationTarget> =
            history.map(|h| h.negative.clone()).unwrap_or_default();
        let mut excluded = history.map(|h| h.excluded.clone()).unwrap_or_default();
        excluded.insert(target.clone());
        let request = Request {
            seeds: vec![Seed {
                target: target.clone(),
                weight: 1.0,
                event: None,
            }],
            negative: negative.clone(),
            excluded,
            actor: viewer.id().map(|id| Actor::User(id.to_owned())),
            surface: "discover".into(),
            viewer: viewer.clone(),
            limit,
            discover: Some((positive, negative.into_iter().collect())),
        };
        match rank(&request, &config).await? {
            Some(rows) => Ok(rows),
            None => fallback(
                StarterRecommender.similar_to(target, viewer, limit).await?,
                THIN,
            ),
        }
    }
}

/// Why a read answers with the starter's rows when the graph supports it too thinly.
const THIN: &str = "the graph's evidence for this surface is below its confidence threshold";

/// The starter's rows, marked as the graph engine's fallback with the reason (REC-007).
fn fallback(
    mut rows: Vec<Recommendation>,
    reason: &str,
) -> Result<Vec<Recommendation>, FrameworkError> {
    for row in &mut rows {
        row.explanation.engine = "starter".into();
        row.explanation.fallback_reason = Some(reason.into());
    }
    Ok(rows)
}

/// A starting point: a target and how strongly it counts.
struct Seed {
    target: RecommendationTarget,
    weight: f64,
    /// The viewer's own event that made the target a seed, when one did.
    event: Option<EventProvenance>,
}

/// One read.
struct Request {
    seeds: Vec<Seed>,
    negative: BTreeSet<RecommendationTarget>,
    excluded: BTreeSet<RecommendationTarget>,
    actor: Option<Actor>,
    surface: String,
    viewer: Viewer,
    limit: usize,
    /// For the discover query: the viewer's positive and negative targets as context.
    discover: Option<(Vec<RecommendationTarget>, Vec<RecommendationTarget>)>,
}

/// What one candidate scored, before blending.
#[derive(Default)]
struct Components {
    affinity: f64,
    semantic: f64,
    social: usize,
    feedback: f64,
    evidence: Vec<ExplanationEvidence>,
    seeds: BTreeSet<RecommendationTarget>,
    via: Option<String>,
}

/// Rank one read, or `None` when the graph's evidence is below the surface's threshold
/// or no graph exists yet.
async fn rank(
    request: &Request,
    config: &RecommendConfig,
) -> Result<Option<Vec<Recommendation>>, FrameworkError> {
    let surface = config.graph_surface(&request.surface);
    let store = super::store().await?;
    let Some(generation) = store.current_generation().await? else {
        return Ok(None);
    };
    let viewer = &request.viewer;
    // Every fact is checked against its live source before it is walked, scored or
    // cited, so one revoked late or by a racing rebuild never counts (REC-005).
    let facts = LiveFacts::new(viewer);

    // Seeds and negatives the viewer may see; nothing outside the scope steers the read.
    let seed_targets: Vec<RecommendationTarget> = request
        .seeds
        .iter()
        .map(|seed| seed.target.clone())
        .collect();
    let seed_documents = facts.visible(&seed_targets).await?;
    let seeds: Vec<&Seed> = request
        .seeds
        .iter()
        .filter(|seed| seed_documents.contains_key(&seed.target))
        .collect();
    let negative_targets: Vec<RecommendationTarget> = request.negative.iter().cloned().collect();
    let negatives = facts.visible(&negative_targets).await?;
    let heaviest = seeds
        .iter()
        .map(|seed| seed.weight)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let by_seed_key: BTreeMap<String, &Seed> = seeds
        .iter()
        .map(|seed| (document_node_key(&seed.target), *seed))
        .collect();

    let mut components: BTreeMap<RecommendationTarget, Components> = BTreeMap::new();
    let traversal = |seeds: Vec<String>, depth: usize| Traversal {
        seeds,
        depth,
        per_node: config.graph_fanout,
        max_nodes: config.graph_max_nodes,
        kinds: Vec::new(),
    };

    // Graph affinity: the strongest scoped path from a seed, weighted by the seed.
    for reached in traverse_with(
        store.as_ref(),
        &generation,
        &traversal(by_seed_key.keys().cloned().collect(), config.graph_depth),
        viewer,
        &facts,
    )
    .await?
    {
        let (Some(target), Some(seed)) = (&reached.node.target, by_seed_key.get(&reached.seed))
        else {
            continue;
        };
        let entry = components.entry(target.clone()).or_default();
        let affinity = reached.weight * seed.weight / heaviest;
        if affinity > entry.affinity {
            entry.affinity = affinity;
            entry.seeds = BTreeSet::from([seed.target.clone()]);
            entry.via = if reached.path.len() > 1 {
                reached
                    .path
                    .get(reached.path.len() - 2)
                    .map(|edge| edge.target.clone())
            } else {
                Some(reached.seed.clone())
            };
            entry
                .evidence
                .retain(|item| !matches!(item, ExplanationEvidence::Edge { .. }));
            entry
                .evidence
                .extend(reached.path.iter().map(|edge| ExplanationEvidence::Edge {
                    key: edge.key.clone(),
                    kind: edge.kind.clone(),
                }));
        }
    }

    // Explicit feedback: nearness to the viewer's own negatives.
    if !negatives.is_empty() {
        for reached in traverse_with(
            store.as_ref(),
            &generation,
            &traversal(negatives.keys().map(document_node_key).collect(), 1),
            viewer,
            &facts,
        )
        .await?
        {
            if let Some(target) = &reached.node.target {
                let entry = components.entry(target.clone()).or_default();
                entry.feedback = entry.feedback.max(reached.weight);
            }
        }
    }

    // Semantic similarity from the search collection's own vectors (REC-006).
    semantic(request, &facts, &seeds, &negatives, &mut components).await?;

    // Social proximity: followed actors' public engagement, counted only where enough of
    // them agree that no single followed actor's activity shows (decision 01M554QJ).
    social(
        request,
        config,
        store.as_ref(),
        &generation,
        &facts,
        &mut components,
    )
    .await?;

    for target in components.keys().cloned().collect::<Vec<_>>() {
        if request.excluded.contains(&target) || seed_documents.contains_key(&target) {
            components.remove(&target);
        }
    }
    let candidates: Vec<RecommendationTarget> = components
        .iter()
        .filter(|(_, c)| {
            c.affinity > 0.0 || c.semantic > 0.0 || c.social >= config.social_min_actors
        })
        .map(|(target, _)| target.clone())
        .collect();
    let documents = facts.visible(&candidates).await?;
    let trust = trust_risk(&documents.keys().cloned().collect::<Vec<_>>()).await?;
    let recency: BTreeMap<&RecommendationTarget, i64> = documents
        .iter()
        .map(|(target, document)| (target, documents::recency(document)))
        .collect();
    let (oldest, newest) = recency
        .values()
        .fold((i64::MAX, i64::MIN), |(lo, hi), value| {
            (lo.min(*value), hi.max(*value))
        });
    let freshness = |target: &RecommendationTarget| -> f64 {
        match recency.get(target) {
            Some(value) if newest > oldest => (*value - oldest) as f64 / (newest - oldest) as f64,
            Some(_) => 1.0,
            None => 0.0,
        }
    };
    let most_social = components
        .values()
        .map(|c| c.social)
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let weight = |bps: u32| f64::from(bps) / 10_000.0;
    let mut scored: Vec<(RecommendationTarget, f64)> = documents
        .keys()
        .filter_map(|target| {
            let c = components.get(target)?;
            let social = if c.social >= config.social_min_actors {
                c.social as f64 / most_social
            } else {
                0.0
            };
            let blended = weight(surface.affinity) * c.affinity
                + weight(surface.semantic) * c.semantic
                + weight(surface.social) * social
                + weight(surface.freshness) * freshness(target)
                - weight(surface.feedback) * c.feedback
                - weight(surface.trust) * trust.get(target).copied().unwrap_or(0.0);
            Some((target.clone(), blended))
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let popular = popular_band(config).await?;
    let mut per_value: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_node: BTreeMap<String, usize> = BTreeMap::new();
    let mut in_band = 0;
    let mut rows = Vec::new();
    let mut graph_backed = 0;
    for (target, score) in scored {
        let (Some(document), Some(c)) = (documents.get(&target), components.get(&target)) else {
            continue;
        };
        let values: Vec<String> = documents::facets(document)?
            .into_iter()
            .flat_map(|(attribute, values)| {
                values
                    .into_iter()
                    .map(move |value| format!("{attribute}={value}"))
            })
            .collect();
        if values
            .iter()
            .any(|value| per_value.get(value).copied().unwrap_or(0) >= surface.per_value)
        {
            continue;
        }
        if c.via
            .as_ref()
            .is_some_and(|via| per_node.get(via).copied().unwrap_or(0) >= surface.per_node)
        {
            continue;
        }
        let banded = popular.contains(&target);
        if banded && in_band >= surface.per_band {
            continue;
        }
        for value in values {
            *per_value.entry(value).or_default() += 1;
        }
        if let Some(via) = &c.via {
            *per_node.entry(via.clone()).or_default() += 1;
        }
        if banded {
            in_band += 1;
        }
        let social = c.social >= config.social_min_actors;
        if c.affinity > 0.0 || social {
            graph_backed += 1;
        }
        rows.push(Recommendation {
            run_id: None,
            target: target.clone(),
            document: document.clone(),
            score: (score * 1_000_000.0).round() as i64,
            rank: rows.len() + 1,
            explanation: explain(c, social, &seed_documents, &seeds, document)?,
        });
        if rows.len() == request.limit {
            break;
        }
    }
    // Confidence: the share of the list the graph itself, not vectors alone, supports.
    let confidence = if rows.is_empty() {
        0
    } else {
        graph_backed * 10_000 / rows.len()
    };
    if confidence < surface.threshold as usize {
        return Ok(None);
    }
    Ok(Some(rows))
}

/// The explanation of one graph row: counts and categories, plus the stored evidence it
/// rests on, every item one the viewer may see (REC-007).
fn explain(
    components: &Components,
    social: bool,
    seed_documents: &BTreeMap<RecommendationTarget, SearchDocument>,
    seeds: &[&Seed],
    document: &SearchDocument,
) -> Result<RecommendationExplanation, FrameworkError> {
    let mut evidence = Vec::new();
    let mut titles = BTreeSet::new();
    let mut affinity_values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let candidate_values = documents::facets(document)?;
    for target in &components.seeds {
        let Some(seed_document) = seed_documents.get(target) else {
            continue;
        };
        if let Some(event) = seeds
            .iter()
            .find(|seed| &seed.target == target)
            .and_then(|seed| seed.event.clone())
        {
            evidence.push(ExplanationEvidence::Event(event));
        }
        evidence.push(ExplanationEvidence::Document(target.clone()));
        if let Some(title) = &seed_document.title {
            titles.insert(title.clone());
        }
        for (attribute, values) in documents::facets(seed_document)? {
            if let Some(candidate) = candidate_values.get(&attribute) {
                let shared: Vec<String> = values
                    .into_iter()
                    .filter(|value| candidate.contains(value))
                    .collect();
                if !shared.is_empty() {
                    affinity_values.entry(attribute).or_default().extend(shared);
                }
            }
        }
    }
    evidence.extend(components.evidence.iter().cloned());
    Ok(RecommendationExplanation {
        peer_count: if social { components.social } else { 0 },
        shared_target_count: components.seeds.len(),
        affinity_values,
        fallback_tier: "graph".into(),
        shared_target_titles: titles.into_iter().collect(),
        engine: "graph".into(),
        evidence,
        fallback_reason: None,
    })
}

/// Nearness in the search collection's own vectors: the engine's recommend from the
/// viewer's positives and negatives, or its discover around a target (REC-006).
async fn semantic(
    request: &Request,
    facts: &LiveFacts,
    seeds: &[&Seed],
    negatives: &BTreeMap<RecommendationTarget, SearchDocument>,
    components: &mut BTreeMap<RecommendationTarget, Components>,
) -> Result<(), FrameworkError> {
    let indexes: BTreeSet<&str> = seeds
        .iter()
        .map(|seed| seed.target.index.as_str())
        .collect();
    for index in indexes {
        let entry = SearchRegistry::by_index(index)?;
        let descriptor = (entry.descriptor)();
        let engine = SearchRegistry::engine_for(index)?;
        if !engine.capabilities().semantic {
            continue;
        }
        let generation = SearchRegistry::generation_for(index).await?;
        let filter = crate::search::apply_scope(descriptor, &request.viewer, None)?;
        let positive: Vec<String> = seeds
            .iter()
            .filter(|seed| seed.target.index == index)
            .map(|seed| seed.target.key.clone())
            .collect();
        let negative: Vec<String> = negatives
            .keys()
            .filter(|target| target.index == index)
            .map(|target| target.key.clone())
            .collect();
        let depth = request.limit.saturating_mul(4);
        let result = match &request.discover {
            Some((context_positive, context_negative)) => {
                // Context pairs come from the viewer's own events the scope covers.
                let visible: BTreeSet<RecommendationTarget> = facts
                    .visible(
                        &context_positive
                            .iter()
                            .chain(context_negative)
                            .cloned()
                            .collect::<Vec<_>>(),
                    )
                    .await?
                    .into_keys()
                    .collect();
                let keys = |targets: &[RecommendationTarget]| -> Vec<String> {
                    targets
                        .iter()
                        .filter(|target| target.index == index && visible.contains(*target))
                        .map(|target| target.key.clone())
                        .collect()
                };
                let target = positive.first().cloned().unwrap_or_default();
                let context =
                    discover_context(&target, &keys(context_positive), &keys(context_negative));
                engine
                    .discover(
                        descriptor,
                        &generation,
                        &VectorDiscover {
                            target,
                            context,
                            filter,
                            depth,
                        },
                    )
                    .await?
            }
            None => {
                engine
                    .recommend(
                        descriptor,
                        &generation,
                        &VectorRecommend {
                            positive: positive.clone(),
                            negative,
                            filter,
                            depth,
                        },
                    )
                    .await?
            }
        };
        let best = result
            .candidates
            .iter()
            .map(|hit| hit.score)
            .fold(0.0_f64, f64::max);
        for hit in result.candidates {
            let target = RecommendationTarget {
                index: index.to_owned(),
                key: hit.document.key.clone(),
            };
            let entry = components.entry(target).or_default();
            let score = if best > 0.0 {
                (hit.score / best).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if score > entry.semantic {
                entry.semantic = score;
                for key in &positive {
                    let document = ExplanationEvidence::Document(RecommendationTarget {
                        index: index.to_owned(),
                        key: key.clone(),
                    });
                    if !entry.evidence.contains(&document) {
                        entry.evidence.push(document);
                    }
                }
            }
        }
    }
    Ok(())
}

/// The discover query's context pairs. Every negative pairs with a positive, the shorter
/// list repeating, and with no positive a negative pairs with the target itself, so no
/// negative the viewer recorded is left out (REC-006). Without negatives there is no pair.
fn discover_context(
    target: &str,
    positives: &[String],
    negatives: &[String],
) -> Vec<(String, String)> {
    if negatives.is_empty() {
        return Vec::new();
    }
    let anchor = [target.to_owned()];
    let positives = if positives.is_empty() {
        &anchor[..]
    } else {
        positives
    };
    (0..positives.len().max(negatives.len()))
        .map(|i| {
            (
                positives[i % positives.len()].clone(),
                negatives[i % negatives.len()].clone(),
            )
        })
        .collect()
}

/// Followed actors' public engagement, per target. A follow is the viewer's own event,
/// read from the stream; an engagement edge exists only for a public event type.
async fn social(
    request: &Request,
    config: &RecommendConfig,
    store: &dyn GraphStore,
    generation: &str,
    facts: &LiveFacts,
    components: &mut BTreeMap<RecommendationTarget, Components>,
) -> Result<(), FrameworkError> {
    let (Some(Actor::User(user)), Some(actor_index)) = (&request.actor, &config.actor_index) else {
        return Ok(());
    };
    if user.is_empty() || config.public_event_types.is_empty() {
        return Ok(());
    }
    let followed: Vec<String> = storage::rows(
        "SELECT DISTINCT target_key FROM recommendation_events WHERE actor_kind = 'user' AND actor_id = ? AND event_type = 'follow' AND event_class = 'positive' AND target_index = ?",
        vec![user.clone().into(), actor_index.clone().into()],
    )
    .await?
    .iter()
    .map(|row| row.get_string("target_key"))
    .collect::<Result<_, _>>()?;
    if followed.is_empty() {
        return Ok(());
    }
    let actors: Vec<String> = followed.iter().map(|user| actor_node_key(user)).collect();
    let edges = store
        .outgoing(
            generation,
            &actors,
            &["engaged".to_owned()],
            config.graph_fanout,
        )
        .await?;
    let edges = facts
        .edges(
            edges
                .into_iter()
                .filter(|edge| edge.scope.permits(&request.viewer))
                .collect(),
        )
        .await?;
    let mut targets: Vec<String> = edges.iter().map(|edge| edge.target.clone()).collect();
    targets.sort();
    targets.dedup();
    let nodes: BTreeMap<String, RecommendationTarget> = facts
        .nodes(
            store
                .nodes(generation, &targets)
                .await?
                .into_iter()
                .filter(|node| node.scope.permits(&request.viewer))
                .collect(),
        )
        .await?
        .into_iter()
        .filter_map(|node| node.target.map(|target| (node.key, target)))
        .collect();
    let mut agreeing: BTreeMap<RecommendationTarget, (BTreeSet<String>, Vec<ExplanationEvidence>)> =
        BTreeMap::new();
    for edge in edges {
        let Some(target) = nodes.get(&edge.target) else {
            continue;
        };
        let entry = agreeing.entry(target.clone()).or_default();
        entry.0.insert(edge.source.clone());
        entry.1.push(ExplanationEvidence::Edge {
            key: edge.key,
            kind: edge.kind,
        });
    }
    for (target, (actors, evidence)) in agreeing {
        if actors.len() < config.social_min_actors {
            continue;
        }
        let entry = components.entry(target).or_default();
        entry.social = actors.len();
        entry.evidence.extend(evidence);
    }
    Ok(())
}

/// Each target's hide and report events weighed against its positive events, 0 to 1.
async fn trust_risk(
    targets: &[RecommendationTarget],
) -> Result<BTreeMap<RecommendationTarget, f64>, FrameworkError> {
    let mut by_index: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for target in targets {
        by_index
            .entry(target.index.as_str())
            .or_default()
            .push(target.key.as_str());
    }
    let mut risk = BTreeMap::new();
    for (index, keys) in by_index {
        for chunk in keys.chunks(KEYS_PER_STATEMENT) {
            let list = vec!["?"; chunk.len()].join(", ");
            let mut values: Vec<sea_orm::Value> = vec![index.to_owned().into()];
            values.extend(
                chunk
                    .iter()
                    .map(|key| sea_orm::Value::from((*key).to_owned())),
            );
            for row in storage::rows(
                &format!("SELECT target_index, target_key, SUM(CASE WHEN event_type IN ('hide', 'report') THEN 1 ELSE 0 END) AS risky, SUM(CASE WHEN event_class = 'positive' THEN 1 ELSE 0 END) AS liked FROM recommendation_events WHERE target_index = ? AND target_key IN ({list}) GROUP BY target_index, target_key"),
                values,
            )
            .await?
            {
                let risky = row.get_int("risky")? as f64;
                let liked = row.get_int("liked")? as f64;
                if risky > 0.0 {
                    risk.insert(storage::target(&row)?, risky / (risky + liked));
                }
            }
        }
    }
    Ok(risk)
}

/// The most popular tenth of the catalog, by positive activity in the popularity window.
async fn popular_band(
    config: &RecommendConfig,
) -> Result<BTreeSet<RecommendationTarget>, FrameworkError> {
    let high_water = storage::scalar(
        "SELECT COALESCE(MAX(id), 0) FROM recommendation_events",
        vec![],
    )
    .await?;
    let popularity = ranking::popularity(high_water, config).await?;
    let mut scores: Vec<i64> = popularity
        .values()
        .copied()
        .filter(|score| *score > 0)
        .collect();
    if scores.is_empty() {
        return Ok(BTreeSet::new());
    }
    scores.sort_unstable();
    let threshold = scores[scores.len() * 9 / 10];
    Ok(popularity
        .into_iter()
        .filter(|(_, score)| *score >= threshold && *score > 0)
        .map(|(target, _)| target)
        .collect())
}

impl GraphRecommender {
    /// The ranking settings a surface uses, for inspection and tests.
    pub fn surface(surface: &str) -> Result<GraphSurface, FrameworkError> {
        Ok(RecommendConfig::current()?.graph_surface(surface))
    }
}
