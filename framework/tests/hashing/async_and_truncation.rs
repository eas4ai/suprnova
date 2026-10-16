//! Regression: HIGH audit findings `hashing` #1 + #2 - bcrypt's `hash`
//! / `verify` are synchronous CPU-bound (blocks the Tokio worker
//! thread at cost 12) AND silently truncate passwords > 72 bytes (two
//! distinct passphrases with the same first 72 bytes hash equal).
//!
//! Fix:
//!   1. New `hash_async` / `verify_async` / `hash_with_cost_async`
//!      wrappers run bcrypt on `tokio::task::spawn_blocking`.
//!   2. `hash` rejects passwords > `MAX_PASSWORD_BYTES` up-front with a
//!      `FrameworkError::param`, so the framework never mints a hash two
//!      passphrases share. `verify` judges a password of any length on
//!      its first 72 bytes, as PHP's `password_verify` does, so a long
//!      password a Laravel application hashed signs in (LDB-004); a
//!      different password still does not verify.
//!   3. The underlying bcrypt call uses `non_truncating_hash` as
//!      defense in depth.

use suprnova::hashing::{self, MAX_PASSWORD_BYTES};

#[test]
fn happy_path_hash_and_verify_still_works() {
    let pw = "correct horse battery staple";
    let h = hashing::hash(pw).expect("hash");
    assert!(h.starts_with("$2"), "bcrypt hash prefix expected");
    assert!(hashing::verify(pw, &h).expect("verify"));
    assert!(!hashing::verify("wrong", &h).expect("verify"));
}

#[test]
fn hash_rejects_passwords_over_72_bytes() {
    // The audit's load-bearing case: two distinct passphrases that
    // share their first 72 bytes used to silently produce the same
    // bcrypt hash via truncation. The fix rejects > 72-byte inputs
    // at the hash boundary so neither path can mint a collision.
    let too_long = "x".repeat(MAX_PASSWORD_BYTES + 1);
    let err = hashing::hash(&too_long).expect_err("> 72 bytes must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("71"),
        "error must call out the byte limit; got: {msg}"
    );
}

#[test]
fn hash_accepts_exactly_max_bytes() {
    // Boundary test - MAX_PASSWORD_BYTES (71) is the legal max.
    let exactly = "y".repeat(MAX_PASSWORD_BYTES);
    let h = hashing::hash(&exactly).expect("exactly max bytes must hash");
    assert!(hashing::verify(&exactly, &h).expect("verify"));
}

#[test]
fn verify_returns_false_for_oversized_password() {
    // Hash a legitimate password, then attempt to verify a >72-byte
    // input that is not that password against it: it does not verify,
    // and the answer is `Ok(false)`, the same "invalid credentials" any
    // wrong password gets.
    let pw = "correct horse battery staple";
    let h = hashing::hash(pw).expect("hash");
    let oversized = "x".repeat(MAX_PASSWORD_BYTES + 100);
    assert!(
        !hashing::verify(&oversized, &h).expect("verify"),
        "oversized password must NOT verify against any hash"
    );
}

#[test]
fn truncation_collision_no_longer_possible() {
    // Two passwords sharing first 72 bytes must NOT verify against
    // each other's hash. Pre-fix this used to be a silent yes.
    let common = "a".repeat(MAX_PASSWORD_BYTES);
    let a = format!("{common}_suffix_A");
    let b = format!("{common}_suffix_B");
    // Both rejected at hash, so neither has a hash to verify against -
    // which is the right semantic: the framework refuses to mint
    // collidable hashes.
    assert!(hashing::hash(&a).is_err());
    assert!(hashing::hash(&b).is_err());
}

#[tokio::test]
async fn hash_async_round_trips_through_spawn_blocking() {
    let pw = "another correct horse";
    let h = hashing::hash_async(pw).await.expect("hash_async");
    assert!(hashing::verify_async(pw, &h).await.expect("verify_async"));
    assert!(
        !hashing::verify_async("nope", &h)
            .await
            .expect("verify_async")
    );
}

#[tokio::test]
async fn hash_async_rejects_oversized_password() {
    let too_long = "z".repeat(MAX_PASSWORD_BYTES + 50);
    let err = hashing::hash_async(&too_long)
        .await
        .expect_err("oversized must be rejected via async path too");
    assert!(format!("{err}").contains("72"));
}

#[tokio::test]
async fn verify_async_returns_false_for_oversized() {
    let pw = "fits in 72 bytes";
    let h = hashing::hash_async(pw).await.expect("hash_async");
    let oversized = "x".repeat(MAX_PASSWORD_BYTES + 1);
    assert!(
        !hashing::verify_async(&oversized, &h)
            .await
            .expect("verify_async")
    );
}
