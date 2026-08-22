//! Versioned seed and instanced snapshot bodies and trusted expectations.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use super::state::{SnapshotSchemaSet, StateExposure};
use super::{SnapshotError, SnapshotErrorKind, SnapshotLimits};
use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::{
    BuildId, ComponentName, ContentDigest, DurationMillis, Generation, InstanceId, IslandSlot,
    KeyId, Revision, RouteIdentity, ScopeFingerprint, UnixMillis,
};

/// Snapshot schema version implemented by iteration 001.
pub const SNAPSHOT_SCHEMA_V1: u16 = 1;

/// Distinct signed body form.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotForm {
    /// Reusable public body without instance or identity authority.
    Seed,
    /// Scoped body carrying instance identity and revision.
    Instance,
}

/// Stable component/build contract independently versioning state, memo, and mount data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComponentContract {
    name: ComponentName,
    contract_digest: ContentDigest,
    state_schema_version: u16,
    memo_schema_version: u16,
    mount_schema_version: u16,
}

impl ComponentContract {
    /// Creates a component contract with non-zero independent schema versions.
    pub fn new(
        name: ComponentName,
        contract_digest: ContentDigest,
        state_schema_version: u16,
        memo_schema_version: u16,
        mount_schema_version: u16,
    ) -> Result<Self, SnapshotError> {
        if state_schema_version == 0 || memo_schema_version == 0 || mount_schema_version == 0 {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidSchema));
        }
        Ok(Self {
            name,
            contract_digest,
            state_schema_version,
            memo_schema_version,
            mount_schema_version,
        })
    }

    /// Returns the registered component name.
    #[must_use]
    pub const fn name(&self) -> &ComponentName {
        &self.name
    }

    /// Returns the canonical generated component contract digest.
    #[must_use]
    pub const fn contract_digest(&self) -> &ContentDigest {
        &self.contract_digest
    }

    pub(crate) fn matches_schemas(&self, schemas: &SnapshotSchemaSet) -> bool {
        self.state_schema_version == schemas.state().version()
            && self.memo_schema_version == schemas.memo().version()
            && self.mount_schema_version == schemas.mount().version()
    }

    pub(crate) fn same_name(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

/// Advisory dependency generation carried as promotion memo, not authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GenerationMemo {
    dependency: ContentDigest,
    generation: Generation,
}

impl GenerationMemo {
    /// Creates one digest-keyed advisory generation.
    #[must_use]
    pub const fn new(dependency: ContentDigest, generation: Generation) -> Self {
        Self {
            dependency,
            generation,
        }
    }
}

/// Trusted fields required to construct a reusable public seed.
#[derive(Clone)]
pub struct SeedFieldsV1 {
    /// Registered component contract.
    pub component: ComponentContract,
    /// Application build that produced the public markup.
    pub build_id: BuildId,
    /// Canonical route digest.
    pub route: RouteIdentity,
    /// Stable island slot within the route.
    pub slot: IslandSlot,
    /// Signing key ID embedded inside the signed body.
    pub key_id: KeyId,
    /// Issuance time in Unix milliseconds.
    pub issued_at: UnixMillis,
    /// Maximum reusable age in milliseconds.
    pub max_age_ms: u64,
    /// Public mount parameters.
    pub mount: CanonicalValue,
    /// Public component state.
    pub state: CanonicalValue,
    /// Public lifecycle memo.
    pub memo: CanonicalValue,
    /// Optional advisory generation observations.
    pub advisory_generations: Vec<GenerationMemo>,
    /// Whether authoritative component refresh precedes the first action.
    pub refresh_on_promote: bool,
    /// Explicit namespaced backward-compatible additions.
    pub extensions: BTreeMap<String, CanonicalValue>,
}

impl fmt::Debug for SeedFieldsV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SeedFieldsV1:redacted>")
    }
}

/// Signed reusable public snapshot body version 1.
#[derive(Clone, Serialize)]
pub struct SeedBodyV1 {
    form: SnapshotForm,
    schema_version: u16,
    component: ComponentContract,
    build_id: BuildId,
    route: RouteIdentity,
    slot: IslandSlot,
    key_id: KeyId,
    issued_at: UnixMillis,
    max_age_ms: DurationMillis,
    mount: CanonicalValue,
    state: CanonicalValue,
    memo: CanonicalValue,
    advisory_generations: Vec<GenerationMemo>,
    refresh_on_promote: bool,
    extensions: BTreeMap<String, CanonicalValue>,
}

impl SeedBodyV1 {
    /// Validates public-state eligibility, schemas, generations, extensions, and age policy.
    pub fn new(
        fields: SeedFieldsV1,
        schemas: &SnapshotSchemaSet,
        limits: &SnapshotLimits,
    ) -> Result<Self, SnapshotError> {
        if !fields.component.matches_schemas(schemas) {
            return Err(SnapshotError::new(SnapshotErrorKind::CompatibilityMismatch));
        }
        if fields.max_age_ms == 0 || fields.max_age_ms > limits.max_seed_age_ms() {
            return Err(SnapshotError::new(SnapshotErrorKind::ValidityTooLong));
        }
        if fields.advisory_generations.len() > limits.max_generations() {
            return Err(SnapshotError::new(SnapshotErrorKind::TooManyGenerations));
        }
        for (index, generation) in fields.advisory_generations.iter().enumerate() {
            if fields.advisory_generations[index + 1..]
                .iter()
                .any(|candidate| candidate.dependency == generation.dependency)
            {
                return Err(SnapshotError::new(SnapshotErrorKind::InvalidSchema));
            }
        }
        schemas
            .mount()
            .validate(&fields.mount, StateExposure::PublicSeed)?;
        schemas
            .state()
            .validate(&fields.state, StateExposure::PublicSeed)?;
        schemas
            .memo()
            .validate(&fields.memo, StateExposure::PublicSeed)?;
        validate_canonical_fields([&fields.mount, &fields.state, &fields.memo], limits)?;
        validate_extensions(&fields.extensions, limits)?;

        Ok(Self {
            form: SnapshotForm::Seed,
            schema_version: SNAPSHOT_SCHEMA_V1,
            component: fields.component,
            build_id: fields.build_id,
            route: fields.route,
            slot: fields.slot,
            key_id: fields.key_id,
            issued_at: fields.issued_at,
            max_age_ms: DurationMillis::new(fields.max_age_ms),
            mount: fields.mount,
            state: fields.state,
            memo: fields.memo,
            advisory_generations: fields.advisory_generations,
            refresh_on_promote: fields.refresh_on_promote,
            extensions: fields.extensions,
        })
    }

    /// Returns the bound component contract.
    #[must_use]
    pub const fn component(&self) -> &ComponentContract {
        &self.component
    }

    /// Returns the canonical route digest.
    #[must_use]
    pub const fn route(&self) -> &RouteIdentity {
        &self.route
    }

    /// Returns the bound island slot.
    #[must_use]
    pub const fn slot(&self) -> &IslandSlot {
        &self.slot
    }

    /// Returns the seed issuance time.
    #[must_use]
    pub const fn issued_at(&self) -> UnixMillis {
        self.issued_at
    }

    /// Returns the bounded reusable age in milliseconds.
    #[must_use]
    pub const fn max_age_ms(&self) -> u64 {
        self.max_age_ms.get()
    }

    /// Returns whether promotion requires authoritative refresh first.
    #[must_use]
    pub const fn refresh_on_promote(&self) -> bool {
        self.refresh_on_promote
    }

    /// Returns advisory generations that never become promotion authority.
    #[must_use]
    pub fn advisory_generations(&self) -> &[GenerationMemo] {
        &self.advisory_generations
    }

    pub(crate) const fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    pub(crate) const fn build_id(&self) -> &BuildId {
        &self.build_id
    }

    pub(crate) const fn state(&self) -> &CanonicalValue {
        &self.state
    }

    pub(crate) const fn memo(&self) -> &CanonicalValue {
        &self.memo
    }

    pub(crate) const fn mount(&self) -> &CanonicalValue {
        &self.mount
    }

    pub(crate) const fn extensions(&self) -> &BTreeMap<String, CanonicalValue> {
        &self.extensions
    }
}

impl fmt::Debug for SeedBodyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SeedBodyV1:redacted>")
    }
}

/// Trusted fields required to construct a scoped instanced snapshot.
#[derive(Clone)]
pub struct InstanceFieldsV1 {
    /// Registered component contract.
    pub component: ComponentContract,
    /// Application build that produced the instance.
    pub build_id: BuildId,
    /// Canonical route digest.
    pub route: RouteIdentity,
    /// Stable island slot within the route.
    pub slot: IslandSlot,
    /// Signing key ID embedded inside the signed body.
    pub key_id: KeyId,
    /// Trusted scope fingerprint from the framework adapter.
    pub scope: ScopeFingerprint,
    /// Server-assigned instance identity.
    pub instance_id: InstanceId,
    /// Monotonic island revision.
    pub revision: Revision,
    /// Issuance time in Unix milliseconds.
    pub issued_at: UnixMillis,
    /// Exclusive validity deadline.
    pub expires_at: UnixMillis,
    /// Scoped component state.
    pub state: CanonicalValue,
    /// Scoped lifecycle memo.
    pub memo: CanonicalValue,
    /// Explicit namespaced backward-compatible additions.
    pub extensions: BTreeMap<String, CanonicalValue>,
}

impl fmt::Debug for InstanceFieldsV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<InstanceFieldsV1:redacted>")
    }
}

/// Signed scoped instanced snapshot body version 1.
#[derive(Clone, Serialize)]
pub struct InstanceBodyV1 {
    form: SnapshotForm,
    schema_version: u16,
    component: ComponentContract,
    build_id: BuildId,
    route: RouteIdentity,
    slot: IslandSlot,
    key_id: KeyId,
    scope: ScopeFingerprint,
    instance_id: InstanceId,
    revision: Revision,
    issued_at: UnixMillis,
    expires_at: UnixMillis,
    state: CanonicalValue,
    memo: CanonicalValue,
    extensions: BTreeMap<String, CanonicalValue>,
}

impl InstanceBodyV1 {
    /// Validates component schemas, scoped state, extensions, and lifetime policy.
    pub fn new(
        fields: InstanceFieldsV1,
        schemas: &SnapshotSchemaSet,
        limits: &SnapshotLimits,
    ) -> Result<Self, SnapshotError> {
        if !fields.component.matches_schemas(schemas) {
            return Err(SnapshotError::new(SnapshotErrorKind::CompatibilityMismatch));
        }
        let lifetime = fields
            .expires_at
            .get()
            .checked_sub(fields.issued_at.get())
            .ok_or_else(|| SnapshotError::new(SnapshotErrorKind::ValidityTooLong))?;
        if lifetime == 0 || lifetime > limits.max_instance_lifetime_ms() {
            return Err(SnapshotError::new(SnapshotErrorKind::ValidityTooLong));
        }
        schemas
            .state()
            .validate(&fields.state, StateExposure::Instanced)?;
        schemas
            .memo()
            .validate(&fields.memo, StateExposure::Instanced)?;
        validate_canonical_fields([&fields.state, &fields.memo], limits)?;
        validate_extensions(&fields.extensions, limits)?;

        Ok(Self {
            form: SnapshotForm::Instance,
            schema_version: SNAPSHOT_SCHEMA_V1,
            component: fields.component,
            build_id: fields.build_id,
            route: fields.route,
            slot: fields.slot,
            key_id: fields.key_id,
            scope: fields.scope,
            instance_id: fields.instance_id,
            revision: fields.revision,
            issued_at: fields.issued_at,
            expires_at: fields.expires_at,
            state: fields.state,
            memo: fields.memo,
            extensions: fields.extensions,
        })
    }

    /// Returns the server-assigned instance identity.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the monotonic snapshot revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the exclusive validity deadline.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    pub(crate) const fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    pub(crate) const fn component(&self) -> &ComponentContract {
        &self.component
    }

    pub(crate) const fn build_id(&self) -> &BuildId {
        &self.build_id
    }

    pub(crate) const fn route(&self) -> &RouteIdentity {
        &self.route
    }

    pub(crate) const fn slot(&self) -> &IslandSlot {
        &self.slot
    }

    pub(crate) const fn scope(&self) -> &ScopeFingerprint {
        &self.scope
    }

    pub(crate) const fn issued_at(&self) -> UnixMillis {
        self.issued_at
    }

    pub(crate) const fn state(&self) -> &CanonicalValue {
        &self.state
    }

    pub(crate) const fn memo(&self) -> &CanonicalValue {
        &self.memo
    }
}

impl fmt::Debug for InstanceBodyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<InstanceBodyV1:redacted>")
    }
}

/// Trusted compatibility and binding expectations for seed verification.
#[derive(Clone, Debug)]
pub struct ExpectedSeedV1 {
    pub(crate) component: ComponentContract,
    pub(crate) build_id: BuildId,
    pub(crate) route: RouteIdentity,
    pub(crate) slot: IslandSlot,
    pub(crate) schemas: SnapshotSchemaSet,
}

impl ExpectedSeedV1 {
    /// Creates trusted seed expectations supplied by framework metadata.
    #[must_use]
    pub const fn new(
        component: ComponentContract,
        build_id: BuildId,
        route: RouteIdentity,
        slot: IslandSlot,
        schemas: SnapshotSchemaSet,
    ) -> Self {
        Self {
            component,
            build_id,
            route,
            slot,
            schemas,
        }
    }

    pub(crate) const fn component(&self) -> &ComponentContract {
        &self.component
    }

    pub(crate) const fn build_id(&self) -> &BuildId {
        &self.build_id
    }

    pub(crate) const fn route(&self) -> &RouteIdentity {
        &self.route
    }

    pub(crate) const fn slot(&self) -> &IslandSlot {
        &self.slot
    }

    pub(crate) const fn schemas(&self) -> &SnapshotSchemaSet {
        &self.schemas
    }
}

/// Trusted compatibility, binding, and scope expectations for instance verification.
#[derive(Clone, Debug)]
pub struct ExpectedInstanceV1 {
    pub(crate) component: ComponentContract,
    pub(crate) build_id: BuildId,
    pub(crate) route: RouteIdentity,
    pub(crate) slot: IslandSlot,
    pub(crate) scope: ScopeFingerprint,
    pub(crate) schemas: SnapshotSchemaSet,
}

impl ExpectedInstanceV1 {
    /// Creates trusted instance expectations supplied by framework metadata.
    #[must_use]
    pub const fn new(
        component: ComponentContract,
        build_id: BuildId,
        route: RouteIdentity,
        slot: IslandSlot,
        scope: ScopeFingerprint,
        schemas: SnapshotSchemaSet,
    ) -> Self {
        Self {
            component,
            build_id,
            route,
            slot,
            scope,
            schemas,
        }
    }
}

fn validate_canonical_fields<'value>(
    fields: impl IntoIterator<Item = &'value CanonicalValue>,
    limits: &SnapshotLimits,
) -> Result<(), SnapshotError> {
    for field in fields {
        to_canonical_bytes(field, limits.input()).map_err(|error| {
            use crate::canonical::CanonicalErrorKind;
            SnapshotError::new(match error.kind() {
                CanonicalErrorKind::TooLarge => SnapshotErrorKind::InputTooLarge,
                CanonicalErrorKind::TooDeep => SnapshotErrorKind::InputTooDeep,
                CanonicalErrorKind::TooManyEntries => SnapshotErrorKind::TooManyEntries,
                _ => SnapshotErrorKind::InvalidStateShape,
            })
        })?;
    }
    Ok(())
}

fn validate_extensions(
    extensions: &BTreeMap<String, CanonicalValue>,
    limits: &SnapshotLimits,
) -> Result<(), SnapshotError> {
    if extensions.len() > limits.max_extensions() {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
    }
    for (name, value) in extensions {
        let valid = name.starts_with("x_")
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
        if !valid {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        }
        validate_canonical_fields([value], limits)?;
    }
    Ok(())
}
