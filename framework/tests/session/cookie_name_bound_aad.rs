//! Cookie ciphertext is bound to its logical cookie name (v2 AAD).
//!
//! These are the mass-logout guards and the name-binding property itself: if
//! one fails, stop and report rather than weakening the test.

#![cfg(feature = "testing")]

use std::sync::LazyLock;

use suprnova::{Cookie, CookiePrefix, Crypt, CryptPurpose, EncryptionKey};

static INSTALLED: LazyLock<()> = LazyLock::new(|| {
    let _ = suprnova::crypto::_test_install_key(EncryptionKey::generate());
});

fn init_crypt() {
    LazyLock::force(&INSTALLED);
}

#[test]
fn ciphertext_from_one_cookie_fails_as_another() {
    init_crypt();
    let cookie = Cookie::encrypted("cookie_a", "payload").expect("encrypt");
    let wire = cookie.value().to_string();

    assert!(
        Cookie::read_encrypted_for("cookie_b", &wire).is_err(),
        "cookie_a's ciphertext must not decrypt as cookie_b"
    );
    assert_eq!(
        Cookie::read_encrypted_for("cookie_a", &wire).expect("own name decrypts"),
        "payload"
    );
}

#[test]
fn legacy_v1_ciphertext_still_decrypts_through_the_window() {
    // Mass-logout guard #1. Pre-upgrade cookies were written with the
    // un-contexted entry point; they must keep working during the window.
    init_crypt();
    let legacy_wire = Crypt::encrypt_string(CryptPurpose::Cookie, "old-session").expect("v1");

    assert_eq!(
        Cookie::read_encrypted_for("suprnova_session", &legacy_wire).expect("window"),
        "old-session"
    );
}

#[test]
fn prefix_flip_does_not_invalidate_existing_cookies() {
    // Mass-logout guard #2. The AAD binds the logical name, so the wire-name
    // change a prefix flip causes must not change the AAD.
    init_crypt();
    let written_before_flip = Cookie::encrypted("suprnova_session", "sid.123").expect("encrypt");
    let wire = written_before_flip.value().to_string();
    let logical = CookiePrefix::strip("__Host-suprnova_session");

    assert_eq!(
        Cookie::read_encrypted_for(logical, &wire).expect("prefix flip is safe"),
        "sid.123"
    );
}

#[test]
#[allow(deprecated)]
fn read_encrypted_remains_the_v1_legacy_reader() {
    // The deprecated reader still round-trips v1 - it exists for app code
    // minted before the upgrade - but does not read v2.
    init_crypt();
    let v1 = Crypt::encrypt_string(CryptPurpose::Cookie, "legacy").expect("v1");
    assert_eq!(Cookie::read_encrypted(&v1).expect("v1 reads"), "legacy");

    let v2 = Cookie::encrypted("some_cookie", "modern").expect("encrypt");
    assert!(
        Cookie::read_encrypted(v2.value()).is_err(),
        "the broken pair must be visibly broken, not silently working"
    );
}
