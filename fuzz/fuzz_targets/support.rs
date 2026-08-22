#![allow(
    dead_code,
    reason = "each fuzz binary uses one half of this shared deterministic setup"
)]

use std::sync::OnceLock;

use suprnova_live::child::{ChildParameterLimits, ExpectedChildParametersV1};
use suprnova_live::component::composition::{
    ChildKey, ChildParameterField, ChildParameterSchema,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, InstanceId, IslandSlot, KeyId, ModelField, Revision,
    RouteIdentity, ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, ExpectedSeedV1, SnapshotLimits, SnapshotSchemaSet,
};
use suprnova_live::state::ModelCodec;

pub(crate) struct SnapshotSetup {
    pub(crate) keys: SnapshotKeyRing,
    pub(crate) seed: ExpectedSeedV1,
    pub(crate) instance: ExpectedInstanceV1,
    pub(crate) limits: SnapshotLimits,
}

pub(crate) struct ChildSetup {
    pub(crate) expected: ExpectedChildParametersV1,
    pub(crate) limits: ChildParameterLimits,
}

pub(crate) fn protocol_limits() -> Option<ProtocolLimits> {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(2_048, 8, 128, 1_024).ok()?,
        max_snapshot_bytes: 1_024,
        max_html_bytes: 1_024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 8,
        max_validation_entries: 8,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .ok()
}

pub(crate) fn snapshot_setup() -> Option<&'static SnapshotSetup> {
    static SETUP: OnceLock<Option<SnapshotSetup>> = OnceLock::new();
    SETUP.get_or_init(build_snapshot_setup).as_ref()
}

pub(crate) fn child_setup() -> Option<&'static ChildSetup> {
    static SETUP: OnceLock<Option<ChildSetup>> = OnceLock::new();
    SETUP.get_or_init(build_child_setup).as_ref()
}

fn build_child_setup() -> Option<ChildSetup> {
    let parameter_schema = ChildParameterSchema::new(
        1,
        vec![ChildParameterField::new(
            ModelField::parse("query").ok()?,
            ModelCodec::String,
            true,
        )],
    )
    .ok()?;
    let expected = ExpectedChildParametersV1::new(
        ScopeFingerprint::from_bytes(&[0x30; 32]).ok()?,
        InstanceId::from_bytes(&[0x40; 16]).ok()?,
        Revision::new(1),
        ChildKey::parse("results").ok()?,
        ContentDigest::from_bytes(&[0x50; 32]).ok()?,
        parameter_schema,
    );
    let limits = ChildParameterLimits::new(
        InputLimits::new(2_048, 8, 128, 512).ok()?,
        50,
        10_000,
    )
    .ok()?;
    Some(ChildSetup { expected, limits })
}

fn build_snapshot_setup() -> Option<SnapshotSetup> {
    let active = KeyRecord::new(
        KeyId::parse("snapshot-v1").ok()?,
        RootKey::new(vec![0x42; 32]).ok()?,
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )
    .ok()?;
    let keys = SnapshotKeyRing::new(active, Vec::new()).ok()?;
    let contract = ComponentContract::new(
        ComponentName::parse("catalog.search").ok()?,
        ContentDigest::from_bytes(&[0x20; 32]).ok()?,
        1,
        1,
        1,
    )
    .ok()?;
    let build = BuildId::parse("build-fuzz-v1").ok()?;
    let route = RouteIdentity::from_bytes(&[0x10; 32]).ok()?;
    let slot = IslandSlot::parse("search-results").ok()?;
    let scope = ScopeFingerprint::from_bytes(&[0x30; 32]).ok()?;
    let schemas = schemas()?;
    let seed = ExpectedSeedV1::new(
        contract.clone(),
        build.clone(),
        route.clone(),
        slot.clone(),
        schemas.clone(),
    );
    let instance = ExpectedInstanceV1::new(contract, build, route, slot, scope, schemas);
    let limits = SnapshotLimits::new(
        InputLimits::new(2_048, 8, 128, 512).ok()?,
        50,
        10_000,
        20_000,
        8,
        8,
    )
    .ok()?;
    Some(SnapshotSetup {
        keys,
        seed,
        instance,
        limits,
    })
}

fn schemas() -> Option<SnapshotSchemaSet> {
    let state = StateSchema::new(
        1,
        vec![FieldSpec::new("query", StateCodec::Json, FieldCategory::Public, true).ok()?],
    )
    .ok()?;
    let memo = StateSchema::new(1, Vec::new()).ok()?;
    let mount = StateSchema::new(1, Vec::new()).ok()?;
    SnapshotSchemaSet::new(state, memo, mount).ok()
}
