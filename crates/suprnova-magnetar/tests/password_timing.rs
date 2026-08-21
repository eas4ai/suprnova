//! Ported timing-oracle regressions, guarded by a spy driver rather than
//! wall clock.
//!
//! Every authentication attempt must perform exactly one bcrypt-format and
//! one Argon2-format verification through the installed driver — unknown
//! email, passwordless account, wrong password, and success included — and
//! each dummy call must carry the same parameter profile as the deployed
//! format it stands in for. The assertions pin provenance and profile
//! equality, never durations, so they cannot flake on a loaded CI box.

#![cfg(all(feature = "password", feature = "seaorm-sqlite"))]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use magnetar::password::{
    CallProvenance, HashAlgorithm, HashParameters, HashWorkProfile, PasswordHashConfig,
    PasswordHashDriver, PasswordVerifier, VerificationCall,
};
use magnetar::plugins::password::{PasswordAttempt, PasswordAuthProvider, PasswordAuthService};
use magnetar::sessions::SessionMetadata;
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use parking_lot::Mutex;
use secrecy::{ExposeSecret, SecretString};

use storage_schema::{StorageSchema, database};

/// One observed driver call.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Observed {
    provenance: CallProvenance,
    profile: HashWorkProfile,
}

/// A spy driver that fabricates cheap deterministic hashes and records every
/// verification call it is driven through.
#[derive(Default)]
struct SpyDriver {
    calls: Mutex<Vec<Observed>>,
}

impl SpyDriver {
    fn drain(&self) -> Vec<Observed> {
        std::mem::take(&mut *self.calls.lock())
    }
}

fn spy_hash(profile: &HashWorkProfile, password: &str) -> String {
    match profile.parameters {
        HashParameters::Bcrypt { cost } => format!("$2b${cost:02}$spy:{password}"),
        HashParameters::Argon2 {
            memory_kib,
            iterations,
            parallelism,
        } => format!("$argon2id$v=19$m={memory_kib},t={iterations},p={parallelism}$spy:{password}"),
    }
}

impl PasswordHashDriver for SpyDriver {
    fn verify(&self, call: &VerificationCall<'_>) -> magnetar::Result<bool> {
        self.calls.lock().push(Observed {
            provenance: call.provenance,
            profile: call.profile,
        });
        Ok(call.hash == spy_hash(&call.profile, call.password.expose_secret()))
    }

    fn mint(&self, profile: &HashWorkProfile, password: &SecretString) -> magnetar::Result<String> {
        Ok(spy_hash(profile, password.expose_secret()))
    }
}

fn deployed_bcrypt() -> HashWorkProfile {
    PasswordHashConfig::default().bcrypt_profile()
}

fn deployed_argon2() -> HashWorkProfile {
    PasswordHashConfig::default().argon2_target()
}

fn verifier(spy: Arc<SpyDriver>) -> PasswordVerifier {
    let verifier = PasswordVerifier::new(spy.clone(), PasswordHashConfig::default())
        .expect("dummy warmup succeeds");
    // Warmup mints are construction-time; measurements start clean.
    spy.drain();
    verifier
}

fn assert_fixed_work(calls: &[Observed], stored: Option<HashAlgorithm>) {
    assert_eq!(
        calls.len(),
        2,
        "every attempt performs exactly one bcrypt and one argon2 call, got {calls:?}"
    );
    let bcrypt = &calls[0];
    let argon2 = &calls[1];
    assert_eq!(bcrypt.profile.algorithm, HashAlgorithm::Bcrypt);
    assert_eq!(argon2.profile.algorithm, HashAlgorithm::Argon2);
    // Dummy calls carry the deployed profile of the format they stand in
    // for; stored calls carry the stored hash's own profile.
    match stored {
        Some(HashAlgorithm::Bcrypt) => {
            assert_eq!(bcrypt.provenance, CallProvenance::Stored);
            assert_eq!(argon2.provenance, CallProvenance::Dummy);
            assert_eq!(argon2.profile, deployed_argon2());
        }
        Some(HashAlgorithm::Argon2) => {
            assert_eq!(bcrypt.provenance, CallProvenance::Dummy);
            assert_eq!(bcrypt.profile, deployed_bcrypt());
            assert_eq!(argon2.provenance, CallProvenance::Stored);
        }
        None => {
            assert_eq!(bcrypt.provenance, CallProvenance::Dummy);
            assert_eq!(bcrypt.profile, deployed_bcrypt());
            assert_eq!(argon2.provenance, CallProvenance::Dummy);
            assert_eq!(argon2.profile, deployed_argon2());
        }
    }
}

#[test]
fn stored_bcrypt_attempts_run_one_real_bcrypt_and_one_argon2_dummy() {
    let spy = Arc::new(SpyDriver::default());
    let verifier = verifier(spy.clone());
    let stored = spy_hash(&deployed_bcrypt(), "correct horse");

    let verdict = verifier
        .verify_attempt(Some(&stored), &SecretString::from("correct horse"))
        .unwrap();
    assert!(verdict.valid);
    assert_fixed_work(&spy.drain(), Some(HashAlgorithm::Bcrypt));

    let verdict = verifier
        .verify_attempt(Some(&stored), &SecretString::from("wrong password"))
        .unwrap();
    assert!(!verdict.valid);
    assert_fixed_work(&spy.drain(), Some(HashAlgorithm::Bcrypt));
}

#[test]
fn stored_argon2_attempts_run_one_bcrypt_dummy_and_one_real_argon2() {
    let spy = Arc::new(SpyDriver::default());
    let verifier = verifier(spy.clone());
    let stored = spy_hash(&deployed_argon2(), "correct horse");

    let verdict = verifier
        .verify_attempt(Some(&stored), &SecretString::from("correct horse"))
        .unwrap();
    assert!(verdict.valid);
    assert_fixed_work(&spy.drain(), Some(HashAlgorithm::Argon2));
}

#[test]
fn absent_and_passwordless_attempts_run_both_warmed_dummies() {
    let spy = Arc::new(SpyDriver::default());
    let verifier = verifier(spy.clone());

    let verdict = verifier
        .verify_attempt(None, &SecretString::from("whatever password"))
        .unwrap();
    assert!(!verdict.valid);
    assert_fixed_work(&spy.drain(), None);

    // The NOT-NULL empty sentinel is never handed to a hash driver either.
    let verdict = verifier
        .verify_attempt(Some(""), &SecretString::from("whatever password"))
        .unwrap();
    assert!(!verdict.valid);
    assert_fixed_work(&spy.drain(), None);
}

#[tokio::test]
async fn provider_paths_are_work_equivalent_for_all_four_outcomes() {
    let spy = Arc::new(SpyDriver::default());
    let shared_verifier = Arc::new(
        PasswordVerifier::new(spy.clone(), PasswordHashConfig::default())
            .expect("dummy warmup succeeds"),
    );
    let db = database().await;
    let storage = Arc::new(SeaOrmStorage::<StorageSchema>::new(db));
    let provider = PasswordAuthService::new(storage.clone(), storage.clone(), shared_verifier);

    // One user with a spy-bcrypt credential, one passwordless user.
    let with_password = storage
        .create_user(NewUser {
            email: "holder@example.test".into(),
            password_hash: Some(spy_hash(&deployed_bcrypt(), "correct horse")),
        })
        .await
        .unwrap();
    storage
        .create_user(NewUser {
            email: "passwordless@example.test".into(),
            password_hash: None,
        })
        .await
        .unwrap();
    spy.drain();

    let attempt = |email: &str, password: &str| PasswordAttempt {
        email: email.to_owned(),
        password: SecretString::from(password.to_owned()),
        metadata: SessionMetadata::default(),
    };

    // Wrong password against the stored bcrypt credential.
    provider
        .authenticate(attempt("holder@example.test", "wrong password"))
        .await
        .unwrap_err();
    assert_fixed_work(&spy.drain(), Some(HashAlgorithm::Bcrypt));

    // Known password; the success upgrades the credential to Argon2id.
    let principal = provider
        .authenticate(attempt("holder@example.test", "correct horse"))
        .await
        .unwrap();
    assert_eq!(principal.user_id(), with_password.user_id);
    assert_fixed_work(&spy.drain(), Some(HashAlgorithm::Bcrypt));

    // After the upgrade the stored format flips: the real work moves to the
    // Argon2 lane and the bcrypt lane becomes the dummy.
    provider
        .authenticate(attempt("holder@example.test", "wrong password"))
        .await
        .unwrap_err();
    assert_fixed_work(&spy.drain(), Some(HashAlgorithm::Argon2));

    // Unknown email.
    provider
        .authenticate(attempt("nobody@example.test", "wrong password"))
        .await
        .unwrap_err();
    assert_fixed_work(&spy.drain(), None);

    // Passwordless account costs exactly what the unknown-email path costs.
    provider
        .authenticate(attempt("passwordless@example.test", "wrong password"))
        .await
        .unwrap_err();
    assert_fixed_work(&spy.drain(), None);
}
