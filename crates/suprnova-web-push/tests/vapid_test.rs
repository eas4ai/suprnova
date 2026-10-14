use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use suprnova_web_push::{VapidKey, VapidSigner};

const LEGACY_JWT_SIMPLE_PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgaWJBcVYaYzQN4OfY\n\
afKgVJJVjhoEhotqn4VKhmeIGI2hRANCAAQcrP+1Xy8s79idies3SyaBFSRSgC3u\n\
oJkWBoE32DnPf8SBpESSME1+9mrBF77+g6jQjxVfK1L59hjdRHApBI4P\n\
-----END PRIVATE KEY-----\n";

#[test]
fn vapid_key_generates_p256_keypair() {
    let key = VapidKey::generate();
    let pub_b64 = key.public_key_uncompressed_b64url();
    assert_eq!(
        pub_b64.len(),
        87,
        "VAPID public key must be 87-char base64url"
    );
    assert!(
        pub_b64.starts_with("B"),
        "uncompressed P-256 point starts with 0x04 → base64url 'B'"
    );
}

#[test]
fn vapid_key_imports_deterministic_raw_scalar_and_derives_public_key() {
    let mut raw = [0_u8; 32];
    raw[31] = 1;

    let key = VapidKey::from_bytes(&raw).expect("scalar one is a valid P-256 private key");
    let public_key = base64_url_no_pad_decode(&key.public_key_uncompressed_b64url()).unwrap();
    let expected = p256::SecretKey::from_slice(&raw)
        .unwrap()
        .public_key()
        .to_encoded_point(false);

    assert_eq!(public_key, expected.as_bytes());
}

#[test]
fn vapid_key_rejects_wrong_raw_key_lengths() {
    for raw in [&[][..], &[7_u8; 31], &[7_u8; 33]] {
        let err = VapidKey::from_bytes(raw).unwrap_err();
        assert!(
            err.to_string().contains("exactly 32 bytes"),
            "unexpected error for {} bytes: {err}",
            raw.len()
        );
    }
}

#[test]
fn vapid_key_rejects_zero_and_out_of_range_scalars() {
    for raw in [[0_u8; 32], [0xff_u8; 32]] {
        let err = VapidKey::from_bytes(&raw).unwrap_err();
        assert!(
            err.to_string().contains("invalid P-256 private key scalar"),
            "unexpected error: {err}"
        );
    }
}

#[test]
fn vapid_key_imports_legacy_jwt_simple_pkcs8_pem() {
    let key = VapidKey::from_pem(LEGACY_JWT_SIMPLE_PKCS8_PEM)
        .expect("jwt-simple PKCS#8 PEM must remain importable");

    assert_eq!(key.to_pem().unwrap(), LEGACY_JWT_SIMPLE_PKCS8_PEM);
}

#[test]
fn vapid_key_pkcs8_pem_round_trips() {
    let original = VapidKey::generate();
    let public_key = original.public_key_uncompressed_b64url();
    let pem = original.to_pem().unwrap();
    let imported = VapidKey::from_pem(&pem).unwrap();

    assert_eq!(imported.public_key_uncompressed_b64url(), public_key);
}

#[test]
fn vapid_key_debug_redacts_private_material() {
    let key = VapidKey::from_bytes(&[0x42_u8; 32]).unwrap();
    let signer = VapidSigner::new(key);

    assert_eq!(
        format!("{signer:?}"),
        "VapidSigner { key: VapidKey([REDACTED]) }"
    );
}

#[test]
fn vapid_signer_produces_jwt_with_three_segments() {
    let key = VapidKey::generate();
    let signer = VapidSigner::new(key);
    let jwt = signer
        .sign("https://example.org", "mailto:admin@example.org", 12 * 3600)
        .unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT must have 3 dot-separated segments");
    let header_bytes = base64_url_no_pad_decode(parts[0]).unwrap();
    let header: serde_json::Value = serde_json::from_slice(&header_bytes).unwrap();
    assert_eq!(header["typ"], "JWT");
    assert_eq!(header["alg"], "ES256");
}

#[test]
fn vapid_signer_emits_exact_es256_header() {
    let signer = VapidSigner::new(VapidKey::generate());
    let jwt = signer
        .sign("https://example.org", "mailto:admin@example.org", 3600)
        .unwrap();
    let header: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&base64_url_no_pad_decode(jwt.split('.').next().unwrap()).unwrap())
            .unwrap();

    let keys: std::collections::BTreeSet<&str> = header.keys().map(String::as_str).collect();
    assert_eq!(keys, ["alg", "typ"].into_iter().collect());
    assert_eq!(header["alg"], "ES256");
    assert_eq!(header["typ"], "JWT");
}

#[test]
fn vapid_signer_emits_verifiable_64_byte_p1363_signature() {
    let mut raw = [0_u8; 32];
    raw[31] = 1;
    let key = VapidKey::from_bytes(&raw).unwrap();
    let public_key = base64_url_no_pad_decode(&key.public_key_uncompressed_b64url()).unwrap();
    let signer = VapidSigner::new(key);
    let jwt = signer
        .sign("https://example.org", "mailto:admin@example.org", 3600)
        .unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    let signature_bytes = base64_url_no_pad_decode(parts[2]).unwrap();

    assert!(
        parts.iter().all(|segment| !segment.contains('=')),
        "compact JWS segments must use unpadded base64url"
    );
    assert_eq!(signature_bytes.len(), 64, "JOSE ES256 uses raw r || s");
    let signature = Signature::from_slice(&signature_bytes).unwrap();
    let verifying_key = VerifyingKey::from_sec1_bytes(&public_key).unwrap();
    verifying_key
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("signature must verify over ASCII header.payload");
}

#[test]
fn vapid_signer_claims_have_aud_sub_exp() {
    let key = VapidKey::generate();
    let signer = VapidSigner::new(key);
    let jwt = signer
        .sign("https://fcm.googleapis.com", "mailto:a@b.com", 12 * 3600)
        .unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    let claims_bytes = base64_url_no_pad_decode(parts[1]).unwrap();
    let claims: serde_json::Value = serde_json::from_slice(&claims_bytes).unwrap();
    assert_eq!(claims["aud"], "https://fcm.googleapis.com");
    assert_eq!(claims["sub"], "mailto:a@b.com");
    let exp = claims["exp"].as_i64().unwrap();
    let now = chrono::Utc::now().timestamp();
    assert!(
        exp > now && exp <= now + 12 * 3600 + 5,
        "exp must be ~12h in the future"
    );
}

#[test]
fn vapid_signer_emits_exact_rfc8292_claim_set() {
    // Lock the JWT claim set down to {iat, exp, sub, aud}. RFC 8292 §2
    // requires aud/sub/exp; we include iat for replay-window tracking.
    // We deliberately DROP `nbf` (jwt-simple defaults to it) because push
    // services with negative clock skew reject the request before nbf
    // passes - observed against some non-FCM endpoints.
    //
    // We also assert NO unexpected extras (e.g. jti, iss, nonce) so a
    // future signer refactor that re-introduces extras
    // fails this test rather than silently shipping a wider claim set.
    let key = VapidKey::generate();
    let signer = VapidSigner::new(key);
    let jwt = signer
        .sign("https://example.org", "mailto:admin@example.org", 12 * 3600)
        .unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    let claims_bytes = base64_url_no_pad_decode(parts[1]).unwrap();
    let claims: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&claims_bytes).unwrap();

    let keys: std::collections::BTreeSet<&str> = claims.keys().map(String::as_str).collect();
    let expected: std::collections::BTreeSet<&str> =
        ["iat", "exp", "sub", "aud"].into_iter().collect();
    assert_eq!(
        keys, expected,
        "claim set must be exactly {{iat, exp, sub, aud}} - extras risk clock-skew rejection on strict push services"
    );

    // nbf rejection is the regression we're guarding - explicit absence.
    assert!(
        !claims.contains_key("nbf"),
        "nbf must be absent - push services with negative clock skew reject otherwise"
    );
}

#[test]
fn vapid_signer_uses_integer_epoch_seconds_and_exact_ttl() {
    const TTL_SECS: i64 = 3600;
    let signer = VapidSigner::new(VapidKey::generate());
    let jwt = signer
        .sign("https://example.org", "mailto:admin@example.org", TTL_SECS)
        .unwrap();
    let claims: serde_json::Value =
        serde_json::from_slice(&base64_url_no_pad_decode(jwt.split('.').nth(1).unwrap()).unwrap())
            .unwrap();
    let iat = claims["iat"].as_i64().expect("iat must be an integer");
    let exp = claims["exp"].as_i64().expect("exp must be an integer");

    assert_eq!(exp, iat + TTL_SECS);
}

fn base64_url_no_pad_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)
}

// ---------------------------------------------------------------------------
// VAPID TTL bounds - RFC 8292 caps the JWT lifetime at 24 hours. Zero /
// negative TTLs would produce already-expired tokens; the previous `as u64`
// cast quietly wrapped negatives into multi-century lifetimes.
// ---------------------------------------------------------------------------

#[test]
fn sign_rejects_zero_ttl() {
    let signer = VapidSigner::new(VapidKey::generate());
    let err = signer
        .sign("https://example.org", "mailto:a@b.com", 0)
        .unwrap_err();
    assert!(
        format!("{err}").contains("TTL must be positive"),
        "got: {err}"
    );
}

#[test]
fn sign_rejects_negative_ttl() {
    let signer = VapidSigner::new(VapidKey::generate());
    let err = signer
        .sign("https://example.org", "mailto:a@b.com", -1)
        .unwrap_err();
    assert!(
        format!("{err}").contains("TTL must be positive"),
        "got: {err}"
    );
}

#[test]
fn sign_rejects_ttl_above_24h() {
    let signer = VapidSigner::new(VapidKey::generate());
    let err = signer
        .sign("https://example.org", "mailto:a@b.com", 24 * 3600 + 1)
        .unwrap_err();
    assert!(format!("{err}").contains("exceeds RFC 8292"), "got: {err}");
}

#[test]
fn sign_accepts_exactly_24h_ttl() {
    let signer = VapidSigner::new(VapidKey::generate());
    let jwt = signer
        .sign("https://example.org", "mailto:a@b.com", 24 * 3600)
        .expect("24h boundary must be accepted");
    assert_eq!(jwt.split('.').count(), 3, "valid JWT must have 3 segments");
}
