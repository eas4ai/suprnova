//! Property coverage for supported canonical values.

use std::collections::BTreeMap;

use proptest::collection::{btree_map, vec};
use proptest::prelude::*;
use suprnova_live::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use suprnova_live::limits::InputLimits;

fn canonical_values() -> impl Strategy<Value = CanonicalValue> {
    let leaf = prop_oneof![
        Just(CanonicalValue::Null),
        any::<bool>().prop_map(CanonicalValue::Bool),
        (-1_000_000_i32..=1_000_000_i32)
            .prop_map(|value| CanonicalValue::number(f64::from(value)).expect("range is finite")),
        "[a-zA-Z0-9 _-]{0,12}".prop_map(CanonicalValue::String),
    ];

    leaf.prop_recursive(3, 32, 4, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..4).prop_map(CanonicalValue::Array),
            btree_map("[a-z]{1,5}", inner, 0..4).prop_map(
                |values: BTreeMap<String, CanonicalValue>| CanonicalValue::Object(values)
            ),
        ]
    })
}

proptest! {
    #[test]
    fn supported_values_round_trip_and_recanonicalize_identically(value in canonical_values()) {
        let limits = InputLimits::new(16 * 1024, 8, 128, 256).expect("test limits are valid");
        let first = to_canonical_bytes(&value, &limits).expect("generated value canonicalizes");
        let parsed = parse_canonical_value(&first, &limits).expect("canonical bytes parse");
        let second = to_canonical_bytes(&parsed, &limits).expect("parsed value canonicalizes");

        prop_assert_eq!(&first, &second);
        prop_assert_eq!(value, parsed);
    }
}
