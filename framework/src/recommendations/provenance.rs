//! Digests are recomputed from stored content, never trusted from a stored checksum.
use super::{RecommendationEvent, storage};
use crate::{DynamicRow, FrameworkError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

/// Pairing identity with content distinguishes immutable facts from retained identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventProvenance {
    /// The event's monotonically allocated SQL identity.
    pub id: i64,
    /// Lowercase SHA256 over canonical event fields, including derived class and score.
    pub digest: String,
}
/// Retention is distinguishable from modification without keeping deleted event content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceState {
    /// Stored fields still match the referenced content.
    Intact,
    /// The identifier exists but at least one canonical field changed.
    Altered,
    /// Application retention removed the identifier.
    Removed,
}
/// Use the stored scoring snapshot so later configuration changes do not alter provenance.
pub fn event_digest(
    event: &RecommendationEvent,
    scoring: super::EventScoring,
) -> Result<String, FrameworkError> {
    let target = event
        .target
        .as_ref()
        .ok_or_else(|| FrameworkError::param("event digest requires target"))?;
    let scope = event
        .access_scope
        .as_ref()
        .filter(|v| !v.is_null())
        .ok_or_else(|| FrameworkError::param("event digest requires scope"))?;
    let version = event
        .source_version
        .ok_or_else(|| FrameworkError::param("event digest requires source version"))?;
    let (kind, id) = event.actor.parts();
    Ok(digest(
        json!({"actor_kind": kind, "actor_id": id, "target_index": target.index,
        "target_key": target.key, "event_type": event.event_type.as_str(), "weight": event.weight,
        "surface": event.surface, "occurred_at": event.occurred_at.timestamp_micros(), "access_scope": scope,
        "source_version": version, "dedupe_key": event.dedupe_key, "event_class": scoring.class.name(), "score": scoring.score}),
    ))
}
fn digest(value: serde_json::Value) -> String {
    hex::encode(Sha256::digest(
        crate::search::document::canonical_json(value).as_bytes(),
    ))
}
pub(crate) fn row_digest(row: &DynamicRow) -> Result<String, FrameworkError> {
    let mut fields = serde_json::Map::new();
    for name in [
        "actor_kind",
        "actor_id",
        "target_index",
        "target_key",
        "event_type",
        "weight",
        "surface",
        "occurred_at",
        "source_version",
        "dedupe_key",
        "event_class",
        "score",
    ] {
        fields.insert(name.into(), row.get_value(name)?);
    }
    fields.insert(
        "access_scope".into(),
        storage::decode::<serde_json::Value>(row, "access_scope")?,
    );
    Ok(digest(serde_json::Value::Object(fields)))
}
pub(crate) async fn verify(reference: &EventProvenance) -> Result<ProvenanceState, FrameworkError> {
    let rows = storage::rows(
        "SELECT * FROM recommendation_events WHERE id = ?",
        vec![reference.id.into()],
    )
    .await?;
    match rows.first() {
        None => Ok(ProvenanceState::Removed),
        Some(row) => Ok(
            if row_digest(row).is_ok_and(|digest| digest == reference.digest) {
                ProvenanceState::Intact
            } else {
                ProvenanceState::Altered
            },
        ),
    }
}
