//! Deterministic state schema, dehydration, hydration, and tagged-codec tests.

mod snapshot_support;

use proptest::collection::vec;
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use snapshot_support::{
    expected_seed, key_ring, public_value, schema_set, seed_fields, snapshot_limits,
};
use suprnova_live::identity::UnixMillis;
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, StateCodec, StateExposure, StateSchema, decode_bytes, decode_i64,
    decode_u64, dehydrate, encode_bytes, encode_i64, encode_u64,
};
use suprnova_live::snapshot::{SeedBodyV1, SnapshotErrorKind, verify_seed};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct PublicState {
    query: String,
    selected: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct PublicMemo {
    page: f64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct PublicMount {
    catalog: String,
}

#[test]
fn explicit_schema_dehydrates_and_verified_capability_hydrates() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = snapshot_limits();
    let state = PublicState {
        query: "rust".to_owned(),
        selected: "1".to_owned(),
    };
    let canonical = dehydrate(
        &state,
        schemas.state(),
        StateExposure::PublicSeed,
        limits.input(),
    )
    .expect("registered public fields dehydrate");
    let mut fields = seed_fields(&keys);
    fields.state = canonical;
    let encoded = SeedBodyV1::new(fields, &schemas, &limits)
        .expect("valid seed constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");
    let verified = verify_seed(
        &encoded,
        &expected_seed(schemas.clone()),
        &keys,
        UnixMillis::new(1_050),
        &limits,
    )
    .expect("valid seed verifies before hydration");

    let hydrated: PublicState = verified
        .hydrate_state(schemas.state())
        .expect("caller-selected registered state type hydrates");

    assert_eq!(hydrated, state);
    assert_eq!(
        verified
            .hydrate_memo::<PublicMemo>(schemas.memo())
            .expect("verified memo hydrates"),
        PublicMemo { page: 1.0 }
    );
    assert_eq!(
        verified
            .hydrate_mount::<PublicMount>(schemas.mount())
            .expect("verified mount parameters hydrate"),
        PublicMount {
            catalog: "primary".to_owned(),
        }
    );
}

#[test]
fn secret_transient_computed_session_and_server_only_fields_never_dehydrate() {
    #[derive(Serialize)]
    struct UnsafeState {
        query: String,
        secret: String,
    }

    let schema = StateSchema::new(
        1,
        vec![
            FieldSpec::new("query", StateCodec::Json, FieldCategory::Public, true)
                .expect("field is valid"),
            FieldSpec::new("secret", StateCodec::Json, FieldCategory::Secret, false)
                .expect("field is valid"),
            FieldSpec::new(
                "transient",
                StateCodec::Json,
                FieldCategory::Transient,
                false,
            )
            .expect("field is valid"),
            FieldSpec::new("computed", StateCodec::Json, FieldCategory::Computed, false)
                .expect("field is valid"),
            FieldSpec::new("server", StateCodec::Json, FieldCategory::ServerOnly, false)
                .expect("field is valid"),
            FieldSpec::new("session", StateCodec::Json, FieldCategory::Session, false)
                .expect("field is valid"),
        ],
    )
    .expect("schema is valid");
    let state = UnsafeState {
        query: "rust".to_owned(),
        secret: "must-not-escape".to_owned(),
    };

    for exposure in [StateExposure::PublicSeed, StateExposure::Instanced] {
        let error = dehydrate(&state, &schema, exposure, snapshot_limits().input())
            .expect_err("excluded categories cannot enter any snapshot");
        assert_eq!(error.kind(), SnapshotErrorKind::ForbiddenStateField);
        assert!(!error.to_string().contains("must-not-escape"));
    }
}

#[test]
fn state_schema_rejects_unknown_missing_and_wrong_codec_values() {
    let schema = schema_set();
    let missing = public_value(r#"{"query":"rust"}"#);
    let missing_error = schema
        .state()
        .validate(&missing, StateExposure::PublicSeed)
        .expect_err("required selected field is present");
    assert_eq!(missing_error.kind(), SnapshotErrorKind::MissingStateField);

    let unknown = public_value(r#"{"query":"rust","selected":"1","extra":true}"#);
    let unknown_error = schema
        .state()
        .validate(&unknown, StateExposure::PublicSeed)
        .expect_err("unknown fields are rejected");
    assert_eq!(unknown_error.kind(), SnapshotErrorKind::UnknownStateField);
}

#[test]
fn tagged_integer_and_byte_codecs_round_trip_losslessly() {
    let signed = encode_i64(i64::MIN);
    assert_eq!(decode_i64(&signed).expect("i64 tag decodes"), i64::MIN);

    let unsigned = encode_u64(u64::MAX);
    assert_eq!(decode_u64(&unsigned).expect("u64 tag decodes"), u64::MAX);

    let bytes = vec![0_u8, 1, 2, 254, 255];
    let encoded = encode_bytes(&bytes, 32).expect("bounded bytes encode");
    assert_eq!(
        decode_bytes(&encoded, 32).expect("bounded bytes decode"),
        bytes
    );
    assert!(encode_bytes(&[0_u8; 33], 32).is_err());

    assert!(decode_i64(&public_value(r#"{"$live":"i64","value":"01"}"#)).is_err());
    assert!(decode_i64(&public_value(r#"{"$live":"i64","value":"-0"}"#)).is_err());
    assert!(decode_u64(&public_value(r#"{"$live":"u64","value":"+1"}"#)).is_err());
    let padded = public_value(r#"{"$live":"bytes","value":"AA=="}"#);
    assert!(decode_bytes(&padded, 32).is_err());
}

#[test]
fn large_finite_json_numbers_remain_ieee754_values_during_verified_hydration() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct LargeState {
        large: f64,
    }

    let keys = key_ring();
    let base = schema_set();
    let large_schema = StateSchema::new(
        1,
        vec![
            FieldSpec::new("large", StateCodec::Json, FieldCategory::Public, true)
                .expect("field is valid"),
        ],
    )
    .expect("schema is valid");
    let schemas = suprnova_live::snapshot::SnapshotSchemaSet::new(
        large_schema,
        base.memo().clone(),
        base.mount().clone(),
    )
    .expect("schema set is valid");
    let limits = snapshot_limits();
    let mut fields = seed_fields(&keys);
    fields.state = public_value(r#"{"large":1e30}"#);
    let encoded = SeedBodyV1::new(fields, &schemas, &limits)
        .expect("large finite number is valid JCS data")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("seed signs");
    let verified = verify_seed(
        &encoded,
        &expected_seed(schemas.clone()),
        &keys,
        UnixMillis::new(1_050),
        &limits,
    )
    .expect("large finite number verifies without integer coercion");

    assert_eq!(
        verified
            .hydrate_state::<LargeState>(schemas.state())
            .expect("large finite number hydrates"),
        LargeState { large: 1e30 }
    );
}

#[test]
fn dehydration_stops_at_the_encoded_byte_limit() {
    let tiny_limits =
        suprnova_live::limits::InputLimits::new(64, 4, 16, 256).expect("test limits are valid");
    let state = PublicState {
        query: "x".repeat(128),
        selected: "1".to_owned(),
    };

    let error = dehydrate(
        &state,
        schema_set().state(),
        StateExposure::PublicSeed,
        &tiny_limits,
    )
    .expect_err("streaming dehydration must stop at the encoded byte bound");

    assert_eq!(error.kind(), SnapshotErrorKind::InputTooLarge);
}

proptest! {
    #[test]
    fn tagged_state_codecs_round_trip_supported_rust_values(
        signed in any::<i64>(),
        unsigned in any::<u64>(),
        bytes in vec(any::<u8>(), 0..128),
    ) {
        prop_assert_eq!(decode_i64(&encode_i64(signed)).expect("encoded i64 decodes"), signed);
        prop_assert_eq!(decode_u64(&encode_u64(unsigned)).expect("encoded u64 decodes"), unsigned);
        let encoded = encode_bytes(&bytes, 128).expect("generated bytes are bounded");
        prop_assert_eq!(decode_bytes(&encoded, 128).expect("encoded bytes decode"), bytes);
    }
}
