//! Bounded typed reconciliation for independently owned nested Live islands.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use sha2::{Digest as _, Sha256};

use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::{ComponentName, ContentDigest, InstanceId, ModelField};
use crate::limits::{
    HARD_MAX_DEPTH as HARD_MAX_INPUT_DEPTH, HARD_MAX_ENTRIES,
    HARD_MAX_STRING_BYTES as HARD_MAX_INPUT_STRING_BYTES, InputLimits,
};
use crate::registry::ComponentRegistry;
use crate::snapshot::SnapshotError;
use crate::snapshot::state::{FieldCategory, FieldSpec, StateCodec};
use crate::state::ModelCodec;

const MAX_KEY_BYTES: usize = 128;
const HARD_MAX_FIELDS: usize = 256;
const HARD_MAX_CHILDREN: usize = 1_024;
const HARD_MAX_COMPOSITION_DEPTH: usize = 64;
const HARD_MAX_PARAMETER_BYTES: usize = 2 * 1024 * 1024;
const PARAMETER_SCHEMA_DOMAIN: &[u8] = b"suprnova-live-child-parameter-schema-v1\0";
const PARAMETER_VALUE_DOMAIN: &[u8] = b"suprnova-live-child-parameter-value-v1\0";

/// Stable developer-supplied identity within one parent ownership scope.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChildKey(String);

impl ChildKey {
    /// Parses a bounded, non-secret, unreserved stable child key.
    pub fn parse(value: &str) -> Result<Self, CompositionError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_KEY_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            });
        if !valid {
            return Err(CompositionError::new(CompositionErrorKind::InvalidKey));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated stable key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One registered typed mount parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildParameterField {
    name: ModelField,
    codec: ModelCodec,
    required: bool,
}

impl ChildParameterField {
    /// Declares a named parameter and its registered codec.
    #[must_use]
    pub const fn new(name: ModelField, codec: ModelCodec, required: bool) -> Self {
        Self {
            name,
            codec,
            required,
        }
    }

    fn snapshot_field(&self) -> Result<FieldSpec, SnapshotError> {
        let codec = match self.codec {
            ModelCodec::I64 => StateCodec::I64Decimal,
            ModelCodec::U64 => StateCodec::U64Decimal,
            _ => StateCodec::Json,
        };
        FieldSpec::new(
            self.name.as_str(),
            codec,
            FieldCategory::Public,
            self.required,
        )
    }
}

/// Infallible fixed-width digest of one parameter schema.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ParameterSchemaDigest([u8; 32]);

impl ParameterSchemaDigest {
    /// Returns the digest bytes used by the child capability layer.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Infallible fixed-width digest of one canonical parameter value.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ParameterValueDigest([u8; 32]);

impl ParameterValueDigest {
    /// Returns the digest bytes used by reconciliation and signing.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Exact, versioned, bounded parameter contract registered with a component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildParameterSchema {
    version: u16,
    fields: BTreeMap<ModelField, ChildParameterField>,
    digest: ParameterSchemaDigest,
}

impl ChildParameterSchema {
    /// Creates a non-zero schema with unique field names and valid codecs.
    pub fn new(version: u16, fields: Vec<ChildParameterField>) -> Result<Self, CompositionError> {
        if version == 0 || fields.len() > HARD_MAX_FIELDS {
            return Err(CompositionError::new(CompositionErrorKind::InvalidSchema));
        }
        let mut indexed = BTreeMap::new();
        for field in fields {
            if field.codec.validate_contract().is_err()
                || indexed.insert(field.name.clone(), field).is_some()
            {
                return Err(CompositionError::new(CompositionErrorKind::InvalidSchema));
            }
        }
        let digest = schema_digest(version, &indexed);
        Ok(Self {
            version,
            fields: indexed,
            digest,
        })
    }

    /// Creates a non-zero schema with no parameters.
    pub fn empty(version: u16) -> Result<Self, CompositionError> {
        Self::new(version, vec![])
    }

    /// Returns the independent parameter schema version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the stable parameter-schema digest.
    #[must_use]
    pub const fn digest(&self) -> &ParameterSchemaDigest {
        &self.digest
    }

    /// Returns the exact registered parameter count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns whether the mount initializer accepts no explicit parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub(crate) fn validate(
        &self,
        value: &CanonicalValue,
        limits: &InputLimits,
    ) -> Result<(), CompositionError> {
        let CanonicalValue::Object(values) = value else {
            return Err(CompositionError::new(
                CompositionErrorKind::InvalidParameters,
            ));
        };
        if values.len() > self.fields.len() {
            return Err(CompositionError::new(
                CompositionErrorKind::InvalidParameters,
            ));
        }
        for (name, value) in values {
            let name = ModelField::parse(name)
                .map_err(|_| CompositionError::new(CompositionErrorKind::InvalidParameters))?;
            let field = self
                .fields
                .get(&name)
                .ok_or_else(|| CompositionError::new(CompositionErrorKind::InvalidParameters))?;
            field
                .codec
                .validate(value, limits)
                .map_err(|_| CompositionError::new(CompositionErrorKind::InvalidParameters))?;
        }
        if self
            .fields
            .values()
            .any(|field| field.required && !values.contains_key(field.name.as_str()))
        {
            return Err(CompositionError::new(
                CompositionErrorKind::InvalidParameters,
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_and_digest(
        &self,
        value: &CanonicalValue,
        limits: &InputLimits,
    ) -> Result<ParameterValueDigest, CompositionError> {
        self.validate(value, limits)?;
        let encoded = to_canonical_bytes(value, limits)
            .map_err(|_| CompositionError::new(CompositionErrorKind::InvalidParameters))?;
        Ok(value_digest(self.digest(), &encoded))
    }

    pub(crate) fn snapshot_fields(&self) -> Result<Vec<FieldSpec>, SnapshotError> {
        self.fields
            .values()
            .map(ChildParameterField::snapshot_field)
            .collect()
    }
}

impl Default for ChildParameterSchema {
    fn default() -> Self {
        Self {
            version: 1,
            fields: BTreeMap::new(),
            digest: schema_digest(1, &BTreeMap::new()),
        }
    }
}

/// Untrusted-by-default declaration emitted by one server render.
#[derive(Clone)]
pub struct ChildDeclaration {
    key: ChildKey,
    component: ComponentName,
    parameters: CanonicalValue,
}

impl ChildDeclaration {
    /// Declares one child by stable key, registered component, and typed values.
    #[must_use]
    pub const fn new(key: ChildKey, component: ComponentName, parameters: CanonicalValue) -> Self {
        Self {
            key,
            component,
            parameters,
        }
    }
}

impl fmt::Debug for ChildDeclaration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildDeclaration")
            .field("key", &self.key)
            .field("component", &self.component)
            .field("parameters", &"<redacted>")
            .finish()
    }
}

/// Server-validated child ready for a fresh independent mount.
#[derive(Clone)]
pub struct PreparedChild {
    key: ChildKey,
    component: ComponentName,
    component_contract: ContentDigest,
    parameter_schema_version: u16,
    parameter_schema: ParameterSchemaDigest,
    parameter_value: ParameterValueDigest,
    params_changed: bool,
    lazy_complete: bool,
    parameters: CanonicalValue,
}

impl PreparedChild {
    /// Converts a completed independent mount into a future reconciliation handle.
    #[must_use]
    pub fn into_handle(self, instance_id: InstanceId) -> ChildHandle {
        ChildHandle {
            key: self.key,
            component: self.component,
            component_contract: self.component_contract,
            parameter_schema_version: self.parameter_schema_version,
            parameter_schema: self.parameter_schema,
            parameter_value: self.parameter_value,
            params_changed: self.params_changed,
            lazy_complete: self.lazy_complete,
            instance_id,
        }
    }

    /// Returns verified parameters for the independent child mount.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        &self.parameters
    }
}

impl fmt::Debug for PreparedChild {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedChild")
            .field("key", &self.key)
            .field("component", &self.component)
            .finish_non_exhaustive()
    }
}

/// Minimal independent child ownership carried between parent renders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildHandle {
    key: ChildKey,
    component: ComponentName,
    component_contract: ContentDigest,
    parameter_schema_version: u16,
    parameter_schema: ParameterSchemaDigest,
    parameter_value: ParameterValueDigest,
    params_changed: bool,
    lazy_complete: bool,
    instance_id: InstanceId,
}

impl ChildHandle {
    /// Returns the stable child key.
    #[must_use]
    pub const fn key(&self) -> &ChildKey {
        &self.key
    }

    /// Returns the independently authorized child instance.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the registered child component contract.
    #[must_use]
    pub const fn component_contract(&self) -> &ContentDigest {
        &self.component_contract
    }

    /// Returns the independently versioned child parameter schema.
    #[must_use]
    pub const fn parameter_schema_version(&self) -> u16 {
        self.parameter_schema_version
    }
}

/// Internal verified value awaiting Task 9 capability signing and child scheduling.
#[derive(Clone)]
pub struct PendingChildParameters {
    child: ChildHandle,
    parameter_schema: ParameterSchemaDigest,
    parameter_schema_version: u16,
    parameter_value: ParameterValueDigest,
    parameters: CanonicalValue,
}

impl PendingChildParameters {
    /// Returns the surviving independent child.
    #[must_use]
    pub const fn child(&self) -> &ChildHandle {
        &self.child
    }

    /// Returns server-validated parameters; this is not browser authority.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        &self.parameters
    }

    /// Returns the schema digest Task 9 binds into its signed capability.
    #[must_use]
    pub const fn parameter_schema(&self) -> &ParameterSchemaDigest {
        &self.parameter_schema
    }

    /// Returns the independently versioned child parameter schema.
    #[must_use]
    pub const fn parameter_schema_version(&self) -> u16 {
        self.parameter_schema_version
    }

    /// Returns the value digest Task 9 binds into its signed capability.
    #[must_use]
    pub const fn parameter_value(&self) -> &ParameterValueDigest {
        &self.parameter_value
    }
}

impl fmt::Debug for PendingChildParameters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingChildParameters")
            .field("child", &self.child)
            .field("parameters", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Complete parent-render classification for one child identity.
#[derive(Clone, Debug)]
pub enum ChildState {
    /// Identity, contract, schema, and parameters are unchanged.
    Unchanged(ChildHandle),
    /// A surviving child must receive a separately authorized parameter update.
    PendingParams(PendingChildParameters),
    /// A new child or contract/schema drift requires a fresh independent mount.
    Remount(PreparedChild),
    /// A prior child is absent and its child-local resources may retire.
    Removed(ChildHandle),
}

/// Bounded ancestor chain used to reject circular or runaway composition.
#[derive(Clone, Debug)]
pub struct CompositionAncestry {
    components: Vec<ComponentName>,
}

impl CompositionAncestry {
    /// Starts a composition chain at one registered parent component.
    #[must_use]
    pub fn root(component: ComponentName) -> Self {
        Self {
            components: vec![component],
        }
    }

    /// Adds one child only when it neither cycles nor exceeds the supplied depth.
    pub fn enter(
        &self,
        _key: ChildKey,
        component: ComponentName,
        max_depth: usize,
    ) -> Result<Self, CompositionError> {
        if self.components.len() >= max_depth {
            return Err(CompositionError::new(CompositionErrorKind::DepthExceeded));
        }
        if self.components.contains(&component) {
            return Err(CompositionError::new(
                CompositionErrorKind::CircularComposition,
            ));
        }
        let mut components = self.components.clone();
        components.push(component);
        Ok(Self { components })
    }
}

/// Per-render and per-tree limits below hard engine ceilings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompositionLimits {
    max_children: usize,
    max_depth: usize,
    max_parameter_bytes: usize,
    max_pending: usize,
    input: InputLimits,
}

impl CompositionLimits {
    /// Creates a non-zero bounded composition policy.
    pub fn new(
        max_children: usize,
        max_depth: usize,
        max_parameter_bytes: usize,
        max_pending: usize,
    ) -> Result<Self, CompositionError> {
        let valid = (1..=HARD_MAX_CHILDREN).contains(&max_children)
            && (1..=HARD_MAX_COMPOSITION_DEPTH).contains(&max_depth)
            && (1..=HARD_MAX_PARAMETER_BYTES).contains(&max_parameter_bytes)
            && max_pending <= max_children;
        if !valid {
            return Err(CompositionError::new(CompositionErrorKind::InvalidLimits));
        }
        let input = InputLimits::new(
            max_parameter_bytes,
            HARD_MAX_INPUT_DEPTH,
            HARD_MAX_ENTRIES,
            max_parameter_bytes.min(HARD_MAX_INPUT_STRING_BYTES),
        )
        .map_err(|_| CompositionError::new(CompositionErrorKind::InvalidLimits))?;
        Ok(Self {
            max_children,
            max_depth,
            max_parameter_bytes,
            max_pending,
            input,
        })
    }
}

/// Stateless deterministic reconciler for one parent render.
#[derive(Clone, Copy, Debug)]
pub struct CompositionPlanner {
    limits: CompositionLimits,
}

impl CompositionPlanner {
    /// Creates a planner from validated limits.
    #[must_use]
    pub const fn new(limits: CompositionLimits) -> Self {
        Self { limits }
    }

    /// Validates declarations and returns deterministic child-local transitions.
    pub fn reconcile(
        &self,
        registry: &ComponentRegistry,
        ancestry: &CompositionAncestry,
        previous: &[ChildHandle],
        declarations: Vec<ChildDeclaration>,
    ) -> Result<Vec<ChildState>, CompositionError> {
        if declarations.len() > self.limits.max_children
            || previous.len() > self.limits.max_children
        {
            return Err(CompositionError::new(CompositionErrorKind::TooManyChildren));
        }
        let mut keys = BTreeSet::new();
        if declarations
            .iter()
            .any(|declaration| !keys.insert(declaration.key.clone()))
        {
            return Err(CompositionError::new(CompositionErrorKind::DuplicateKey));
        }
        let mut previous_by_key = BTreeMap::new();
        for child in previous {
            if previous_by_key
                .insert(child.key.clone(), child.clone())
                .is_some()
            {
                return Err(CompositionError::new(CompositionErrorKind::DuplicateKey));
            }
        }

        let input_limits = self.limits.input;
        let mut states = Vec::with_capacity(declarations.len().saturating_add(previous.len()));
        let mut pending_count = 0usize;
        for declaration in declarations {
            ancestry.enter(
                declaration.key.clone(),
                declaration.component.clone(),
                self.limits.max_depth,
            )?;
            let descriptor = registry
                .resolve(&declaration.component)
                .map_err(|_| CompositionError::new(CompositionErrorKind::UnknownComponent))?;
            let schema = descriptor.parameter_schema();
            schema.validate(&declaration.parameters, &input_limits)?;
            let encoded = to_canonical_bytes(&declaration.parameters, &input_limits)
                .map_err(|_| CompositionError::new(CompositionErrorKind::InvalidParameters))?;
            if encoded.len() > self.limits.max_parameter_bytes {
                return Err(CompositionError::new(
                    CompositionErrorKind::ParametersTooLarge,
                ));
            }
            let parameter_value = value_digest(schema.digest(), &encoded);
            let prepared = PreparedChild {
                key: declaration.key.clone(),
                component: declaration.component,
                component_contract: descriptor.contract_digest().clone(),
                parameter_schema_version: schema.version(),
                parameter_schema: schema.digest().clone(),
                parameter_value: parameter_value.clone(),
                params_changed: descriptor.supports_params_changed(),
                lazy_complete: descriptor.supports_lazy_complete(),
                parameters: declaration.parameters,
            };
            let Some(existing) = previous_by_key.remove(&declaration.key) else {
                states.push(ChildState::Remount(prepared));
                continue;
            };
            if existing.component != prepared.component
                || existing.component_contract != prepared.component_contract
                || existing.parameter_schema_version != prepared.parameter_schema_version
                || existing.parameter_schema != prepared.parameter_schema
                || existing.params_changed != prepared.params_changed
                || existing.lazy_complete != prepared.lazy_complete
            {
                states.push(ChildState::Remount(prepared));
            } else if existing.parameter_value == parameter_value {
                states.push(ChildState::Unchanged(existing));
            } else {
                pending_count = pending_count.saturating_add(1);
                if pending_count > self.limits.max_pending {
                    return Err(CompositionError::new(CompositionErrorKind::TooManyPending));
                }
                states.push(ChildState::PendingParams(PendingChildParameters {
                    child: existing,
                    parameter_schema: prepared.parameter_schema,
                    parameter_schema_version: prepared.parameter_schema_version,
                    parameter_value: prepared.parameter_value,
                    parameters: prepared.parameters,
                }));
            }
        }
        states.extend(previous_by_key.into_values().map(ChildState::Removed));
        Ok(states)
    }
}

/// Child-local recovery directive for a rejected parameter or lazy operation.
#[derive(Clone, Debug)]
pub struct ChildFailureRecovery {
    child: ChildHandle,
}

impl ChildFailureRecovery {
    /// Targets recovery to one independently owned child.
    #[must_use]
    pub const fn for_child(child: ChildHandle) -> Self {
        Self { child }
    }

    /// Returns the child that must refresh or remount.
    #[must_use]
    pub const fn child(&self) -> &ChildHandle {
        &self.child
    }

    /// Child failure never requests rollback of an already accepted parent morph.
    #[must_use]
    pub const fn rolls_back_parent(&self) -> bool {
        false
    }
}

/// Closed redacted composition failure class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionErrorKind {
    /// A stable key was empty, too long, or outside the unreserved grammar.
    InvalidKey,
    /// A parameter schema version, field set, or codec was invalid.
    InvalidSchema,
    /// Planner limits were zero, inconsistent, or above engine ceilings.
    InvalidLimits,
    /// Two children used the same key in one ownership scope.
    DuplicateKey,
    /// A child component was absent from the immutable registry.
    UnknownComponent,
    /// Parameter names, required values, or registered codecs did not match.
    InvalidParameters,
    /// Canonical parameter bytes exceeded the configured bound.
    ParametersTooLarge,
    /// Child count exceeded the configured per-parent bound.
    TooManyChildren,
    /// Pending parameter updates exceeded their configured bound.
    TooManyPending,
    /// Composition exceeded the configured ancestor depth.
    DepthExceeded,
    /// A component recurred in its own active ancestor chain.
    CircularComposition,
}

/// Redacted composition error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CompositionError {
    kind: CompositionErrorKind,
}

impl CompositionError {
    const fn new(kind: CompositionErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure class.
    #[must_use]
    pub const fn kind(self) -> CompositionErrorKind {
        self.kind
    }
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            CompositionErrorKind::InvalidKey => "invalid_child_key",
            CompositionErrorKind::InvalidSchema => "invalid_child_parameter_schema",
            CompositionErrorKind::InvalidLimits => "invalid_composition_limits",
            CompositionErrorKind::DuplicateKey => "duplicate_child_key",
            CompositionErrorKind::UnknownComponent => "unknown_child_component",
            CompositionErrorKind::InvalidParameters => "invalid_child_parameters",
            CompositionErrorKind::ParametersTooLarge => "child_parameters_too_large",
            CompositionErrorKind::TooManyChildren => "too_many_children",
            CompositionErrorKind::TooManyPending => "too_many_pending_children",
            CompositionErrorKind::DepthExceeded => "composition_depth_exceeded",
            CompositionErrorKind::CircularComposition => "circular_composition",
        })
    }
}

impl fmt::Debug for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for CompositionError {}

fn schema_digest(
    version: u16,
    fields: &BTreeMap<ModelField, ChildParameterField>,
) -> ParameterSchemaDigest {
    let mut digest = Sha256::new();
    digest.update(PARAMETER_SCHEMA_DOMAIN);
    digest.update(version.to_be_bytes());
    for field in fields.values() {
        digest.update((field.name.as_str().len() as u64).to_be_bytes());
        digest.update(field.name.as_str().as_bytes());
        digest.update([u8::from(field.required)]);
        update_codec_digest(&mut digest, &field.codec);
    }
    ParameterSchemaDigest(digest.finalize().into())
}

fn update_codec_digest(digest: &mut Sha256, codec: &ModelCodec) {
    match codec {
        ModelCodec::String => digest.update([0]),
        ModelCodec::Boolean => digest.update([1]),
        ModelCodec::I64 => digest.update([2]),
        ModelCodec::U64 => digest.update([3]),
        ModelCodec::F64 => digest.update([4]),
        ModelCodec::Json => digest.update([5]),
        ModelCodec::Date => digest.update([6]),
        ModelCodec::DateTime => digest.update([7]),
        ModelCodec::Uuid => digest.update([8]),
        ModelCodec::Enumeration(values) => {
            digest.update([9]);
            digest.update((values.len() as u64).to_be_bytes());
            for value in values {
                digest.update((value.len() as u64).to_be_bytes());
                digest.update(value.as_bytes());
            }
        }
        ModelCodec::List(inner) => {
            digest.update([10]);
            update_codec_digest(digest, inner);
        }
        ModelCodec::Map(inner) => {
            digest.update([11]);
            update_codec_digest(digest, inner);
        }
    }
}

fn value_digest(schema: &ParameterSchemaDigest, encoded: &[u8]) -> ParameterValueDigest {
    let mut digest = Sha256::new();
    digest.update(PARAMETER_VALUE_DOMAIN);
    digest.update(schema.as_bytes());
    digest.update((encoded.len() as u64).to_be_bytes());
    digest.update(encoded);
    ParameterValueDigest(digest.finalize().into())
}
