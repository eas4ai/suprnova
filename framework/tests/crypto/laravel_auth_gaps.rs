//! Contracts for the encrypter that holds its own key and for the key
//! generators that return the refusal of the random source.
//!
//! The tests that read `Crypt` run alone in a child process (see
//! `own_process`): the key ring is sealed by its first installer, so the
//! child installs the one key the test names, from `APP_KEY`, the way an
//! application boots.

use serde::{Deserialize, Serialize};
use suprnova::{Crypt, CryptPurpose, Encrypter, EncryptionKey};

/// The variable that hands the child the second key, which `Crypt` never
/// installs.
const OTHER_KEY: &str = "SUPRNOVA_TEST_ENCRYPTER_OTHER_KEY";

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Credentials {
    tenant_id: i64,
    token: String,
}

fn credentials() -> Credentials {
    Credentials {
        tenant_id: 42,
        token: "tok_live_123".to_string(),
    }
}

fn key(byte: u8) -> EncryptionKey {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([byte; 32]);
    EncryptionKey::from_base64(&encoded).expect("a 32-byte key")
}

/// Run the `_child` test `name` alone in a child process whose `APP_KEY`
/// is `app_key`, and fail unless it ran and passed.
fn run_alone_under_app_key(name: &str, app_key: &EncryptionKey, other: &EncryptionKey) {
    let child = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command(name)
            .env("APP_ENV", "production")
            .env("APP_KEY", app_key.to_base64())
            .env(OTHER_KEY, other.to_base64())
            .env_remove("APP_KEY_PREVIOUS")
            .env_remove("APP_PREVIOUS_KEYS")
            .spawn()
            .expect("spawn the child process")
    };
    let output = child
        .wait_with_output()
        .expect("wait for the child process");
    crate::own_process::assert_child_passed(&output);
}

/// The two keys a child reads: the `APP_KEY` its boot installs, and the
/// other key no `Crypt` call ever sees.
fn child_keys() -> (EncryptionKey, EncryptionKey) {
    let app_key = std::env::var("APP_KEY").expect("the parent sets APP_KEY");
    let other = std::env::var(OTHER_KEY).expect("the parent sets the other key");
    (
        EncryptionKey::from_base64(&app_key).expect("APP_KEY decodes"),
        EncryptionKey::from_base64(&other).expect("the other key decodes"),
    )
}

#[test]
fn an_encrypter_opens_what_an_encrypter_of_the_same_key_wrote() {
    let writer = Encrypter::new(key(1));
    let reader = Encrypter::new(key(1));

    let wire = writer
        .encrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "tok_live_123")
        .unwrap();
    assert_ne!(wire, "tok_live_123");
    assert!(Crypt::appears_encrypted(&wire));
    assert_eq!(
        reader
            .decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", &wire)
            .unwrap(),
        "tok_live_123"
    );

    // A fresh nonce for each call: the same text twice is two wires, and
    // both open.
    let again = writer
        .encrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "tok_live_123")
        .unwrap();
    assert_ne!(again, wire);
    assert_eq!(
        reader
            .decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", &again)
            .unwrap(),
        "tok_live_123"
    );

    // The empty text and a cookie value under its name round-trip too.
    let empty = writer
        .encrypt_string_for(CryptPurpose::Cursor, "", "")
        .unwrap();
    assert_eq!(
        reader
            .decrypt_string_for(CryptPurpose::Cursor, "", &empty)
            .unwrap(),
        ""
    );
    let cookie = writer
        .encrypt_string_for(CryptPurpose::Cookie, "tenant_session", "session-id")
        .unwrap();
    assert_eq!(
        reader
            .decrypt_string_for(CryptPurpose::Cookie, "tenant_session", &cookie)
            .unwrap(),
        "session-id"
    );

    // A serialised value round-trips, and so does a clone of the
    // encrypter.
    let wire = writer.encrypt(CryptPurpose::Cast, &credentials()).unwrap();
    let decoded: Credentials = reader.clone().decrypt(CryptPurpose::Cast, &wire).unwrap();
    assert_eq!(decoded, credentials());
}

#[test]
fn an_encrypter_of_another_key_opens_nothing() {
    let writer = Encrypter::new(key(1));
    let other = Encrypter::new(key(2));

    let wire = writer
        .encrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "tok_live_123")
        .unwrap();
    let err = other
        .decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", &wire)
        .unwrap_err();
    assert!(err.to_string().contains("AEAD decrypt failed"), "{err}");

    let wire = writer.encrypt(CryptPurpose::Cast, &credentials()).unwrap();
    let err = other
        .decrypt::<Credentials>(CryptPurpose::Cast, &wire)
        .unwrap_err();
    assert!(err.to_string().contains("AEAD decrypt failed"), "{err}");
}

#[test]
fn a_value_opens_under_its_own_purpose_and_context_only() {
    let encrypter = Encrypter::new(key(3));
    let wire = encrypter
        .encrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "tok_live_123")
        .unwrap();

    for (purpose, context) in [
        (CryptPurpose::Cursor, "tenants.api_token"),
        (CryptPurpose::TwoFactorSecret, "tenants.api_token"),
        (CryptPurpose::Cookie, "tenants.api_token"),
        (CryptPurpose::Cast, "tenants.webhook_secret"),
        (CryptPurpose::Cast, ""),
    ] {
        let err = encrypter
            .decrypt_string_for(purpose, context, &wire)
            .expect_err("a value must not open under another purpose or context");
        assert!(err.to_string().contains("AEAD decrypt failed"), "{err}");
    }

    // A cookie opens under the name it was written for and no other.
    let cookie = encrypter
        .encrypt_string_for(CryptPurpose::Cookie, "tenant_session", "session-id")
        .unwrap();
    assert!(
        encrypter
            .decrypt_string_for(CryptPurpose::Cookie, "other_cookie", &cookie)
            .is_err()
    );

    // A serialised value opens under its own purpose only.
    let wire = encrypter
        .encrypt(CryptPurpose::Cursor, &credentials())
        .unwrap();
    assert!(
        encrypter
            .decrypt::<Credentials>(CryptPurpose::Cast, &wire)
            .is_err()
    );
}

#[test]
fn a_changed_or_malformed_wire_is_refused() {
    let encrypter = Encrypter::new(key(4));
    let wire = encrypter
        .encrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "tok_live_123")
        .unwrap();

    let mut bytes = wire.into_bytes();
    let last = bytes.len() - 1;
    bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
    let changed = String::from_utf8(bytes).unwrap();
    assert!(
        encrypter
            .decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", &changed)
            .is_err()
    );

    let err = encrypter
        .decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "not/base64+=")
        .unwrap_err();
    assert!(
        err.to_string().contains("Encrypter base64 decode failed"),
        "{err}"
    );

    let err = encrypter
        .decrypt::<Credentials>(CryptPurpose::Cast, "YWJj")
        .unwrap_err();
    assert!(err.to_string().contains("AEAD wire too short"), "{err}");

    // A value that is JSON of another type names the mistake and quotes
    // none of the value.
    let wire = encrypter
        .encrypt(CryptPurpose::Cast, &"tok_live_123")
        .unwrap();
    let err = encrypter
        .decrypt::<Credentials>(CryptPurpose::Cast, &wire)
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("Encrypter JSON decode failed"),
        "{message}"
    );
    assert!(!message.contains("tok_live_123"), "{message}");
}

#[test]
fn an_encrypter_refuses_a_cookie_value_without_its_name() {
    let encrypter = Encrypter::new(key(5));
    let cookie = encrypter
        .encrypt_string_for(CryptPurpose::Cookie, "tenant_session", "session-id")
        .unwrap();

    let err = encrypter
        .encrypt(CryptPurpose::Cookie, &credentials())
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("Encrypter::encrypt_string_for"),
        "{message}"
    );
    assert!(!message.contains("tok_live_123"), "{message}");

    let err = encrypter
        .decrypt::<String>(CryptPurpose::Cookie, &cookie)
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("Encrypter::decrypt_string_for"),
        "{message}"
    );
    assert!(!message.contains(&cookie), "{message}");
}

#[test]
fn an_encrypter_never_prints_its_key() {
    let secret = key(0xAB);
    let encoded = secret.to_base64();
    let printed = format!("{:?}", Encrypter::new(secret));
    assert!(printed.contains("REDACTED"), "{printed}");
    assert!(!printed.contains(&encoded), "{printed}");
    assert!(!printed.contains("171"), "{printed}");
}

#[test]
fn an_encryption_key_is_32_bytes_and_nothing_else() {
    use base64::Engine as _;
    let encode =
        |len: usize| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![7u8; len]);

    let err = EncryptionKey::from_base64(&encode(16)).unwrap_err();
    assert!(
        err.to_string().contains("must decode to 32 bytes, got 16"),
        "{err}"
    );
    for len in [0, 24, 31, 33, 64] {
        assert!(
            EncryptionKey::from_base64(&encode(len)).is_err(),
            "a {len}-byte key must be refused"
        );
    }
    let accepted = EncryptionKey::from_base64(&encode(32)).unwrap();
    assert_eq!(accepted.as_bytes(), &[7u8; 32]);
}

#[test]
fn the_key_generators_return_a_fresh_32_byte_key_on_each_call() {
    let first = EncryptionKey::try_generate().unwrap();
    let second = EncryptionKey::try_generate().unwrap();
    let third = Crypt::try_generate_key().unwrap();
    let fourth = Crypt::try_generate_key().unwrap();

    let keys = [&first, &second, &third, &fourth];
    for (i, a) in keys.iter().enumerate() {
        // 32 bytes encode to 43 characters without padding, and decode
        // back to the same key.
        let encoded = a.to_base64();
        assert_eq!(encoded.len(), 43);
        assert_eq!(
            EncryptionKey::from_base64(&encoded).unwrap().as_bytes(),
            a.as_bytes()
        );
        for b in &keys[i + 1..] {
            assert_ne!(a.as_bytes(), b.as_bytes(), "two calls returned one key");
        }
    }

    // A generated key drives an encrypter.
    let encrypter = Encrypter::new(third);
    let wire = encrypter
        .encrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "tok_live_123")
        .unwrap();
    assert!(
        Encrypter::new(fourth)
            .decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", &wire)
            .is_err()
    );
    assert_eq!(
        encrypter
            .decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", &wire)
            .unwrap(),
        "tok_live_123"
    );
}

#[test]
fn a_value_crypt_wrote_under_app_key_opens_with_an_encrypter_of_that_key() {
    run_alone_under_app_key(
        "laravel_auth_gaps::a_value_crypt_wrote_under_app_key_opens_with_an_encrypter_of_that_key_child",
        &EncryptionKey::try_generate().unwrap(),
        &EncryptionKey::try_generate().unwrap(),
    );
}

#[test]
fn a_value_crypt_wrote_under_app_key_opens_with_an_encrypter_of_that_key_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let (app_key, other_key) = child_keys();
    suprnova::boot::initialize_crypt_or_exit();
    assert!(Crypt::is_initialized());

    let same = Encrypter::new(app_key);
    let other = Encrypter::new(other_key);

    // A contexted string, for a stored value and for a cookie.
    let wire = Crypt::encrypt_string_for(CryptPurpose::Cast, "users.ssn", "123-45-6789").unwrap();
    assert_eq!(
        same.decrypt_string_for(CryptPurpose::Cast, "users.ssn", &wire)
            .unwrap(),
        "123-45-6789"
    );
    assert!(
        other
            .decrypt_string_for(CryptPurpose::Cast, "users.ssn", &wire)
            .is_err()
    );
    assert!(
        same.decrypt_string_for(CryptPurpose::Cast, "users.email", &wire)
            .is_err()
    );
    let cookie =
        Crypt::encrypt_string_for(CryptPurpose::Cookie, "suprnova_session", "session-id").unwrap();
    assert_eq!(
        same.decrypt_string_for(CryptPurpose::Cookie, "suprnova_session", &cookie)
            .unwrap(),
        "session-id"
    );

    // A value stored without a context opens with one, as it does
    // through `Crypt`.
    let stored = Crypt::encrypt_string(CryptPurpose::Cast, "stored-before").unwrap();
    assert_eq!(
        same.decrypt_string_for(CryptPurpose::Cast, "users.ssn", &stored)
            .unwrap(),
        "stored-before"
    );

    // A serialised value.
    let wire = Crypt::encrypt(CryptPurpose::Cast, &credentials()).unwrap();
    assert_eq!(
        same.decrypt::<Credentials>(CryptPurpose::Cast, &wire)
            .unwrap(),
        credentials()
    );
    assert!(
        other
            .decrypt::<Credentials>(CryptPurpose::Cast, &wire)
            .is_err()
    );

    // The other way: what the encrypter of `APP_KEY` writes, `Crypt`
    // opens.
    let wire = same
        .encrypt_string_for(CryptPurpose::Cast, "users.ssn", "987-65-4321")
        .unwrap();
    assert_eq!(
        Crypt::decrypt_string_for(CryptPurpose::Cast, "users.ssn", &wire).unwrap(),
        "987-65-4321"
    );
    let wire = same.encrypt(CryptPurpose::Cast, &credentials()).unwrap();
    assert_eq!(
        Crypt::decrypt::<Credentials>(CryptPurpose::Cast, &wire).unwrap(),
        credentials()
    );
}

#[test]
fn building_an_encrypter_leaves_crypt_as_it_was() {
    run_alone_under_app_key(
        "laravel_auth_gaps::building_an_encrypter_leaves_crypt_as_it_was_child",
        &EncryptionKey::try_generate().unwrap(),
        &EncryptionKey::try_generate().unwrap(),
    );
}

#[test]
fn building_an_encrypter_leaves_crypt_as_it_was_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let (app_key, other_key) = child_keys();

    // Before boot: an encrypter and a generated key install nothing.
    assert!(!Crypt::is_initialized());
    let early = Encrypter::new(other_key.clone());
    let early_wire = early
        .encrypt_string_for(CryptPurpose::Cast, "users.ssn", "early")
        .unwrap();
    Crypt::try_generate_key().unwrap();
    assert!(!Crypt::is_initialized());
    let err = Crypt::encrypt_string_for(CryptPurpose::Cast, "users.ssn", "early").unwrap_err();
    assert!(
        err.to_string().contains("Crypt is not initialized"),
        "{err}"
    );

    // Boot installs `APP_KEY`, not the key an encrypter held first.
    suprnova::boot::initialize_crypt_or_exit();
    assert!(Crypt::is_initialized());
    assert!(Crypt::decrypt_string_for(CryptPurpose::Cast, "users.ssn", &early_wire).is_err());

    // After boot: what `Crypt` wrote before the encrypter was built still
    // opens, and what an encrypter of another key writes does not.
    let before = Crypt::encrypt_string_for(CryptPurpose::Cast, "users.ssn", "before").unwrap();
    let late = Encrypter::new(other_key);
    let late_wire = late
        .encrypt_string_for(CryptPurpose::Cast, "users.ssn", "late")
        .unwrap();
    Crypt::try_generate_key().unwrap();
    assert_eq!(
        Crypt::decrypt_string_for(CryptPurpose::Cast, "users.ssn", &before).unwrap(),
        "before"
    );
    assert!(Crypt::decrypt_string_for(CryptPurpose::Cast, "users.ssn", &late_wire).is_err());

    // `Crypt` still writes under `APP_KEY` alone.
    let after = Crypt::encrypt_string_for(CryptPurpose::Cast, "users.ssn", "after").unwrap();
    assert_eq!(
        Encrypter::new(app_key)
            .decrypt_string_for(CryptPurpose::Cast, "users.ssn", &after)
            .unwrap(),
        "after"
    );
    assert!(
        late.decrypt_string_for(CryptPurpose::Cast, "users.ssn", &after)
            .is_err()
    );
}
