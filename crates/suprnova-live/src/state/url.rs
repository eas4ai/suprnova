//! Typed shareable query bindings with replace-only reflection or real navigation.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::canonical::CanonicalValue;
use crate::identity::ModelField;
use crate::limits::InputLimits;
use crate::snapshot::state::{FieldCategory, decode_i64, decode_u64};

use super::{BindingIssueKind, ModelCodec};

/// Whether URL state reflects into the current query or performs real navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlBindingMode {
    /// Replace the same route's query without creating a history entry.
    Reflect,
    /// Produce ordinary route navigation intent with normal document history.
    Navigate,
}

/// Closed URL-binding construction or conversion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlErrorKind {
    /// The query key was empty, too long, or outside its ASCII grammar.
    InvalidQueryKey,
    /// The field category is sensitive, transient, or server-authoritative.
    ForbiddenCategory,
    /// The codec is not a scalar shareable URL representation.
    UnsupportedCodec,
    /// Two fields claimed the same query key.
    DuplicateQueryKey,
    /// The same field was registered more than once.
    DuplicateField,
    /// A query value failed its registered typed codec.
    InvalidValue,
    /// A URL value exceeded the configured input bound.
    InputTooLarge,
}

impl UrlErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidQueryKey => "invalid_url_query_key",
            Self::ForbiddenCategory => "forbidden_url_field_category",
            Self::UnsupportedCodec => "unsupported_url_codec",
            Self::DuplicateQueryKey => "duplicate_url_query_key",
            Self::DuplicateField => "duplicate_url_field",
            Self::InvalidValue => "invalid_url_value",
            Self::InputTooLarge => "url_value_too_large",
        }
    }
}

/// Redacted URL binding failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UrlError {
    kind: UrlErrorKind,
}

impl UrlError {
    const fn new(kind: UrlErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed URL failure.
    #[must_use]
    pub const fn kind(self) -> UrlErrorKind {
        self.kind
    }
}

impl fmt::Display for UrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl Error for UrlError {}

/// One registered scalar URL representation for component state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UrlBinding {
    query_key: String,
    category: FieldCategory,
    codec: ModelCodec,
    mode: UrlBindingMode,
    omit_default: bool,
}

impl UrlBinding {
    /// Creates a shareable scalar binding under an explicit navigation policy.
    pub fn new(
        query_key: &str,
        category: FieldCategory,
        codec: ModelCodec,
        mode: UrlBindingMode,
        omit_default: bool,
    ) -> Result<Self, UrlError> {
        let valid_key = !query_key.is_empty()
            && query_key.len() <= 64
            && query_key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
        if !valid_key {
            return Err(UrlError::new(UrlErrorKind::InvalidQueryKey));
        }
        if !matches!(
            category,
            FieldCategory::State | FieldCategory::Public | FieldCategory::Model
        ) {
            return Err(UrlError::new(UrlErrorKind::ForbiddenCategory));
        }
        if !url_scalar_codec(&codec) {
            return Err(UrlError::new(UrlErrorKind::UnsupportedCodec));
        }
        codec
            .validate_contract()
            .map_err(|_| UrlError::new(UrlErrorKind::InvalidValue))?;
        Ok(Self {
            query_key: query_key.to_owned(),
            category,
            codec,
            mode,
            omit_default,
        })
    }

    /// Returns the stable query key.
    #[must_use]
    pub fn query_key(&self) -> &str {
        &self.query_key
    }

    /// Returns the field category validated when the binding was built.
    #[must_use]
    pub const fn category(&self) -> FieldCategory {
        self.category
    }

    /// Returns the registered scalar codec.
    #[must_use]
    pub const fn codec(&self) -> &ModelCodec {
        &self.codec
    }

    /// Returns replace reflection or real navigation semantics.
    #[must_use]
    pub const fn mode(&self) -> UrlBindingMode {
        self.mode
    }

    /// Returns whether an encoded value equal to the declared default is omitted.
    #[must_use]
    pub const fn omit_default(&self) -> bool {
        self.omit_default
    }

    /// Encodes one typed scalar into its query-string value.
    pub fn encode<T: Serialize + 'static>(
        &self,
        value: &T,
        limits: &InputLimits,
    ) -> Result<String, UrlError> {
        let canonical = self
            .codec
            .encode(value, limits)
            .map_err(map_binding_issue)?;
        match (&self.codec, canonical) {
            (_, CanonicalValue::String(value)) => Ok(value),
            (ModelCodec::Boolean, CanonicalValue::Bool(value)) => Ok(value.to_string()),
            (ModelCodec::I64, value) => decode_i64(&value)
                .map(|value| value.to_string())
                .map_err(|_| UrlError::new(UrlErrorKind::InvalidValue)),
            (ModelCodec::U64, value) => decode_u64(&value)
                .map(|value| value.to_string())
                .map_err(|_| UrlError::new(UrlErrorKind::InvalidValue)),
            _ => Err(UrlError::new(UrlErrorKind::InvalidValue)),
        }
    }

    /// Decodes one query-string value through the registered Rust codec.
    pub fn decode<T: DeserializeOwned + 'static>(
        &self,
        value: &str,
        limits: &InputLimits,
    ) -> Result<T, UrlError> {
        if value.len() > limits.max_string_bytes() || value.len() > limits.max_bytes() {
            return Err(UrlError::new(UrlErrorKind::InputTooLarge));
        }
        let canonical = match self.codec {
            ModelCodec::Boolean => match value {
                "true" => CanonicalValue::Bool(true),
                "false" => CanonicalValue::Bool(false),
                _ => return Err(UrlError::new(UrlErrorKind::InvalidValue)),
            },
            _ => CanonicalValue::String(value.to_owned()),
        };
        self.codec
            .decode(&canonical, limits)
            .map_err(map_binding_issue)
    }

    /// Encodes a value unless default omission is enabled and values are equal.
    pub fn encode_if_changed<T: Serialize + PartialEq + 'static>(
        &self,
        value: &T,
        default: &T,
        limits: &InputLimits,
    ) -> Result<Option<String>, UrlError> {
        if self.omit_default && value == default {
            return Ok(None);
        }
        self.encode(value, limits).map(Some)
    }
}

/// Immutable URL binding catalog with unique fields and query keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UrlBindingSet {
    fields: BTreeMap<ModelField, UrlBinding>,
}

impl UrlBindingSet {
    /// Builds the exact URL catalog and rejects ambiguous ownership.
    pub fn new(bindings: Vec<(ModelField, UrlBinding)>) -> Result<Self, UrlError> {
        let mut fields = BTreeMap::new();
        let mut query_keys = BTreeSet::new();
        for (field, binding) in bindings {
            if !query_keys.insert(binding.query_key.clone()) {
                return Err(UrlError::new(UrlErrorKind::DuplicateQueryKey));
            }
            if fields.insert(field, binding).is_some() {
                return Err(UrlError::new(UrlErrorKind::DuplicateField));
            }
        }
        Ok(Self { fields })
    }

    /// Resolves a URL binding only by its registered field identity.
    #[must_use]
    pub fn get(&self, field: &ModelField) -> Option<&UrlBinding> {
        self.fields.get(field)
    }
}

fn url_scalar_codec(codec: &ModelCodec) -> bool {
    matches!(
        codec,
        ModelCodec::String
            | ModelCodec::Boolean
            | ModelCodec::I64
            | ModelCodec::U64
            | ModelCodec::Date
            | ModelCodec::DateTime
            | ModelCodec::Uuid
            | ModelCodec::Enumeration(_)
    )
}

fn map_binding_issue(issue: super::BindingIssue) -> UrlError {
    UrlError::new(match issue.kind() {
        BindingIssueKind::InputTooLarge | BindingIssueKind::StringTooLong => {
            UrlErrorKind::InputTooLarge
        }
        _ => UrlErrorKind::InvalidValue,
    })
}
