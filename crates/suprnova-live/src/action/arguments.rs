//! Registered bounded action argument schemas and typed preparation.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;

use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::ModelField;
use crate::limits::InputLimits;
use crate::state::ModelCodec;

use super::ActionError;

const HARD_MAX_ACTION_ARGUMENTS: usize = 128;

/// One generated typed action argument contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionArgumentField {
    name: ModelField,
    codec: ModelCodec,
    required: bool,
}

impl ActionArgumentField {
    /// Creates one typed argument after validating its codec contract.
    pub fn new(name: ModelField, codec: ModelCodec, required: bool) -> Result<Self, ActionError> {
        codec
            .validate_contract()
            .map_err(|_| ActionError::invalid_arguments())?;
        Ok(Self {
            name,
            codec,
            required,
        })
    }

    /// Returns the stable generated argument name.
    #[must_use]
    pub const fn name(&self) -> &ModelField {
        &self.name
    }

    /// Returns the registered typed codec.
    #[must_use]
    pub const fn codec(&self) -> &ModelCodec {
        &self.codec
    }

    /// Returns whether missing and null input are forbidden.
    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }
}

/// Immutable generated action-argument schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionArgumentSchema {
    fields: BTreeMap<ModelField, ActionArgumentField>,
}

impl ActionArgumentSchema {
    /// Builds a bounded unique schema.
    pub fn new(fields: Vec<ActionArgumentField>) -> Result<Self, ActionError> {
        if fields.len() > HARD_MAX_ACTION_ARGUMENTS {
            return Err(ActionError::invalid_arguments());
        }
        let mut indexed = BTreeMap::new();
        for field in fields {
            if indexed.insert(field.name.clone(), field).is_some() {
                return Err(ActionError::invalid_arguments());
            }
        }
        Ok(Self { fields: indexed })
    }

    /// Creates an action contract with no browser arguments.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            fields: BTreeMap::new(),
        }
    }

    /// Returns generated fields in canonical name order.
    #[must_use]
    pub fn fields(&self) -> impl ExactSizeIterator<Item = &ActionArgumentField> {
        self.fields.values()
    }

    pub(crate) fn field(&self, name: &ModelField) -> Option<&ActionArgumentField> {
        self.fields.get(name)
    }
}

/// Untrusted browser action-argument value before registered schema validation.
pub struct RawActionArguments {
    value: CanonicalValue,
}

impl RawActionArguments {
    /// Creates raw input; schema and bounds are enforced by [`ActionTable`](super::ActionTable).
    #[must_use]
    pub const fn new(value: CanonicalValue) -> Self {
        Self { value }
    }

    /// Creates the canonical empty argument object.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(CanonicalValue::Object(BTreeMap::new()))
    }
}

/// Schema-authorized action arguments whose values remain redacted until typed access.
pub struct PreparedActionArguments {
    schema: ActionArgumentSchema,
    values: BTreeMap<String, CanonicalValue>,
    canonical: CanonicalValue,
    limits: InputLimits,
}

impl PreparedActionArguments {
    pub(crate) fn prepare(
        schema: &ActionArgumentSchema,
        raw: RawActionArguments,
        limits: &InputLimits,
    ) -> Result<Self, ActionError> {
        to_canonical_bytes(&raw.value, limits).map_err(|_| ActionError::invalid_arguments())?;
        let CanonicalValue::Object(values) = raw.value else {
            return Err(ActionError::invalid_arguments());
        };
        if values.len() > HARD_MAX_ACTION_ARGUMENTS {
            return Err(ActionError::invalid_arguments());
        }
        for (name, value) in &values {
            let name = ModelField::parse(name).map_err(|_| ActionError::invalid_arguments())?;
            let field = schema
                .field(&name)
                .ok_or_else(ActionError::invalid_arguments)?;
            if matches!(value, CanonicalValue::Null) {
                if field.required {
                    return Err(ActionError::invalid_arguments());
                }
            } else {
                field
                    .codec
                    .validate(value, limits)
                    .map_err(|_| ActionError::invalid_arguments())?;
            }
        }
        for field in schema.fields.values() {
            if field.required && !values.contains_key(field.name.as_str()) {
                return Err(ActionError::invalid_arguments());
            }
        }
        let canonical = CanonicalValue::Object(values.clone());
        Ok(Self {
            schema: schema.clone(),
            values,
            canonical,
            limits: *limits,
        })
    }

    /// Decodes one generated argument using its registered Rust codec.
    pub fn decode<T: DeserializeOwned + 'static>(&self, name: &str) -> Result<T, ActionError> {
        let name = ModelField::parse(name).map_err(|_| ActionError::invalid_arguments())?;
        let field = self
            .schema
            .field(&name)
            .ok_or_else(ActionError::invalid_arguments)?;
        let value = self.values.get(name.as_str());
        match value {
            Some(CanonicalValue::Null) | None if !field.required => {
                serde_json::from_value(serde_json::Value::Null)
                    .map_err(|_| ActionError::invalid_arguments())
            }
            Some(value) => field
                .codec
                .decode(value, &self.limits)
                .map_err(|_| ActionError::invalid_arguments()),
            None => Err(ActionError::invalid_arguments()),
        }
    }

    /// Returns the bounded canonical argument object for validation and semantic hashing.
    #[must_use]
    pub const fn canonical(&self) -> &CanonicalValue {
        &self.canonical
    }
}

impl fmt::Debug for PreparedActionArguments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedActionArguments")
            .field("argument_count", &self.values.len())
            .finish()
    }
}
