//! The starter reads one published run and computes scoped seed similarity on demand.
use super::{
    Actor, GenerateRun, GenerationOutcome, RecommendConfig, Recommendation,
    RecommendationExplanation, RecommendationRun, RecommendationTarget, Recommender,
    documents::{self, Catalog},
    history::Histories,
    ranking::{self, Ranked, Reason},
    storage,
};
use crate::{DynamicRow, FrameworkError, Viewer};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};

/// SQL histories and registered search documents suffice before a graph has enough data.
/// Reads enforce the current peer minimum before digest invalidation can regenerate old
/// rows, so an upgrade never exposes a lone peer while waiting for the next batch.
/// Ranked pages hydrate only candidates and their shared evidence to keep small reads
/// independent of catalog size; loader and database failures remain errors.
#[derive(Debug, Default)]
pub struct StarterRecommender;
#[async_trait]
impl Recommender for StarterRecommender {
    async fn generate(&self, run: GenerateRun) -> Result<GenerationOutcome, FrameworkError> {
        super::generation::generate(run).await
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
        let config = RecommendConfig::current()?;
        let facts = super::graph::LiveFacts::new(viewer);
        // A small lookahead absorbs filtered targets without loading the catalog. Cap
        // each page so even a large request keeps SQL and document batches bounded.
        let page_size = limit.saturating_add(8).min(500);
        let mut run_id: Option<String> = None;
        let mut after_rank = 0;
        let mut results = Vec::new();
        loop {
            let rows =
                ranked_page(actor, surface, run_id.as_deref(), after_rank, page_size).await?;
            let Some(first) = rows.first() else {
                break;
            };
            run_id = Some(first.get_string("run_id")?);
            let exhausted = rows.len() < page_size;
            let mut targets = BTreeSet::new();
            let mut candidates = Vec::new();
            for row in rows {
                after_rank = row.get_int("rank")?;
                let target = storage::target(&row)?;
                let reason = stored_reason(&row)?;
                if reason.tier == "collaborative" && reason.peers < config.min_peers {
                    continue;
                }
                targets.insert(target.clone());
                targets.extend(reason.shared.iter().cloned());
                candidates.push((row, target, reason));
            }
            let catalog = facts.visible(&Vec::from_iter(targets)).await?;
            for (row, target, reason) in candidates {
                let Some(document) = catalog.get(&target) else {
                    continue;
                };
                results.push(Recommendation {
                    run_id: Some(row.get_string("run_id")?),
                    target,
                    document: document.clone(),
                    score: row.get_int("score")?,
                    rank: results.len() + 1,
                    explanation: project(&reason, &catalog)?,
                });
                if results.len() == limit {
                    return Ok(results);
                }
            }
            if exhausted {
                break;
            }
        }
        Ok(results)
    }
    async fn similar_to(
        &self,
        target: &RecommendationTarget,
        viewer: &Viewer,
        limit: usize,
    ) -> Result<Vec<Recommendation>, FrameworkError> {
        let catalog = documents::visible(viewer).await?;
        if !catalog.contains_key(target) {
            return Err(FrameworkError::param(
                "recommendation seed is missing or outside the viewer's scope",
            ));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let config = RecommendConfig::current()?;
        let high_water = storage::scalar(
            "SELECT COALESCE(MAX(id), 0) FROM recommendation_events",
            vec![],
        )
        .await?;
        let actor = viewer.id().map(|id| Actor::User(id.into()));
        let histories = Histories::for_target(
            target,
            actor.as_ref(),
            high_water,
            config.history,
            config.peers,
        )
        .await?;
        let excluded = actor
            .as_ref()
            .and_then(|actor| histories.actors.get(actor))
            .map(|h| &h.excluded);
        let mut peers = histories.by_target.get(target).cloned().unwrap_or_default();
        peers.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        peers.truncate(config.peers);
        let mut candidates = BTreeMap::<RecommendationTarget, Ranked>::new();
        for (actor, _) in peers {
            if let Some(history) = histories.actors.get(&actor) {
                for (key, positive) in &history.positive {
                    if key == target
                        || excluded.is_some_and(|excluded| excluded.contains(key))
                        || !catalog.contains_key(key)
                    {
                        continue;
                    }
                    let candidate = candidates.entry(key.clone()).or_insert_with(|| Ranked {
                        target: key.clone(),
                        score: 0,
                        reason: Reason {
                            tier: "collaborative".into(),
                            shared: [target.clone()].into(),
                            ..Default::default()
                        },
                    });
                    candidate.score = candidate.score.saturating_add(positive.score);
                    candidate.reason.peers += 1;
                }
            }
        }
        // Co-occurrence fewer peers share would let the peer count point at one person.
        candidates.retain(|_, candidate| candidate.reason.peers >= config.min_peers);
        for mut semantic in ranking::semantic(target, &catalog, viewer, catalog.len()).await? {
            if excluded.is_some_and(|excluded| excluded.contains(&semantic.target)) {
                continue;
            }
            if let Some(candidate) = candidates.get_mut(&semantic.target) {
                candidate.score = candidate.score.saturating_add(semantic.score / 1000);
            } else {
                semantic.score /= 1000;
                candidates.insert(semantic.target.clone(), semantic);
            }
        }
        ranking::ordered(candidates.into_values().collect())
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(rank, candidate)| {
                let document = catalog.get(&candidate.target).ok_or_else(|| {
                    FrameworkError::internal("authorized recommendation candidate disappeared")
                })?;
                Ok(Recommendation {
                    run_id: None,
                    target: candidate.target,
                    document: document.clone(),
                    score: candidate.score,
                    rank: rank + 1,
                    explanation: project(&candidate.reason, &catalog)?,
                })
            })
            .collect()
    }
}
/// Select a run with the first page's evidence, then pin refills to that run so
/// concurrent publication cannot combine recommendations from different runs.
async fn ranked_page(
    actor: &Actor,
    surface: &str,
    run_id: Option<&str>,
    after_rank: i64,
    limit: usize,
) -> Result<Vec<DynamicRow>, FrameworkError> {
    let (run, mut bindings): (&str, Vec<sea_orm::Value>) = match run_id {
        Some(id) => ("?", vec![id.into()]),
        None => (
            "(SELECT id FROM recommendation_runs WHERE state = ? ORDER BY started_at DESC, id DESC LIMIT 1)",
            vec!["complete".into()],
        ),
    };
    let (kind, actor_id) = actor.parts();
    bindings.extend([
        kind.into(),
        actor_id.into(),
        surface.into(),
        after_rank.into(),
        (limit as i64).into(),
    ]);
    storage::rows(&format!("SELECT r.run_id, r.target_index, r.target_key, r.score, r.rank, e.peer_count, e.shared_target_count, e.attribute_values, e.shared_targets, e.fallback_tier FROM starter_recommendations r JOIN recommendation_explanations e ON e.id = r.explanation_id WHERE r.run_id = {run} AND r.actor_kind = ? AND r.actor_id = ? AND r.surface = ? AND r.rank > ? ORDER BY r.rank LIMIT ?"), bindings).await
}

fn stored_reason(row: &DynamicRow) -> Result<Reason, FrameworkError> {
    Ok(Reason {
        peers: row.get_int("peer_count")? as usize,
        shared: storage::decode(row, "shared_targets")?,
        attributes: storage::decode(row, "attribute_values")?,
        tier: row.get_string("fallback_tier")?,
        event_ids: BTreeSet::new(),
    })
}

fn project(
    reason: &Reason,
    catalog: &Catalog,
) -> Result<RecommendationExplanation, FrameworkError> {
    let mut titles = BTreeSet::new();
    let mut allowed_values = BTreeMap::<String, BTreeSet<String>>::new();
    for target in &reason.shared {
        if let Some(document) = catalog.get(target) {
            if let Some(title) = &document.title {
                titles.insert(title.clone());
            }
            for (attribute, values) in documents::facets(document)? {
                allowed_values.entry(attribute).or_default().extend(values);
            }
        }
    }
    let affinity_values = reason
        .attributes
        .iter()
        .filter_map(|(attribute, values)| {
            let allowed = allowed_values.get(attribute)?;
            let projected: Vec<_> = values
                .iter()
                .filter(|value| allowed.contains(*value))
                .cloned()
                .collect();
            (!projected.is_empty()).then(|| (attribute.clone(), projected))
        })
        .collect();
    Ok(RecommendationExplanation {
        peer_count: reason.peers,
        shared_target_count: reason.shared.len(),
        affinity_values,
        fallback_tier: reason.tier.clone(),
        shared_target_titles: titles.into_iter().collect(),
        engine: "starter".into(),
        evidence: Vec::new(),
        fallback_reason: None,
    })
}
pub(crate) fn run_from_row(row: &DynamicRow) -> Result<RecommendationRun, FrameworkError> {
    Ok(RecommendationRun {
        id: row.get_string("id")?,
        started_at: row.get_int("started_at")?,
        duration_ms: row.get_int("duration_ms")?,
        events_read: row.get_int("events_read")?,
        new_events: row.get_int("new_events")?,
        users_covered: row.get_int("users_covered")?,
        items_scored: row.get_int("items_scored")?,
        coverage: row.get_float("coverage")?,
        fallback_share: row.get_float("fallback_share")?,
        high_water_mark: row.get_int("high_water_mark")?,
        catalog_digest: row.get_string("catalog_digest")?,
        ranking_digest: row.get_string("ranking_digest")?,
        state: row.get_string("state")?,
        error: row.get_optional_string("error")?,
    })
}
pub(crate) async fn status(limit: usize) -> Result<Vec<RecommendationRun>, FrameworkError> {
    if limit > 1000 {
        return Err(FrameworkError::param(
            "recommendations status limit exceeds 1000",
        ));
    }
    storage::rows(
        "SELECT * FROM recommendation_runs ORDER BY started_at DESC, id DESC LIMIT ?",
        vec![(limit as i64).into()],
    )
    .await?
    .iter()
    .map(run_from_row)
    .collect()
}
