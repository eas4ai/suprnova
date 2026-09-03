//! Keys are canonical, bounded, versioned, inspectable, and never carry raw
//! request material.

use std::collections::{BTreeMap, HashMap};

use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{BuildId, KeyId, RouteIdentity, UnixMillis};
use suprnova_live::render_cache::VarianceDimension;
use suprnova_live::render_cache::key::{
    MAX_PARAM_BYTES, MAX_PARAMS, RenderKey, RenderKeyDimensions, RenderKeyInput,
};
use suprnova_live::render_cache::variance::{DimensionValue, PrivateMaterial, VarianceDescriptor};

fn keys_from(root: u8) -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![root; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn keys() -> SnapshotKeyRing {
    keys_from(9)
}

// `RouteIdentity` is a bare purpose-specific digest (32 bytes, no recoverable
// text), so tests build one from fixed bytes the way every other suite in
// this crate does and carry the human-readable pattern separately in
// `route_pattern`, which is what inspection actually shows.
fn input(query: &[(&str, &str)]) -> RenderKeyInput {
    RenderKeyInput {
        route: RouteIdentity::from_bytes(&[0x51; 32]).expect("route"),
        route_pattern: "/catalog/{category}".to_owned(),
        params: BTreeMap::from([("category".to_owned(), "shoes".to_owned())]),
        query: query
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        host: None,
        media: "text/html".to_owned(),
        encoding: None,
        build: BuildId::parse("build-1").expect("build"),
        epoch: 1,
        variance: VarianceDescriptor::new(),
    }
}

#[test]
fn equivalent_inputs_derive_the_same_key_and_different_ones_differ() {
    let keys = keys();
    let a = RenderKey::derive(&input(&[("page", "2"), ("sort", "asc")]), &keys).expect("key a");
    let b = RenderKey::derive(&input(&[("sort", "asc"), ("page", "2")]), &keys).expect("key b");
    assert_eq!(a, b, "query order is canonical");
    let c = RenderKey::derive(&input(&[("page", "3"), ("sort", "asc")]), &keys).expect("key c");
    assert_ne!(a, c, "different values never collapse");
    let mut other_build = input(&[("page", "2"), ("sort", "asc")]);
    other_build.build = BuildId::parse("build-2").expect("build");
    assert_ne!(
        a,
        RenderKey::derive(&other_build, &keys).expect("key"),
        "builds are namespaced"
    );
    let mut other_epoch = input(&[("page", "2"), ("sort", "asc")]);
    other_epoch.epoch = 2;
    assert_ne!(
        a,
        RenderKey::derive(&other_epoch, &keys).expect("key"),
        "epochs are namespaced"
    );
}

#[test]
fn the_encoded_key_is_versioned_bounded_and_free_of_request_material() {
    let keys = keys();
    let mut with_private = input(&[]);
    with_private
        .variance
        .declare(
            VarianceDimension::Principal,
            DimensionValue::Private(PrivateMaterial::principal(&keys, "user-7", 1)),
        )
        .expect("principal");
    let key = RenderKey::derive(&with_private, &keys).expect("key");
    let encoded = key.to_base64url();
    assert!(encoded.starts_with("rk1."), "{encoded}");
    assert!(encoded.len() <= 64);
    assert!(!encoded.contains("shoes") && !encoded.contains("user-7"));
    let dimensions = key.dimensions();
    assert_eq!(dimensions.route(), "/catalog/{category}");
    assert_eq!(
        dimensions.params().get("category").map(String::as_str),
        Some("shoes")
    );
    assert!(
        !format!("{dimensions:?}").contains("user-7"),
        "private values are digests in inspection"
    );
}

#[test]
fn bounds_fail_closed() {
    let keys = keys();
    let mut too_many = input(&[]);
    for index in 0..33 {
        too_many.params.insert(format!("p{index}"), "v".to_owned());
    }
    assert!(RenderKey::derive(&too_many, &keys).is_err());
    let mut too_long = input(&[]);
    too_long.query.insert("page".to_owned(), "x".repeat(513));
    assert!(RenderKey::derive(&too_long, &keys).is_err());

    // A bound that rejects its own limit is as wrong as one that admits a
    // value past it: exactly `MAX_PARAMS` entries and exactly
    // `MAX_PARAM_BYTES` bytes must both still succeed.
    let mut exactly_max_params = input(&[]);
    for index in 0..(MAX_PARAMS - exactly_max_params.params.len()) {
        exactly_max_params
            .params
            .insert(format!("p{index}"), "v".to_owned());
    }
    assert_eq!(
        exactly_max_params.params.len() + exactly_max_params.query.len(),
        MAX_PARAMS
    );
    assert!(RenderKey::derive(&exactly_max_params, &keys).is_ok());

    let mut exactly_max_param_bytes = input(&[]);
    exactly_max_param_bytes
        .query
        .insert("page".to_owned(), "x".repeat(MAX_PARAM_BYTES));
    assert!(RenderKey::derive(&exactly_max_param_bytes, &keys).is_ok());
}

#[test]
fn route_pattern_bounds_fail_closed() {
    let keys = keys();
    let mut empty_pattern = input(&[]);
    empty_pattern.route_pattern = String::new();
    assert!(RenderKey::derive(&empty_pattern, &keys).is_err());

    let mut long_pattern = input(&[]);
    long_pattern.route_pattern = "/".repeat(257);
    assert!(RenderKey::derive(&long_pattern, &keys).is_err());

    let mut control_pattern = input(&[]);
    control_pattern.route_pattern = "/catalog/\u{0007}".to_owned();
    assert!(RenderKey::derive(&control_pattern, &keys).is_err());
}

#[test]
fn a_key_parsed_from_its_encoded_text_equals_the_key_it_was_derived_from() {
    let keys = keys();
    let published = RenderKey::derive(&input(&[("page", "2")]), &keys).expect("key");
    let encoded = published.to_base64url();
    let parsed = RenderKey::from_base64url(&encoded).expect("parsed");

    assert_eq!(published, parsed, "digest equality must ignore dimensions");
    assert_eq!(published.cmp(&parsed), std::cmp::Ordering::Equal);
    assert_ne!(
        published.dimensions(),
        parsed.dimensions(),
        "dimensions still differ: derive keeps them, from_base64url does not"
    );
    assert_eq!(parsed.dimensions(), &RenderKeyDimensions::opaque());

    let mut store: HashMap<RenderKey, &str> = HashMap::new();
    store.insert(published.clone(), "published-entry");
    assert_eq!(
        store.get(&parsed),
        Some(&"published-entry"),
        "a key parsed back from storage must hit the slot it was published under"
    );
}

#[test]
fn for_test_is_deterministic_per_pattern_and_differs_across_patterns() {
    let keys = keys();
    let repeated_a = RenderKey::for_test(&keys, "/catalog/{category}");
    let repeated_b = RenderKey::for_test(&keys, "/catalog/{category}");
    let different = RenderKey::for_test(&keys, "/orders/{id}");
    assert_eq!(
        repeated_a, repeated_b,
        "the same pattern and ring derive the same key"
    );
    assert_ne!(
        repeated_a, different,
        "different patterns derive different keys"
    );
    assert_eq!(repeated_a.dimensions().route(), "/catalog/{category}");
}
