//! Complete entries are bounded, versioned, integrity-protected, and
//! inspectable without the body.

use std::collections::BTreeMap;

use bytes::Bytes;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{ContentDigest, KeyId, RouteIdentity, UnixMillis};
use suprnova_live::render_cache::composite::{
    CompositeEntry, HeaderPiece, HeaderTemplate, Segment, SegmentGraph, ShellIsland,
    SlotFailurePolicy, StitchSlot, surrounding_digest,
};
use suprnova_live::render_cache::entry::{
    CompleteEntry, DecodedEntry, EntryHeader, EntryKind, EntryLimits, SafeHeaders, Validator,
    decode, encode, encode_composite, encode_raw_header_for_test,
    encode_raw_header_for_test_with_kind, inspect,
};
use suprnova_live::render_cache::generation::GenerationSet;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::variance::{
    DimensionValue, PrivateMaterial, VarianceDescriptor, VarianceDimension,
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
    entry_with(keys, VarianceDescriptor::new(), GenerationSet::default())
}

fn entry_with_variance(keys: &SnapshotKeyRing, variance: VarianceDescriptor) -> CompleteEntry {
    entry_with(keys, variance, GenerationSet::default())
}

fn entry_with_observed(keys: &SnapshotKeyRing, observed: GenerationSet) -> CompleteEntry {
    entry_with(keys, VarianceDescriptor::new(), observed)
}

fn entry_with(
    keys: &SnapshotKeyRing,
    variance: VarianceDescriptor,
    observed: GenerationSet,
) -> CompleteEntry {
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
            observed,
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

/// The same header shape [`entry`] builds, as a `serde_json::Value` rather
/// than a typed `EntryHeader`, so a test can inject content the typed
/// constructors (`SafeHeaders::from_pairs`, `VarianceDescriptor::declare`)
/// would refuse before encoding it through
/// [`encode_raw_header_for_test`], which still produces a correctly-signed
/// entry.
fn base_header_json(keys: &SnapshotKeyRing) -> serde_json::Value {
    serde_json::json!({
        "key": RenderKey::for_test(keys, "/hello").to_base64url(),
        "class": "public_shared",
        "variance": {},
        "published_at_ms": 1_000,
        "fresh_ms": 60_000,
        "stale_servable_ms": 0,
        "stale_on_error_ms": 0,
        "observed": {},
        "epoch": 1,
        "seed_deadline_ms": serde_json::Value::Null,
        "status": 200,
        "headers": {
            "content-type": "text/html; charset=utf-8",
            "cache-control": "private",
        },
        "content_encoding": serde_json::Value::Null,
    })
}

/// A minimal Composite entry: a shell with one nonce hole and one stitch
/// slot, plus one shell island and one nonce-bearing header template - the
/// same shape as `composite::tests::graph_and_shell`, reused here at the
/// codec boundary rather than duplicated.
fn composite_entry(keys: &SnapshotKeyRing) -> CompositeEntry {
    let head = b"<!doctype html><html><body>".to_vec();
    let tail = b"</body></html>".to_vec();
    let shell = Bytes::from([head.as_slice(), tail.as_slice()].concat());
    let mut graph = SegmentGraph {
        segments: vec![
            Segment::Literal {
                len: head.len() as u32,
            },
            Segment::Slot { index: 0 },
            Segment::Nonce,
            Segment::Literal {
                len: tail.len() as u32,
            },
        ],
        slots: vec![StitchSlot {
            route: RouteIdentity::from_bytes(&[4u8; 32])
                .expect("route")
                .to_base64url(),
            slot: "counter".to_owned(),
            document_key: "doc-counter".to_owned(),
            component: "app.counter".to_owned(),
            contract_digest: ContentDigest::from_bytes(&[2u8; 32])
                .expect("digest")
                .to_base64url(),
            protocol: 1,
            build: "suprnova-1.0.0".to_owned(),
            parameters: "{}".to_owned(),
            flags: BTreeMap::new(),
            on_failure: SlotFailurePolicy::FailDocument,
            surrounding: String::new(),
        }],
        shell_islands: vec![ShellIsland {
            slot: "seed".to_owned(),
            document_key: "doc-seed".to_owned(),
        }],
        nonce_headers: vec![HeaderTemplate {
            name: "content-security-policy".to_owned(),
            pieces: vec![
                HeaderPiece::Text {
                    text: "script-src 'nonce-".to_owned(),
                },
                HeaderPiece::Nonce,
                HeaderPiece::Text {
                    text: "'".to_owned(),
                },
            ],
        }],
    };
    graph.slots[0].surrounding = surrounding_digest(&graph, &shell, 0).expect("surrounding");
    let mut header = entry(keys).header().clone();
    header.class = RepresentationClass::PublicShellStitched;
    CompositeEntry::new(header, graph, shell).expect("composite entry")
}

/// Final review, F11: the stored header is JSON and every tag in it is
/// `snake_case`, like every other public JSON name in this crate. The
/// representation class and the dimension value enum both serialize that
/// way, and a header written in that form decodes.
///
/// Proven by revert: with `#[serde(rename_all = "snake_case")]` removed from
/// `RepresentationClass`, the first assertion sees `"PublicShared"`; removed
/// from `DimensionValue`, the second sees `"Anonymous"`.
#[test]
fn the_stored_header_uses_snake_case_enum_tags() {
    let keys = keys();
    let header = serde_json::to_value(entry(&keys).header()).expect("header serializes");
    assert_eq!(header["class"], serde_json::json!("public_shared"));
    assert_eq!(
        serde_json::to_value(DimensionValue::Anonymous).expect("anonymous serializes"),
        serde_json::json!("anonymous")
    );
    assert_eq!(
        serde_json::to_value(DimensionValue::Public("en".to_owned())).expect("public serializes"),
        serde_json::json!({ "public": "en" })
    );
    let private = PrivateMaterial::principal(&keys, "user-7", 0);
    let private_json = serde_json::to_value(DimensionValue::Private(private)).expect("serializes");
    assert!(
        private_json.get("private").is_some(),
        "private material is tagged `private`, got {private_json}"
    );

    // A header in the documented form decodes: `base_header_json` is written
    // in exactly these tags, and the round trip below is what every
    // hand-built header fixture in this file relies on.
    let body = Bytes::from_static(b"<!doctype html><html><body>hello</body></html>");
    let encoded = encode_raw_header_for_test(&base_header_json(&keys), &body, &keys);
    let decoded = decode(&encoded, &keys, &EntryLimits::default())
        .expect("a header written with snake_case tags decodes")
        .into_complete()
        .expect("complete");
    assert_eq!(decoded.header().class, RepresentationClass::PublicShared);
}

#[test]
fn a_complete_entry_round_trips_with_a_strong_validator_over_exact_bytes() {
    let keys = keys();
    let entry = entry(&keys);
    let encoded = encode(&entry, &keys).expect("encode");
    let decoded = decode(&encoded, &keys, &EntryLimits::default())
        .expect("decode")
        .into_complete()
        .expect("complete");
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
fn bounds_and_unsafe_headers_fail_closed() {
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
    let decoded = decode(&encoded, &keys, &EntryLimits::default())
        .expect("decode")
        .into_complete()
        .expect("complete");
    assert_eq!(
        decoded.header(),
        original.header(),
        "the declared application dimension survives the round trip"
    );
    assert_eq!(decoded.header().variance.dimensions().len(), 1);
}

#[test]
fn a_populated_observed_generation_set_round_trips_through_encode_and_decode() {
    let keys = keys();
    let mut observed = GenerationSet::default();
    observed.insert_digest([0x11; 32], 7).expect("within bound");
    observed
        .insert_digest([0x22; 32], 42)
        .expect("within bound");
    let original = entry_with_observed(&keys, observed);
    let encoded = encode(&original, &keys).expect("encode");
    let decoded = decode(&encoded, &keys, &EntryLimits::default())
        .expect("decode")
        .into_complete()
        .expect("complete");
    assert_eq!(
        decoded.header(),
        original.header(),
        "the observed generation set survives the round trip"
    );
    assert_eq!(decoded.header().observed.len(), 2);
}

#[test]
fn a_generation_key_shorter_than_64_characters_fails_to_deserialize() {
    let key = &"1a".repeat(32)[..63];
    let json = format!("{{\"{key}\":7}}");
    assert!(
        serde_json::from_str::<GenerationSet>(&json).is_err(),
        "a 63-character key must never decode"
    );
}

#[test]
fn a_generation_key_longer_than_64_characters_fails_to_deserialize() {
    let key = format!("{}0", "1a".repeat(32));
    let json = format!("{{\"{key}\":7}}");
    assert!(
        serde_json::from_str::<GenerationSet>(&json).is_err(),
        "a 65-character key must never decode"
    );
}

#[test]
fn a_non_hex_character_in_a_generation_key_fails_to_deserialize() {
    let mut key = "1a".repeat(32);
    key.replace_range(0..1, "z");
    let json = format!("{{\"{key}\":7}}");
    assert!(
        serde_json::from_str::<GenerationSet>(&json).is_err(),
        "a non-hex character anywhere in an otherwise 64-character key must never decode"
    );
}

#[test]
fn an_uppercase_spelling_of_a_generation_key_fails_to_deserialize() {
    let key = "1a".repeat(32).to_ascii_uppercase();
    let json = format!("{{\"{key}\":7}}");
    assert!(
        serde_json::from_str::<GenerationSet>(&json).is_err(),
        "an uppercase 64-character key must not denote the same digest a lowercase key would"
    );
}

#[test]
fn a_mixed_case_spelling_of_a_generation_key_fails_to_deserialize() {
    let mut key = "1a".repeat(32);
    key.replace_range(2..3, "A");
    let json = format!("{{\"{key}\":7}}");
    assert!(
        serde_json::from_str::<GenerationSet>(&json).is_err(),
        "a mixed-case key must not denote the same digest its all-lowercase spelling would"
    );
}

#[test]
fn a_forbidden_response_header_with_a_valid_integrity_tag_still_fails_to_decode() {
    let keys = keys();
    let mut header = base_header_json(&keys);
    header["headers"] = serde_json::json!({ "set-cookie": "a=b" });
    let body = Bytes::from_static(b"<!doctype html><html><body>hello</body></html>");
    let encoded = encode_raw_header_for_test(&header, &body, &keys);
    let error = decode(&encoded, &keys, &EntryLimits::default())
        .expect_err("a forbidden header must never replay even with a valid integrity tag");
    assert_eq!(error.kind(), RenderCacheErrorKind::EntryInvalid);
}

#[test]
fn variance_beyond_the_declared_dimension_bound_with_a_valid_integrity_tag_still_fails_to_decode() {
    let keys = keys();
    let mut header = base_header_json(&keys);
    let mut dimensions = serde_json::Map::new();
    for index in 0..25 {
        dimensions.insert(
            format!("app:d{index}"),
            serde_json::Value::String("anonymous".to_owned()),
        );
    }
    header["variance"] = serde_json::Value::Object(dimensions);
    let body = Bytes::from_static(b"<!doctype html><html><body>hello</body></html>");
    let encoded = encode_raw_header_for_test(&header, &body, &keys);
    let error = decode(&encoded, &keys, &EntryLimits::default()).expect_err(
        "more declared dimensions than the maximum must never replay even with a valid \
         integrity tag",
    );
    assert_eq!(error.kind(), RenderCacheErrorKind::EntryInvalid);
}

#[test]
fn a_composite_entry_round_trips_and_inspects_without_its_shell() {
    let keys = keys();
    let entry = composite_entry(&keys);
    let encoded = encode_composite(&entry, &keys).expect("encodes");
    let inspection = inspect(&encoded, &EntryLimits::default()).expect("inspects");
    assert_eq!(inspection.kind, EntryKind::Composite);
    assert_eq!(inspection.class, RepresentationClass::PublicShellStitched);
    assert_eq!(inspection.slots, 1);
    assert_eq!(inspection.body_bytes, entry.shell().len());
    match decode(&encoded, &keys, &EntryLimits::default()).expect("decodes") {
        DecodedEntry::Composite(decoded) => {
            assert_eq!(&decoded, &entry);
            assert_eq!(decoded.structural_digest(), entry.structural_digest());
        }
        DecodedEntry::Complete(_) => panic!("a composite entry decoded as complete"),
    }
}

#[test]
fn a_complete_entry_inspects_with_zero_slots() {
    let keys = keys();
    let encoded = encode(&entry(&keys), &keys).expect("encodes");
    assert_eq!(
        inspect(&encoded, &EntryLimits::default())
            .expect("inspects")
            .slots,
        0
    );
}

#[test]
fn every_single_bit_flip_of_a_composite_entry_fails_to_decode() {
    let keys = keys();
    let encoded = encode_composite(&composite_entry(&keys), &keys).expect("encodes");
    for byte in 0..encoded.len() {
        for bit in 0..8 {
            let mut corrupted = encoded.to_vec();
            corrupted[byte] ^= 1 << bit;
            let error = decode(&Bytes::from(corrupted), &keys, &EntryLimits::default())
                .expect_err("corrupt fails closed");
            assert!(
                matches!(
                    error.kind(),
                    RenderCacheErrorKind::EntryInvalid | RenderCacheErrorKind::EntryUnsupported
                ),
                "byte {byte} bit {bit}"
            );
        }
    }
}

#[test]
fn a_composite_entry_signed_by_another_ring_is_rejected() {
    let encoded = encode_composite(&composite_entry(&keys()), &keys()).expect("encodes");
    assert!(decode(&encoded, &keys_from(9), &EntryLimits::default()).is_err());
}

#[test]
fn a_composite_header_whose_graph_disagrees_with_its_shell_is_rejected() {
    let keys = keys();
    let entry = composite_entry(&keys);
    let mut header =
        serde_json::to_value(suprnova_live::render_cache::composite::CompositeHeader {
            entry: entry.header().clone(),
            graph: entry.graph().clone(),
        })
        .expect("header json");
    header["graph"]["segments"][0]["len"] = serde_json::json!(entry.shell().len() as u64 - 1);
    let encoded =
        encode_raw_header_for_test_with_kind(&header, entry.shell(), &keys, EntryKind::Composite);
    assert_eq!(
        decode(&encoded, &keys, &EntryLimits::default())
            .map(|_| ())
            .map_err(|e| e.kind()),
        Err(RenderCacheErrorKind::EntryInvalid)
    );
}

#[test]
fn a_composite_header_whose_class_is_not_stitched_is_rejected() {
    let keys = keys();
    let entry = composite_entry(&keys);
    let mut header =
        serde_json::to_value(suprnova_live::render_cache::composite::CompositeHeader {
            entry: entry.header().clone(),
            graph: entry.graph().clone(),
        })
        .expect("header json");
    header["class"] = serde_json::json!("public_shared");
    let encoded =
        encode_raw_header_for_test_with_kind(&header, entry.shell(), &keys, EntryKind::Composite);
    assert_eq!(
        decode(&encoded, &keys, &EntryLimits::default())
            .map(|_| ())
            .map_err(|e| e.kind()),
        Err(RenderCacheErrorKind::EntryInvalid)
    );
}
