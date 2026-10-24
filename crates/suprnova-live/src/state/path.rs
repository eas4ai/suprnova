//! Bounded registered paths with explicit stable collection keys.

use std::error::Error;
use std::fmt;

const MAX_PATH_BYTES: usize = 256;
const MAX_SEGMENTS: usize = 16;
const MAX_SEGMENT_BYTES: usize = 64;

/// Closed reason a model path was not safe and statically addressable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathErrorKind {
    /// The path grammar was malformed or contained a forbidden character.
    Malformed,
    /// The path or one segment exceeded its byte bound.
    TooLong,
    /// The path contained too many nested segments.
    TooDeep,
    /// A numeric collection index was used instead of a stable item key.
    UnstableCollectionPath,
}

impl PathErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Malformed => "malformed_model_path",
            Self::TooLong => "model_path_too_long",
            Self::TooDeep => "model_path_too_deep",
            Self::UnstableCollectionPath => "unstable_collection_path",
        }
    }
}

/// Redacted model-path construction error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PathError {
    kind: PathErrorKind,
}

impl PathError {
    const fn new(kind: PathErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> PathErrorKind {
        self.kind
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl Error for PathError {}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PathSegment {
    Field(String),
    StableKey(String),
}

/// Validated registered model path.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ModelPath {
    canonical: String,
    segments: Vec<PathSegment>,
}

impl ModelPath {
    /// Parses fields separated by dots and optional stable keys such as `items[sku-1].quantity`.
    pub fn parse(value: &str) -> Result<Self, PathError> {
        if value.is_empty() {
            return Err(PathError::new(PathErrorKind::Malformed));
        }
        if value.len() > MAX_PATH_BYTES {
            return Err(PathError::new(PathErrorKind::TooLong));
        }

        let mut segments = Vec::new();
        for raw in value.split('.') {
            if raw.is_empty() {
                return Err(PathError::new(PathErrorKind::Malformed));
            }
            let (field, stable_key) = parse_segment(raw)?;
            if field.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(PathError::new(PathErrorKind::UnstableCollectionPath));
            }
            segments.push(PathSegment::Field(field.to_owned()));
            if let Some(stable_key) = stable_key {
                segments.push(PathSegment::StableKey(stable_key.to_owned()));
            }
            if segments.len() > MAX_SEGMENTS {
                return Err(PathError::new(PathErrorKind::TooDeep));
            }
        }
        Ok(Self {
            canonical: value.to_owned(),
            segments,
        })
    }

    /// Returns the validated canonical path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    pub(crate) fn conflicts_with(&self, other: &Self) -> bool {
        self.segments != other.segments
            && (self.segments.starts_with(&other.segments)
                || other.segments.starts_with(&self.segments))
    }
}

impl fmt::Debug for ModelPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ModelPath")
            .field(&self.canonical)
            .finish()
    }
}

fn parse_segment(value: &str) -> Result<(&str, Option<&str>), PathError> {
    let (field, stable_key) = if let Some(open) = value.find('[') {
        if !value.ends_with(']') || value[open + 1..value.len() - 1].contains(['[', ']']) {
            return Err(PathError::new(PathErrorKind::Malformed));
        }
        (&value[..open], Some(&value[open + 1..value.len() - 1]))
    } else {
        if value.contains(']') {
            return Err(PathError::new(PathErrorKind::Malformed));
        }
        (value, None)
    };
    validate_atom(field)?;
    if let Some(stable_key) = stable_key {
        validate_atom(stable_key)?;
        if stable_key.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(PathError::new(PathErrorKind::UnstableCollectionPath));
        }
    }
    Ok((field, stable_key))
}

fn validate_atom(value: &str) -> Result<(), PathError> {
    if value.is_empty() {
        return Err(PathError::new(PathErrorKind::Malformed));
    }
    if value.len() > MAX_SEGMENT_BYTES {
        return Err(PathError::new(PathErrorKind::TooLong));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PathError::new(PathErrorKind::Malformed));
    }
    Ok(())
}
