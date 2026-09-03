//! Keys are canonical, bounded, versioned, inspectable, and never carry raw
//! request material.

use std::collections::BTreeMap;

use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{BuildId, KeyId, RouteIdentity, UnixMillis};
use suprnova_live::render_cache::VarianceDimension;
use suprnova_live::render_cache::key::{RenderKey, RenderKeyInput};
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
}
