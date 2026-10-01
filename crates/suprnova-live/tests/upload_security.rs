//! Upload transfer-authority and redaction contract tests.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::host::{
    HostScopeFacts, PrincipalFingerprint, SessionFingerprint, TenantFingerprint,
};
use suprnova_live::identity::{ComponentName, KeyId, ModelField, ScopeFingerprint, UnixMillis};
use suprnova_live::upload::{
    TransferGrant, TransferGrantCodec, TransferGrantRequest, TransferGrantScope, UploadErrorKind,
    UploadHandle,
};

const HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-123456789abc";
const OTHER_HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-abcdef012345";
const ROOT_SENTINEL: &[u8; 32] = b"upload-grant-root-secret-0000001";

fn fingerprint(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn host_scope(scope: u8, session: u8, principal: u8, tenant: u8) -> HostScopeFacts {
    HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&fingerprint(scope)).expect("scope"),
        Some(SessionFingerprint::from_bytes(&fingerprint(session)).expect("session")),
        Some(PrincipalFingerprint::from_bytes(&fingerprint(principal)).expect("principal")),
        Some(TenantFingerprint::from_bytes(&fingerprint(tenant)).expect("tenant")),
    )
}

fn codec() -> TransferGrantCodec {
    let active = KeyRecord::new(
        KeyId::parse("upload-key-1").expect("key id"),
        RootKey::new(ROOT_SENTINEL.to_vec()).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(50_000),
        UnixMillis::new(100_000),
    )
    .expect("key record");
    TransferGrantCodec::new(SnapshotKeyRing::new(active, Vec::new()).expect("key ring"))
}

fn authority() -> TransferGrantScope {
    TransferGrantScope::new(
        UploadHandle::parse(HANDLE).expect("handle"),
        ComponentName::parse("profile.edit").expect("component"),
        ModelField::parse("avatar").expect("field"),
        host_scope(1, 2, 3, 4),
        1,
    )
}

fn issue() -> (TransferGrantCodec, TransferGrantScope, TransferGrant) {
    let codec = codec();
    let authority = authority();
    let issued = codec
        .issue(
            TransferGrantRequest::new(authority.clone(), UnixMillis::new(20_000)),
            UnixMillis::new(1_000),
        )
        .expect("issue grant");
    let grant = TransferGrant::parse(issued.grant().expose_bearer()).expect("wire round trip");
    (codec, authority, grant)
}

fn canonically_tamper_signature(
    grant: &TransferGrant,
    byte_index: usize,
    bit: u8,
) -> TransferGrant {
    let mut parts = grant.expose_bearer().split('.').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4);
    let encoded = parts[3];
    let mut signature = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("canonical signature");
    assert_eq!(signature.len(), 32);
    assert_eq!(URL_SAFE_NO_PAD.encode(&signature), encoded);
    signature[byte_index] ^= bit;
    let forged = URL_SAFE_NO_PAD.encode(signature);
    assert_eq!(forged.len(), encoded.len());
    parts[3] = &forged;
    TransferGrant::parse(&parts.join(".")).expect("canonical forged transfer grant")
}

#[test]
fn transfer_grant_round_trip_binds_every_authority_fact() {
    let (codec, authority, grant) = issue();

    let verified = codec
        .verify(&grant, &authority, UnixMillis::new(1_001))
        .expect("verify grant");

    assert_eq!(verified.handle(), authority.handle());
    assert_eq!(verified.component(), authority.component());
    assert_eq!(verified.field(), authority.field());
    assert_eq!(verified.scope(), authority.host_scope());
    assert_eq!(verified.expires_at(), UnixMillis::new(20_000));
    assert_eq!(verified.upload_protocol(), 1);
}

#[test]
fn transfer_grant_rejects_cross_upload_component_field_and_host_scope_reuse() {
    let (codec, authority, grant) = issue();
    let cases = [
        TransferGrantScope::new(
            UploadHandle::parse(OTHER_HANDLE).expect("other handle"),
            authority.component().clone(),
            authority.field().clone(),
            authority.host_scope().clone(),
            1,
        ),
        TransferGrantScope::new(
            authority.handle().clone(),
            ComponentName::parse("profile.other").expect("component"),
            authority.field().clone(),
            authority.host_scope().clone(),
            1,
        ),
        TransferGrantScope::new(
            authority.handle().clone(),
            authority.component().clone(),
            ModelField::parse("resume").expect("field"),
            authority.host_scope().clone(),
            1,
        ),
        TransferGrantScope::new(
            authority.handle().clone(),
            authority.component().clone(),
            authority.field().clone(),
            host_scope(9, 2, 3, 4),
            1,
        ),
        TransferGrantScope::new(
            authority.handle().clone(),
            authority.component().clone(),
            authority.field().clone(),
            host_scope(1, 9, 3, 4),
            1,
        ),
        TransferGrantScope::new(
            authority.handle().clone(),
            authority.component().clone(),
            authority.field().clone(),
            host_scope(1, 2, 9, 4),
            1,
        ),
        TransferGrantScope::new(
            authority.handle().clone(),
            authority.component().clone(),
            authority.field().clone(),
            host_scope(1, 2, 3, 9),
            1,
        ),
    ];

    for wrong_scope in cases {
        assert_eq!(
            codec
                .verify(&grant, &wrong_scope, UnixMillis::new(1_001))
                .expect_err("cross-scope grant reuse must fail")
                .kind(),
            UploadErrorKind::ScopeMismatch
        );
    }
}

#[test]
fn transfer_grant_expiry_protocol_and_tampering_fail_closed() {
    let (codec, authority, grant) = issue();

    assert_eq!(
        codec
            .verify(&grant, &authority, UnixMillis::new(20_000))
            .expect_err("expiry is exclusive")
            .kind(),
        UploadErrorKind::GrantExpired
    );

    let wrong_protocol = TransferGrantScope::new(
        authority.handle().clone(),
        authority.component().clone(),
        authority.field().clone(),
        authority.host_scope().clone(),
        2,
    );
    assert_eq!(
        codec
            .verify(&grant, &wrong_protocol, UnixMillis::new(1_001))
            .expect_err("protocol substitution must fail")
            .kind(),
        UploadErrorKind::ScopeMismatch
    );

    let tampered = canonically_tamper_signature(&grant, 0, 0x01);
    assert_eq!(
        codec
            .verify(&tampered, &authority, UnixMillis::new(1_001))
            .expect_err("tampered grant must fail")
            .kind(),
        UploadErrorKind::InvalidGrant
    );
}

#[test]
fn every_canonical_signature_bit_mutation_reaches_exact_mac_rejection() {
    let (codec, authority, grant) = issue();

    for byte_index in 0..32 {
        for bit in [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80] {
            let tampered = canonically_tamper_signature(&grant, byte_index, bit);
            assert_eq!(
                codec
                    .verify(&tampered, &authority, UnixMillis::new(1_001))
                    .expect_err("canonical signature mutation must fail MAC verification")
                    .kind(),
                UploadErrorKind::InvalidGrant,
                "byte {byte_index} bit {bit:#04x}"
            );
        }
    }
}

#[test]
fn transfer_grant_is_bounded_and_implicitly_redacted() {
    let codec = codec();
    let authority = authority();
    let issued = codec
        .issue(
            TransferGrantRequest::new(authority, UnixMillis::new(20_000)),
            UnixMillis::new(1_000),
        )
        .expect("issue grant");
    let sentinel = String::from_utf8_lossy(ROOT_SENTINEL);

    for debug in [
        format!("{codec:?}"),
        format!("{issued:?}"),
        format!("{:?}", issued.grant()),
    ] {
        assert!(!debug.contains(sentinel.as_ref()));
        assert!(!debug.contains(issued.grant().expose_bearer()));
    }
    assert!(
        !serde_json::to_string(issued.handle())
            .expect("serialize non-authority handle")
            .contains(issued.grant().expose_bearer())
    );

    for malformed in [
        "",
        "v1.only-three.parts",
        "v2.key.body.signature",
        &"x".repeat(4_097),
    ] {
        assert_eq!(
            TransferGrant::parse(malformed)
                .expect_err("malformed token must fail")
                .kind(),
            UploadErrorKind::InvalidGrantEncoding
        );
    }
}
