//! Registered bounded action argument schemas and typed preparation.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;

use crate::canonical::{CanonicalValue, check_canonical};
use crate::identity::ModelField;
use crate::limits::InputLimits;
use crate::state::{FieldIssue, FieldIssueKind, ModelCodec, object_schema};

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
    /// The parameter names in the order the action declares them, which is
    /// the order positional directive arguments such as `remove(42)` bind.
    declared: Vec<ModelField>,
}

impl ActionArgumentSchema {
    /// Builds a bounded unique schema.
    pub fn new(fields: Vec<ActionArgumentField>) -> Result<Self, ActionError> {
        if fields.len() > HARD_MAX_ACTION_ARGUMENTS {
            return Err(ActionError::invalid_arguments());
        }
        let mut indexed = BTreeMap::new();
        let mut declared = Vec::with_capacity(fields.len());
        for field in fields {
            declared.push(field.name.clone());
            if indexed.insert(field.name.clone(), field).is_some() {
                return Err(ActionError::invalid_arguments());
            }
        }
        Ok(Self {
            fields: indexed,
            declared,
        })
    }

    /// Creates an action contract with no browser arguments.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            fields: BTreeMap::new(),
            declared: Vec::new(),
        }
    }

    /// Returns generated fields in canonical name order.
    #[must_use]
    pub fn fields(&self) -> impl ExactSizeIterator<Item = &ActionArgumentField> {
        self.fields.values()
    }

    /// Returns generated fields in the order the action declares them: the
    /// order a directive's positional literal arguments map to.
    pub fn declared(&self) -> impl Iterator<Item = &ActionArgumentField> {
        self.declared
            .iter()
            .filter_map(|name| self.fields.get(name))
    }

    pub(crate) fn field(&self, name: &ModelField) -> Option<&ActionArgumentField> {
        self.fields.get(name)
    }

    /// The JSON Schema (draft 2020-12) an agent tool takes for these
    /// arguments, or `None` when an argument's codec has none
    /// ([`ModelCodec::json_schema`]).
    ///
    /// The object follows what preparation enforces: every declared field in
    /// declared order, the required ones listed, an optional one also
    /// accepting `null`, and no other field, since preparation refuses an
    /// unknown name.
    #[must_use]
    pub fn json_schema(&self) -> Option<serde_json::Value> {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for field in self.declared() {
            let schema = field.codec.json_schema()?;
            let schema = if field.required {
                required.push(serde_json::Value::String(field.name.as_str().to_owned()));
                schema
            } else {
                serde_json::json!({ "anyOf": [schema, { "type": "null" }] })
            };
            properties.insert(field.name.as_str().to_owned(), schema);
        }
        Some(object_schema(properties, required))
    }

    /// Every field of `value` that preparation would refuse, and why, in
    /// field-name order: an unknown name, a missing or `null` required
    /// field, or a value its codec refuses.
    ///
    /// Preparation answers a whole request with one closed error, which is
    /// what the browser runtime needs. An agent writes its arguments by
    /// hand, and naming every field lets it correct them in one call;
    /// preparation still runs and decides.
    #[must_use]
    pub fn field_issues(&self, value: &CanonicalValue, limits: &InputLimits) -> Vec<FieldIssue> {
        let CanonicalValue::Object(values) = value else {
            return vec![FieldIssue::new("", FieldIssueKind::NotAnObject)];
        };
        let mut issues = Vec::new();
        for (name, value) in values {
            let Some(field) = ModelField::parse(name)
                .ok()
                .and_then(|parsed| self.fields.get(&parsed))
            else {
                issues.push(FieldIssue::new(name.clone(), FieldIssueKind::Unknown));
                continue;
            };
            if matches!(value, CanonicalValue::Null) {
                if field.required {
                    issues.push(FieldIssue::new(name.clone(), FieldIssueKind::Null));
                }
            } else if let Err(issue) = field.codec.validate(value, limits) {
                issues.push(FieldIssue::new(
                    name.clone(),
                    FieldIssueKind::Invalid(issue.kind()),
                ));
            }
        }
        for field in self.fields.values() {
            if field.required && !values.contains_key(field.name.as_str()) {
                issues.push(FieldIssue::new(
                    field.name.as_str(),
                    FieldIssueKind::Missing,
                ));
            }
        }
        issues.sort_by(|left, right| left.field().cmp(right.field()));
        issues
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
    /// The argument object as the browser sent it, validated and kept: the
    /// values a typed decode reads and the canonical object one value.
    canonical: CanonicalValue,
    limits: InputLimits,
}

impl PreparedActionArguments {
    pub(crate) fn prepare(
        schema: &ActionArgumentSchema,
        raw: RawActionArguments,
        limits: &InputLimits,
    ) -> Result<Self, ActionError> {
        // Only the limit check: the canonical bytes themselves are not kept,
        // so they are counted rather than written into a copy of the value.
        check_canonical(&raw.value, limits).map_err(|_| ActionError::invalid_arguments())?;
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
        Ok(Self {
            schema: schema.clone(),
            canonical: CanonicalValue::Object(values),
            limits: *limits,
        })
    }

    /// The argument object; `prepare` stores nothing else.
    fn values(&self) -> Option<&BTreeMap<String, CanonicalValue>> {
        match &self.canonical {
            CanonicalValue::Object(values) => Some(values),
            _ => None,
        }
    }

    /// Decodes one generated argument using its registered Rust codec.
    pub fn decode<T: DeserializeOwned + 'static>(&self, name: &str) -> Result<T, ActionError> {
        let name = ModelField::parse(name).map_err(|_| ActionError::invalid_arguments())?;
        let field = self
            .schema
            .field(&name)
            .ok_or_else(ActionError::invalid_arguments)?;
        let value = self.values().and_then(|values| values.get(name.as_str()));
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
            .field("argument_count", &self.values().map_or(0, BTreeMap::len))
            .finish()
    }
}
