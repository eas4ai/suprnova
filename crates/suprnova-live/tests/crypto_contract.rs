//! Snapshot key-ring and signature contract tests.

use suprnova_live::crypto::{
    KeyErrorKind, KeyRecord, RootKey, SnapshotKeyRing, SnapshotPurpose, SnapshotSignature,
};
use suprnova_live::identity::{KeyId, UnixMillis};

fn record(
    key_id: &str,
    fill: u8,
    active_from: u64,
    sign_until: u64,
    verify_until: u64,
) -> KeyRecord {
    KeyRecord::new(
        KeyId::parse(key_id).expect("test key id is valid"),
        RootKey::new(vec![fill; 32]).expect("test root key is strong"),
        UnixMillis::new(active_from),
        UnixMillis::new(sign_until),
        UnixMillis::new(verify_until),
    )
    .expect("test key window is valid")
}

#[test]
fn signs_with_the_active_key_and_verifies_in_constant_time_api() {
    let ring = SnapshotKeyRing::new(record("active-v1", 0x11, 10, 100, 150), Vec::new())
        .expect("key ring is valid");
    let body = br#"{"component":"catalog.search"}"#;

    let signed = ring
        .sign(SnapshotPurpose::SeedV1, body, UnixMillis::new(50))
        .expect("active key signs");

    assert_eq!(signed.key_id().as_str(), "active-v1");
    assert_eq!(signed.signature().as_bytes().len(), 32);
    ring.verify(
        signed.key_id(),
        SnapshotPurpose::SeedV1,
        body,
        signed.signature(),
        UnixMillis::new(50),
    )
    .expect("matching body and purpose verify");
}

#[test]
fn seed_and_instance_purposes_cannot_be_substituted() {
    let ring = SnapshotKeyRing::new(record("active-v1", 0x22, 0, 100, 100), Vec::new())
        .expect("key ring is valid");
    let body = br#"{"same":"body"}"#;
    let seed = ring
        .sign(SnapshotPurpose::SeedV1, body, UnixMillis::new(10))
        .expect("seed purpose signs");
    let instance = ring
        .sign(SnapshotPurpose::InstanceV1, body, UnixMillis::new(10))
        .expect("instance purpose signs");

    assert_ne!(seed.signature().as_bytes(), instance.signature().as_bytes());
    let error = ring
        .verify(
            seed.key_id(),
            SnapshotPurpose::InstanceV1,
            body,
            seed.signature(),
            UnixMillis::new(10),
        )
        .expect_err("cross-purpose substitution must fail");
    assert_eq!(error.kind(), KeyErrorKind::SignatureMismatch);
}

#[test]
fn tampered_body_and_wrong_root_key_fail_closed() {
    let ring = SnapshotKeyRing::new(record("active-v1", 0x33, 0, 100, 100), Vec::new())
        .expect("key ring is valid");
    let signed = ring
        .sign(SnapshotPurpose::SeedV1, b"original", UnixMillis::new(10))
        .expect("active key signs");

    let tamper_error = ring
        .verify(
            signed.key_id(),
            SnapshotPurpose::SeedV1,
            b"tampered",
            signed.signature(),
            UnixMillis::new(10),
        )
        .expect_err("tampered body must fail");
    assert_eq!(tamper_error.kind(), KeyErrorKind::SignatureMismatch);

    let wrong_ring = SnapshotKeyRing::new(record("active-v1", 0x44, 0, 100, 100), Vec::new())
        .expect("key ring is valid");
    let wrong_key_error = wrong_ring
        .verify(
            signed.key_id(),
            SnapshotPurpose::SeedV1,
            b"original",
            signed.signature(),
            UnixMillis::new(10),
        )
        .expect_err("same key id with wrong root must fail");
    assert_eq!(wrong_key_error.kind(), KeyErrorKind::SignatureMismatch);
}

#[test]
fn rotation_overlap_verifies_old_keys_but_signs_only_with_current_key() {
    let old = record("old-v1", 0x55, 0, 50, 100);
    let old_ring = SnapshotKeyRing::new(old.clone(), Vec::new()).expect("old ring is valid");
    let old_signature = old_ring
        .sign(SnapshotPurpose::InstanceV1, b"state", UnixMillis::new(25))
        .expect("old key signs during its window");

    let current = record("current-v1", 0x66, 50, 150, 200);
    let rotated = SnapshotKeyRing::new(current, vec![old]).expect("rotation ring is valid");
    rotated
        .verify(
            old_signature.key_id(),
            SnapshotPurpose::InstanceV1,
            b"state",
            old_signature.signature(),
            UnixMillis::new(75),
        )
        .expect("old key verifies during overlap");

    let current_signature = rotated
        .sign(SnapshotPurpose::InstanceV1, b"state", UnixMillis::new(75))
        .expect("current key signs");
    assert_eq!(current_signature.key_id().as_str(), "current-v1");

    let retired = rotated
        .verify(
            old_signature.key_id(),
            SnapshotPurpose::InstanceV1,
            b"state",
            old_signature.signature(),
            UnixMillis::new(100),
        )
        .expect_err("old key is retired at its exclusive deadline");
    assert_eq!(retired.kind(), KeyErrorKind::KeyRetired);
}

#[test]
fn key_windows_unknown_ids_and_malformed_signatures_are_classified() {
    let ring = SnapshotKeyRing::new(record("future-v1", 0x77, 50, 100, 120), Vec::new())
        .expect("key ring is valid");
    let future_error = ring
        .sign(SnapshotPurpose::SeedV1, b"state", UnixMillis::new(49))
        .expect_err("not-yet-active signing key fails closed");
    assert_eq!(future_error.kind(), KeyErrorKind::KeyNotActive);

    let unknown = ring
        .verify(
            &KeyId::parse("unknown-v1").expect("test key id is valid"),
            SnapshotPurpose::SeedV1,
            b"state",
            &SnapshotSignature::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("32 zero bytes are a structurally valid signature"),
            UnixMillis::new(60),
        )
        .expect_err("unknown key fails closed");
    assert_eq!(unknown.kind(), KeyErrorKind::UnknownKey);

    let malformed =
        SnapshotSignature::parse("too-short").expect_err("signature must contain exactly 32 bytes");
    assert_eq!(malformed.kind(), KeyErrorKind::InvalidSignatureEncoding);
}

#[test]
fn weak_roots_invalid_windows_and_duplicate_ids_are_rejected() {
    let weak = RootKey::new(vec![0x88; 31]).expect_err("root key requires at least 256 bits");
    assert_eq!(weak.kind(), KeyErrorKind::WeakRootKey);
    assert_eq!(format!("{weak:?}"), "weak_root_key");
    let oversized = RootKey::new(vec![0x88; 65]).expect_err("root key configuration is bounded");
    assert_eq!(oversized.kind(), KeyErrorKind::WeakRootKey);

    let invalid_window = KeyRecord::new(
        KeyId::parse("invalid-v1").expect("test key id is valid"),
        RootKey::new(vec![0x99; 32]).expect("root key is strong"),
        UnixMillis::new(20),
        UnixMillis::new(20),
        UnixMillis::new(30),
    )
    .expect_err("signing window must be non-empty");
    assert_eq!(invalid_window.kind(), KeyErrorKind::InvalidKeyWindow);

    let duplicate = SnapshotKeyRing::new(
        record("duplicate-v1", 0xaa, 0, 100, 100),
        vec![record("duplicate-v1", 0xbb, 0, 50, 100)],
    )
    .expect_err("key ids are unique across the ring");
    assert_eq!(duplicate.kind(), KeyErrorKind::DuplicateKeyId);

    let too_many = (0_u8..8)
        .map(|index| record(&format!("verify-{index}"), index, 0, 50, 100))
        .collect();
    let bounded = SnapshotKeyRing::new(record("active-v1", 0xcc, 0, 100, 100), too_many)
        .expect_err("the key ring has a fixed record bound");
    assert_eq!(bounded.kind(), KeyErrorKind::TooManyKeys);
}
