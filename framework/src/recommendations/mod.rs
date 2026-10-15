//! Immutable recommendation facts build a versioned input for starter and graph engines.
mod commands;
mod config;
mod documents;
mod engine;
mod event;
mod generation;
mod history;
pub mod migrations;
mod provenance;
mod ranking;
mod starter;
mod storage;

use crate::{FrameworkError, Viewer};
pub use config::RecommendConfig;
pub use engine::{
    GenerateRun, GenerationOutcome, Recommendation, RecommendationExplanation, RecommendationRun,
    Recommender,
};
pub use event::{
    Actor, EventClass, EventScoring, EventType, RecommendationEvent, RecommendationTarget,
};
pub use migrations::migrations;
pub use provenance::{EventProvenance, ProvenanceState, event_digest};
pub use starter::StarterRecommender;
use std::sync::Arc;

/// A single entry point keeps analytics, application writes and recommendation reads consistent.
pub struct Recommend;
impl Recommend {
    /// Append once per actor and dedupe key, returning the original fact on every retry.
    pub async fn record(event: RecommendationEvent) -> Result<EventProvenance, FrameworkError> {
        let target = event
            .target
            .as_ref()
            .ok_or_else(|| FrameworkError::param("recommendation event requires a target"))?;
        target.validate()?;
        event.actor.validate()?;
        event::validate_text(&event.surface, 128, "surface")?;
        event::validate_text(&event.dedupe_key, 128, "dedupe key")?;
        let version = event.source_version.ok_or_else(|| {
            FrameworkError::param("recommendation event requires a source version")
        })?;
        let scope = event
            .access_scope
            .as_ref()
            .filter(|value| !value.is_null())
            .ok_or_else(|| {
                FrameworkError::param("recommendation event requires an access scope")
            })?;
        let descriptor = (crate::SearchRegistry::by_index(&target.index)?.descriptor)();
        let expected = crate::search::apply_scope(descriptor, &Viewer::Guest, None)?
            .unwrap_or(crate::Filter::And(Vec::new()));
        let supplied: crate::Filter = serde_json::from_value(scope.clone()).map_err(|e| {
            FrameworkError::param(format!(
                "recommendation access scope must be a search filter: {e}"
            ))
        })?;
        if supplied != expected {
            return Err(FrameworkError::param(
                "recommendation access scope must equal the index descriptor's guest filter",
            ));
        }
        let scoring = event
            .event_type
            .scoring(event.weight, &RecommendConfig::current()?)?;
        let (kind, actor) = event.actor.parts();
        let suffix = if storage::backend()? == sea_orm::DbBackend::MySql {
            " ON DUPLICATE KEY UPDATE id = id"
        } else {
            " ON CONFLICT (actor_kind, actor_id, dedupe_key) DO NOTHING"
        };
        storage::execute(&format!("INSERT INTO recommendation_events (actor_kind, actor_id, target_index, target_key, event_type, weight, surface, occurred_at, access_scope, source_version, dedupe_key, event_class, score) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?){suffix}"), vec![
            kind.into(), actor.into(), target.index.clone().into(), target.key.clone().into(),
            event.event_type.as_str().into(), event.weight.into(), event.surface.into(), event.occurred_at.timestamp_micros().into(),
            crate::search::document::canonical_json(scope.clone()).into(), version.into(), event.dedupe_key.clone().into(), scoring.class.name().into(), scoring.score.into(),
        ]).await?;
        let row = storage::rows("SELECT * FROM recommendation_events WHERE actor_kind = ? AND actor_id = ? AND dedupe_key = ?", vec![kind.into(), actor.into(), event.dedupe_key.into()]).await?
            .into_iter().next().ok_or_else(|| FrameworkError::database("recorded recommendation event disappeared before provenance could be read"))?;
        Ok(EventProvenance {
            id: row.get_int("id")?,
            digest: provenance::row_digest(&row)?,
        })
    }
    /// The analytics lane calls this only after session, target and position validation.
    /// Enabled=false is a deliberate no-op; guest hashes must come from the server session.
    pub async fn record_search_event(
        event_type: EventType,
        target: RecommendationTarget,
        actor: Actor,
        surface: &str,
        dedupe_key: &str,
    ) -> Result<Option<EventProvenance>, FrameworkError> {
        if !RecommendConfig::current()?.enabled {
            return Ok(None);
        }
        if !matches!(event_type, EventType::Impression | EventType::Click) {
            return Err(FrameworkError::param(
                "search recommendation events must be impression or click",
            ));
        }
        Self::record(
            RecommendationEvent::new(event_type, target, Some(actor), surface, dedupe_key).await?,
        )
        .await
        .map(Some)
    }
    /// Authenticated request viewers are resolved when get executes, never inferred from user.
    pub fn for_user(user: impl Into<String>) -> RecommendationBuilder {
        RecommendationBuilder::new(Query::Actor(Actor::User(user.into())))
    }
    /// Guest recommendations use an explicit hashed server session just like event recording.
    pub fn for_actor(actor: Actor) -> RecommendationBuilder {
        RecommendationBuilder::new(Query::Actor(actor))
    }
    /// Seed authorization precedes both collaborative and semantic similarity.
    pub fn similar_to(target: RecommendationTarget) -> RecommendationBuilder {
        RecommendationBuilder::new(Query::Similar(target))
    }
    /// Applications may replace the starter through the container's Recommender binding.
    pub fn engine() -> Result<Arc<dyn Recommender>, FrameworkError> {
        if crate::App::has_binding::<dyn Recommender>() {
            crate::App::resolve_make::<dyn Recommender>()
        } else {
            Ok(Arc::new(StarterRecommender))
        }
    }
    /// On-demand and scheduled calls share the same lock and publication path.
    pub async fn generate(run: GenerateRun) -> Result<GenerationOutcome, FrameworkError> {
        Self::engine()?.generate(run).await
    }
    /// Recompute the canonical content rather than trusting a persisted checksum.
    pub async fn verify_provenance(
        reference: &EventProvenance,
    ) -> Result<ProvenanceState, FrameworkError> {
        provenance::verify(reference).await
    }
    /// Operators see completed and failed attempts in descending start order.
    pub async fn status(limit: usize) -> Result<Vec<RecommendationRun>, FrameworkError> {
        starter::status(limit).await
    }
    /// Only an explicit application retention operation deletes stream facts.
    pub async fn prune(older_than_days: u32) -> Result<u64, FrameworkError> {
        if older_than_days == 0 {
            return Err(FrameworkError::param(
                "recommendations retention requires --older-than greater than zero",
            ));
        }
        let cutoff = crate::clock::now()
            .checked_sub_signed(chrono::Duration::days(i64::from(older_than_days)))
            .ok_or_else(|| {
                FrameworkError::param(
                    "recommendations --older-than exceeds the supported calendar range",
                )
            })?;
        storage::execute(
            "DELETE FROM recommendation_events WHERE occurred_at < ?",
            vec![cutoff.timestamp_micros().into()],
        )
        .await
    }
}
#[derive(Clone)]
enum Query {
    Actor(Actor),
    Similar(RecommendationTarget),
}
/// Fluent reads defer Auth resolution and validation until the fallible get boundary.
#[derive(Clone)]
pub struct RecommendationBuilder {
    query: Query,
    surface: String,
    limit: usize,
    viewer: Option<Viewer>,
}
impl RecommendationBuilder {
    fn new(query: Query) -> Self {
        Self {
            query,
            surface: "home".into(),
            limit: 10,
            viewer: None,
        }
    }
    /// Placement selection keeps independent surfaces from sharing a rank sequence.
    pub fn surface(mut self, surface: impl Into<String>) -> Self {
        self.surface = surface.into();
        self
    }
    /// Zero is permitted for empty displays; excessive requests fail at get.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
    /// Explicit viewers support background work without treating the subject as authorization.
    pub fn for_viewer(mut self, viewer: Viewer) -> Self {
        self.viewer = Some(viewer);
        self
    }
    /// Every returned target and title passes engine and live document scope for this viewer.
    pub async fn get(self) -> Result<Vec<Recommendation>, FrameworkError> {
        event::validate_text(&self.surface, 128, "surface")?;
        if self.limit > 10_000 {
            return Err(FrameworkError::param(
                "recommendation read limit exceeds 10000",
            ));
        }
        let viewer = match self.viewer {
            Some(viewer) => viewer,
            None => Viewer::current().await?,
        };
        let engine = Recommend::engine()?;
        match self.query {
            Query::Actor(actor) => {
                actor.validate()?;
                engine
                    .for_user(&actor, &self.surface, &viewer, self.limit)
                    .await
            }
            Query::Similar(target) => {
                target.validate()?;
                engine.similar_to(&target, &viewer, self.limit).await
            }
        }
    }
}
