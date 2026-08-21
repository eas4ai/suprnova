//! The captured legacy corpus drives the dual-format verifier.
//!
//! The corpus was minted by the live deployed code (framework bcrypt and
//! torii `password_auth` Argon2id) while both codebases were current; these
//! tests never regenerate it with Magnetar's own dependencies. The
//! deterministic non-secret inputs are the ASCII byte `p` repeated to each
//! recorded length, as documented in `tests/fixtures/README.md`.

mod fixtures;

use std::sync::Arc;

use magnetar::password::{
    HashAlgorithm, PasswordHashConfig, PasswordVerifier, RehashOutcome, StandardPasswordHashDriver,
};
use secrecy::SecretString;
use serde_json::Value;

fn corpus(path: &str) -> Value {
    let raw = std::fs::read_to_string(fixtures::repository_path(path))
        .expect("hash corpus must be readable");
    serde_json::from_str(&raw).expect("hash corpus must be JSON")
}

fn plaintext(bytes: u64) -> SecretString {
    SecretString::from("p".repeat(usize::try_from(bytes).expect("fixture lengths fit usize")))
}

fn deployed_verifier() -> PasswordVerifier {
    PasswordVerifier::new(
        Arc::new(StandardPasswordHashDriver),
        PasswordHashConfig::default(),
    )
    .expect("dummy warmup succeeds")
}

#[test]
fn corpus_provenance_matches_the_pinned_live_sources() {
    let bcrypt = corpus("tests/fixtures/hashes/suprnova-bcrypt.json");
    assert_eq!(bcrypt["source"], "eas4ai/suprnova");
    assert_eq!(
        bcrypt["revision"],
        "27f7ddf4bb6c523c4ffa42fa12e4a568a7990f88"
    );
    assert_eq!(bcrypt["algorithm"]["name"], "bcrypt");
    assert_eq!(bcrypt["algorithm"]["cost"], 12);

    let argon2 = corpus("tests/fixtures/hashes/torii-argon2.json");
    assert_eq!(argon2["source"], "eas4ai/suprnova-torii-rs");
    assert_eq!(
        argon2["revision"],
        "968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d"
    );
    assert_eq!(argon2["algorithm"]["name"], "argon2id");
    assert_eq!(argon2["algorithm"]["memory_kib"], 19456);
    assert_eq!(argon2["algorithm"]["iterations"], 2);
    assert_eq!(argon2["algorithm"]["parallelism"], 1);

    // The verifier's deployed defaults are pinned to the same profiles the
    // corpus records; drifting one without the other is a spec change.
    let config = PasswordHashConfig::default();
    assert_eq!(config.bcrypt_cost, 12);
    assert_eq!(config.argon2_memory_kib, 19_456);
    assert_eq!(config.argon2_iterations, 2);
    assert_eq!(config.argon2_parallelism, 1);
}

#[test]
fn corpus_covers_the_boundary_lengths_including_bcrypt_rejections() {
    let bcrypt = corpus("tests/fixtures/hashes/suprnova-bcrypt.json");
    let records = bcrypt["records"].as_array().expect("bcrypt records");
    let by_len = |bytes: u64| {
        records
            .iter()
            .find(|record| record["bytes"] == bytes)
            .unwrap_or_else(|| panic!("bcrypt corpus is missing the {bytes}-byte case"))
    };
    for bytes in [32, 71] {
        assert_eq!(by_len(bytes)["status"], "generated");
        assert!(
            by_len(bytes)["hash"]
                .as_str()
                .unwrap()
                .starts_with("$2b$12$")
        );
    }
    // The live framework rejects bcrypt inputs above 71 bytes up front, so
    // the corpus records the rejection instead of a truncated hash.
    for bytes in [72, 73, 128] {
        assert_eq!(by_len(bytes)["status"], "rejected");
    }

    let argon2 = corpus("tests/fixtures/hashes/torii-argon2.json");
    let records = argon2["records"].as_array().expect("argon2 records");
    for bytes in [32_u64, 71, 72, 73, 128] {
        let record = records
            .iter()
            .find(|record| record["bytes"] == bytes)
            .unwrap_or_else(|| panic!("argon2 corpus is missing the {bytes}-byte case"));
        assert!(
            record["hash"]
                .as_str()
                .unwrap()
                .starts_with("$argon2id$v=19$m=19456,t=2,p=1$")
        );
    }
}

#[test]
fn legacy_bcrypt_hashes_verify_and_upgrade_to_the_argon2id_target() {
    let verifier = deployed_verifier();
    let bcrypt = corpus("tests/fixtures/hashes/suprnova-bcrypt.json");
    for record in bcrypt["records"].as_array().expect("bcrypt records") {
        if record["status"] != "generated" {
            continue;
        }
        let bytes = record["bytes"].as_u64().expect("record length");
        let hash = record["hash"].as_str().expect("record hash");
        let verdict = verifier
            .verify_attempt(Some(hash), &plaintext(bytes))
            .expect("verification runs");
        assert!(verdict.valid, "legacy bcrypt {bytes}-byte hash must verify");
        match verdict.rehash {
            RehashOutcome::Upgraded(upgraded) => {
                assert!(
                    upgraded.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
                    "bcrypt upgrades to the pinned Argon2id target, got {upgraded}"
                );
            }
            other => panic!("bcrypt login must upgrade, got {other:?}"),
        }
        // The wrong password fails against the same stored hash.
        let wrong = verifier
            .verify_attempt(Some(hash), &SecretString::from("q".repeat(bytes as usize)))
            .expect("verification runs");
        assert!(!wrong.valid);
    }
}

#[test]
fn legacy_argon2_hashes_verify_at_every_length_and_never_downgrade() {
    let verifier = deployed_verifier();
    let argon2 = corpus("tests/fixtures/hashes/torii-argon2.json");
    for record in argon2["records"].as_array().expect("argon2 records") {
        let bytes = record["bytes"].as_u64().expect("record length");
        let hash = record["hash"].as_str().expect("record hash");
        let verdict = verifier
            .verify_attempt(Some(hash), &plaintext(bytes))
            .expect("verification runs");
        assert!(
            verdict.valid,
            "legacy argon2 {bytes}-byte hash must verify (including 128 bytes)"
        );
        // The corpus matches the pinned target exactly: a successful login
        // never re-hashes it, so a stronger hash can never be downgraded.
        assert_eq!(verdict.rehash, RehashOutcome::NotNeeded);
    }
}

#[test]
fn over_length_input_against_a_bcrypt_hash_is_a_mismatch_not_an_error() {
    let verifier = deployed_verifier();
    let bcrypt = corpus("tests/fixtures/hashes/suprnova-bcrypt.json");
    let hash = bcrypt["records"][0]["hash"].as_str().expect("32-byte hash");
    // 72+ byte inputs can never match a framework-accepted bcrypt hash; the
    // attempt reports invalid credentials with no length-based error.
    let verdict = verifier
        .verify_attempt(Some(hash), &plaintext(128))
        .expect("verification runs");
    assert!(!verdict.valid);
}

#[test]
fn target_minting_produces_the_pinned_profile() {
    let verifier = deployed_verifier();
    let minted = verifier
        .mint_target(&SecretString::from("correct horse battery staple"))
        .expect("minting succeeds");
    assert!(minted.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    let verdict = verifier
        .verify_attempt(
            Some(&minted),
            &SecretString::from("correct horse battery staple"),
        )
        .expect("verification runs");
    assert!(verdict.valid);
    assert_eq!(verdict.rehash, RehashOutcome::NotNeeded);
    assert_eq!(
        verifier.config().argon2_target().algorithm,
        HashAlgorithm::Argon2
    );
}
