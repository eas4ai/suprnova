//! Closed typed model codecs with bounded canonical input and lossless integer tags.

use std::any::{Any, TypeId};
use std::collections::BTreeSet;
use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::canonical::{CanonicalErrorKind, CanonicalValue, to_canonical_bytes};
use crate::limits::InputLimits;
use crate::snapshot::state::{decode_i64, decode_u64, encode_i64, encode_u64};

use super::ModelPath;

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Closed reason a registered model value could not be converted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingIssueKind {
    /// The canonical value had the wrong structural type for the codec.
    InvalidType,
    /// The value used the right shape but was outside the codec's value domain.
    InvalidValue,
    /// The value or encoded result exceeded its byte bound.
    InputTooLarge,
    /// The value exceeded its nesting bound.
    InputTooDeep,
    /// The value exceeded its collection-entry bound.
    TooManyEntries,
    /// A string or object key exceeded its byte bound.
    StringTooLong,
    /// Typed Rust serialization or deserialization did not match the registered codec.
    TypeMismatch,
}

impl BindingIssueKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidType => "invalid_binding_type",
            Self::InvalidValue => "invalid_binding_value",
            Self::InputTooLarge => "binding_input_too_large",
            Self::InputTooDeep => "binding_input_too_deep",
            Self::TooManyEntries => "too_many_binding_entries",
            Self::StringTooLong => "binding_string_too_long",
            Self::TypeMismatch => "binding_type_mismatch",
        }
    }
}

/// Redacted field-level conversion issue.
#[derive(Clone, Eq, PartialEq)]
pub struct BindingIssue {
    kind: BindingIssueKind,
    path: Option<ModelPath>,
}

impl BindingIssue {
    pub(crate) const fn new(kind: BindingIssueKind) -> Self {
        Self { kind, path: None }
    }

    pub(crate) fn at_path(mut self, path: ModelPath) -> Self {
        self.path = Some(path);
        self
    }

    /// Returns the closed conversion failure.
    #[must_use]
    pub const fn kind(&self) -> BindingIssueKind {
        self.kind
    }

    /// Returns the registered safe path associated with the issue.
    #[must_use]
    pub const fn path(&self) -> Option<&ModelPath> {
        self.path.as_ref()
    }
}

impl fmt::Debug for BindingIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BindingIssue")
            .field("kind", &self.kind)
            .field("path", &self.path)
            .finish()
    }
}

impl fmt::Display for BindingIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

/// Why one named field of an object an agent sent would be refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldIssueKind {
    /// The value is not an object of named fields.
    NotAnObject,
    /// The schema declares no field of this name.
    Unknown,
    /// A required field is absent.
    Missing,
    /// A required field is `null`.
    Null,
    /// The field's codec refuses its value.
    Invalid(BindingIssueKind),
}

impl FieldIssueKind {
    /// What is wrong, in words an agent can act on.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NotAnObject => "must be an object of named fields",
            Self::Unknown => "is not a field this tool accepts",
            Self::Missing => "is required",
            Self::Null => "is required and cannot be null",
            Self::Invalid(BindingIssueKind::InvalidType) => "has the wrong type",
            Self::Invalid(BindingIssueKind::InvalidValue | BindingIssueKind::TypeMismatch) => {
                "is not a value this field accepts"
            }
            Self::Invalid(
                BindingIssueKind::InputTooLarge
                | BindingIssueKind::InputTooDeep
                | BindingIssueKind::TooManyEntries
                | BindingIssueKind::StringTooLong,
            ) => "is larger than this field accepts",
        }
    }
}

/// One named field of an object an agent sent that the engine would
/// refuse, and why.
///
/// The engine itself refuses such input with one closed, redacted error:
/// the browser runtime only ever sends what its own directives built. An
/// agent writes the input by hand, so a check that names each field lets it
/// correct the call; the engine's refusal still decides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldIssue {
    field: String,
    kind: FieldIssueKind,
}

impl FieldIssue {
    pub(crate) fn new(field: impl Into<String>, kind: FieldIssueKind) -> Self {
        Self {
            field: field.into(),
            kind,
        }
    }

    /// The field's name as the agent sent it; empty for
    /// [`FieldIssueKind::NotAnObject`].
    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Why the field would be refused.
    #[must_use]
    pub const fn kind(&self) -> FieldIssueKind {
        self.kind
    }
}

/// The JSON Schema dialect every agent-facing schema the engine derives
/// names.
pub const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// A closed draft 2020-12 object schema of `properties`, listing `required`
/// when any field is.
pub(crate) fn object_schema(
    properties: serde_json::Map<String, serde_json::Value>,
    required: Vec<serde_json::Value>,
) -> serde_json::Value {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "$schema".to_owned(),
        serde_json::Value::String(JSON_SCHEMA_DIALECT.to_owned()),
    );
    schema.insert(
        "type".to_owned(),
        serde_json::Value::String("object".to_owned()),
    );
    schema.insert(
        "properties".to_owned(),
        serde_json::Value::Object(properties),
    );
    if !required.is_empty() {
        schema.insert("required".to_owned(), serde_json::Value::Array(required));
    }
    schema.insert(
        "additionalProperties".to_owned(),
        serde_json::Value::Bool(false),
    );
    serde_json::Value::Object(schema)
}

/// Registered typed conversion applied to one model field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelCodec {
    /// UTF-8 string.
    String,
    /// JSON boolean, including unchecked checkbox `false`.
    Boolean,
    /// Lossless signed 64-bit integer.
    I64,
    /// Lossless unsigned 64-bit integer.
    U64,
    /// Finite interoperable JSON number.
    F64,
    /// Explicitly registered JSON shape decoded through Serde.
    Json,
    /// ISO calendar date (`YYYY-MM-DD`).
    Date,
    /// RFC 3339 offset datetime.
    DateTime,
    /// Hyphenated UUID string.
    Uuid,
    /// One of a bounded registered set of string variants.
    Enumeration(Vec<String>),
    /// Ordered list of registered values, including multi-select input.
    List(Box<Self>),
    /// String-keyed map of registered values.
    Map(Box<Self>),
}

impl ModelCodec {
    /// Creates a list codec whose entries use the provided codec.
    #[must_use]
    pub fn list(entry: Self) -> Self {
        Self::List(Box::new(entry))
    }

    /// Creates a string-keyed map codec whose values use the provided codec.
    #[must_use]
    pub fn map(value: Self) -> Self {
        Self::Map(Box::new(value))
    }

    /// Creates a bounded nonempty enumeration with unique stable names.
    pub fn enumeration<I, S>(variants: I) -> Result<Self, BindingIssue>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let variants = variants.into_iter().map(Into::into).collect::<Vec<_>>();
        if variants.is_empty() || variants.len() > 256 {
            return Err(BindingIssue::new(BindingIssueKind::TooManyEntries));
        }
        let mut unique = BTreeSet::new();
        for variant in &variants {
            if variant.is_empty()
                || variant.len() > 128
                || !variant.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
                })
                || !unique.insert(variant.clone())
            {
                return Err(BindingIssue::new(BindingIssueKind::InvalidValue));
            }
        }
        Ok(Self::Enumeration(unique.into_iter().collect()))
    }

    /// Decodes a non-null canonical value into the registered Rust type.
    pub fn decode<T: DeserializeOwned + 'static>(
        &self,
        value: &CanonicalValue,
        limits: &InputLimits,
    ) -> Result<T, BindingIssue> {
        validate_limits(value, limits)?;
        if TypeId::of::<T>() == TypeId::of::<Date>() && matches!(self, Self::Date) {
            return downcast(date_value(value)?);
        }
        if TypeId::of::<T>() == TypeId::of::<OffsetDateTime>() && matches!(self, Self::DateTime) {
            return downcast(datetime_value(value)?);
        }
        if TypeId::of::<T>() == TypeId::of::<Uuid>() && matches!(self, Self::Uuid) {
            return downcast(uuid_value(value)?);
        }
        let normalized = self.normalized_serde(value, limits, 0)?;
        serde_json::from_value(normalized)
            .map_err(|_| BindingIssue::new(BindingIssueKind::TypeMismatch))
    }

    /// Encodes a registered Rust value into its bounded canonical representation.
    pub fn encode<T: Serialize + 'static>(
        &self,
        value: &T,
        limits: &InputLimits,
    ) -> Result<CanonicalValue, BindingIssue> {
        let any = value as &dyn Any;
        let canonical = match self {
            Self::Date => CanonicalValue::String(
                any.downcast_ref::<Date>()
                    .ok_or_else(|| BindingIssue::new(BindingIssueKind::TypeMismatch))?
                    .to_string(),
            ),
            Self::DateTime => CanonicalValue::String(
                any.downcast_ref::<OffsetDateTime>()
                    .ok_or_else(|| BindingIssue::new(BindingIssueKind::TypeMismatch))?
                    .format(&Rfc3339)
                    .map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))?,
            ),
            Self::Uuid => CanonicalValue::String(
                any.downcast_ref::<Uuid>()
                    .ok_or_else(|| BindingIssue::new(BindingIssueKind::TypeMismatch))?
                    .hyphenated()
                    .to_string(),
            ),
            _ => {
                let serialized = serde_json::to_value(value)
                    .map_err(|_| BindingIssue::new(BindingIssueKind::TypeMismatch))?;
                self.encode_serde(serialized, limits, 0)?
            }
        };
        validate_limits(&canonical, limits)?;
        Ok(canonical)
    }

    /// The JSON Schema (draft 2020-12) of exactly the values this codec
    /// accepts, or `None` when the codec is [`ModelCodec::Json`], directly
    /// or as the entry of a list or map.
    ///
    /// An agent reads a tool's schema as the contract of its input, so each
    /// shape follows what [`validate`](Self::validate) enforces: a 64-bit
    /// integer is a JSON number only inside the interoperable safe range, or
    /// else its canonical decimal string; a UUID is the hyphenated form in
    /// either case. `Json` accepts any canonical JSON its Serde type reads,
    /// which no schema derived here can promise, so it has none.
    #[must_use]
    pub fn json_schema(&self) -> Option<serde_json::Value> {
        use serde_json::json;

        const SAFE: i64 = 9_007_199_254_740_991;
        Some(match self {
            Self::String => json!({ "type": "string" }),
            Self::Boolean => json!({ "type": "boolean" }),
            Self::I64 => json!({
                "anyOf": [
                    { "type": "integer", "minimum": -SAFE, "maximum": SAFE },
                    { "type": "string", "pattern": "^(0|-?[1-9][0-9]*)$" },
                ]
            }),
            Self::U64 => json!({
                "anyOf": [
                    { "type": "integer", "minimum": 0, "maximum": SAFE },
                    { "type": "string", "pattern": "^(0|[1-9][0-9]*)$" },
                ]
            }),
            Self::F64 => json!({ "type": "number" }),
            Self::Json => return None,
            Self::Date => json!({ "type": "string", "format": "date" }),
            Self::DateTime => json!({ "type": "string", "format": "date-time" }),
            Self::Uuid => json!({
                "type": "string",
                "format": "uuid",
                "pattern":
                    "^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$",
            }),
            Self::Enumeration(variants) => json!({ "type": "string", "enum": variants }),
            Self::List(entry) => json!({ "type": "array", "items": entry.json_schema()? }),
            Self::Map(value) => json!({
                "type": "object",
                "additionalProperties": value.json_schema()?,
            }),
        })
    }

    /// Whether [`json_schema`](Self::json_schema) describes this codec.
    #[must_use]
    pub fn has_json_schema(&self) -> bool {
        match self {
            Self::Json => false,
            Self::List(codec) | Self::Map(codec) => codec.has_json_schema(),
            _ => true,
        }
    }

    pub(crate) fn validate(
        &self,
        value: &CanonicalValue,
        limits: &InputLimits,
    ) -> Result<(), BindingIssue> {
        validate_limits(value, limits)?;
        match self {
            Self::Date => date_value(value).map(|_| ()),
            Self::DateTime => datetime_value(value).map(|_| ()),
            Self::Uuid => uuid_value(value).map(|_| ()),
            _ => self.normalized_serde(value, limits, 0).map(|_| ()),
        }
    }

    pub(crate) fn validate_contract(&self) -> Result<(), BindingIssue> {
        self.validate_contract_at_depth(0)
    }

    fn validate_contract_at_depth(&self, depth: usize) -> Result<(), BindingIssue> {
        if depth > 16 {
            return Err(BindingIssue::new(BindingIssueKind::InputTooDeep));
        }
        match self {
            Self::Enumeration(variants) => {
                if variants.is_empty() || variants.len() > 256 {
                    return Err(BindingIssue::new(BindingIssueKind::TooManyEntries));
                }
                let mut previous = None;
                for variant in variants {
                    let valid = !variant.is_empty()
                        && variant.len() <= 128
                        && variant.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || matches!(byte, b'_' | b'-' | b'.' | b':')
                        })
                        && previous.is_none_or(|previous| previous < variant);
                    if !valid {
                        return Err(BindingIssue::new(BindingIssueKind::InvalidValue));
                    }
                    previous = Some(variant);
                }
                Ok(())
            }
            Self::List(codec) | Self::Map(codec) => codec.validate_contract_at_depth(depth + 1),
            _ => Ok(()),
        }
    }

    fn normalized_serde(
        &self,
        value: &CanonicalValue,
        limits: &InputLimits,
        depth: usize,
    ) -> Result<serde_json::Value, BindingIssue> {
        if depth > limits.max_depth() {
            return Err(BindingIssue::new(BindingIssueKind::InputTooDeep));
        }
        match self {
            Self::String => match value {
                CanonicalValue::String(value) => Ok(serde_json::Value::String(value.clone())),
                _ => Err(BindingIssue::new(BindingIssueKind::InvalidType)),
            },
            Self::Boolean => match value {
                CanonicalValue::Bool(value) => Ok(serde_json::Value::Bool(*value)),
                _ => Err(BindingIssue::new(BindingIssueKind::InvalidType)),
            },
            Self::I64 => integer_value(value).map(serde_json::Value::Number),
            Self::U64 => unsigned_value(value).map(serde_json::Value::Number),
            Self::F64 => match value {
                CanonicalValue::Number(value) => serde_json::Number::from_f64(value.get())
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| BindingIssue::new(BindingIssueKind::InvalidValue)),
                _ => Err(BindingIssue::new(BindingIssueKind::InvalidType)),
            },
            Self::Json => value
                .to_serde_value()
                .map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue)),
            Self::Date => date_value(value).map(|value| serde_json::json!(value.to_string())),
            Self::DateTime => datetime_value(value).and_then(|value| {
                value
                    .format(&Rfc3339)
                    .map(serde_json::Value::String)
                    .map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))
            }),
            Self::Uuid => uuid_value(value)
                .map(|value| serde_json::Value::String(value.hyphenated().to_string())),
            Self::Enumeration(variants) => match value {
                CanonicalValue::String(value) if variants.binary_search(value).is_ok() => {
                    Ok(serde_json::Value::String(value.clone()))
                }
                CanonicalValue::String(_) => Err(BindingIssue::new(BindingIssueKind::InvalidValue)),
                _ => Err(BindingIssue::new(BindingIssueKind::InvalidType)),
            },
            Self::List(codec) => match value {
                CanonicalValue::Array(values) => values
                    .iter()
                    .map(|value| codec.normalized_serde(value, limits, depth + 1))
                    .collect::<Result<Vec<_>, _>>()
                    .map(serde_json::Value::Array),
                _ => Err(BindingIssue::new(BindingIssueKind::InvalidType)),
            },
            Self::Map(codec) => match value {
                CanonicalValue::Object(values) => values
                    .iter()
                    .map(|(key, value)| {
                        codec
                            .normalized_serde(value, limits, depth + 1)
                            .map(|value| (key.clone(), value))
                    })
                    .collect::<Result<serde_json::Map<_, _>, _>>()
                    .map(serde_json::Value::Object),
                _ => Err(BindingIssue::new(BindingIssueKind::InvalidType)),
            },
        }
    }

    fn encode_serde(
        &self,
        value: serde_json::Value,
        limits: &InputLimits,
        depth: usize,
    ) -> Result<CanonicalValue, BindingIssue> {
        if depth > limits.max_depth() {
            return Err(BindingIssue::new(BindingIssueKind::InputTooDeep));
        }
        match self {
            Self::I64 => value
                .as_i64()
                .map(encode_i64)
                .ok_or_else(|| BindingIssue::new(BindingIssueKind::TypeMismatch)),
            Self::U64 => value
                .as_u64()
                .map(encode_u64)
                .ok_or_else(|| BindingIssue::new(BindingIssueKind::TypeMismatch)),
            Self::List(codec) => match value {
                serde_json::Value::Array(values) => values
                    .into_iter()
                    .map(|value| codec.encode_serde(value, limits, depth + 1))
                    .collect::<Result<Vec<_>, _>>()
                    .map(CanonicalValue::Array),
                _ => Err(BindingIssue::new(BindingIssueKind::TypeMismatch)),
            },
            Self::Map(codec) => match value {
                serde_json::Value::Object(values) => values
                    .into_iter()
                    .map(|(key, value)| {
                        codec
                            .encode_serde(value, limits, depth + 1)
                            .map(|value| (key, value))
                    })
                    .collect::<Result<_, _>>()
                    .map(CanonicalValue::Object),
                _ => Err(BindingIssue::new(BindingIssueKind::TypeMismatch)),
            },
            _ => {
                let canonical = CanonicalValue::from_serde_value(value)
                    .map_err(|_| BindingIssue::new(BindingIssueKind::TypeMismatch))?;
                self.validate(&canonical, limits)?;
                Ok(canonical)
            }
        }
    }
}

fn validate_limits(value: &CanonicalValue, limits: &InputLimits) -> Result<(), BindingIssue> {
    to_canonical_bytes(value, limits)
        .map(|_| ())
        .map_err(|error| {
            BindingIssue::new(match error.kind() {
                CanonicalErrorKind::TooLarge => BindingIssueKind::InputTooLarge,
                CanonicalErrorKind::TooDeep => BindingIssueKind::InputTooDeep,
                CanonicalErrorKind::TooManyEntries => BindingIssueKind::TooManyEntries,
                CanonicalErrorKind::StringTooLong => BindingIssueKind::StringTooLong,
                _ => BindingIssueKind::InvalidValue,
            })
        })
}

fn integer_value(value: &CanonicalValue) -> Result<serde_json::Number, BindingIssue> {
    let parsed = match value {
        CanonicalValue::String(value) => parse_i64(value)?,
        CanonicalValue::Number(value)
            if value.get().fract() == 0.0 && value.get().abs() <= MAX_SAFE_INTEGER =>
        {
            value.get() as i64
        }
        CanonicalValue::Object(_) => {
            decode_i64(value).map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))?
        }
        _ => return Err(BindingIssue::new(BindingIssueKind::InvalidType)),
    };
    Ok(serde_json::Number::from(parsed))
}

fn unsigned_value(value: &CanonicalValue) -> Result<serde_json::Number, BindingIssue> {
    let parsed = match value {
        CanonicalValue::String(value) => parse_u64(value)?,
        CanonicalValue::Number(value)
            if value.get().fract() == 0.0
                && value.get().is_sign_positive()
                && value.get() <= MAX_SAFE_INTEGER =>
        {
            value.get() as u64
        }
        CanonicalValue::Object(_) => {
            decode_u64(value).map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))?
        }
        _ => return Err(BindingIssue::new(BindingIssueKind::InvalidType)),
    };
    Ok(serde_json::Number::from(parsed))
}

fn parse_i64(value: &str) -> Result<i64, BindingIssue> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    let canonical = value == "0"
        || (!digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical {
        return Err(BindingIssue::new(BindingIssueKind::InvalidValue));
    }
    value
        .parse()
        .map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))
}

fn parse_u64(value: &str) -> Result<u64, BindingIssue> {
    let canonical = value == "0"
        || (!value.is_empty()
            && !value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical {
        return Err(BindingIssue::new(BindingIssueKind::InvalidValue));
    }
    value
        .parse()
        .map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))
}

fn date_value(value: &CanonicalValue) -> Result<Date, BindingIssue> {
    let CanonicalValue::String(value) = value else {
        return Err(BindingIssue::new(BindingIssueKind::InvalidType));
    };
    let format = time::format_description::parse("[year]-[month]-[day]")
        .map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))?;
    Date::parse(value, &format).map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))
}

fn datetime_value(value: &CanonicalValue) -> Result<OffsetDateTime, BindingIssue> {
    let CanonicalValue::String(value) = value else {
        return Err(BindingIssue::new(BindingIssueKind::InvalidType));
    };
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))
}

fn uuid_value(value: &CanonicalValue) -> Result<Uuid, BindingIssue> {
    let CanonicalValue::String(value) = value else {
        return Err(BindingIssue::new(BindingIssueKind::InvalidType));
    };
    let parsed =
        Uuid::parse_str(value).map_err(|_| BindingIssue::new(BindingIssueKind::InvalidValue))?;
    if parsed.hyphenated().to_string() != value.to_ascii_lowercase() {
        return Err(BindingIssue::new(BindingIssueKind::InvalidValue));
    }
    Ok(parsed)
}

fn downcast<T: 'static, V: Any>(value: V) -> Result<T, BindingIssue> {
    let boxed: Box<dyn Any> = Box::new(value);
    boxed
        .downcast::<T>()
        .map(|value| *value)
        .map_err(|_| BindingIssue::new(BindingIssueKind::TypeMismatch))
}
