//! Windowing happens in SQL before event rows cross into the engine.
use super::{Actor, EventProvenance, RecommendationTarget, provenance, storage};
use crate::{DynamicRow, FrameworkError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct Positive {
    pub score: i64,
    pub event_id: i64,
    pub time: i64,
}
#[derive(Debug, Default)]
pub(crate) struct History {
    pub positive: BTreeMap<RecommendationTarget, Positive>,
    pub negative: BTreeSet<RecommendationTarget>,
    pub excluded: BTreeSet<RecommendationTarget>,
    pub surfaces: BTreeSet<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HistorySnapshot {
    pub digest: String,
    pub positive_targets: BTreeSet<RecommendationTarget>,
}
pub(crate) type HistorySnapshots = BTreeMap<String, HistorySnapshot>;
pub(crate) fn actor_key(actor: &Actor) -> String {
    let (kind, id) = actor.parts();
    format!("{kind}:{id}")
}
pub(crate) fn actor_from_key(key: &str) -> Result<Actor, FrameworkError> {
    match key.split_once(':') {
        Some(("user", id)) => Ok(Actor::User(id.into())),
        Some(("guest", id)) => Ok(Actor::Guest(id.into())),
        _ => Err(FrameworkError::database(
            "invalid recommendation history snapshot actor",
        )),
    }
}
#[derive(Default)]
pub(crate) struct Histories {
    pub actors: BTreeMap<Actor, History>,
    pub by_target: BTreeMap<RecommendationTarget, Vec<(Actor, i64)>>,
    pub events_read: i64,
    pub evidence: BTreeMap<i64, EventProvenance>,
}
impl Histories {
    pub fn snapshots(&self) -> HistorySnapshots {
        self.actors.iter().map(|(actor, history)| {
            let positives: Vec<_> = history.positive.iter().map(|(target, positive)| serde_json::json!({
                "target": target, "score": positive.score, "id": positive.event_id, "time": positive.time,
            })).collect();
            let content = crate::search::document::canonical_json(serde_json::json!({
                "positive": positives, "excluded": history.excluded, "surfaces": history.surfaces,
            }));
            (actor_key(actor), HistorySnapshot {
                digest: hex::encode(Sha256::digest(content.as_bytes())),
                positive_targets: history.positive.keys().cloned().collect(),
            })
        }).collect()
    }

    /// One actor's bounded history and the provenance of its positives, for a read that
    /// ranks live instead of from a batch.
    pub async fn for_actor(actor: &Actor, cap: usize) -> Result<Self, FrameworkError> {
        let high_water = storage::scalar(
            "SELECT COALESCE(MAX(id), 0) FROM recommendation_events",
            vec![],
        )
        .await?;
        let (kind, id) = actor.parts();
        let row = DynamicRow(serde_json::Map::from_iter([
            ("actor_kind".to_owned(), serde_json::Value::from(kind)),
            ("actor_id".to_owned(), serde_json::Value::from(id)),
        ]));
        let mut histories = Self::default();
        histories
            .load_chunk(std::slice::from_ref(&row), high_water, cap)
            .await?;
        Ok(histories)
    }

    pub async fn load(high_water: i64, cap: usize) -> Result<Self, FrameworkError> {
        let actors = storage::rows(
            "SELECT DISTINCT actor_kind, actor_id FROM recommendation_events WHERE id <= ?",
            vec![high_water.into()],
        )
        .await?;
        let mut histories = Self::default();
        for chunk in actors.chunks(8) {
            histories.load_chunk(chunk, high_water, cap).await?;
            tokio::task::yield_now().await;
        }
        for (actor, history) in &histories.actors {
            for (target, positive) in &history.positive {
                histories
                    .by_target
                    .entry(target.clone())
                    .or_default()
                    .push((actor.clone(), positive.time));
            }
        }
        for peers in histories.by_target.values_mut() {
            peers.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        Ok(histories)
    }
    async fn load_chunk(
        &mut self,
        actors: &[DynamicRow],
        high_water: i64,
        cap: usize,
    ) -> Result<(), FrameworkError> {
        let predicate = vec!["(actor_kind = ? AND actor_id = ?)"; actors.len()].join(" OR ");
        let mut bindings: Vec<sea_orm::Value> = vec![high_water.into(), "seen".into()];
        for actor in actors {
            bindings.push(actor.get_string("actor_kind")?.into());
            bindings.push(actor.get_string("actor_id")?.into());
        }
        let mut history_bindings = bindings.clone();
        history_bindings.push((cap as i64).into());
        let rows = storage::rows(&format!("SELECT * FROM (SELECT recommendation_events.*, ROW_NUMBER() OVER (PARTITION BY actor_kind, actor_id ORDER BY occurred_at DESC, id DESC) AS history_rank FROM recommendation_events WHERE id <= ? AND event_class <> ? AND ({predicate})) bounded WHERE history_rank <= ?"), history_bindings).await?;
        self.events_read += rows.len() as i64;
        for row in &rows {
            self.add(row)?;
        }
        let mut evidence_ids = BTreeSet::new();
        for row in actors {
            if let Some(history) = self.actors.get(&storage::actor(row)?) {
                evidence_ids.extend(history.positive.values().map(|positive| positive.event_id));
            }
        }
        for row in rows {
            let id = row.get_int("id")?;
            if evidence_ids.contains(&id) {
                self.evidence.insert(
                    id,
                    EventProvenance {
                        id,
                        digest: provenance::row_digest(&row)?,
                    },
                );
            }
        }
        // Impressions are aggregated in SQL and do not consume positive/negative history slots.
        for row in storage::rows(&format!("SELECT actor_kind, actor_id, target_index, target_key, surface FROM recommendation_events WHERE id <= ? AND event_class = ? AND ({predicate}) GROUP BY actor_kind, actor_id, target_index, target_key, surface"), bindings).await? {
            let history = self.actors.entry(storage::actor(&row)?).or_default();
            history.excluded.insert(storage::target(&row)?);
            history.surfaces.insert(row.get_string("surface")?);
        }
        Ok(())
    }
    fn add(&mut self, row: &DynamicRow) -> Result<(), FrameworkError> {
        let actor = storage::actor(row)?;
        let target = storage::target(row)?;
        let history = self.actors.entry(actor).or_default();
        history.surfaces.insert(row.get_string("surface")?);
        history.excluded.insert(target.clone());
        if row.get_string("event_class")? == "negative" {
            history.negative.insert(target.clone());
        }
        if row.get_string("event_class")? == "positive" {
            let score = row.get_int("score")?;
            let time = row.get_int("occurred_at")?;
            let event_id = row.get_int("id")?;
            history
                .positive
                .entry(target)
                .and_modify(|positive| {
                    positive.score = positive.score.saturating_add(score);
                    if (time, event_id) > (positive.time, positive.event_id) {
                        positive.time = time;
                        positive.event_id = event_id;
                    }
                })
                .or_insert(Positive {
                    score,
                    event_id,
                    time,
                });
        }
        Ok(())
    }
}
