//! Complete entries are bounded, versioned, integrity-protected, and
//! inspectable without the body.

use bytes::Bytes;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::entry::{
    CompleteEntry, EntryHeader, EntryKind, EntryLimits, SafeHeaders, Validator, decode, encode,
    encode_with_kind_for_test, inspect,
};
use suprnova_live::render_cache::generation::GenerationSet;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::variance::{
    DimensionValue, VarianceDescriptor, VarianceDimension,
};
use suprnova_live::render_cache::{RenderCacheErrorKind, RepresentationClass};

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
    keys_from(3)
}

fn entry(keys: &SnapshotKeyRing) -> CompleteEntry {
    entry_with_variance(keys, VarianceDescriptor::new())
}

fn entry_with_variance(keys: &SnapshotKeyRing, variance: VarianceDescriptor) -> CompleteEntry {
    let body = Bytes::from_static(b"<!doctype html><html><body>hello</body></html>");
    CompleteEntry::new(
        EntryHeader {
            key: RenderKey::for_test(keys, "/hello"),
            class: RepresentationClass::PublicShared,
            variance,
            published_at_ms: 1_000,
            fresh_ms: 60_000,
            stale_servable_ms: 0,
            stale_on_error_ms: 0,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms: None,
            status: 200,
            headers: SafeHeaders::from_pairs([
                ("content-type", "text/html; charset=utf-8"),
                ("cache-control", "private"),
            ])
            .expect("safe"),
            content_encoding: None,
        },
        body,
    )
}

/// Encodes the fixture header with the kind byte forced to `kind`, using the
/// crate's `#[doc(hidden)]` test-only encoder so an unsupported-kind entry
/// can still be produced with a correctly-computed integrity tag.
fn encode_kind_for_test(keys: &SnapshotKeyRing, kind: EntryKind) -> Bytes {
    encode_with_kind_for_test(&entry(keys), keys, kind)
}

#[test]
fn a_complete_entry_round_trips_with_a_strong_validator_over_exact_bytes() {
    let keys = keys();
    let entry = entry(&keys);
    let encoded = encode(&entry, &keys).expect("encode");
    let decoded = decode(&encoded, &keys, &EntryLimits::default()).expect("decode");
    assert_eq!(decoded.header(), entry.header());
    assert_eq!(decoded.body(), entry.body());
    assert_eq!(decoded.validator(), &Validator::strong_for(entry.body()));
    assert_eq!(
        decoded.validator().etag(),
        format!(
            "\"sha256-{}\"",
            Validator::strong_for(entry.body()).digest_base64url()
        )
    );
}

#[test]
fn every_corruption_is_a_miss_and_never_a_partial_entry() {
    let keys = keys();
    let encoded = encode(&entry(&keys), &keys).expect("encode");
    for index in 0..encoded.len() {
        let mut corrupt = encoded.to_vec();
        corrupt[index] ^= 0x55;
        let error = decode(&Bytes::from(corrupt), &keys, &EntryLimits::default())
            .expect_err("corrupt fails closed");
        assert!(
            matches!(
                error.kind(),
                RenderCacheErrorKind::EntryInvalid | RenderCacheErrorKind::EntryUnsupported
            ),
            "byte {index}"
        );
    }
    let truncated = encoded.slice(..encoded.len() / 2);
    assert!(decode(&truncated, &keys, &EntryLimits::default()).is_err());
    let other = keys_from(4);
    assert_eq!(
        decode(&encoded, &other, &EntryLimits::default())
            .expect_err("foreign key")
            .kind(),
        RenderCacheErrorKind::EntryInvalid
    );
}

#[test]
fn bounds_unsafe_headers_and_unsupported_kinds_fail_closed() {
    let keys = keys();
    let encoded = encode(&entry(&keys), &keys).expect("encode");
    let tiny = EntryLimits {
        max_body_bytes: 8,
        ..EntryLimits::default()
    };
    assert_eq!(
        decode(&encoded, &keys, &tiny)
            .expect_err("oversized")
            .kind(),
        RenderCacheErrorKind::EntryInvalid
    );
    assert!(SafeHeaders::from_pairs([("set-cookie", "a=b")]).is_err());
    assert!(SafeHeaders::from_pairs([("transfer-encoding", "chunked")]).is_err());
    assert!(
        SafeHeaders::from_pairs([("x-request-id", "abc")]).is_err(),
        "per-request tracing headers never replay"
    );
    let composite = encode_kind_for_test(&keys, EntryKind::Composite);
    assert_eq!(
        decode(&composite, &keys, &EntryLimits::default())
            .expect_err("composite")
            .kind(),
        RenderCacheErrorKind::EntryUnsupported
    );
}

#[test]
fn inspection_reads_metadata_without_the_body() {
    let keys = keys();
    let encoded = encode(&entry(&keys), &keys).expect("encode");
    let inspection = inspect(&encoded, &EntryLimits::default()).expect("inspect");
    assert_eq!(inspection.kind, EntryKind::Complete);
    assert_eq!(inspection.class, RepresentationClass::PublicShared);
    assert_eq!(inspection.body_bytes, 46);
    assert_eq!(inspection.status, 200);
    assert!(
        !format!("{inspection:?}").contains("hello"),
        "the body never appears in inspection"
    );
}

#[test]
fn a_declared_application_dimension_round_trips_through_encode_and_decode() {
    let keys = keys();
    let mut variance = VarianceDescriptor::new();
    variance
        .declare(
            VarianceDimension::Application("checkout".to_owned()),
            DimensionValue::Public("v2".to_owned()),
        )
        .expect("declare");
    let original = entry_with_variance(&keys, variance);
    let encoded = encode(&original, &keys).expect("encode");
    let decoded = decode(&encoded, &keys, &EntryLimits::default()).expect("decode");
    assert_eq!(
        decoded.header(),
        original.header(),
        "the declared application dimension survives the round trip"
    );
    assert_eq!(decoded.header().variance.dimensions().len(), 1);
}
