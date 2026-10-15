//! Deterministic tiers preserve collaborative priority while filling sparse histories.
use super::{
    Actor, RecommendConfig, RecommendationTarget,
    documents::{self, Catalog},
    history::{Histories, History},
};
use crate::{FrameworkError, Viewer};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

#[derive(Debug, Clone, Default)]
pub(crate) struct Reason {
    pub peers: usize,
    pub shared: BTreeSet<RecommendationTarget>,
    pub attributes: BTreeMap<String, Vec<String>>,
    pub tier: String,
    pub event_ids: BTreeSet<i64>,
}
#[derive(Debug, Clone)]
pub(crate) struct Ranked {
    pub target: RecommendationTarget,
    pub score: i64,
    pub reason: Reason,
}
#[derive(Default)]
struct AffinitySignal {
    score: i64,
    targets: BTreeSet<RecommendationTarget>,
    events: BTreeSet<i64>,
}
fn peers(actor: &Actor, history: &History, histories: &Histories, cap: usize) -> Vec<Actor> {
    let lists: Vec<_> = history
        .positive
        .keys()
        .filter_map(|target| histories.by_target.get(target))
        .collect();
    let mut latest = BinaryHeap::new();
    for (list, entries) in lists.iter().enumerate() {
        if let Some((peer, time)) = entries.first() {
            latest.push((*time, Reverse(peer.clone()), list, 0));
        }
    }
    let mut sampled = BTreeMap::new();
    while sampled.len() < cap {
        let Some((time, Reverse(peer), list, position)) = latest.pop() else {
            break;
        };
        if &peer != actor {
            sampled.entry(peer).or_insert(time);
        }
        if let Some((peer, time)) = lists[list].get(position + 1) {
            latest.push((*time, Reverse(peer.clone()), list, position + 1));
        }
    }
    sampled.into_keys().collect()
}

pub(crate) fn collaborative(
    actor: &Actor,
    history: &History,
    histories: &Histories,
    cap: usize,
) -> Vec<Ranked> {
    let mut candidates = BTreeMap::<RecommendationTarget, Ranked>::new();
    for peer in peers(actor, history, histories, cap) {
        let Some(peer_history) = histories.actors.get(&peer) else {
            continue;
        };
        let mut eligible = peer_history
            .positive
            .iter()
            .filter(|(target, _)| !history.excluded.contains(*target))
            .peekable();
        if eligible.peek().is_none() {
            continue;
        }
        // Shared evidence is needed only for peers that actually contribute an unseen candidate.
        let shared: BTreeSet<_> = history
            .positive
            .keys()
            .filter(|target| peer_history.positive.contains_key(*target))
            .cloned()
            .collect();
        for (target, positive) in eligible {
            let candidate = candidates.entry(target.clone()).or_insert_with(|| Ranked {
                target: target.clone(),
                score: 0,
                reason: Reason {
                    tier: "collaborative".into(),
                    ..Default::default()
                },
            });
            candidate.score = candidate.score.saturating_add(positive.score);
            candidate.reason.peers += 1;
            candidate.reason.shared.extend(shared.iter().cloned());
            candidate.reason.event_ids.insert(positive.event_id);
            for target in &shared {
                if let Some(own) = history.positive.get(target) {
                    candidate.reason.event_ids.insert(own.event_id);
                }
            }
        }
    }
    ordered(candidates.into_values().collect())
}
pub(crate) fn ordered(mut candidates: Vec<Ranked>) -> Vec<Ranked> {
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.target.cmp(&b.target)));
    candidates
}
pub(crate) fn fill(
    history: &History,
    catalog: &Catalog,
    seeds: &Catalog,
    popularity: &BTreeMap<RecommendationTarget, i64>,
    mut candidates: Vec<Ranked>,
) -> Result<Vec<Ranked>, FrameworkError> {
    let mut used = history.excluded.clone();
    used.extend(candidates.iter().map(|candidate| candidate.target.clone()));
    let mut affinity = Vec::new();
    let mut signals = BTreeMap::<(String, String), AffinitySignal>::new();
    for (target, positive) in &history.positive {
        let Some(document) = seeds.get(target) else {
            continue;
        };
        for (attribute, values) in documents::facets(document)? {
            for value in values {
                let signal = signals.entry((attribute.clone(), value)).or_default();
                signal.score = signal.score.saturating_add(positive.score);
                signal.targets.insert(target.clone());
                signal.events.insert(positive.event_id);
            }
        }
    }
    for (target, document) in catalog {
        if used.contains(target) {
            continue;
        }
        let mut candidate = Ranked {
            target: target.clone(),
            score: 0,
            reason: Reason {
                tier: "affinity".into(),
                ..Default::default()
            },
        };
        for (attribute, values) in documents::facets(document)? {
            for value in values {
                let Some(signal) = signals.get(&(attribute.clone(), value.clone())) else {
                    continue;
                };
                candidate.score = candidate.score.saturating_add(signal.score);
                candidate
                    .reason
                    .attributes
                    .entry(attribute.clone())
                    .or_default()
                    .push(value);
                candidate
                    .reason
                    .shared
                    .extend(signal.targets.iter().cloned());
                candidate
                    .reason
                    .event_ids
                    .extend(signal.events.iter().copied());
            }
        }
        if candidate.score > 0 {
            affinity.push(candidate);
        }
    }
    for candidate in ordered(affinity) {
        used.insert(candidate.target.clone());
        candidates.push(candidate);
    }
    let popular = ordered(
        popularity
            .iter()
            .filter(|(target, score)| {
                **score > 0 && catalog.contains_key(*target) && !used.contains(*target)
            })
            .map(|(target, score)| Ranked {
                target: target.clone(),
                score: *score,
                reason: Reason {
                    tier: "popularity".into(),
                    ..Default::default()
                },
            })
            .collect(),
    );
    for candidate in popular {
        used.insert(candidate.target.clone());
        candidates.push(candidate);
    }
    let recent = ordered(
        catalog
            .iter()
            .filter(|(target, _)| !used.contains(*target))
            .map(|(target, document)| Ranked {
                target: target.clone(),
                score: documents::recency(document),
                reason: Reason {
                    tier: "recent".into(),
                    ..Default::default()
                },
            })
            .collect(),
    );
    candidates.extend(recent);
    Ok(candidates)
}
pub(crate) async fn popularity(
    high_water: i64,
    config: &RecommendConfig,
) -> Result<BTreeMap<RecommendationTarget, i64>, FrameworkError> {
    let cutoff = crate::clock::now()
        .checked_sub_signed(chrono::Duration::days(i64::from(config.popularity_window)))
        .ok_or_else(|| {
            FrameworkError::param(
                "RECOMMEND_POPULARITY_WINDOW exceeds the supported calendar range",
            )
        })?;
    let signed = if super::storage::backend()? == sea_orm::DbBackend::MySql {
        "SIGNED"
    } else {
        "BIGINT"
    };
    let rows = super::storage::rows(&format!("SELECT target_index, target_key, CAST(SUM(score) AS {signed}) AS popularity FROM recommendation_events WHERE id <= ? AND occurred_at >= ? AND event_class = ? GROUP BY target_index, target_key"), vec![high_water.into(), cutoff.timestamp_micros().into(), "positive".into()]).await?;
    rows.into_iter()
        .map(|row| Ok((super::storage::target(&row)?, row.get_int("popularity")?)))
        .collect()
}
/// Seed embedding uses the index's exact document profile and never creates another vector store.
pub(crate) async fn semantic(
    target: &RecommendationTarget,
    catalog: &Catalog,
    viewer: &Viewer,
    depth: usize,
) -> Result<Vec<Ranked>, FrameworkError> {
    let Some(document) = catalog.get(target) else {
        return Ok(Vec::new());
    };
    let descriptor = (crate::SearchRegistry::by_index(&target.index)?.descriptor)();
    if !crate::search::evaluate_scope(descriptor, viewer, document) {
        return Ok(Vec::new());
    }
    let engine = crate::SearchRegistry::engine_for(&target.index)?;
    let generation = crate::SearchRegistry::generation_for(&target.index).await?;
    let settings = engine.read_settings(descriptor, &generation).await?;
    let name = settings
        .index_time
        .embedder
        .clone()
        .unwrap_or(crate::SearchConfig::current()?.embedder.as_str().into());
    if name == "none" || !engine.capabilities().semantic || document.embed_text.is_none() {
        return Ok(Vec::new());
    }
    let embedder = crate::SearchRegistry::embedder_named(&target.index, &name)?;
    let input = crate::EmbedInput {
        text: document.embed_text.clone().unwrap_or_default(),
        title: document.title.clone(),
        media: Vec::new(),
        document_template: (!settings.index_time.embed_template.is_empty())
            .then_some(settings.index_time.embed_template),
    };
    let vectors = embedder.embed_documents(&[input]).await?;
    if vectors.len() != 1 {
        return Err(FrameworkError::internal(
            "recommendation embedder must return exactly one seed vector",
        ));
    }
    let vector = vectors.into_iter().next().ok_or_else(|| {
        FrameworkError::internal("recommendation embedder returned no seed vector")
    })?;
    let vector = crate::validate_embedding(
        vector.dense,
        embedder.profile(),
        settings.index_time.dimensions,
    )?;
    let filter = crate::search::apply_scope(descriptor, viewer, None)?;
    let result = engine
        .semantic(
            descriptor,
            &generation,
            &vector.dense,
            filter.as_ref(),
            depth,
        )
        .await?;
    Ok(result
        .candidates
        .into_iter()
        .filter_map(|hit| {
            let key = RecommendationTarget {
                index: hit.document.index,
                key: hit.document.key,
            };
            if &key == target || !catalog.contains_key(&key) {
                return None;
            }
            Some(Ranked {
                target: key,
                score: (hit.score * 1_000_000.0).round() as i64,
                reason: Reason {
                    tier: "semantic".into(),
                    shared: [target.clone()].into(),
                    ..Default::default()
                },
            })
        })
        .collect())
}
