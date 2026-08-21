//! Canonical value and error types.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq};

/// Maximum integer magnitude represented as an untagged JSON integer.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Why canonical parsing or serialization failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalErrorKind {
    /// Encoded input or output exceeded the byte limit.
    TooLarge,
    /// Array/object nesting exceeded the depth limit.
    TooDeep,
    /// Total array elements plus object members exceeded the count limit.
    TooManyEntries,
    /// A decoded string or key exceeded its byte limit.
    StringTooLong,
    /// An object contained the same key more than once.
    DuplicateKey,
    /// Input bytes were not valid UTF-8.
    InvalidUtf8,
    /// A JSON number was outside the supported interoperable profile.
    InvalidNumber,
    /// Input was not exactly one syntactically valid JSON value.
    InvalidJson,
    /// A validated value could not be serialized canonically.
    SerializationFailed,
}

impl CanonicalErrorKind {
    /// Returns the stable machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "input_too_large",
            Self::TooDeep => "input_too_deep",
            Self::TooManyEntries => "too_many_entries",
            Self::StringTooLong => "string_too_long",
            Self::DuplicateKey => "duplicate_key",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::InvalidNumber => "invalid_number",
            Self::InvalidJson => "invalid_json",
            Self::SerializationFailed => "serialization_failed",
        }
    }
}

/// Redacted canonical codec error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalError {
    kind: CanonicalErrorKind,
}

impl CanonicalError {
    pub(crate) const fn new(kind: CanonicalErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> CanonicalErrorKind {
        self.kind
    }
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl Error for CanonicalError {}

/// Finite IEEE-754 value serialized with the RFC 8785 number profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalNumber(f64);

impl CanonicalNumber {
    /// Validates a finite double and normalizes negative zero.
    pub fn new(value: f64) -> Result<Self, CanonicalError> {
        if !value.is_finite() {
            return Err(CanonicalError::new(CanonicalErrorKind::InvalidNumber));
        }
        Ok(Self(if value == 0.0 { 0.0 } else { value }))
    }

    pub(crate) fn from_i64(value: i64) -> Result<Self, CanonicalError> {
        if value.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(CanonicalError::new(CanonicalErrorKind::InvalidNumber));
        }
        Self::new(value as f64)
    }

    pub(crate) fn from_u64(value: u64) -> Result<Self, CanonicalError> {
        if value > MAX_SAFE_INTEGER {
            return Err(CanonicalError::new(CanonicalErrorKind::InvalidNumber));
        }
        Self::new(value as f64)
    }

    /// Returns the validated finite double.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Serialize for CanonicalNumber {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

/// JSON value admitted by the signed RFC 8785-compatible Live profile.
#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalValue {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// Finite interoperable JSON number.
    Number(CanonicalNumber),
    /// Unicode string bounded by its containing parser limits.
    String(String),
    /// Ordered sequence of canonical values.
    Array(Vec<Self>),
    /// Unique-key object. Canonical serialization owns RFC 8785 key ordering.
    Object(BTreeMap<String, Self>),
}

impl CanonicalValue {
    /// Constructs a validated finite numeric value.
    pub fn number(value: f64) -> Result<Self, CanonicalError> {
        CanonicalNumber::new(value).map(Self::Number)
    }
}

impl Serialize for CanonicalValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            }
            Self::Object(values) => {
                let mut map = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}
