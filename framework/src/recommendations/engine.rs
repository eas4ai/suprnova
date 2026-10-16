//! Replaceable engines share run boundaries and safe public projections.
use super::{Actor, RecommendationTarget};
use crate::{FrameworkError, SearchDocument, Viewer};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Optional selection leaves other actors and surfaces intact in the next snapshot.
#[derive(Debug, Clone, Default)]
pub struct GenerateRun {
    /// Only this actor is regenerated when supplied by an operator.
    pub user: Option<String>,
    /// Only this surface is regenerated when supplied by an operator.
    pub surface: Option<String>,
}
impl GenerateRun {
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        if let Some(user) = &self.user {
            super::event::validate_text(user, 128, "user")?;
        }
        if let Some(surface) = &self.surface {
            super::event::validate_text(surface, 128, "surface")?;
        }
        Ok(())
    }
}
/// Durable statistics make bounded reads and fallback coverage observable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecommendationRun {
    /// UUID identifies all rows and evidence staged by this batch.
    pub id: String,
    /// UTC start in microseconds, shared by status and latest-run selection.
    pub started_at: i64,
    /// Monotonic wall time is measured in milliseconds.
    pub duration_ms: i64,
    /// Number of capped positive and negative history facts loaded by this run.
    pub events_read: i64,
    /// New stream facts after the previous completed high-water mark.
    pub new_events: i64,
    /// Distinct actors, including guests, represented by the completed snapshot.
    pub users_covered: i64,
    /// Distinct candidates considered across regenerated actor-surface pairs.
    pub items_scored: i64,
    /// Fraction of actor-surface pairs with at least one recommendation.
    pub coverage: f64,
    /// Fraction of output rows supplied by affinity, popularity or recency.
    pub fallback_share: f64,
    /// Highest event id incorporated by an unfiltered batch.
    pub high_water_mark: i64,
    /// A search catalog digest invalidates copied rankings when indexed content changes.
    pub catalog_digest: String,
    /// Running, complete or failed explicitly separates staging from publication.
    pub state: String,
    /// Failures retain their actionable reason without publishing partial rows.
    pub error: Option<String>,
}
/// Contention is an immediate observable outcome rather than an overlapping batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GenerationOutcome {
    /// Another owner holds recommendations:generate.
    Locked,
    /// One complete snapshot is now available to readers.
    Completed(RecommendationRun),
}
/// Explanations deliberately omit peer identities, interaction rows and private evidence ids.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecommendationExplanation {
    /// Count of sampled peers contributing this candidate, without their identities.
    pub peer_count: usize,
    /// Count of distinct positive targets shared with contributing peers.
    pub shared_target_count: usize,
    /// Facet values from authorized seed documents explain content affinity.
    pub affinity_values: BTreeMap<String, Vec<String>>,
    /// Collaborative, semantic, affinity, popularity or recent identifies the ranking tier.
    pub fallback_tier: String,
    /// Shared titles are hydrated and scope-checked for this viewer at read time.
    pub shared_target_titles: Vec<String>,
}
/// Results carry only registered search documents and the viewer's projected explanation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    /// Materialized rows name the completed snapshot; similar-to results are computed live.
    pub run_id: Option<String>,
    /// The stable index plus model key travels across engines and surfaces.
    pub target: RecommendationTarget,
    /// Declared attributes are safe for display after the live scope check.
    pub document: SearchDocument,
    /// Scores are deterministic within each ordered tier.
    pub score: i64,
    /// One-based rank reflects the returned, authorized sequence.
    pub rank: usize,
    /// Counts and scoped categories explain the recommendation without exposing peers.
    pub explanation: RecommendationExplanation,
}
/// A different recommender can replace the starter without changing the facade's callers.
#[async_trait]
pub trait Recommender: Send + Sync {
    /// Generation owns atomic publication, failure cleanup and the renewable batch lock.
    async fn generate(&self, run: GenerateRun) -> Result<GenerationOutcome, FrameworkError>;
    /// Read exactly one completed snapshot with the target index's current viewer scope.
    async fn for_user(
        &self,
        actor: &Actor,
        surface: &str,
        viewer: &Viewer,
        limit: usize,
    ) -> Result<Vec<Recommendation>, FrameworkError>;
    /// Refuse an inaccessible seed before scoring co-occurrence or semantic neighbours.
    async fn similar_to(
        &self,
        target: &RecommendationTarget,
        viewer: &Viewer,
        limit: usize,
    ) -> Result<Vec<Recommendation>, FrameworkError>;
}
