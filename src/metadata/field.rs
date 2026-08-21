//! Registered component-field metadata.

use crate::identity::ModelField;
use crate::snapshot::state::{FieldCategory, StateCodec};
use crate::state::{BindingTiming, ModelCodec, UrlBinding};

use super::{MetadataError, MetadataErrorKind};

/// Canonical metadata for one component state field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldMetadata {
    name: ModelField,
    category: FieldCategory,
    codec: StateCodec,
    required: bool,
    model_codec: Option<ModelCodec>,
    session_codec: Option<ModelCodec>,
    binding_timing: Option<BindingTiming>,
    url_binding: Option<UrlBinding>,
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
            model_codec: None,
            session_codec: None,
            binding_timing: None,
            url_binding: None,
        }
    }

    /// Attaches an explicit typed model codec and synchronization timing.
    pub fn with_model_binding(
        mut self,
        codec: ModelCodec,
        timing: BindingTiming,
    ) -> Result<Self, MetadataError> {
        if !matches!(
            self.category,
            FieldCategory::Model | FieldCategory::Transient
        ) {
            return Err(MetadataError::new(
                MetadataErrorKind::InvalidBindingMetadata,
            ));
        }
        if !timing.is_valid() || codec.validate_contract().is_err() || self.model_codec.is_some() {
            return Err(MetadataError::new(
                MetadataErrorKind::InvalidBindingMetadata,
            ));
        }
        self.model_codec = Some(codec);
        self.binding_timing = Some(timing);
        Ok(self)
    }

    /// Attaches the typed codec used by the host session port for this field.
    pub fn with_session_binding(mut self, codec: ModelCodec) -> Result<Self, MetadataError> {
        if self.category != FieldCategory::Session
            || codec.validate_contract().is_err()
            || self.session_codec.is_some()
        {
            return Err(MetadataError::new(
                MetadataErrorKind::InvalidBindingMetadata,
            ));
        }
        self.session_codec = Some(codec);
        Ok(self)
    }

    /// Attaches typed URL metadata whose category must match this field.
    pub fn with_url_binding(mut self, binding: UrlBinding) -> Result<Self, MetadataError> {
        if binding.category() != self.category
            || binding.codec().validate_contract().is_err()
            || self.url_binding.is_some()
            || self
                .model_codec
                .as_ref()
                .is_some_and(|codec| codec != binding.codec())
        {
            return Err(MetadataError::new(
                MetadataErrorKind::InvalidBindingMetadata,
            ));
        }
        self.url_binding = Some(binding);
        Ok(self)
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

    /// Returns the registered model codec, if this field is browser-proposable.
    #[must_use]
    pub const fn model_codec(&self) -> Option<&ModelCodec> {
        self.model_codec.as_ref()
    }

    /// Returns the registered session codec, if this is session-only state.
    #[must_use]
    pub const fn session_codec(&self) -> Option<&ModelCodec> {
        self.session_codec.as_ref()
    }

    /// Returns model synchronization timing, if this field is browser-proposable.
    #[must_use]
    pub const fn binding_timing(&self) -> Option<BindingTiming> {
        self.binding_timing
    }

    /// Returns typed URL metadata, if this field is URL-bound.
    #[must_use]
    pub const fn url_binding(&self) -> Option<&UrlBinding> {
        self.url_binding.as_ref()
    }
}
