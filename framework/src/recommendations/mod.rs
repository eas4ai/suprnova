//! Immutable recommendation facts build a versioned input for starter and graph engines.
mod commands;
mod config;
mod documents;
mod engine;
mod event;
mod generation;
pub mod graph;
mod history;
pub mod migrations;
mod provenance;
mod ranking;
mod starter;
mod storage;

use crate::{FrameworkError, Viewer};
pub use config::{GraphSurface, RecommendConfig};
pub(crate) use config::{scoped as scoped_config, within as within_config};
pub use engine::{
    ExplanationEvidence, GenerateRun, GenerationOutcome, Recommendation, RecommendationExplanation,
    RecommendationRun, Recommender,
};
pub use event::{
    Actor, EventClass, EventScoring, EventType, RecommendationEvent, RecommendationTarget,
};
pub use migrations::migrations;
pub use provenance::{EventProvenance, ProvenanceState, event_digest};
pub use starter::StarterRecommender;
use std::sync::Arc;

/// Events retention removes per page, so the graph revokes each page's facts in turn.
const PRUNE_PAGE: i64 = 10_000;

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
        // The scope is read again from the target so a caller cannot store a boundary the
        // target never had; a target changed since the event was built must be rebuilt.
        let entry = crate::SearchRegistry::by_index(&target.index)?;
        let document = (entry.load_document)(&target.key).await?.ok_or_else(|| {
            FrameworkError::param("recommendation event target does not exist or is not searchable")
        })?;
        if *scope != event::access_scope((entry.descriptor)(), &document)? {
            return Err(FrameworkError::param(
                "recommendation access scope must equal the target's scope at record time; build the event again",
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
    /// Applications may replace the engine through the container's Recommender binding;
    /// otherwise RECOMMEND_ENGINE picks the starter or the graph engine.
    pub fn engine() -> Result<Arc<dyn Recommender>, FrameworkError> {
        if crate::App::has_binding::<dyn Recommender>() {
            crate::App::resolve_make::<dyn Recommender>()
        } else if RecommendConfig::current()?.engine == "graph" {
            Ok(Arc::new(graph::GraphRecommender))
        } else {
            Ok(Arc::new(StarterRecommender))
        }
    }
    /// Documents near `target`, steered by the viewer's own positive and negative events;
    /// the graph engine answers it with the search engine's discover operation (REC-006).
    pub fn discover(target: RecommendationTarget) -> RecommendationBuilder {
        RecommendationBuilder::new(Query::Discover(target))
    }
    /// On-demand and scheduled calls share the same lock and publication path.
    pub async fn generate(run: GenerateRun) -> Result<GenerationOutcome, FrameworkError> {
        Self::engine()?.generate(run).await
    }
    /// Rebuild the graph engine's facts from the stream and the declared relations into a
    /// new generation, switching readers only when it is complete. `None` means another
    /// rebuild holds the lock.
    pub async fn rebuild_graph() -> Result<Option<graph::GraphBuild>, FrameworkError> {
        graph::rebuild().await
    }
    /// Declare a relation the graph engine derives edges from, read from each document's
    /// search projection.
    pub fn graph_relation(relation: graph::GraphRelation) -> Result<(), FrameworkError> {
        graph::declare(relation)
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
        // Pages of ids, oldest first, let the graph revoke the facts each page held before
        // the page goes: a failed revocation leaves the events for the retry to find, and
        // an aggregate's provenance, its oldest event, goes no later than any other (REC-005).
        let mut removed = 0;
        loop {
            let ids: Vec<i64> = storage::rows(
                "SELECT id FROM recommendation_events WHERE occurred_at < ? ORDER BY occurred_at, id LIMIT ?",
                vec![cutoff.timestamp_micros().into(), PRUNE_PAGE.into()],
            )
            .await?
            .iter()
            .map(|row| row.get_int("id"))
            .collect::<Result<_, _>>()?;
            if ids.is_empty() {
                return Ok(removed);
            }
            graph::events_removed(&ids).await?;
            for chunk in ids.chunks(200) {
                let list = vec!["?"; chunk.len()].join(", ");
                removed += storage::execute(
                    &format!("DELETE FROM recommendation_events WHERE id IN ({list})"),
                    chunk.iter().map(|id| sea_orm::Value::from(*id)).collect(),
                )
                .await?;
            }
        }
    }
}
#[derive(Clone)]
enum Query {
    Actor(Actor),
    Similar(RecommendationTarget),
    Discover(RecommendationTarget),
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
            Query::Discover(target) => {
                target.validate()?;
                // The viewer's own events steer the query, so only a signed-in viewer has
                // any; a guest's discover is the target's plain neighbourhood.
                let actor = match &viewer {
                    Viewer::Authenticated { id, .. } => Actor::User(id.clone()),
                    Viewer::Guest => Actor::User(String::new()),
                };
                engine.discover(&target, &actor, &viewer, self.limit).await
            }
        }
    }
}
