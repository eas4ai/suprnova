//! Deterministic signing and verify-before-hydration snapshot codecs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    ComponentContract, ExpectedInstanceV1, ExpectedSeedV1, GenerationMemo, InstanceBodyV1,
    InstanceFieldsV1, SNAPSHOT_SCHEMA_V1, SeedBodyV1, SeedFieldsV1, SnapshotError,
    SnapshotErrorKind, SnapshotForm, SnapshotLimits, VerifiedInstanceV1, VerifiedSeedV1,
};
use crate::canonical::{
    CanonicalError, CanonicalErrorKind, CanonicalValue, parse_canonical_value, to_canonical_bytes,
};
use crate::crypto::{SnapshotKeyRing, SnapshotPurpose, SnapshotSignature};
use crate::identity::{
    BuildId, ComponentName, ContentDigest, DurationMillis, Generation, InstanceId, IslandSlot,
    KeyId, Revision, RouteIdentity, ScopeFingerprint, UnixMillis,
};

#[derive(Serialize)]
struct EnvelopeRef<'body> {
    body: &'body CanonicalValue,
    signature: &'body SnapshotSignature,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentWire {
    name: String,
    contract_digest: String,
    state_schema_version: u16,
    memo_schema_version: u16,
    mount_schema_version: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationWire {
    dependency: String,
    generation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedWire {
    form: String,
    schema_version: u16,
    component: ComponentWire,
    build_id: String,
    route: String,
    slot: String,
    key_id: String,
    issued_at: String,
    max_age_ms: String,
    mount: serde_json::Value,
    state: serde_json::Value,
    memo: serde_json::Value,
    advisory_generations: Vec<GenerationWire>,
    refresh_on_promote: bool,
    extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceWire {
    form: String,
    schema_version: u16,
    component: ComponentWire,
    build_id: String,
    route: String,
    slot: String,
    key_id: String,
    scope: String,
    instance_id: String,
    revision: String,
    issued_at: String,
    expires_at: String,
    state: serde_json::Value,
    memo: serde_json::Value,
    extensions: BTreeMap<String, serde_json::Value>,
}

/// Bounded untrusted claims used only to select registered verification expectations.
pub(crate) struct UnverifiedInstanceAuthorityV1 {
    component: ComponentContract,
    build_id: BuildId,
    route: RouteIdentity,
    slot: IslandSlot,
    scope: ScopeFingerprint,
}

impl UnverifiedInstanceAuthorityV1 {
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
}

impl SeedBodyV1 {
    /// Produces a deterministic signed seed envelope.
    pub fn sign(
        &self,
        keys: &SnapshotKeyRing,
        now: UnixMillis,
        limits: &SnapshotLimits,
    ) -> Result<Vec<u8>, SnapshotError> {
        if self.key_id() != keys.active_key_id() {
            return Err(SnapshotError::new(SnapshotErrorKind::SigningKeyMismatch));
        }
        validate_seed_time(self, now, limits)?;
        sign_body(self, SnapshotPurpose::SeedV1, keys, now, limits)
    }
}

impl InstanceBodyV1 {
    /// Produces a deterministic signed instanced envelope.
    pub fn sign(
        &self,
        keys: &SnapshotKeyRing,
        now: UnixMillis,
        limits: &SnapshotLimits,
    ) -> Result<Vec<u8>, SnapshotError> {
        if self.key_id() != keys.active_key_id() {
            return Err(SnapshotError::new(SnapshotErrorKind::SigningKeyMismatch));
        }
        validate_instance_time(self, now, limits)?;
        sign_body(self, SnapshotPurpose::InstanceV1, keys, now, limits)
    }
}

fn sign_body<T: Serialize>(
    body: &T,
    purpose: SnapshotPurpose,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &SnapshotLimits,
) -> Result<Vec<u8>, SnapshotError> {
    let body = canonical_from_serializable(body, limits)?;
    let canonical_body = to_canonical_bytes(&body, limits.input()).map_err(map_canonical)?;
    let signed = keys
        .sign(purpose, &canonical_body, now)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::SignatureInvalid))?;
    canonical_from_serializable(
        &EnvelopeRef {
            body: &body,
            signature: signed.signature(),
        },
        limits,
    )
    .and_then(|envelope| to_canonical_bytes(&envelope, limits.input()).map_err(map_canonical))
}

/// Verifies a seed envelope before constructing a hydration capability.
pub fn verify_seed(
    encoded: &[u8],
    expected: &ExpectedSeedV1,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &SnapshotLimits,
) -> Result<VerifiedSeedV1, SnapshotError> {
    let body_value = verify_envelope(
        encoded,
        SnapshotForm::Seed,
        SnapshotPurpose::SeedV1,
        keys,
        now,
        limits,
    )?;
    let wire: SeedWire = deserialize_body(&body_value)?;
    if wire.form != "seed" {
        return Err(SnapshotError::new(SnapshotErrorKind::WrongForm));
    }
    if wire.schema_version != SNAPSHOT_SCHEMA_V1 {
        return Err(SnapshotError::new(SnapshotErrorKind::UnsupportedSchema));
    }
    let body = SeedBodyV1::new(seed_fields_from_wire(wire)?, &expected.schemas, limits)?;
    validate_seed_expectations(&body, expected)?;
    validate_seed_time(&body, now, limits)?;
    Ok(VerifiedSeedV1::new(body))
}

/// Verifies an instanced envelope before constructing a hydration capability.
pub fn verify_instance(
    encoded: &[u8],
    expected: &ExpectedInstanceV1,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &SnapshotLimits,
) -> Result<VerifiedInstanceV1, SnapshotError> {
    let body_value = verify_envelope(
        encoded,
        SnapshotForm::Instance,
        SnapshotPurpose::InstanceV1,
        keys,
        now,
        limits,
    )?;
    let wire: InstanceWire = deserialize_body(&body_value)?;
    if wire.form != "instance" {
        return Err(SnapshotError::new(SnapshotErrorKind::WrongForm));
    }
    if wire.schema_version != SNAPSHOT_SCHEMA_V1 {
        return Err(SnapshotError::new(SnapshotErrorKind::UnsupportedSchema));
    }
    let body = InstanceBodyV1::new(instance_fields_from_wire(wire)?, &expected.schemas, limits)?;
    validate_instance_expectations(&body, expected)?;
    validate_instance_time(&body, now, limits)?;
    Ok(VerifiedInstanceV1::new(body))
}

/// Parses bounded instanced claims without granting signature or hydration authority.
pub(crate) fn inspect_instance_authority(
    encoded: &[u8],
    limits: &SnapshotLimits,
) -> Result<UnverifiedInstanceAuthorityV1, SnapshotError> {
    let envelope = parse_canonical_value(encoded, limits.input()).map_err(map_canonical)?;
    let CanonicalValue::Object(fields) = envelope else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    };
    if fields.len() != 2 || !fields.contains_key("body") || !fields.contains_key("signature") {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    }
    let body = fields
        .get("body")
        .ok_or_else(|| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?;
    let wire: InstanceWire = deserialize_body(body)?;
    if wire.form != "instance" {
        return Err(SnapshotError::new(SnapshotErrorKind::WrongForm));
    }
    if wire.schema_version != SNAPSHOT_SCHEMA_V1 {
        return Err(SnapshotError::new(SnapshotErrorKind::UnsupportedSchema));
    }
    let fields = instance_fields_from_wire(wire)?;
    Ok(UnverifiedInstanceAuthorityV1 {
        component: fields.component,
        build_id: fields.build_id,
        route: fields.route,
        slot: fields.slot,
        scope: fields.scope,
    })
}

fn verify_envelope(
    encoded: &[u8],
    expected_form: SnapshotForm,
    purpose: SnapshotPurpose,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &SnapshotLimits,
) -> Result<CanonicalValue, SnapshotError> {
    let envelope = parse_canonical_value(encoded, limits.input()).map_err(map_canonical)?;
    let CanonicalValue::Object(fields) = envelope else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    };
    if fields.len() != 2 || !fields.contains_key("body") || !fields.contains_key("signature") {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    }
    let body = fields
        .get("body")
        .ok_or_else(|| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?;
    let CanonicalValue::Object(body_fields) = body else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    };
    let form = string_field(body_fields, "form")?;
    if form
        != match expected_form {
            SnapshotForm::Seed => "seed",
            SnapshotForm::Instance => "instance",
        }
    {
        return Err(SnapshotError::new(SnapshotErrorKind::WrongForm));
    }
    if integer_field(body_fields, "schema_version")? != u64::from(SNAPSHOT_SCHEMA_V1) {
        return Err(SnapshotError::new(SnapshotErrorKind::UnsupportedSchema));
    }
    let key_id = KeyId::parse(string_field(body_fields, "key_id")?)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?;
    let CanonicalValue::String(signature) = fields
        .get("signature")
        .ok_or_else(|| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?
    else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    };
    let signature = SnapshotSignature::parse(signature)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::SignatureInvalid))?;
    let canonical_body = to_canonical_bytes(body, limits.input()).map_err(map_canonical)?;
    keys.verify(&key_id, purpose, &canonical_body, &signature, now)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::SignatureInvalid))?;
    Ok(body.clone())
}

fn string_field<'value>(
    fields: &'value BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<&'value str, SnapshotError> {
    let Some(CanonicalValue::String(value)) = fields.get(name) else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    };
    Ok(value)
}

fn integer_field(
    fields: &BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<u64, SnapshotError> {
    let Some(CanonicalValue::Number(value)) = fields.get(name) else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    };
    let value = value.get();
    if value < 0.0 || value.fract() != 0.0 {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidEnvelope));
    }
    Ok(value as u64)
}

fn canonical_from_serializable<T: Serialize>(
    value: &T,
    limits: &SnapshotLimits,
) -> Result<CanonicalValue, SnapshotError> {
    let serde_value = serde_json::to_value(value)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?;
    let canonical = CanonicalValue::from_serde_value(serde_value).map_err(map_canonical)?;
    to_canonical_bytes(&canonical, limits.input()).map_err(map_canonical)?;
    Ok(canonical)
}

fn deserialize_body<T: for<'de> Deserialize<'de>>(
    value: &CanonicalValue,
) -> Result<T, SnapshotError> {
    let serde_value = value
        .to_serde_value()
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?;
    serde_json::from_value(serde_value)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))
}

fn component_from_wire(wire: ComponentWire) -> Result<ComponentContract, SnapshotError> {
    ComponentContract::new(
        ComponentName::parse(&wire.name)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        ContentDigest::parse(&wire.contract_digest)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        wire.state_schema_version,
        wire.memo_schema_version,
        wire.mount_schema_version,
    )
}

fn canonical_from_wire(value: serde_json::Value) -> Result<CanonicalValue, SnapshotError> {
    CanonicalValue::from_serde_value(value)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))
}

fn extensions_from_wire(
    values: BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, CanonicalValue>, SnapshotError> {
    values
        .into_iter()
        .map(|(key, value)| canonical_from_wire(value).map(|value| (key, value)))
        .collect()
}

fn seed_fields_from_wire(wire: SeedWire) -> Result<SeedFieldsV1, SnapshotError> {
    let generations = wire
        .advisory_generations
        .into_iter()
        .map(|generation| {
            Ok(GenerationMemo::new(
                ContentDigest::parse(&generation.dependency)
                    .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
                Generation::parse(&generation.generation)
                    .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
            ))
        })
        .collect::<Result<Vec<_>, SnapshotError>>()?;
    Ok(SeedFieldsV1 {
        component: component_from_wire(wire.component)?,
        build_id: BuildId::parse(&wire.build_id)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        route: RouteIdentity::parse(&wire.route)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        slot: IslandSlot::parse(&wire.slot)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        key_id: KeyId::parse(&wire.key_id)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        issued_at: UnixMillis::parse(&wire.issued_at)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        max_age_ms: DurationMillis::parse(&wire.max_age_ms)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?
            .get(),
        mount: canonical_from_wire(wire.mount)?,
        state: canonical_from_wire(wire.state)?,
        memo: canonical_from_wire(wire.memo)?,
        advisory_generations: generations,
        refresh_on_promote: wire.refresh_on_promote,
        extensions: extensions_from_wire(wire.extensions)?,
    })
}

fn instance_fields_from_wire(wire: InstanceWire) -> Result<InstanceFieldsV1, SnapshotError> {
    Ok(InstanceFieldsV1 {
        component: component_from_wire(wire.component)?,
        build_id: BuildId::parse(&wire.build_id)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        route: RouteIdentity::parse(&wire.route)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        slot: IslandSlot::parse(&wire.slot)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        key_id: KeyId::parse(&wire.key_id)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        scope: ScopeFingerprint::parse(&wire.scope)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        instance_id: InstanceId::parse(&wire.instance_id)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        revision: Revision::parse(&wire.revision)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        issued_at: UnixMillis::parse(&wire.issued_at)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        expires_at: UnixMillis::parse(&wire.expires_at)
            .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidEnvelope))?,
        state: canonical_from_wire(wire.state)?,
        memo: canonical_from_wire(wire.memo)?,
        extensions: extensions_from_wire(wire.extensions)?,
    })
}

fn validate_seed_expectations(
    body: &SeedBodyV1,
    expected: &ExpectedSeedV1,
) -> Result<(), SnapshotError> {
    if !body.component().same_name(&expected.component)
        || body.route() != &expected.route
        || body.slot() != &expected.slot
    {
        return Err(SnapshotError::new(SnapshotErrorKind::BindingMismatch));
    }
    if body.component() != &expected.component || body.build_id() != &expected.build_id {
        return Err(SnapshotError::new(SnapshotErrorKind::CompatibilityMismatch));
    }
    Ok(())
}

fn validate_instance_expectations(
    body: &InstanceBodyV1,
    expected: &ExpectedInstanceV1,
) -> Result<(), SnapshotError> {
    if !body.component().same_name(&expected.component)
        || body.route() != &expected.route
        || body.slot() != &expected.slot
        || body.scope() != &expected.scope
    {
        return Err(SnapshotError::new(SnapshotErrorKind::BindingMismatch));
    }
    if body.component() != &expected.component || body.build_id() != &expected.build_id {
        return Err(SnapshotError::new(SnapshotErrorKind::CompatibilityMismatch));
    }
    Ok(())
}

fn validate_seed_time(
    body: &SeedBodyV1,
    now: UnixMillis,
    limits: &SnapshotLimits,
) -> Result<(), SnapshotError> {
    let latest_acceptable_issue = now.get().saturating_add(limits.max_clock_skew_ms());
    if body.issued_at().get() > latest_acceptable_issue {
        return Err(SnapshotError::new(SnapshotErrorKind::IssuedInFuture));
    }
    let deadline = body
        .issued_at()
        .get()
        .saturating_add(body.max_age_ms())
        .saturating_add(limits.max_clock_skew_ms());
    if now.get() > deadline {
        return Err(SnapshotError::new(SnapshotErrorKind::Expired));
    }
    Ok(())
}

fn validate_instance_time(
    body: &InstanceBodyV1,
    now: UnixMillis,
    limits: &SnapshotLimits,
) -> Result<(), SnapshotError> {
    let latest_acceptable_issue = now.get().saturating_add(limits.max_clock_skew_ms());
    if body.issued_at().get() > latest_acceptable_issue {
        return Err(SnapshotError::new(SnapshotErrorKind::IssuedInFuture));
    }
    if now.get()
        > body
            .expires_at()
            .get()
            .saturating_add(limits.max_clock_skew_ms())
    {
        return Err(SnapshotError::new(SnapshotErrorKind::Expired));
    }
    Ok(())
}

fn map_canonical(error: CanonicalError) -> SnapshotError {
    let kind = match error.kind() {
        CanonicalErrorKind::TooLarge => SnapshotErrorKind::InputTooLarge,
        CanonicalErrorKind::TooDeep => SnapshotErrorKind::InputTooDeep,
        CanonicalErrorKind::TooManyEntries => SnapshotErrorKind::TooManyEntries,
        CanonicalErrorKind::DuplicateKey => SnapshotErrorKind::DuplicateField,
        _ => SnapshotErrorKind::InvalidEnvelope,
    };
    SnapshotError::new(kind)
}
