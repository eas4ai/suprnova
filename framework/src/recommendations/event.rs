//! Immutable facts have server-owned actors and portable search document identities.
use super::RecommendConfig;
use crate::{FrameworkError, SearchRegistry, Viewer};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Index plus model key prevents collisions across heterogeneous search indexes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RecommendationTarget {
    /// The search index owns the scope, attributes and engine for this target.
    pub index: String,
    /// The model key is textual so integer and UUID models share the stream.
    pub key: String,
}
impl RecommendationTarget {
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        validate_text(&self.index, 128, "target index")?;
        validate_text(&self.key, 255, "target key")?;
        SearchRegistry::by_index(&self.index)?;
        Ok(())
    }
}

/// Actors can only enter through Auth or an explicit trusted server-side value.
/// This type and RecommendationEvent intentionally do not implement Deserialize.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Actor {
    /// Application code may explicitly name a user for imports or background work.
    User(String),
    /// A SHA256 session digest prevents persisting raw guest session credentials.
    Guest(String),
}
impl Actor {
    /// Hash at the server boundary so no raw session identifier enters storage.
    pub fn guest(session: &str) -> Result<Self, FrameworkError> {
        if session.is_empty() {
            return Err(FrameworkError::param(
                "guest actor requires a nonempty server session",
            ));
        }
        Ok(Self::Guest(hex::encode(Sha256::digest(session.as_bytes()))))
    }
    /// Guests require a server-provided session; no client body can provide an actor.
    pub async fn current(session: Option<&str>) -> Result<Self, FrameworkError> {
        match Viewer::current().await? {
            Viewer::Authenticated { id, .. } => Ok(Self::User(id)),
            Viewer::Guest => {
                Self::guest(session.ok_or_else(|| {
                    FrameworkError::param("guest actor requires a server session")
                })?)
            }
        }
    }
    pub(crate) fn parts(&self) -> (&str, &str) {
        match self {
            Self::User(id) => ("user", id),
            Self::Guest(id) => ("guest", id),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        let (_, id) = self.parts();
        validate_text(id, 128, "actor id")?;
        if matches!(self, Self::Guest(_))
            && (id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
        {
            return Err(FrameworkError::param(
                "guest actor requires a lowercase SHA256 session hash",
            ));
        }
        Ok(())
    }
}
/// Class is separate from score so weighted impressions remain seen-only facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventClass {
    /// Positive interactions supply affinity and collaborative candidates.
    Positive,
    /// Negative interactions exclude targets rather than making them popular.
    Negative,
    /// Impressions suppress repeats without creating positive affinity.
    Seen,
}
impl EventClass {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::Seen => "seen",
        }
    }
}
/// Application types can declare both class and base score without replacing the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventScoring {
    /// Class controls whether an event supplies candidates, exclusions or impressions.
    pub class: EventClass,
    /// Score includes the basis-point adjustment when returned by EventType::scoring.
    pub score: i32,
}
impl EventScoring {
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        if self.score.abs_diff(0) > 1_000_000 {
            return Err(FrameworkError::param(
                "event score must be between -1000000 and 1000000",
            ));
        }
        Ok(())
    }
}
/// Every shipped interaction has an explicit class; unknown application names are positive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// Opening a target is a positive view.
    View,
    /// Consumption supplies the same base affinity as a view.
    Consume,
    /// Completion signals stronger affinity than starting consumption.
    Complete,
    /// Skipping excludes a target from the actor's results.
    Skip,
    /// Hiding explicitly excludes the target.
    Hide,
    /// Reporting explicitly excludes the target.
    Report,
    /// Likes carry a stronger signal than views.
    Like,
    /// Favorites indicate durable interest.
    Favorite,
    /// Shares indicate a strong positive interaction.
    Share,
    /// Comments indicate deliberate engagement.
    Comment,
    /// Follows contribute a weaker positive signal.
    Follow,
    /// Impressions mark seen targets without affinity.
    Impression,
    /// Validated search clicks provide a small positive signal.
    Click,
    /// Purchases carry the strongest shipped base score.
    Purchase,
    /// Server-owned application names default to positive with a base score of 500.
    Custom(String),
}
impl EventType {
    /// Stable names form the graph handoff and environment override keys.
    pub fn as_str(&self) -> &str {
        match self {
            Self::View => "view",
            Self::Consume => "consume",
            Self::Complete => "complete",
            Self::Skip => "skip",
            Self::Hide => "hide",
            Self::Report => "report",
            Self::Like => "like",
            Self::Favorite => "favorite",
            Self::Share => "share",
            Self::Comment => "comment",
            Self::Follow => "follow",
            Self::Impression => "impression",
            Self::Click => "click",
            Self::Purchase => "purchase",
            Self::Custom(name) => name,
        }
    }
    /// Stored names recover shipped semantics even when an application constructs Custom.
    pub fn from_name(name: &str) -> Result<Self, FrameworkError> {
        validate_text(name, 128, "event type")?;
        Ok(match name {
            "view" => Self::View,
            "consume" => Self::Consume,
            "complete" => Self::Complete,
            "skip" => Self::Skip,
            "hide" => Self::Hide,
            "report" => Self::Report,
            "like" => Self::Like,
            "favorite" => Self::Favorite,
            "share" => Self::Share,
            "comment" => Self::Comment,
            "follow" => Self::Follow,
            "impression" => Self::Impression,
            "click" => Self::Click,
            "purchase" => Self::Purchase,
            other => Self::Custom(other.into()),
        })
    }
    /// REC-002's base table adds signed integer weight / 20 without changing class.
    pub fn scoring(
        &self,
        weight: i32,
        config: &RecommendConfig,
    ) -> Result<EventScoring, FrameworkError> {
        let kind = Self::from_name(self.as_str())?;
        if !(-10_000..=10_000).contains(&weight) {
            return Err(FrameworkError::param(
                "event weight must be between -10000 and 10000 basis points",
            ));
        }
        let (class, score) = match kind {
            Self::View | Self::Consume => (EventClass::Positive, 1000),
            Self::Complete | Self::Comment => (EventClass::Positive, 1200),
            Self::Like => (EventClass::Positive, 1400),
            Self::Share => (EventClass::Positive, 1500),
            Self::Favorite => (EventClass::Positive, 1600),
            Self::Purchase => (EventClass::Positive, 2000),
            Self::Follow => (EventClass::Positive, 800),
            Self::Click => (EventClass::Positive, 300),
            Self::Impression => (EventClass::Seen, 0),
            Self::Skip | Self::Hide | Self::Report => (EventClass::Negative, -1000),
            Self::Custom(_) => (EventClass::Positive, 500),
        };
        let mut scoring = config
            .event_types
            .get(self.as_str())
            .copied()
            .unwrap_or(EventScoring { class, score });
        scoring.validate()?;
        scoring.score += weight / 20;
        Ok(scoring)
    }
}
/// Explicit optional metadata lets recording reject incomplete facts rather than inventing evidence.
///
/// No Deserialize implementation exists, so a client body cannot choose its actor:
/// ```compile_fail
/// let event: suprnova::RecommendationEvent = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationEvent {
    /// The server names the interaction, including application-specific types.
    pub event_type: EventType,
    /// Signed basis points adjust the base score by weight / 20.
    pub weight: i32,
    /// Auth or trusted application code owns the identity.
    pub actor: Actor,
    /// Index and model key identify the fact's target.
    pub target: Option<RecommendationTarget>,
    /// Surfaces keep home, radio and other placements distinct.
    pub surface: String,
    /// UTC microseconds are retained exactly across supported SQL dialects.
    pub occurred_at: DateTime<Utc>,
    /// The index's guest filter is captured as canonical JSON at recording time.
    pub access_scope: Option<serde_json::Value>,
    /// The target's search document version makes later derivations traceable.
    pub source_version: Option<i64>,
    /// Actor-local retry identity never rewrites the original event.
    pub dedupe_key: String,
}
impl RecommendationEvent {
    /// Load metadata from the target's registered search document instead of trusting a body.
    pub async fn new(
        event_type: EventType,
        target: RecommendationTarget,
        actor: Option<Actor>,
        surface: impl Into<String>,
        dedupe_key: impl Into<String>,
    ) -> Result<Self, FrameworkError> {
        target.validate()?;
        let entry = SearchRegistry::by_index(&target.index)?;
        let document = (entry.load_document)(&target.key).await?.ok_or_else(|| {
            FrameworkError::param("recommendation event target does not exist or is not searchable")
        })?;
        let filter = crate::search::apply_scope((entry.descriptor)(), &Viewer::Guest, None)?
            .unwrap_or(crate::Filter::And(Vec::new()));
        Ok(Self {
            event_type,
            weight: 0,
            actor: match actor {
                Some(actor) => actor,
                None => Actor::current(None).await?,
            },
            target: Some(target),
            surface: surface.into(),
            occurred_at: Utc::now(),
            access_scope: Some(serde_json::to_value(filter).map_err(|e| {
                FrameworkError::internal(format!("serialize recommendation scope: {e}"))
            })?),
            source_version: Some(document.source_version),
            dedupe_key: dedupe_key.into(),
        })
    }
}
pub(crate) fn validate_text(value: &str, max: usize, name: &str) -> Result<(), FrameworkError> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(FrameworkError::param(format!(
            "recommendation {name} requires 1 to {max} bytes without control characters"
        )));
    }
    Ok(())
}
