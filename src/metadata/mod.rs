//! Canonical generated component, field, action, and version metadata.

mod component;
mod digest;
mod field;
mod method;
mod version;

use std::error::Error;
use std::fmt;

pub use component::ComponentMetadata;
pub use field::FieldMetadata;
pub use method::ActionMetadata;
pub use version::ContractVersions;

/// Closed reason component metadata construction failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataErrorKind {
    /// A component, state, action, checker, or protocol version was zero.
    InvalidVersion,
    /// The component requires a Live protocol this engine does not implement.
    UnsupportedProtocol,
    /// More fields were declared than the bounded metadata profile permits.
    TooManyFields,
    /// More actions were declared than the bounded metadata profile permits.
    TooManyActions,
    /// The component declared the same field identity more than once.
    DuplicateField,
    /// The component declared the same action identity more than once.
    DuplicateAction,
    /// Canonical contract metadata could not be encoded within fixed bounds.
    ContractEncodingFailed,
}

impl MetadataErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidVersion => "invalid_metadata_version",
            Self::UnsupportedProtocol => "unsupported_component_protocol",
            Self::TooManyFields => "too_many_component_fields",
            Self::TooManyActions => "too_many_component_actions",
            Self::DuplicateField => "duplicate_component_field",
            Self::DuplicateAction => "duplicate_component_action",
            Self::ContractEncodingFailed => "component_contract_encoding_failed",
        }
    }
}

/// Redacted component-metadata construction error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MetadataError {
    kind: MetadataErrorKind,
}

impl MetadataError {
    pub(crate) const fn new(kind: MetadataErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed construction failure.
    #[must_use]
    pub const fn kind(self) -> MetadataErrorKind {
        self.kind
    }
}

impl fmt::Display for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for MetadataError {}
