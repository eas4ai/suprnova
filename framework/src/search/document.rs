//! Typed documents keep engine writes and public projections on the same schema.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::FrameworkError;

/// A declared attribute's storage type makes filters portable across engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeType {
    /// Text preserves the original value for ranking and display.
    Text,
    /// Signed integers allow exact bound comparisons without float conversion.
    Integer,
    /// Finite floating point values support numeric ranges.
    Float,
    /// Booleans represent flags without string coercion.
    Bool,
    /// UTC instants let every engine compare the same point in time.
    DateTime,
    /// Coordinates support radius filters and distance ordering.
    Geo,
    /// Text lists expose each member as a filter and facet value.
    TextList,
}

/// Geographic coordinates use degrees so callers need no engine-specific format.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    /// Latitude in degrees north, bounded to [-90, 90].
    pub lat: f64,
    /// Longitude in degrees east, bounded to [-180, 180].
    pub lng: f64,
}

impl GeoPoint {
    /// Reject coordinates that would make an engine's distance result invalid.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        if !self.lat.is_finite() || !(-90.0..=90.0).contains(&self.lat) {
            return Err(FrameworkError::param(
                "geo latitude must be finite and between -90 and 90",
            ));
        }
        if !self.lng.is_finite() || !(-180.0..=180.0).contains(&self.lng) {
            return Err(FrameworkError::param(
                "geo longitude must be finite and between -180 and 180",
            ));
        }
        Ok(())
    }

    /// Haversine distance in meters gives filters and sorts one shared earth model.
    pub fn distance_to(&self, other: &Self) -> f64 {
        let dlat = (other.lat - self.lat).to_radians();
        let dlng = (other.lng - self.lng).to_radians();
        let a = (dlat / 2.0).sin().powi(2)
            + self.lat.to_radians().cos()
                * other.lat.to_radians().cos()
                * (dlng / 2.0).sin().powi(2);
        6_371_008.8 * 2.0 * a.clamp(0.0, 1.0).sqrt().asin()
    }
}

/// Typed values prevent an engine from silently changing a declared attribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AttributeValue {
    /// Original text is retained for highlighting and raw projections.
    Text(String),
    /// Signed integers preserve exact comparisons and serde round trips.
    Integer(i64),
    /// Finite floats serve numeric filters and facets.
    Float(f64),
    /// Flags remain distinct from strings and numbers.
    Bool(bool),
    /// UTC timestamps have the same ordering on every engine.
    DateTime(DateTime<Utc>),
    /// Coordinates keep latitude and longitude together.
    Geo(GeoPoint),
    /// Lists hold typed members so text lists do not lose facet identity.
    List(Vec<AttributeValue>),
    /// Missing optional fields remain queryable through null predicates.
    Null,
}

impl AttributeValue {
    /// Convert a model field or filter literal under its declared type.
    /// Integers outside i64 and non-finite floats are errors rather than lossy values.
    pub fn from_json(
        value: &serde_json::Value,
        ty: AttributeType,
        attribute: &str,
    ) -> Result<Self, FrameworkError> {
        if value.is_null() {
            return Ok(Self::Null);
        }
        let wrong = || FrameworkError::param(format!("attribute '{attribute}' must be {ty:?}"));
        match ty {
            AttributeType::Text => value
                .as_str()
                .map(|v| Self::Text(v.into()))
                .ok_or_else(wrong),
            AttributeType::Integer => value.as_i64().map(Self::Integer).ok_or_else(wrong),
            AttributeType::Float => value
                .as_f64()
                .filter(|n| n.is_finite())
                .map(Self::Float)
                .ok_or_else(wrong),
            AttributeType::Bool => value.as_bool().map(Self::Bool).ok_or_else(wrong),
            AttributeType::DateTime => {
                let text = value.as_str().ok_or_else(wrong)?;
                let dt = DateTime::parse_from_rfc3339(text)
                    .map(|v| v.with_timezone(&Utc))
                    .or_else(|_| {
                        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                            .map(|v| v.and_utc())
                    })
                    .or_else(|_| {
                        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
                            .map(|v| v.and_utc())
                    })
                    .map_err(|_| wrong())?;
                Ok(Self::DateTime(dt))
            }
            AttributeType::Geo => {
                let point: GeoPoint = serde_json::from_value(value.clone()).map_err(|_| wrong())?;
                point.validate().map_err(|_| wrong())?;
                Ok(Self::Geo(point))
            }
            AttributeType::TextList => {
                let values = value.as_array().ok_or_else(wrong)?;
                let mut list = Vec::with_capacity(values.len());
                for value in values {
                    list.push(Self::Text(value.as_str().ok_or_else(wrong)?.into()));
                }
                Ok(Self::List(list))
            }
        }
    }

    /// Text access avoids serde conversion in tokenization and highlighting.
    pub fn as_text(&self) -> Option<&str> {
        if let Self::Text(value) = self {
            Some(value)
        } else {
            None
        }
    }

    /// A numeric view supports scoring and stats without changing stored types.
    pub fn numeric(&self) -> Option<f64> {
        match self {
            Self::Integer(n) => Some(*n as f64),
            Self::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// Compare values of the same declared type; nulls are ordered by callers.
    pub fn compare(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => Some(a.cmp(b)),
            (Self::Integer(a), Self::Integer(b)) => Some(a.cmp(b)),
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(b),
            (Self::Bool(a), Self::Bool(b)) => Some(a.cmp(b)),
            (Self::DateTime(a), Self::DateTime(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }

    /// Stable facet labels keep integer and datetime presentation predictable.
    pub fn facet_key(&self) -> String {
        match self {
            Self::Text(v) => v.clone(),
            Self::Integer(v) => v.to_string(),
            Self::Float(v) => {
                if *v == 0.0 {
                    "0".into()
                } else {
                    v.to_string()
                }
            }
            Self::Bool(v) => v.to_string(),
            Self::DateTime(v) => v.to_rfc3339(),
            Self::Null => "null".into(),
            Self::Geo(v) => format!(
                "{},{}",
                if v.lat == 0.0 { 0.0 } else { v.lat },
                if v.lng == 0.0 { 0.0 } else { v.lng }
            ),
            Self::List(v) => v.iter().map(Self::facet_key).collect::<Vec<_>>().join(","),
        }
    }

    /// Return plain JSON for endpoint projections without the storage type wrapper.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Text(v) => v.clone().into(),
            Self::Integer(v) => (*v).into(),
            Self::Float(v) => serde_json::Number::from_f64(*v)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Self::Bool(v) => (*v).into(),
            Self::DateTime(v) => v.to_rfc3339().into(),
            Self::Geo(v) => serde_json::json!({"lat":v.lat,"lng":v.lng}),
            Self::List(v) => serde_json::Value::Array(v.iter().map(Self::to_json).collect()),
            Self::Null => serde_json::Value::Null,
        }
    }
}

/// One model projection is the shared input for engines, embedders and recommenders.
///
/// ```
/// use suprnova::{SearchDocument, AttributeValue};
/// let document = SearchDocument {
///     index: "notes".into(), key: "1".into(), source_version: 1,
///     attributes: [("title".into(), AttributeValue::Text("Rust".into()))].into(),
///     embed_text: None, title: None,
/// };
/// assert_eq!(document.content_hash("settings").len(), 64);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchDocument {
    /// The declared index selects the document's schema and engine.
    pub index: String,
    /// String keys work for integer, UUID and text primary keys.
    pub key: String,
    /// Monotonic model versions protect writes against queue reorder.
    pub source_version: i64,
    /// Only declared values may be exposed through raw search results.
    pub attributes: BTreeMap<String, AttributeValue>,
    /// Embedding source text is independent of lexical column weights.
    pub embed_text: Option<String>,
    /// A named title lets profiles apply their required document template.
    pub title: Option<String>,
}

impl SearchDocument {
    /// Hash canonical content and the generation settings for idempotent writes.
    /// Source version is deliberately excluded: an unchanged save advances freshness
    /// but must not rewrite the engine. Canonical JSON fixes map order across serialization backends.
    pub fn content_hash(&self, settings_digest: &str) -> String {
        let value = serde_json::json!({
            "index": self.index, "key": self.key, "attributes": self.attributes,
            "embed_text": self.embed_text, "title": self.title,
        });
        let mut hash = Sha256::new();
        hash.update(canonical_json(value).as_bytes());
        hash.update(settings_digest.as_bytes());
        hex::encode(hash.finalize())
    }

    /// Validate projection shape before any engine accepts values.
    pub fn validate(
        &self,
        descriptor: &super::registry::IndexDescriptor,
    ) -> Result<(), FrameworkError> {
        if self.index != descriptor.name || self.key.is_empty() {
            return Err(FrameworkError::param(format!(
                "index '{}' document has the wrong index or an empty key",
                descriptor.name
            )));
        }
        for declaration in &descriptor.attributes {
            let value = self.attributes.get(&declaration.name).ok_or_else(|| {
                FrameworkError::param(format!(
                    "attribute '{}' is missing from index '{}'",
                    declaration.name, descriptor.name
                ))
            })?;
            Self::validate_value(value, declaration.ty, &declaration.name)?;
        }
        for name in self.attributes.keys() {
            descriptor.attribute(name)?;
        }
        Ok(())
    }

    fn validate_value(
        value: &AttributeValue,
        ty: AttributeType,
        name: &str,
    ) -> Result<(), FrameworkError> {
        if matches!(value, AttributeValue::Null) {
            return Ok(());
        }
        let correct = match (ty, value) {
            (AttributeType::Text, AttributeValue::Text(_))
            | (AttributeType::Integer, AttributeValue::Integer(_))
            | (AttributeType::Bool, AttributeValue::Bool(_))
            | (AttributeType::DateTime, AttributeValue::DateTime(_)) => true,
            (AttributeType::Float, AttributeValue::Float(v)) => v.is_finite(),
            (AttributeType::Geo, AttributeValue::Geo(v)) => v.validate().is_ok(),
            (AttributeType::TextList, AttributeValue::List(v)) => {
                v.iter().all(|v| matches!(v, AttributeValue::Text(_)))
            }
            _ => false,
        };
        if correct {
            Ok(())
        } else {
            Err(FrameworkError::param(format!(
                "attribute '{name}' does not match {ty:?}"
            )))
        }
    }
}

/// Ordered object keys give digests the same bytes with preserve_order enabled.
pub(crate) fn canonical_json(value: serde_json::Value) -> String {
    fn ordered(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let sorted: BTreeMap<_, _> = map.into_iter().collect();
                serde_json::Value::Object(
                    sorted.into_iter().map(|(k, v)| (k, ordered(v))).collect(),
                )
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.into_iter().map(ordered).collect())
            }
            value => value,
        }
    }
    ordered(value).to_string()
}
