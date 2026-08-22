//! Two-factor lifecycle, matched-step replay protection, recovery codes,
//! and the universal factor promotion path.

#![cfg(all(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "two-factor",
    feature = "seaorm-sqlite"
))]

#[path = "fixtures/factor_harness.rs"]
mod factor;
#[path = "fixtures/password_harness.rs"]
mod harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use async_trait::async_trait;

use chrono::{DateTime, Utc};
use magnetar::crypto::{AeadEncryptor, CryptoPurpose, Encryptor};
use magnetar::password::{LockoutConfig, LockoutService};
use magnetar::sessions::JwtEpochStore;
use magnetar::storage::{AttemptStats, LockoutStore, UserStore};
use magnetar::two_factor::{
    TwoFactorConfig, TwoFactorService, TwoFactorStore, totp, totp::STEP_SECONDS,
};
use serde_json::json;

use factor::{FactorWorld, credential_actor, factor_world, send, totp_code_at, totp_code_now};
use harness::{login_request, post_json, register_request};

const EMAIL: &str = "rowan@example.test";
const PASSWORD: &str = "orange tabby cat";

struct FailingResetLockout {
    inner: Arc<dyn LockoutStore>,
}

#[async_trait]
impl LockoutStore for FailingResetLockout {
    async fn record_attempt_and_stats(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<AttemptStats> {
        self.inner
            .record_attempt_and_stats(identity, at, context, window_start)
            .await
    }

    async fn attempt_stats(
        &self,
        identity: &str,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<AttemptStats> {
        self.inner.attempt_stats(identity, window_start).await
    }

    async fn clear_attempts(&self, _identity: &str) -> magnetar::Result<u64> {
        Err(magnetar::Error::Internal {
            message: "forced lockout reset failure".to_owned(),
        })
    }

    async fn cleanup_attempts_before(&self, before: DateTime<Utc>) -> magnetar::Result<u64> {
        self.inner.cleanup_attempts_before(before).await
    }
}

fn service_with_failing_lockout_reset(world: &FactorWorld) -> TwoFactorService {
    let lockout = Arc::new(LockoutService::new(
        Arc::new(FailingResetLockout {
            inner: world.storage.clone(),
        }),
        world.storage.clone(),
        LockoutConfig::default(),
    ));
    TwoFactorService::new(
        Arc::new(storage_schema::sql_two_factor::SqlTwoFactorStore(
            world.db.clone(),
        )),
        world.storage.clone(),
        lockout,
        Arc::new(AeadEncryptor::new([21; 32])),
        TwoFactorConfig::default(),
    )
}

fn decrypt_recovery_codes(ciphertext: &[u8]) -> Vec<String> {
    let plaintext = AeadEncryptor::new([21; 32])
        .decrypt(CryptoPurpose::TwoFactorRecovery, ciphertext)
        .unwrap();
    String::from_utf8(plaintext)
        .unwrap()
        .lines()
        .map(String::from)
        .collect()
}

async fn registered_user(world: &FactorWorld) -> String {
    let reply = send(world, register_request(EMAIL, PASSWORD)).await;
    assert_eq!(reply.status, 200);
    world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .expect("registration created the user")
        .user_id
}

/// Enroll and confirm through the service; returns the enrollment.
async fn confirmed_enrollment(
    world: &FactorWorld,
    user_id: &str,
) -> magnetar::two_factor::EnrollmentResponse {
    let actor = credential_actor(world, user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    enrollment
}

#[tokio::test]
async fn stale_re_enroll_wrong_and_valid_proofs_are_indistinguishable() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    let store = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone());
    let before = store.find_enrollment(&user_id).await.unwrap().unwrap();
    world.storage.bump_auth_epoch(&user_id).await.unwrap();

    let wrong = world
        .two_factor
        .re_enroll(&actor, "not-a-valid-proof")
        .await
        .unwrap_err();
    let valid = world
        .two_factor
        .re_enroll(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap_err();

    assert_eq!(wrong, valid);
    assert_eq!(
        valid,
        magnetar::Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        }
    );
    let after = store.find_enrollment(&user_id).await.unwrap().unwrap();
    assert_eq!(after.last_used_timestep, before.last_used_timestep);
}

#[tokio::test]
async fn stale_regenerate_wrong_and_valid_proofs_are_indistinguishable() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    let store = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone());
    let before = store.find_enrollment(&user_id).await.unwrap().unwrap();
    world.storage.bump_auth_epoch(&user_id).await.unwrap();

    let wrong = world
        .two_factor
        .regenerate_recovery_codes(&actor, "not-a-valid-proof")
        .await
        .unwrap_err();
    let valid = world
        .two_factor
        .regenerate_recovery_codes(&actor, &enrollment.recovery_codes[0])
        .await
        .unwrap_err();

    assert_eq!(wrong, valid);
    assert_eq!(
        valid,
        magnetar::Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        }
    );
    let after = store.find_enrollment(&user_id).await.unwrap().unwrap();
    assert_eq!(after.recovery_codes, before.recovery_codes);
}

#[tokio::test]
async fn re_enroll_returns_artifacts_after_lockout_reset_failure() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    let service = service_with_failing_lockout_reset(&world);

    let rotated = service
        .re_enroll(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .expect("committed rotation still returns its one-time artifacts");

    let row = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone())
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(row.rotation_pending);
    let secret = AeadEncryptor::new([21; 32])
        .decrypt(CryptoPurpose::TwoFactorSecret, &row.secret)
        .unwrap();
    assert!(
        totp::matched_step(
            &secrecy::SecretString::from(String::from_utf8(secret).unwrap()),
            &totp_code_now(&rotated.otpauth_url),
            Utc::now(),
        )
        .unwrap()
        .is_some()
    );
    assert_eq!(
        decrypt_recovery_codes(row.recovery_codes.as_deref().unwrap()),
        rotated.recovery_codes
    );
}

#[tokio::test]
async fn regenerate_returns_artifacts_after_lockout_reset_failure() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    let service = service_with_failing_lockout_reset(&world);

    let regenerated = service
        .regenerate_recovery_codes(&actor, &enrollment.recovery_codes[0])
        .await
        .expect("committed recovery rotation still returns its one-time artifacts");

    let row = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone())
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decrypt_recovery_codes(row.recovery_codes.as_deref().unwrap()),
        regenerated
    );
}

#[tokio::test]
async fn pending_totp_enrollment_cannot_be_confirmed_after_actor_epoch_changes() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrolling_actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&enrolling_actor).await.unwrap();

    world.storage.bump_auth_epoch(&user_id).await.unwrap();
    let current_actor = credential_actor(&world, &user_id).await;
    let error = world
        .two_factor
        .confirm(&current_actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        magnetar::Error::NotFound { resource, identifier }
            if resource == "credential actor" && identifier == "expired or revoked"
    ));
    let row = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone())
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .expect("pending enrollment remains");
    assert!(row.confirmed_at.is_none());
}

#[tokio::test]
async fn stale_actor_re_enroll_does_not_claim_a_valid_totp_timestep() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    let store = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone());
    let before = store
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .expect("confirmed enrollment exists");

    world.storage.bump_auth_epoch(&user_id).await.unwrap();
    let error = world
        .two_factor
        .re_enroll(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        magnetar::Error::NotFound { resource, identifier }
            if resource == "credential actor" && identifier == "expired or revoked"
    ));
    let after = store
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .expect("confirmed enrollment remains");
    assert_eq!(after.last_used_timestep, before.last_used_timestep);
}

#[tokio::test]
async fn stale_actor_regenerate_does_not_consume_a_valid_recovery_code() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    let store = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone());
    let before = store
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .expect("confirmed enrollment exists");

    world.storage.bump_auth_epoch(&user_id).await.unwrap();
    let error = world
        .two_factor
        .regenerate_recovery_codes(&actor, &enrollment.recovery_codes[0])
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        magnetar::Error::NotFound { resource, identifier }
            if resource == "credential actor" && identifier == "expired or revoked"
    ));
    let after = store
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .expect("confirmed enrollment remains");
    assert_eq!(after.recovery_codes, before.recovery_codes);
}

#[tokio::test]
async fn plain_enroll_preserves_a_pending_proof_gated_rotation() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    world
        .two_factor
        .re_enroll(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    let store = storage_schema::sql_two_factor::SqlTwoFactorStore(world.db.clone());
    let pending = store
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .expect("pending rotated enrollment exists");
    assert!(pending.confirmed_at.is_none());

    let error = world.two_factor.enroll(&actor).await.unwrap_err();

    assert!(matches!(
        error,
        magnetar::Error::Conflict { resource, .. }
            if resource == "two-factor enrollment"
    ));
    let after = store
        .find_enrollment(&user_id)
        .await
        .unwrap()
        .expect("pending rotated enrollment remains");
    assert_eq!(after.secret, pending.secret);
    assert_eq!(after.recovery_codes, pending.recovery_codes);
}
#[tokio::test]
async fn enrollment_is_inactive_until_confirmed() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let actor = credential_actor(&world, &user_id).await;

    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    assert_eq!(enrollment.recovery_codes.len(), 10);
    assert!(enrollment.qr_code_svg.starts_with("<svg"));
    assert!(
        !world.two_factor.is_enabled(&user_id).await.unwrap(),
        "pending enrollment counts as disabled"
    );

    // An unconfirmed enrollment never interrupts login.
    let login = send(&world, login_request(EMAIL, PASSWORD)).await;
    assert_eq!(login.status, 200);
    assert!(login.grant.is_some(), "pending enrollment does not gate");

    // A second enroll may overwrite the pending row.
    world.two_factor.enroll(&actor).await.unwrap();

    // Confirmation activates it; a repeat enroll is refused without proof.
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    assert!(world.two_factor.is_enabled(&user_id).await.unwrap());
    let refused = world.two_factor.enroll(&actor).await.unwrap_err();
    assert!(matches!(&refused, magnetar::Error::Conflict { .. }));
}

#[tokio::test]
async fn matched_step_claims_close_every_replay_edge() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrollment = confirmed_enrollment(&world, &user_id).await;
    let now = Utc::now().timestamp();

    // A forward-edge code (the next timestep) is inside the skew window
    // and verifies once, claiming its matched step — NOT server time.
    let forward = totp_code_at(&enrollment.otpauth_url, now + STEP_SECONDS);
    assert!(world.two_factor.verify(&user_id, &forward).await.unwrap());

    // The same forward code can never be accepted again — including when
    // the server actually reaches that timestep (the deployed forward-edge
    // stamp left this replayable; the matched-step contract does not).
    assert!(!world.two_factor.verify(&user_id, &forward).await.unwrap());

    // Any code at or behind the claimed step is refused outright.
    let current = totp_code_at(&enrollment.otpauth_url, now);
    let backward = totp_code_at(&enrollment.otpauth_url, now - STEP_SECONDS);
    assert!(!world.two_factor.verify(&user_id, &current).await.unwrap());
    assert!(!world.two_factor.verify(&user_id, &backward).await.unwrap());
}

#[tokio::test]
async fn concurrent_verifies_of_one_code_have_one_winner() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrollment = confirmed_enrollment(&world, &user_id).await;
    // The forward edge is the interesting race: both concurrent requests
    // match the same step and race the conditional claim.
    let code = totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + STEP_SECONDS,
    );

    let (a, b) = tokio::join!(
        world.two_factor.verify(&user_id, &code),
        world.two_factor.verify(&user_id, &code),
    );
    let wins = [a.unwrap(), b.unwrap()];
    assert_eq!(
        wins.iter().filter(|win| **win).count(),
        1,
        "exactly one concurrent verify may claim the matched step"
    );
}

#[tokio::test]
async fn recovery_codes_are_single_use_down_to_an_empty_blob() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrollment = confirmed_enrollment(&world, &user_id).await;
    let codes = enrollment.recovery_codes.clone();

    // One code consumes exactly once, concurrently.
    let (a, b) = tokio::join!(
        world.two_factor.consume_recovery_code(&user_id, &codes[0]),
        world.two_factor.consume_recovery_code(&user_id, &codes[0]),
    );
    assert_eq!(
        [a.unwrap(), b.unwrap()].iter().filter(|win| **win).count(),
        1,
        "exactly one concurrent consume may take a code"
    );

    // Consume the rest; the final code leaves nothing behind.
    for code in &codes[1..] {
        assert!(
            world
                .two_factor
                .consume_recovery_code(&user_id, code)
                .await
                .unwrap()
        );
    }
    for code in &codes {
        assert!(
            !world
                .two_factor
                .consume_recovery_code(&user_id, code)
                .await
                .unwrap(),
            "an exhausted blob refuses every code"
        );
    }
}

#[tokio::test]
async fn rotation_paths_demand_proof_of_possession() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrollment = confirmed_enrollment(&world, &user_id).await;
    let actor = credential_actor(&world, &user_id).await;

    // Wrong proof: refused and counted against the lockout budget.
    let refused = world
        .two_factor
        .regenerate_recovery_codes(&actor, "000000")
        .await
        .unwrap_err();
    assert!(matches!(
        &refused,
        magnetar::Error::InvalidInput { field, .. } if field == "proof"
    ));
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        1
    );

    // A recovery code is valid proof; rotation replaces the whole set and
    // resets the counter.
    let rotated = world
        .two_factor
        .regenerate_recovery_codes(&actor, &enrollment.recovery_codes[0])
        .await
        .unwrap();
    assert_eq!(rotated.len(), 10);
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        0
    );
    assert!(
        !world
            .two_factor
            .consume_recovery_code(&user_id, &enrollment.recovery_codes[1])
            .await
            .unwrap(),
        "rotation invalidates the prior set"
    );
    assert!(
        world.two_factor.is_enabled(&user_id).await.unwrap(),
        "recovery rotation leaves the secret and confirmation untouched"
    );

    // Re-enrolling rotates the secret and demands re-confirmation.
    let rotated_enrollment = world
        .two_factor
        .re_enroll(
            &actor,
            &totp_code_at(
                &enrollment.otpauth_url,
                Utc::now().timestamp() + STEP_SECONDS,
            ),
        )
        .await
        .unwrap();
    assert!(
        !world.two_factor.is_enabled(&user_id).await.unwrap(),
        "re-enrollment is pending until confirmed against the new secret"
    );
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&rotated_enrollment.otpauth_url))
        .await
        .unwrap();
    assert!(world.two_factor.is_enabled(&user_id).await.unwrap());

    // Disable reports the transition exactly once.
    assert!(world.two_factor.disable(&actor).await.unwrap());
    assert!(!world.two_factor.disable(&actor).await.unwrap());
}

#[tokio::test]
async fn every_primary_provider_promotes_through_one_gate() {
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrollment = confirmed_enrollment(&world, &user_id).await;

    // Password lane: challenge instead of session.
    let password = send(&world, login_request(EMAIL, PASSWORD)).await;
    assert_eq!(password.status, 200);
    assert!(password.grant.is_none());
    let selector = password.body.unwrap()["challenge_selector"]
        .as_str()
        .unwrap()
        .to_owned();

    // The challenge is single-use and issues exactly one session.
    let code = totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + STEP_SECONDS,
    );
    let completed = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": selector, "code": code, "remember": true}),
        ),
    )
    .await;
    assert_eq!(completed.status, 200);
    assert_eq!(completed.grant.expect("one session").user_id(), user_id);
    assert!(
        completed.remember_issued,
        "the login-time remember preference rides the challenge"
    );
    let replayed = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": selector, "code": code}),
        ),
    )
    .await;
    assert_ne!(replayed.status, 200, "a completed challenge cannot replay");

    // Magic-link lane: same gate, same contract.
    send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    let link = world.mail.last_payload().unwrap()["magic_link"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = link.split("token=").nth(1).unwrap().to_owned();
    let mut verify =
        magnetar::plugin::WireRequest::new(magnetar::plugin::Method::Get, "/magic-link/verify");
    verify.query.insert("token".into(), token);
    let magic = send(&world, verify).await;
    assert!(magic.grant.is_none());
    let magic_selector = magic.body.unwrap()["challenge_selector"]
        .as_str()
        .unwrap()
        .to_owned();
    // The password challenge just claimed the forward edge, so no TOTP
    // code can verify again inside this timestep — exactly the hardened
    // contract. A recovery code completes the challenge instead, proving
    // the second proof path through the same gate.
    let completed = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({
                "challenge_selector": magic_selector,
                "code": enrollment.recovery_codes[0],
            }),
        ),
    )
    .await;
    assert_eq!(completed.status, 200);
    assert!(completed.grant.is_some());

    // The passkey lane is proven in tests/passkey_flows.rs with a real
    // authenticator; a cancelled/expired challenge yields no session.
    let stale = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": "challenge-does-not-exist", "code": "000000"}),
        ),
    )
    .await;
    assert_eq!(stale.status, 400);
    assert!(stale.grant.is_none());
}
