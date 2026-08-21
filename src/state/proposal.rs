//! Pre-setter proposal authorization and lossless field outcomes.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::de::DeserializeOwned;

use crate::canonical::{CanonicalErrorKind, CanonicalValue, to_canonical_bytes};
use crate::limits::InputLimits;
use crate::snapshot::state::FieldCategory;

use super::{BindingIssue, BindingIssueKind, ModelCodec, ModelPath, PathErrorKind};

const HARD_MAX_PROPOSALS: usize = 4_096;
const HARD_MAX_ISSUES: usize = 1_024;

/// Lossless pre-application state of one browser proposal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposedValue<T> {
    /// The request did not propose this registered field.
    Missing,
    /// The request explicitly proposed JSON null.
    Null,
    /// The value was present but failed its registered typed codec.
    Invalid(BindingIssue),
    /// The value was present and decoded without coercion.
    Valid(T),
}

/// Observable result of applying one proposal to owned component state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalApplication {
    /// No proposal existed and no setter ran.
    Missing,
    /// Null was proposed for a required field and no setter ran.
    Null(BindingIssue),
    /// Conversion failed and no setter ran.
    Invalid(BindingIssue),
    /// A valid value, or null for an optional field, was applied once.
    Applied,
}

/// Closed reason a proposal batch was rejected before generated setters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProposalErrorKind {
    /// Registered codec metadata was malformed or exceeded its static bounds.
    InvalidSchema,
    /// The batch exceeded its configured proposal count.
    TooManyProposals,
    /// The batch exceeded its configured field-issue count.
    TooManyIssues,
    /// A path was malformed, too long, or too deep.
    MalformedPath,
    /// A collection used a mutable numeric position instead of a stable key.
    UnstableCollectionPath,
    /// No registered field owned the proposed path.
    UnknownField,
    /// The registered field was not browser-proposable.
    ForbiddenField,
    /// The same registered path appeared more than once.
    DuplicatePath,
    /// Parent and descendant paths were proposed in the same batch.
    ConflictingPaths,
    /// Total bounded proposal bytes were exceeded.
    InputTooLarge,
    /// Proposal input exceeded its nesting bound.
    InputTooDeep,
    /// Proposal input exceeded its collection-entry bound.
    TooManyEntries,
    /// Proposal input exceeded its string bound.
    StringTooLong,
}

impl ProposalErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidSchema => "invalid_model_binding_schema",
            Self::TooManyProposals => "too_many_model_proposals",
            Self::TooManyIssues => "too_many_binding_issues",
            Self::MalformedPath => "malformed_model_path",
            Self::UnstableCollectionPath => "unstable_collection_path",
            Self::UnknownField => "unknown_model_field",
            Self::ForbiddenField => "forbidden_model_field",
            Self::DuplicatePath => "duplicate_model_path",
            Self::ConflictingPaths => "conflicting_model_paths",
            Self::InputTooLarge => "model_proposals_too_large",
            Self::InputTooDeep => "model_proposals_too_deep",
            Self::TooManyEntries => "too_many_model_entries",
            Self::StringTooLong => "model_string_too_long",
        }
    }
}

/// Redacted proposal-batch rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProposalError {
    kind: ProposalErrorKind,
}

impl ProposalError {
    const fn new(kind: ProposalErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed batch rejection.
    #[must_use]
    pub const fn kind(self) -> ProposalErrorKind {
        self.kind
    }
}

impl fmt::Display for ProposalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl Error for ProposalError {}

/// Invalid proposal limit configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProposalLimitError;

impl fmt::Display for ProposalLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_proposal_limits")
    }
}

impl Error for ProposalLimitError {}

/// Bounded proposal count, issue count, and canonical input profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProposalLimits {
    max_proposals: usize,
    max_issues: usize,
    input: InputLimits,
}

impl ProposalLimits {
    /// Creates nonzero limits below the engine hard ceilings.
    pub fn new(
        max_proposals: usize,
        max_issues: usize,
        input: InputLimits,
    ) -> Result<Self, ProposalLimitError> {
        if max_proposals == 0
            || max_proposals > HARD_MAX_PROPOSALS
            || max_issues == 0
            || max_issues > HARD_MAX_ISSUES
        {
            return Err(ProposalLimitError);
        }
        Ok(Self {
            max_proposals,
            max_issues,
            input,
        })
    }

    /// Returns the canonical per-batch input limits.
    #[must_use]
    pub const fn input_limits(&self) -> InputLimits {
        self.input
    }
}

impl Default for ProposalLimits {
    fn default() -> Self {
        Self {
            max_proposals: 128,
            max_issues: 32,
            input: InputLimits::default(),
        }
    }
}

/// One raw browser proposal before path authorization or typed conversion.
pub struct RawModelProposal {
    path: String,
    value: CanonicalValue,
}

impl RawModelProposal {
    /// Creates an untrusted proposal; validation occurs only against a registered schema.
    #[must_use]
    pub fn new(path: impl Into<String>, value: CanonicalValue) -> Self {
        Self {
            path: path.into(),
            value,
        }
    }
}

/// Registered path, mutation category, and typed codec for one component field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelFieldBinding {
    path: ModelPath,
    category: FieldCategory,
    codec: ModelCodec,
}

impl ModelFieldBinding {
    /// Creates one bounded field binding without expanding its mutation authority.
    pub fn new(
        path: &str,
        category: FieldCategory,
        codec: ModelCodec,
    ) -> Result<Self, super::PathError> {
        Ok(Self {
            path: ModelPath::parse(path)?,
            category,
            codec,
        })
    }

    /// Returns the registered model path.
    #[must_use]
    pub const fn path(&self) -> &ModelPath {
        &self.path
    }

    /// Returns the field's closed mutation category.
    #[must_use]
    pub const fn category(&self) -> FieldCategory {
        self.category
    }

    /// Returns the field's typed model codec.
    #[must_use]
    pub const fn codec(&self) -> &ModelCodec {
        &self.codec
    }
}

/// Immutable registered binding schema used before generated setter dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelBindingSchema {
    fields: BTreeMap<ModelPath, ModelFieldBinding>,
}

impl ModelBindingSchema {
    /// Builds an exact schema and rejects duplicate registered paths.
    pub fn new(fields: Vec<ModelFieldBinding>) -> Result<Self, ProposalError> {
        let mut indexed = BTreeMap::new();
        for field in fields {
            field
                .codec
                .validate_contract()
                .map_err(|_| ProposalError::new(ProposalErrorKind::InvalidSchema))?;
            if indexed.insert(field.path.clone(), field).is_some() {
                return Err(ProposalError::new(ProposalErrorKind::DuplicatePath));
            }
        }
        Ok(Self { fields: indexed })
    }

    fn field(&self, path: &ModelPath) -> Option<&ModelFieldBinding> {
        self.fields.get(path)
    }
}

/// Authorized batch whose values remain redacted until typed field access.
pub struct ProposalBatch {
    schema: ModelBindingSchema,
    values: BTreeMap<ModelPath, CanonicalValue>,
    issues: Vec<BindingIssue>,
    input_limits: InputLimits,
}

impl ProposalBatch {
    /// Authorizes every path, bounds the whole batch, and records conversion issues.
    pub fn prepare(
        schema: &ModelBindingSchema,
        proposals: Vec<RawModelProposal>,
        limits: &ProposalLimits,
    ) -> Result<Self, ProposalError> {
        if proposals.len() > limits.max_proposals {
            return Err(ProposalError::new(ProposalErrorKind::TooManyProposals));
        }
        let mut values = BTreeMap::<ModelPath, CanonicalValue>::new();
        let mut issues = Vec::new();
        let mut total_bytes = 0_usize;
        for proposal in proposals {
            let path = ModelPath::parse(&proposal.path).map_err(|error| {
                ProposalError::new(match error.kind() {
                    PathErrorKind::UnstableCollectionPath => {
                        ProposalErrorKind::UnstableCollectionPath
                    }
                    _ => ProposalErrorKind::MalformedPath,
                })
            })?;
            let field = schema
                .field(&path)
                .ok_or_else(|| ProposalError::new(ProposalErrorKind::UnknownField))?;
            if !matches!(
                field.category,
                FieldCategory::Model | FieldCategory::Transient
            ) {
                return Err(ProposalError::new(ProposalErrorKind::ForbiddenField));
            }
            if values.contains_key(&path) {
                return Err(ProposalError::new(ProposalErrorKind::DuplicatePath));
            }
            if values.keys().any(|existing| existing.conflicts_with(&path)) {
                return Err(ProposalError::new(ProposalErrorKind::ConflictingPaths));
            }

            let bytes = to_canonical_bytes(&proposal.value, &limits.input)
                .map_err(map_canonical_error)?
                .len();
            total_bytes = total_bytes
                .checked_add(bytes)
                .ok_or_else(|| ProposalError::new(ProposalErrorKind::InputTooLarge))?;
            if total_bytes > limits.input.max_bytes() {
                return Err(ProposalError::new(ProposalErrorKind::InputTooLarge));
            }

            if !matches!(proposal.value, CanonicalValue::Null)
                && let Err(issue) = field.codec.validate(&proposal.value, &limits.input)
            {
                issues.push(issue.at_path(path.clone()));
                if issues.len() > limits.max_issues {
                    return Err(ProposalError::new(ProposalErrorKind::TooManyIssues));
                }
            }
            values.insert(path, proposal.value);
        }
        Ok(Self {
            schema: schema.clone(),
            values,
            issues,
            input_limits: limits.input,
        })
    }

    /// Returns bounded redacted conversion issues in proposal order.
    #[must_use]
    pub fn issues(&self) -> &[BindingIssue] {
        &self.issues
    }

    /// Decodes one known field while preserving missing, null, invalid, and valid states.
    #[must_use]
    pub fn proposed<T: DeserializeOwned + 'static>(&self, path: &ModelPath) -> ProposedValue<T> {
        let Some(field) = self.schema.field(path) else {
            return ProposedValue::Invalid(
                BindingIssue::new(BindingIssueKind::InvalidValue).at_path(path.clone()),
            );
        };
        let Some(value) = self.values.get(path) else {
            return ProposedValue::Missing;
        };
        if matches!(value, CanonicalValue::Null) {
            return ProposedValue::Null;
        }
        match field.codec.decode(value, &self.input_limits) {
            Ok(value) => ProposedValue::Valid(value),
            Err(issue) => ProposedValue::Invalid(issue.at_path(path.clone())),
        }
    }

    /// Applies a required typed value exactly once and never mutates on other outcomes.
    pub fn apply_required<S, T, F>(
        &self,
        path: &ModelPath,
        state: &mut S,
        setter: F,
    ) -> ProposalApplication
    where
        T: DeserializeOwned + 'static,
        F: FnOnce(&mut S, T),
    {
        match self.proposed(path) {
            ProposedValue::Missing => ProposalApplication::Missing,
            ProposedValue::Null => ProposalApplication::Null(
                BindingIssue::new(BindingIssueKind::InvalidType).at_path(path.clone()),
            ),
            ProposedValue::Invalid(issue) => ProposalApplication::Invalid(issue),
            ProposedValue::Valid(value) => {
                setter(state, value);
                ProposalApplication::Applied
            }
        }
    }

    /// Applies null as `None`, a valid value as `Some`, and never mutates otherwise.
    pub fn apply_optional<S, T, F>(
        &self,
        path: &ModelPath,
        state: &mut S,
        setter: F,
    ) -> ProposalApplication
    where
        T: DeserializeOwned + 'static,
        F: FnOnce(&mut S, Option<T>),
    {
        match self.proposed(path) {
            ProposedValue::Missing => ProposalApplication::Missing,
            ProposedValue::Null => {
                setter(state, None);
                ProposalApplication::Applied
            }
            ProposedValue::Invalid(issue) => ProposalApplication::Invalid(issue),
            ProposedValue::Valid(value) => {
                setter(state, Some(value));
                ProposalApplication::Applied
            }
        }
    }
}

impl fmt::Debug for ProposalBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProposalBatch")
            .field("proposal_count", &self.values.len())
            .field("issue_count", &self.issues.len())
            .finish()
    }
}

fn map_canonical_error(error: crate::canonical::CanonicalError) -> ProposalError {
    ProposalError::new(match error.kind() {
        CanonicalErrorKind::TooLarge => ProposalErrorKind::InputTooLarge,
        CanonicalErrorKind::TooDeep => ProposalErrorKind::InputTooDeep,
        CanonicalErrorKind::TooManyEntries => ProposalErrorKind::TooManyEntries,
        CanonicalErrorKind::StringTooLong => ProposalErrorKind::StringTooLong,
        _ => ProposalErrorKind::InputTooLarge,
    })
}
