//! Shared deterministic values for snapshot integration tests.

#![allow(
    dead_code,
    reason = "shared helpers are used by separate integration-test crates"
)]

use std::collections::BTreeMap;

use suprnova_live::canonical::{CanonicalValue, parse_canonical_value};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, Generation, InstanceId, IslandSlot, KeyId, Revision,
    RouteIdentity, ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedSeedV1, GenerationMemo, InstanceFieldsV1, SeedFieldsV1,
    SnapshotLimits, SnapshotSchemaSet,
};

pub(crate) fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

pub(crate) fn key_ring() -> SnapshotKeyRing {
    SnapshotKeyRing::new(
        KeyRecord::new(
            KeyId::parse("snapshot-v1").expect("test key id is valid"),
            RootKey::new(vec![0x42; 32]).expect("test key is strong"),
            UnixMillis::new(0),
            UnixMillis::new(10_000),
            UnixMillis::new(20_000),
        )
        .expect("test key window is valid"),
        Vec::new(),
    )
    .expect("test key ring is valid")
}

pub(crate) fn component_contract() -> ComponentContract {
    ComponentContract::new(
        ComponentName::parse("catalog.search").expect("component name is valid"),
        ContentDigest::from_bytes(&bytes::<32>(0x20)).expect("digest is valid"),
        1,
        1,
        1,
    )
    .expect("component contract is valid")
}

pub(crate) fn route(start: u8) -> RouteIdentity {
    RouteIdentity::from_bytes(&bytes::<32>(start)).expect("route digest is valid")
}

pub(crate) fn public_value(json: &str) -> CanonicalValue {
    parse_canonical_value(json.as_bytes(), snapshot_limits().input())
        .expect("test canonical value is valid")
}

pub(crate) fn schema_set() -> SnapshotSchemaSet {
    let state = StateSchema::new(
        1,
        vec![
            FieldSpec::new("query", StateCodec::Json, FieldCategory::Public, true)
                .expect("field is valid"),
            FieldSpec::new("selected", StateCodec::Json, FieldCategory::Public, true)
                .expect("field is valid"),
        ],
    )
    .expect("state schema is valid");
    let memo = StateSchema::new(
        1,
        vec![
            FieldSpec::new("page", StateCodec::Json, FieldCategory::Public, true)
                .expect("field is valid"),
        ],
    )
    .expect("memo schema is valid");
    let mount = StateSchema::new(
        1,
        vec![
            FieldSpec::new("catalog", StateCodec::Json, FieldCategory::Public, true)
                .expect("field is valid"),
        ],
    )
    .expect("mount schema is valid");
    SnapshotSchemaSet::new(state, memo, mount).expect("schema versions agree")
}

pub(crate) fn snapshot_limits() -> SnapshotLimits {
    SnapshotLimits::new(
        InputLimits::new(4_096, 4, 64, 512).expect("input limits are valid"),
        50,
        10_000,
        20_000,
        8,
        8,
    )
    .expect("snapshot limits are valid")
}

pub(crate) fn seed_fields(keys: &SnapshotKeyRing) -> SeedFieldsV1 {
    SeedFieldsV1 {
        component: component_contract(),
        build_id: BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route: route(1),
        slot: IslandSlot::parse("search-results").expect("slot is valid"),
        key_id: keys.active_key_id().clone(),
        issued_at: UnixMillis::new(1_000),
        max_age_ms: 500,
        mount: public_value(r#"{"catalog":"primary"}"#),
        state: public_value(r#"{"query":"rust","selected":"1"}"#),
        memo: public_value(r#"{"page":1}"#),
        advisory_generations: vec![GenerationMemo::new(
            ContentDigest::from_bytes(&bytes::<32>(0x70)).expect("digest is valid"),
            Generation::new(9),
        )],
        refresh_on_promote: true,
        extensions: BTreeMap::new(),
    }
}

pub(crate) fn instance_fields(keys: &SnapshotKeyRing) -> InstanceFieldsV1 {
    InstanceFieldsV1 {
        component: component_contract(),
        build_id: BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route: route(1),
        slot: IslandSlot::parse("search-results").expect("slot is valid"),
        key_id: keys.active_key_id().clone(),
        scope: ScopeFingerprint::from_bytes(&bytes::<32>(0x90)).expect("scope is valid"),
        instance_id: InstanceId::from_bytes(&bytes::<16>(0xb0)).expect("instance is valid"),
        revision: Revision::new(7),
        issued_at: UnixMillis::new(1_000),
        expires_at: UnixMillis::new(2_000),
        state: public_value(r#"{"query":"rust","selected":"1"}"#),
        memo: public_value(r#"{"page":1}"#),
        extensions: BTreeMap::new(),
    }
}

pub(crate) fn expected_seed(schemas: SnapshotSchemaSet) -> ExpectedSeedV1 {
    ExpectedSeedV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        schemas,
    )
}
