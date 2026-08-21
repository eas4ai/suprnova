//! Registered component-field metadata.

use crate::identity::ModelField;
use crate::snapshot::state::{FieldCategory, StateCodec};

/// Canonical metadata for one component state field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldMetadata {
    name: ModelField,
    category: FieldCategory,
    codec: StateCodec,
    required: bool,
}

impl FieldMetadata {
    /// Creates field metadata from already validated identities and closed enums.
    #[must_use]
    pub const fn new(
        name: ModelField,
        category: FieldCategory,
        codec: StateCodec,
        required: bool,
    ) -> Self {
        Self {
            name,
            category,
            codec,
            required,
        }
    }

    /// Returns the registered field identity.
    #[must_use]
    pub const fn name(&self) -> &ModelField {
        &self.name
    }

    /// Returns the field's snapshot/binding category.
    #[must_use]
    pub const fn category(&self) -> FieldCategory {
        self.category
    }

    /// Returns the field's canonical state codec.
    #[must_use]
    pub const fn codec(&self) -> StateCodec {
        self.codec
    }

    /// Returns whether the field is required in eligible state exposure forms.
    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }
}
