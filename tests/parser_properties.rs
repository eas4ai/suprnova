//! Property coverage for bounded request and response parsers.

mod protocol_support;

use std::collections::BTreeMap;

use proptest::prelude::*;
use suprnova_live::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::InputLimits;
use suprnova_live::protocol::{parse_update_request, parse_update_response};
use suprnova_live::snapshot::verify_seed;

fn canonical_values() -> impl Strategy<Value = CanonicalValue> {
    let leaf = prop_oneof![
        Just(CanonicalValue::Null),
        any::<bool>().prop_map(CanonicalValue::Bool),
        (-9_007_199_254_740_991_i64..=9_007_199_254_740_991_i64).prop_map(|value| {
            CanonicalValue::number(value as f64).expect("generated integer is finite")
        }),
        "[a-zA-Z0-9 _-]{0,32}".prop_map(CanonicalValue::String),
    ];

    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(CanonicalValue::Array),
            prop::collection::btree_map("[a-z]{1,8}", inner, 0..8).prop_map(CanonicalValue::Object),
        ]
    })
}

fn malformed_envelopes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just(br#"{}"#.to_vec()),
        Just(br#"{"body":null,"signature":"bad"}"#.to_vec()),
        Just(br#"{"body":{"form":"seed","schema_version":1},"signature":null}"#.to_vec()),
        prop::collection::vec(any::<u8>(), 0..1_024),
    ]
}

proptest! {
    #[test]
    fn arbitrary_external_bytes_never_panic_any_iteration_001_parser(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let canonical_limits = InputLimits::new(2_048, 8, 128, 512).expect("limits are valid");
        let _ = parse_canonical_value(&bytes, &canonical_limits);
        let _ = parse_update_request(&bytes, &protocol_support::limits());
        let _ = parse_update_response(&bytes, &protocol_support::limits());
    }

    #[test]
    fn explicit_extension_objects_recanonicalize_stably(
        entries in prop::collection::btree_map("x_[a-z]{1,8}", any::<bool>(), 0..8)
    ) {
        let value = CanonicalValue::Object(entries.into_iter().map(|(key, value)| (key, CanonicalValue::Bool(value))).collect());
        let limits = InputLimits::new(4_096, 8, 128, 512).expect("limits are valid");
        let encoded = to_canonical_bytes(&value, &limits).expect("value canonicalizes");
        let reparsed = parse_canonical_value(&encoded, &limits).expect("encoded value parses");
        prop_assert_eq!(to_canonical_bytes(&reparsed, &limits).expect("value recanonicalizes"), encoded);
    }

    #[test]
    fn valid_canonical_trees_round_trip_stably(value in canonical_values()) {
        let limits = InputLimits::new(16_384, 8, 512, 1_024).expect("limits are valid");
        let encoded = to_canonical_bytes(&value, &limits).expect("bounded generated value canonicalizes");
        let reparsed = parse_canonical_value(&encoded, &limits).expect("canonical bytes parse");
        prop_assert_eq!(reparsed, value);
    }

    #[test]
    fn malformed_snapshot_envelopes_are_always_classified(encoded in malformed_envelopes()) {
        let keys = protocol_support::snapshot_support::key_ring();
        let schemas = protocol_support::snapshot_support::schema_set();
        let expected = protocol_support::snapshot_support::expected_seed(schemas);
        let result = verify_seed(
            &encoded,
            &expected,
            &keys,
            UnixMillis::new(1_010),
            &protocol_support::snapshot_support::snapshot_limits(),
        );
        prop_assert!(result.is_err());
    }

    #[test]
    fn every_random_signed_seed_mutation_is_rejected(
        index in any::<prop::sample::Index>(),
        mask in 1_u8..=u8::MAX,
    ) {
        let keys = protocol_support::snapshot_support::key_ring();
        let schemas = protocol_support::snapshot_support::schema_set();
        let expected = protocol_support::snapshot_support::expected_seed(schemas);
        let mut encoded = protocol_support::seed_snapshot().into_bytes();
        let position = index.index(encoded.len());
        encoded[position] ^= mask;

        let result = verify_seed(
            &encoded,
            &expected,
            &keys,
            UnixMillis::new(1_010),
            &protocol_support::snapshot_support::snapshot_limits(),
        );
        prop_assert!(result.is_err());
    }
}

#[test]
fn schema_evolution_extensions_preserve_names_and_values() {
    let value = CanonicalValue::Object(BTreeMap::from([
        ("x_future_bool".to_owned(), CanonicalValue::Bool(true)),
        (
            "x_future_object".to_owned(),
            CanonicalValue::Object(BTreeMap::from([(
                "mode".to_owned(),
                CanonicalValue::String("future".to_owned()),
            )])),
        ),
    ]));
    let limits = InputLimits::new(4_096, 8, 128, 512).expect("limits are valid");
    let encoded = to_canonical_bytes(&value, &limits).expect("extensions canonicalize");
    let reparsed = parse_canonical_value(&encoded, &limits).expect("extensions parse");

    assert_eq!(reparsed, value);
}
