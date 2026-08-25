//! Hostile-input, purpose-separation, and secret-containment tests for subscriptions.

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use suprnova_live::async_updates::{
    AuthorizationMemo, SubscriptionErrorKind, TopicName, TransportCredential,
    TrustedMountParameters,
};
use suprnova_live::crypto::{KeyErrorKind, KeyRecord, RootKey, SnapshotKeyRing, SnapshotPurpose};
use suprnova_live::identity::{KeyId, UnixMillis};
use zeroize::Zeroizing;

const CREDENTIAL_SENTINEL: &str = "async-transport-secret-sentinel-7cb7c19d";

fn key_ring() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("purpose-key-1").expect("key id"),
        RootKey::new(vec![0x53; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(50_000),
        UnixMillis::new(100_000),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

#[test]
fn descriptor_hkdf_purpose_is_separate_from_every_existing_authority() {
    let ring = key_ring();
    let body = br#"{"v":1}"#;
    let signed = ring
        .sign(
            SnapshotPurpose::AsyncSubscriptionV1,
            body,
            UnixMillis::new(1_000),
        )
        .expect("sign descriptor purpose");

    let hkdf = Hkdf::<Sha256>::new(Some(b"suprnova-live/snapshot-hkdf/v1"), &[0x53; 32]);
    let mut derived = [0_u8; 32];
    hkdf.expand(b"suprnova-live/async-subscription/v1", &mut derived)
        .expect("fixed HKDF output");
    let mut expected =
        <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(&derived).expect("fixed HMAC key");
    expected.update(body);
    assert_eq!(
        signed.signature().as_bytes(),
        expected.finalize().into_bytes().as_slice(),
        "the descriptor purpose string is an exact protocol contract"
    );

    for wrong in [
        SnapshotPurpose::SeedV1,
        SnapshotPurpose::InstanceV1,
        SnapshotPurpose::ChildParametersV1,
        SnapshotPurpose::UploadGrantV1,
    ] {
        assert!(
            ring.verify(
                signed.key_id(),
                wrong,
                body,
                signed.signature(),
                UnixMillis::new(1_001),
            )
            .is_err(),
            "descriptor MAC must not verify under {wrong:?}"
        );
    }
}

#[test]
fn descriptor_key_rotation_accepts_only_configured_overlapping_key_ids() {
    let old_signer = SnapshotKeyRing::new(
        KeyRecord::new(
            KeyId::parse("async-old").expect("key id"),
            RootKey::new(vec![0x61; 32]).expect("root"),
            UnixMillis::new(0),
            UnixMillis::new(2_000),
            UnixMillis::new(8_000),
        )
        .expect("old key"),
        Vec::new(),
    )
    .expect("old ring");
    let body = br#"{"v":1}"#;
    let signed = old_signer
        .sign(
            SnapshotPurpose::AsyncSubscriptionV1,
            body,
            UnixMillis::new(1_000),
        )
        .expect("old signature");
    let rotated = SnapshotKeyRing::new(
        KeyRecord::new(
            KeyId::parse("async-new").expect("key id"),
            RootKey::new(vec![0x62; 32]).expect("root"),
            UnixMillis::new(2_000),
            UnixMillis::new(7_000),
            UnixMillis::new(9_000),
        )
        .expect("new key"),
        vec![
            KeyRecord::new(
                KeyId::parse("async-old").expect("key id"),
                RootKey::new(vec![0x61; 32]).expect("root"),
                UnixMillis::new(0),
                UnixMillis::new(2_000),
                UnixMillis::new(8_000),
            )
            .expect("overlap key"),
        ],
    )
    .expect("rotated ring");

    rotated
        .verify(
            signed.key_id(),
            SnapshotPurpose::AsyncSubscriptionV1,
            body,
            signed.signature(),
            UnixMillis::new(3_000),
        )
        .expect("configured overlap verifies");

    assert_eq!(
        rotated
            .verify(
                &KeyId::parse("async-unknown").expect("key id"),
                SnapshotPurpose::AsyncSubscriptionV1,
                body,
                signed.signature(),
                UnixMillis::new(3_000),
            )
            .expect_err("an unconfigured key id never verifies")
            .kind(),
        KeyErrorKind::UnknownKey
    );
    assert_eq!(
        rotated
            .verify(
                signed.key_id(),
                SnapshotPurpose::AsyncSubscriptionV1,
                body,
                signed.signature(),
                UnixMillis::new(8_000),
            )
            .expect_err("overlap verification deadline is exclusive")
            .kind(),
        KeyErrorKind::KeyRetired
    );
}

#[test]
fn transport_credential_has_only_an_authority_bearing_accessor_and_redacted_debug() {
    let credential =
        TransportCredential::from_host_authority_bearer(CREDENTIAL_SENTINEL.as_bytes().to_vec())
            .expect("credential");

    assert_eq!(
        credential.expose_authorization_bearer(),
        CREDENTIAL_SENTINEL.as_bytes()
    );
    assert!(!format!("{credential:?}").contains(CREDENTIAL_SENTINEL));
}

#[test]
fn rejected_credential_input_is_zeroizing_before_bounded_validation() {
    assert_eq!(
        TransportCredential::from_zeroizing_host_authority_bearer(Zeroizing::new(vec![0xa5; 15]))
            .expect_err("short credentials are rejected while zeroizing storage owns the bytes")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );
}

#[test]
fn directive_interpolation_cannot_construct_topics_or_authorization_memos() {
    for interpolated in [
        "tenant/{principal}/orders",
        "tenant/{{ principal }}/orders",
        "tenant/${principal}/orders",
        "wss://example.invalid/{topic}",
    ] {
        assert!(
            TopicName::parse(interpolated).is_err(),
            "interpolated directive input must be rejected: {interpolated}"
        );
    }

    for untrusted in ["${principal}", "7/orders", "{tenant}", ""] {
        assert!(
            TrustedMountParameters::new(vec![("tenant".to_owned(), untrusted.to_owned())]).is_err(),
            "trusted topic parameters must be one validated canonical segment: {untrusted}"
        );
    }
    assert!(
        TrustedMountParameters::new(vec![
            ("tenant".to_owned(), "7".to_owned()),
            ("tenant".to_owned(), "8".to_owned()),
        ])
        .is_err(),
        "duplicate trusted mount parameter names are ambiguous"
    );

    assert_eq!(
        AuthorizationMemo::parse(&"m".repeat(513))
            .expect_err("authorization memo must be bounded")
            .kind(),
        SubscriptionErrorKind::InvalidAuthorizationMemo
    );
}

#[test]
fn topic_segments_reject_traversal_empty_and_encoded_separators_at_exact_bounds() {
    for hostile in [
        ".",
        "..",
        "/tenant",
        "tenant/",
        "tenant//orders",
        "tenant/./orders",
        "tenant/../orders",
        "tenant/%2F/orders",
        "tenant/%2f/orders",
    ] {
        assert!(
            TopicName::parse(hostile).is_err(),
            "topic segment grammar must reject {hostile:?}"
        );
    }

    assert!(TopicName::parse(&"t".repeat(256)).is_ok());
    assert!(TopicName::parse(&"t".repeat(257)).is_err());

    for hostile in [".", "..", "tenant/7", "%2F", "%2f"] {
        assert!(
            TrustedMountParameters::new(vec![("tenant".to_owned(), hostile.to_owned())]).is_err(),
            "mount parameter values are exactly one canonical topic segment: {hostile:?}"
        );
    }
    for hostile_name in [".", ".."] {
        assert!(
            TrustedMountParameters::new(vec![(hostile_name.to_owned(), "7".to_owned())]).is_err(),
            "mount parameter names cannot be traversal segments: {hostile_name:?}"
        );
    }
    assert!(TrustedMountParameters::new(vec![("tenant".to_owned(), "t".repeat(128))]).is_ok());
    assert!(TrustedMountParameters::new(vec![("tenant".to_owned(), "t".repeat(129))]).is_err());
}

#[test]
fn trusted_mount_topic_parameters_are_redacted_from_debug() {
    let sentinel = "private-tenant-parameter-sentinel";
    let parameters = TrustedMountParameters::new(vec![("tenant".to_owned(), sentinel.to_owned())])
        .expect("trusted mount parameter");

    assert!(!format!("{parameters:?}").contains(sentinel));
}
