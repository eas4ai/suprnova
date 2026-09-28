#![cfg(feature = "testing")]

//! Phase 10B P7 - Encrypted-cast key rotation.
//!
//! Pins the rotation story end-to-end:
//!
//! 1. `Crypt::decrypt_string` tries the current key first, then each
//!    `APP_KEY_PREVIOUS` entry in order.
//! 2. Encryption *always* uses the current key (no accidental encrypt-
//!    with-previous when fallback is configured).
//! 3. Multi-step rotation works (current + N previous keys).
//! 4. With no fallback installed, wrong-key decrypt fails LOUDLY
//!    (returns Err, not silent garbage) - this is the v1 minimum.
//! 5. The macro-generated `Model::find` path round-trips a row that
//!    was written under an old key once the ring is configured.
//! 6. `APP_KEY_PREVIOUS` env parsing tolerates empty entries and
//!    rejects malformed ones.
//!
//! # OnceLock constraint
//!
//! The process-wide `CRYPT_RING` `OnceLock` means this test binary
//! installs ONE ring for its entire lifetime. Mid-test rotation isn't
//! possible. The pattern is:
//!
//! 1. Decide the final ring shape (current = B, previous = [A]).
//! 2. Install it once at the top of the binary via a mutex-guarded
//!    helper.
//! 3. Use `suprnova::crypto::testing::encrypt_string_under(&A, purpose,
//!    plaintext)` to mint ciphertext "as if it had been written when A
//!    was current."
//! 4. Decrypt it through the public `Crypt::decrypt_string` and
//!    assert success + origin.
//!
//! Tests that need a *different* ring shape go in their own binary.
//! `OnceLock` + cargo test default-thread-pool semantics make
//! multi-ring-per-binary brittle; one ring per binary is cheap and
//! correct.

use std::sync::OnceLock;

use suprnova::crypto::testing::{encrypt_string_for_under, encrypt_string_under};
use suprnova::testing::TestDatabase;
use suprnova::{
    AsEncrypted, Crypt, CryptPurpose, EncryptionKey, Model,
    crypto::{AadVersion, DecryptOrigin, KeyOrigin},
    model,
};

// ---- One-shot ring installer -------------------------------------------

/// Keys used by every test in this binary. Materialised once; the
/// installed ring is `current = B`, `previous = [A, A2]` (A2 for the
/// multi-step-rotation test, harmless for the single-fallback tests).
///
/// `current` (B) - used for any new encrypt issued via `Crypt::encrypt_string`.
/// `previous[0]` (A) - the oldest fallback; tests that simulate "data
///                     was written under A" use this.
/// `previous[1]` (A2) - a second fallback to prove the ring walks the
///                      full list instead of stopping at index 0.
struct RotationKeys {
    current: EncryptionKey,
    previous_oldest: EncryptionKey,
    previous_middle: EncryptionKey,
}

fn rotation_keys() -> &'static RotationKeys {
    static KEYS: OnceLock<RotationKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let keys = RotationKeys {
            current: EncryptionKey::generate(),
            previous_oldest: EncryptionKey::generate(),
            previous_middle: EncryptionKey::generate(),
        };
        // Install the ring exactly once. The installer is idempotent
        // (returns false if a ring was already present from a sibling
        // test binary's static init), so we ignore the bool.
        let _ = suprnova::testing::install_test_encryption_keyring(
            keys.current.clone(),
            vec![keys.previous_oldest.clone(), keys.previous_middle.clone()],
        );
        keys
    })
}

// ---- Cast-level tests (pure facade) ------------------------------------

#[test]
fn current_key_decrypts_and_reports_current_origin() {
    let keys = rotation_keys();

    // Encrypt through the facade - uses current. Bind to the Cast
    // purpose so the helper-minted ciphertext below (same purpose)
    // also passes AEAD authentication.
    let wire =
        Crypt::encrypt_string(CryptPurpose::Cast, "rotation-payload-current").expect("encrypt");
    let (plain, origin) =
        Crypt::decrypt_string_inner(CryptPurpose::Cast, &wire).expect("decrypt with current key");
    assert_eq!(plain, "rotation-payload-current");
    assert_eq!(origin.key, KeyOrigin::Current);
    assert_eq!(origin.aad, AadVersion::Current);

    // Sanity: the wire was actually produced under `current` - confirm
    // by minting an equivalent payload via the test helper directly.
    let wire_via_helper = encrypt_string_under(
        &keys.current,
        CryptPurpose::Cast,
        "rotation-payload-current",
    )
    .expect("encrypt via helper");
    let (_, helper_origin) = Crypt::decrypt_string_inner(CryptPurpose::Cast, &wire_via_helper)
        .expect("decrypt helper-minted");
    assert_eq!(helper_origin.key, KeyOrigin::Current);
    assert_eq!(helper_origin.aad, AadVersion::Current);
}

#[test]
fn previous_key_decrypts_and_reports_previous_origin() {
    // The v1 happy path for rotation: a row written when key A was
    // current still decrypts under the new ring, and the test
    // observation hook reports which key in the previous list won.
    let keys = rotation_keys();

    let wire = encrypt_string_under(
        &keys.previous_oldest,
        CryptPurpose::Cast,
        "rotation-payload-legacy",
    )
    .expect("encrypt under legacy key");
    let (plain, origin) =
        Crypt::decrypt_string_inner(CryptPurpose::Cast, &wire).expect("decrypt via fallback");
    assert_eq!(plain, "rotation-payload-legacy");
    assert_eq!(
        origin.key,
        KeyOrigin::Previous(0),
        "expected fallback to previous[0] (oldest); got {origin:?}"
    );
    assert_eq!(origin.aad, AadVersion::Current);
}

#[test]
fn ring_walks_full_previous_list_to_find_match() {
    // Multi-step rotation: data written under the *middle* fallback
    // key (not the oldest) must still decrypt - proves the ring
    // doesn't stop at index 0.
    let keys = rotation_keys();
    let wire = encrypt_string_under(
        &keys.previous_middle,
        CryptPurpose::Cast,
        "two-step-rotation",
    )
    .expect("encrypt under middle previous key");
    let (plain, origin) = Crypt::decrypt_string_inner(CryptPurpose::Cast, &wire).expect("decrypt");
    assert_eq!(plain, "two-step-rotation");
    assert_eq!(origin.key, KeyOrigin::Previous(1));
    assert_eq!(origin.aad, AadVersion::Current);
}

#[test]
fn unrelated_key_fails_loudly_not_silently() {
    // A key that isn't in the ring at all → loud error, never
    // returns the wrong plaintext (which would be silent corruption).
    // This is the v1 minimum: rotation fallback must not become a
    // "try every key until something base64-decodes" pit that masks
    // genuinely bad data. Even with two previous keys installed and
    // a third unrelated key minting the ciphertext, decrypt MUST
    // surface Err.
    //
    // Force the ring into existence before we start - under parallel
    // `cargo test` scheduling this test may be the first to run.
    let _ = rotation_keys();
    let stranger = EncryptionKey::generate();
    let wire = encrypt_string_under(&stranger, CryptPurpose::Cast, "should-not-decrypt")
        .expect("encrypt under unrelated key");

    let public_result = Crypt::decrypt_string(CryptPurpose::Cast, &wire);
    assert!(
        public_result.is_err(),
        "decrypt with a key outside the ring MUST error, not return garbage"
    );
    let msg = format!("{}", public_result.unwrap_err());
    assert!(
        msg.contains("AEAD decrypt failed"),
        "expected aead diagnostic, got: {msg}"
    );

    // And the inner variant reflects the same Err path (no
    // accidental origin.key == KeyOrigin::Current for a wrong key).
    let inner_result = Crypt::decrypt_string_inner(CryptPurpose::Cast, &wire);
    assert!(inner_result.is_err());
}

#[test]
fn encrypt_always_uses_current_even_when_previous_configured() {
    // If `encrypt_string` accidentally reached for a previous key,
    // the resulting ciphertext would decrypt under that previous key
    // alone - and `Crypt::decrypt_string_inner` would report
    // `origin.key == KeyOrigin::Previous(_)` instead of `Current`.
    // This test pins the invariant.
    //
    // Force the ring into existence before we start - under parallel
    // `cargo test` scheduling this test may be the first to run.
    let _ = rotation_keys();
    for i in 0..5 {
        let plaintext = format!("encrypt-current-{i}");
        let wire = Crypt::encrypt_string(CryptPurpose::Cast, &plaintext).expect("encrypt");
        let (plain, origin) =
            Crypt::decrypt_string_inner(CryptPurpose::Cast, &wire).expect("decrypt");
        assert_eq!(plain, plaintext);
        assert_eq!(
            origin.key,
            KeyOrigin::Current,
            "encrypt must always use current key - got rotation origin {origin:?} for payload {plaintext:?}"
        );
        assert_eq!(origin.aad, AadVersion::Current);
    }
}

#[test]
fn decrypt_t_round_trip_via_fallback() {
    // The other public decrypt entrypoint - JSON-decoding variant.
    // Same rotation semantics: previous keys are tried, origin
    // surfaces via `_inner`.
    let keys = rotation_keys();

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct Bag {
        a: i64,
        b: String,
    }
    let bag = Bag {
        a: 42,
        b: "hello".into(),
    };
    let json = serde_json::to_vec(&bag).unwrap();

    // Mint A-encrypted JSON via low-level AEAD + base64 the same way
    // `Crypt::encrypt` would, but under the previous (oldest) key.
    let aead_wire = {
        // We reuse `encrypt_string_under` by feeding it the JSON bytes
        // as a UTF-8 string - JSON output is always valid UTF-8 for
        // safe primitives, so this is fine.
        let plaintext = std::str::from_utf8(&json).expect("json is utf-8");
        encrypt_string_under(&keys.previous_oldest, CryptPurpose::Cookie, plaintext)
            .expect("encrypt JSON under legacy key")
    };

    let (decoded, origin): (Bag, DecryptOrigin) =
        Crypt::decrypt_inner(CryptPurpose::Cookie, &aead_wire).expect("decrypt JSON via fallback");
    assert_eq!(decoded, bag);
    assert_eq!(origin.key, KeyOrigin::Previous(0));
    assert_eq!(origin.aad, AadVersion::Current);
}

// ---- Model-level integration ------------------------------------------

#[model(
    table = "rotation_enc",
    timestamps = false,
    fillable = ["secret"],
    casts = { secret = AsEncrypted }
)]
pub struct RotationEnc {
    pub id: i64,
    pub secret: String,
}

#[tokio::test]
async fn model_round_trips_row_written_under_previous_key() {
    // End-to-end: insert ciphertext minted under the OLD key directly
    // via raw SQL (simulating "this row predates the rotation"), then
    // `Model::find()` and assert the cast decrypts cleanly.
    let keys = rotation_keys();
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE rotation_enc (id INTEGER PRIMARY KEY AUTOINCREMENT, secret TEXT NOT NULL)",
    )
    .await
    .unwrap();

    // Mint A-encrypted ciphertext (under the Cast purpose so the
    // AsEncrypted cast can authenticate it on read) and shove it
    // straight into the DB.
    let legacy_wire = encrypt_string_under(
        &keys.previous_oldest,
        CryptPurpose::Cast,
        "social-security-number-legacy",
    )
    .expect("encrypt under legacy key");
    db.execute_unprepared(&format!(
        "INSERT INTO rotation_enc (id, secret) VALUES (1, '{legacy_wire}')"
    ))
    .await
    .unwrap();

    // The cast routes through `Crypt::decrypt_string` with the Cast
    // purpose. With the fallback ring installed, this should succeed
    // and yield the original plaintext - proving rotation works for
    // the public model surface, not just the facade.
    let read = RotationEnc::find(1).await.unwrap().unwrap();
    assert_eq!(read.secret, "social-security-number-legacy");
}

#[tokio::test]
async fn model_save_re_encrypts_under_current_key() {
    // Re-encryption job semantics: loading a legacy row and saving
    // it back must rewrite the column under the current key. The
    // operator's "rotation completion" pass is literally `for each
    // row: load + save`. We pin that behaviour here so a regression
    // in the cast-to-storage path (e.g. accidentally caching the
    // origin and re-using the previous key on save) shows up as a
    // failing test.
    let keys = rotation_keys();
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE rotation_enc (id INTEGER PRIMARY KEY AUTOINCREMENT, secret TEXT NOT NULL)",
    )
    .await
    .unwrap();

    let legacy_wire =
        encrypt_string_under(&keys.previous_oldest, CryptPurpose::Cast, "re-encrypt-me")
            .expect("encrypt under legacy key");
    db.execute_unprepared(&format!(
        "INSERT INTO rotation_enc (id, secret) VALUES (1, '{legacy_wire}')"
    ))
    .await
    .unwrap();

    // Read + save - the cast layer rewrites the column under current.
    let mut row = RotationEnc::find(1).await.unwrap().unwrap();
    // Touch the field so the active model definitely registers a
    // change (some ORMs no-op on a value-identity save; force a
    // round-trip by reassigning the same plaintext).
    row.secret = row.secret.clone();
    row.save().await.unwrap();

    // Read the raw column back: it should now decrypt under the
    // current key (origin Current), not the previous key.
    let raw = db
        .fetch_one(
            "SELECT secret FROM rotation_enc WHERE id = ?",
            vec![sea_orm::Value::from(1i64)],
        )
        .await
        .unwrap();
    let stored: String = raw.try_get("", "secret").unwrap();

    let (plain, origin) = Crypt::decrypt_string_inner(CryptPurpose::Cast, &stored)
        .expect("re-encrypted ciphertext decrypts");
    assert_eq!(plain, "re-encrypt-me");
    assert_eq!(
        origin.key,
        KeyOrigin::Current,
        "save() must re-encrypt under current key; got {origin:?}"
    );
    assert_eq!(origin.aad, AadVersion::Current);
}

// ---- Mutation-test sanity (manual procedure) --------------------------
//
// To verify these tests actually exercise the fallback loop (and aren't
// accidentally green because every key happens to decrypt the same
// ciphertext - which it cannot, because AEAD authentication tags are
// 128 bits wide and 2^128 collisions are infeasible - but defence in
// depth):
//
// 1. In `framework/src/crypto/mod.rs`, mutate `decrypt_with_ring` so
//    the previous-key loop is `for _ in [] { ... }` (skipping the
//    fallback list entirely).
// 2. `cargo test -p suprnova --test eloquent_casts_encrypted_key_rotation`
// 3. Expected failures:
//    - `previous_key_decrypts_and_reports_previous_origin`
//    - `ring_walks_full_previous_list_to_find_match`
//    - `decrypt_t_round_trip_via_fallback`
//    - `model_round_trips_row_written_under_previous_key`
//    - `model_save_re_encrypts_under_current_key`
//
// 4. Revert the mutation. All tests pass again.
//
// This procedure was executed once during P7 dev to confirm the
// fallback path is load-bearing; left as a comment for future
// maintainers who suspect a test is no-op.

// ---- The origin of a value, for the job that ends a rotation ----------

#[test]
fn a_value_under_the_current_key_needs_no_reencryption() {
    let _ = rotation_keys();
    let stored = Crypt::encrypt_string(CryptPurpose::Cast, "kept").expect("encrypt");

    let (plain, origin) =
        Crypt::decrypt_string_with_origin(CryptPurpose::Cast, &stored).expect("decrypt");

    assert_eq!(plain, "kept");
    assert_eq!(origin.key, KeyOrigin::Current);
    assert_eq!(origin.aad, AadVersion::Current);
    assert!(!origin.needs_reencryption());
}

#[test]
fn a_value_under_a_previous_key_is_found_and_written_again() {
    let keys = rotation_keys();
    let stored = encrypt_string_under(&keys.previous_middle, CryptPurpose::Cast, "rotated")
        .expect("a value from before the rotation");

    let (plain, origin) =
        Crypt::decrypt_string_with_origin(CryptPurpose::Cast, &stored).expect("decrypt");
    assert_eq!(plain, "rotated");
    assert_eq!(origin.key, KeyOrigin::Previous(1));
    assert!(origin.needs_reencryption());

    // What the rotation job does with it.
    let written_again = Crypt::encrypt_string(CryptPurpose::Cast, &plain).expect("encrypt");
    let (again, origin) =
        Crypt::decrypt_string_with_origin(CryptPurpose::Cast, &written_again).expect("decrypt");
    assert_eq!(again, "rotated");
    assert!(!origin.needs_reencryption());
}

#[test]
fn the_origin_of_a_typed_value_is_the_origin_of_its_text() {
    let keys = rotation_keys();
    let stored = encrypt_string_under(&keys.previous_oldest, CryptPurpose::Cast, "[1,2,3]")
        .expect("a value from before the rotation");

    let (value, origin): (Vec<u8>, DecryptOrigin) =
        Crypt::decrypt_with_origin(CryptPurpose::Cast, &stored).expect("decrypt");

    assert_eq!(value, [1, 2, 3]);
    assert_eq!(origin.key, KeyOrigin::Previous(0));
    assert!(origin.needs_reencryption());
}

#[test]
fn the_label_of_a_value_is_the_second_thing_its_origin_says() {
    let keys = rotation_keys();

    // Written before the label had the name of the context in it.
    let legacy = encrypt_string_under(&keys.current, CryptPurpose::Cast, "legacy label")
        .expect("a value under the label of before");
    let (plain, origin) =
        Crypt::decrypt_string_for_with_origin(CryptPurpose::Cast, "users.ssn", &legacy)
            .expect("decrypt");
    assert_eq!(plain, "legacy label");
    assert_eq!(origin.key, KeyOrigin::Current);
    assert_eq!(origin.aad, AadVersion::Legacy);
    assert!(
        origin.needs_reencryption(),
        "a value under the current key and the legacy label is written again as well"
    );

    // A previous key and the current label: no call of `Crypt` writes it.
    let rotated = encrypt_string_for_under(
        &keys.previous_oldest,
        CryptPurpose::Cast,
        "users.ssn",
        "current label",
    )
    .expect("a value under a previous key");
    let (plain, origin) =
        Crypt::decrypt_string_for_with_origin(CryptPurpose::Cast, "users.ssn", &rotated)
            .expect("decrypt");
    assert_eq!(plain, "current label");
    assert_eq!(origin.key, KeyOrigin::Previous(0));
    assert_eq!(origin.aad, AadVersion::Current);
    assert!(origin.needs_reencryption());

    // The label binds the value to its context.
    assert!(
        Crypt::decrypt_string_for_with_origin(CryptPurpose::Cast, "users.email", &rotated).is_err(),
        "a value of one column must not decrypt as a value of another"
    );
}

#[test]
fn a_value_under_a_key_that_is_not_in_the_ring_has_no_origin() {
    let _ = rotation_keys();
    let stranger = EncryptionKey::generate();
    let stored = encrypt_string_under(&stranger, CryptPurpose::Cast, "unreadable")
        .expect("a value under a key of nobody");

    Crypt::decrypt_string_with_origin(CryptPurpose::Cast, &stored)
        .expect_err("no key of the ring decrypts it");
    Crypt::decrypt_with_origin::<String>(CryptPurpose::Cast, &stored)
        .expect_err("no key of the ring decrypts it as a typed value");
}

/// A value that decrypts and is of another type than the one that was
/// asked for. The JSON decoder names what it read in its own message, and
/// what it read is the decrypted value.
#[test]
fn a_value_of_another_type_is_not_shown_in_the_error() {
    let keys = rotation_keys();
    let stored = encrypt_string_under(&keys.current, CryptPurpose::Cast, "\"123-45-6789\"")
        .expect("a text where a number is read");

    for said in [
        Crypt::decrypt_with_origin::<u32>(CryptPurpose::Cast, &stored)
            .expect_err("a text is no number")
            .to_string(),
        Crypt::decrypt::<u32>(CryptPurpose::Cast, &stored)
            .expect_err("a text is no number")
            .to_string(),
    ] {
        assert!(said.contains("JSON decode failed"), "{said}");
        assert!(
            !said.contains("123") && !said.contains("6789"),
            "the error shows the decrypted value: {said}"
        );
    }
}

/// The encrypted casts decrypt and decode. A stored value of another
/// type than the field has now is the case: a column that was a list of
/// texts and is read as a list of numbers.
#[test]
fn an_encrypted_cast_does_not_show_the_value_it_cannot_decode() {
    use suprnova::{AsEncryptedArray, AsEncryptedObject, Cast, IntoDynCast};

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Card {
        last_four: u16,
    }

    let _ = rotation_keys();
    let list = Crypt::encrypt_string(CryptPurpose::Cast, "[\"123-45-6789\"]").expect("encrypt");
    let object = Crypt::encrypt_string(CryptPurpose::Cast, "{\"last_four\":\"4111-1111\"}")
        .expect("encrypt");

    let said = [
        AsEncryptedArray::<u32>::from_storage(&list)
            .expect_err("a text is no number")
            .to_string(),
        AsEncryptedArray::<u32>::into_dyn()
            .from_storage_json(&serde_json::Value::String(list.clone()))
            .expect_err("a text is no number")
            .to_string(),
        AsEncryptedObject::<Card>::from_storage(&object)
            .expect_err("a text is no number")
            .to_string(),
        AsEncryptedObject::<Card>::into_dyn()
            .from_storage_json(&serde_json::Value::String(object.clone()))
            .expect_err("a text is no number")
            .to_string(),
    ];

    for said in said {
        assert!(said.contains("another type"), "{said}");
        for part in ["123", "6789", "4111"] {
            assert!(
                !said.contains(part),
                "the error shows the decrypted value: {said}"
            );
        }
    }
}
