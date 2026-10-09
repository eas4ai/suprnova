//! Integration tests for the `Crypt` facade.
//!
//! The encryption key lives in a process-wide `OnceLock`, so all tests
//! in this file share one key. We install it lazily under a mutex and
//! serialize the suite for deterministic ordering.

#[cfg(feature = "testing")]
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "testing")]
use serde::{Deserialize, Serialize};
use suprnova::Crypt;
#[cfg(feature = "testing")]
use suprnova::{AadVersion, CryptPurpose, EncryptionKey, KeyOrigin};

#[cfg(feature = "testing")]
static TEST_LOCK: Mutex<()> = Mutex::new(());
#[cfg(feature = "testing")]
static INSTALLED: OnceLock<()> = OnceLock::new();

#[cfg(feature = "testing")]
fn ensure_key() {
    INSTALLED.get_or_init(|| {
        // Install a deterministic-but-test-only key once.
        let key = EncryptionKey::generate();
        // Ignore the bool return - another test in another file might
        // have already installed a key; either way `encrypt_string`
        // works on whatever is installed.
        let _ = suprnova::crypto::_test_install_key(key);
    });
}

#[cfg(feature = "testing")]
#[test]
fn round_trip_string() {
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let wire = Crypt::encrypt_string(CryptPurpose::Cast, "hello, world").unwrap();
    assert_ne!(wire, "hello, world");
    let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire).unwrap();
    assert_eq!(plain, "hello, world");
}

#[cfg(feature = "testing")]
#[test]
fn tamper_detection() {
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let wire = Crypt::encrypt_string(CryptPurpose::Cast, "don't touch me").unwrap();
    let mut bytes = wire.into_bytes();
    let idx = bytes.len() - 1;
    // Flip one ASCII character to something else in the base64 alphabet
    bytes[idx] = if bytes[idx] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(bytes).unwrap();
    assert!(Crypt::decrypt_string(CryptPurpose::Cast, &tampered).is_err());
}

#[cfg(feature = "testing")]
#[test]
fn url_safe_no_padding() {
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    // Encrypt enough data that the base64 output covers multiple
    // alphabet positions; padding would show up at the end.
    let wire = Crypt::encrypt_string(
        CryptPurpose::Cast,
        "the quick brown fox jumps over the lazy dog -- multiple times",
    )
    .unwrap();
    assert!(!wire.contains('+'));
    assert!(!wire.contains('/'));
    assert!(!wire.contains('='));
}

#[cfg(feature = "testing")]
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Payload {
    user_id: i64,
    role: String,
}

#[cfg(feature = "testing")]
#[test]
fn encrypt_t_round_trip() {
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let payload = Payload {
        user_id: 7,
        role: "admin".to_string(),
    };
    let wire = Crypt::encrypt(CryptPurpose::Cast, &payload).unwrap();
    let decoded: Payload = Crypt::decrypt(CryptPurpose::Cast, &wire).unwrap();
    assert_eq!(decoded, payload);
}

#[cfg(feature = "testing")]
#[test]
fn cross_purpose_ciphertext_is_rejected() {
    // The domain-separation guarantee: ciphertext minted under one
    // purpose must NOT decrypt under another, even with the same key.
    // This is the property that blocks cross-surface ciphertext replay
    // (e.g. forging a cookie out of a stolen cursor payload, or
    // injecting a 2FA recovery blob into a cookie slot).
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let cookie_wire =
        Crypt::encrypt_string_for(CryptPurpose::Cookie, "suprnova_session", "session-id-42")
            .unwrap();
    // Same wire, every other purpose - must reject.
    for foreign in [
        CryptPurpose::Cursor,
        CryptPurpose::TwoFactorSecret,
        CryptPurpose::TwoFactorRecovery,
        CryptPurpose::Cast,
    ] {
        let err = Crypt::decrypt_string(foreign, &cookie_wire).unwrap_err();
        assert!(
            format!("{err}").contains("AEAD decrypt failed"),
            "{:?} should reject cookie ciphertext with AEAD failure, got: {err}",
            foreign
        );
    }
    // Sanity: original purpose and cookie name still decrypt.
    let plain =
        Crypt::decrypt_string_for(CryptPurpose::Cookie, "suprnova_session", &cookie_wire).unwrap();
    assert_eq!(plain, "session-id-42");
}

#[cfg(feature = "testing")]
#[test]
fn two_factor_secret_and_recovery_are_distinct_domains() {
    // Recovery codes and TOTP secret live in the same row but distinct
    // columns. Distinct purposes mean an attacker with write access to
    // one column cannot replay it into the other and have the read
    // path silently succeed.
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let secret_wire =
        Crypt::encrypt_string(CryptPurpose::TwoFactorSecret, "JBSWY3DPEHPK3PXP").unwrap();
    assert!(Crypt::decrypt_string(CryptPurpose::TwoFactorRecovery, &secret_wire).is_err());
    let recovery_wire =
        Crypt::encrypt_string(CryptPurpose::TwoFactorRecovery, "code-a\ncode-b\ncode-c").unwrap();
    assert!(Crypt::decrypt_string(CryptPurpose::TwoFactorSecret, &recovery_wire).is_err());
}

#[cfg(feature = "testing")]
#[test]
fn appears_encrypted_matches_real_ciphertext() {
    // A real `Crypt::encrypt_string` output is always recognised by
    // `appears_encrypted`. Mirrors the contract Laravel relies on in
    // its `EncryptCookies` middleware to skip already-encrypted
    // cookies on the egress pass.
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let wire = Crypt::encrypt_string(CryptPurpose::Cast, "hello").unwrap();
    assert!(Crypt::appears_encrypted(&wire));
    // Even the empty plaintext, encrypted, is recognised - the
    // ciphertext still carries nonce + tag.
    let wire_empty = Crypt::encrypt_string(CryptPurpose::Cast, "").unwrap();
    assert!(Crypt::appears_encrypted(&wire_empty));
}

#[cfg(feature = "testing")]
#[test]
fn encrypting_a_cookie_value_without_its_name_is_refused() {
    // A cookie value whose label has no cookie name in it would open in
    // every cookie, so no call writes one - with a key installed, and
    // whatever the value. The error names the calls that bind the name,
    // and nothing of the value.
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let payload = Payload {
        user_id: 4711,
        role: "refused-role".to_string(),
    };
    for err in [
        Crypt::encrypt_string(CryptPurpose::Cookie, "refused-plaintext").unwrap_err(),
        Crypt::encrypt(CryptPurpose::Cookie, &payload).unwrap_err(),
    ] {
        let message = format!("{err}");
        assert!(message.contains("Cookie::encrypted"), "{message}");
        assert!(message.contains("Crypt::encrypt_string_for"), "{message}");
        assert!(!message.contains("refused-plaintext"), "{message}");
        assert!(!message.contains("refused-role"), "{message}");
    }
}

#[cfg(feature = "testing")]
#[test]
fn reading_a_cookie_value_without_its_name_is_refused() {
    // Only a read with the cookie's name checks the name the value was
    // written for, so a read without it opens nothing - not even a value
    // that opens under its name. The error names the read to use, and
    // nothing of the value.
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let plaintext = r#"{"user_id":4711,"role":"refused-role"}"#;
    let wire =
        Crypt::encrypt_string_for(CryptPurpose::Cookie, "suprnova_session", plaintext).unwrap();
    let errors = [
        Crypt::decrypt_string(CryptPurpose::Cookie, &wire).unwrap_err(),
        Crypt::decrypt_string_with_origin(CryptPurpose::Cookie, &wire).unwrap_err(),
        Crypt::decrypt::<Payload>(CryptPurpose::Cookie, &wire).unwrap_err(),
        Crypt::decrypt_with_origin::<Payload>(CryptPurpose::Cookie, &wire).unwrap_err(),
    ];
    for err in errors {
        let message = format!("{err}");
        assert!(message.contains("Cookie::read_encrypted_for"), "{message}");
        assert!(!message.contains(&wire), "{message}");
        assert!(!message.contains("refused-role"), "{message}");
    }
    let opened =
        Crypt::decrypt_string_for(CryptPurpose::Cookie, "suprnova_session", &wire).unwrap();
    assert_eq!(opened, plaintext);
}

#[cfg(feature = "testing")]
#[test]
fn a_stored_value_without_a_context_still_opens_with_one() {
    // The fallback stays for the purposes that hold stored values: a cast
    // column written without a context opens when it is read with one,
    // and its origin says that it is to be written again.
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let wire = Crypt::encrypt_string(CryptPurpose::Cast, "stored-without-context").unwrap();
    let (plain, origin) =
        Crypt::decrypt_string_for_with_origin(CryptPurpose::Cast, "users.secret", &wire).unwrap();
    assert_eq!(plain, "stored-without-context");
    assert_eq!(origin.key, KeyOrigin::Current);
    assert_eq!(origin.aad, AadVersion::Legacy);
    assert!(origin.needs_reencryption());
}

#[test]
fn appears_encrypted_rejects_plaintext_and_short_payloads() {
    // No Crypt::init required - `appears_encrypted` is a static
    // shape check, never touches the keyring.
    assert!(!Crypt::appears_encrypted("plain text with spaces"));
    assert!(!Crypt::appears_encrypted(""));
    // Valid base64 but too short to be a nonce+tag (28 bytes min).
    assert!(!Crypt::appears_encrypted("YWJj")); // "abc" - 3 bytes
    // Non-base64-url characters.
    assert!(!Crypt::appears_encrypted("not/valid+base64="));
}

#[cfg(feature = "testing")]
#[test]
fn previous_key_count_accessors_agree() {
    // The bool/usize accessors must always agree. We can't assert
    // exact zero because other test files in the same process may
    // have installed a keyring with previous keys.
    let _g = TEST_LOCK.lock().unwrap();
    ensure_key();
    let n = Crypt::previous_key_count();
    assert_eq!(Crypt::has_previous_keys(), n > 0);
}
