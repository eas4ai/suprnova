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
use magnetar::storage::{
    AttemptFinalization, AttemptReservation, AttemptStats, CredentialActor, LockoutStore, UserStore,
};
use magnetar::two_factor::{
    TwoFactorConfig, TwoFactorProofClaim, TwoFactorRow, TwoFactorService, TwoFactorStore, totp,
    totp::STEP_SECONDS,
};
use parking_lot::Mutex;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, EntityTrait, IntoActiveModel};
use serde_json::json;
use tokio::sync::Barrier;

use factor::{FactorWorld, credential_actor, factor_world, send, totp_code_at, totp_code_now};
use harness::{login_request, post_json, register_request};

const EMAIL: &str = "rowan@example.test";
const PASSWORD: &str = "orange tabby cat";

struct CoordinatedAttemptStore {
    attempts: Mutex<Vec<CoordinatedAttempt>>,
    preflight: Option<Barrier>,
    fail_records: bool,
}

struct CoordinatedAttempt {
    id: String,
    attempted_at: DateTime<Utc>,
    pending: bool,
}

impl CoordinatedAttemptStore {
    fn racing(parties: usize) -> Self {
        Self {
            attempts: Mutex::new(Vec::new()),
            preflight: Some(Barrier::new(parties)),
            fail_records: false,
        }
    }

    fn failing() -> Self {
        Self {
            attempts: Mutex::new(Vec::new()),
            preflight: None,
            fail_records: true,
        }
    }

    fn stats(&self, window_start: DateTime<Utc>) -> AttemptStats {
        let attempts = self.attempts.lock();
        let mut in_window = attempts
            .iter()
            .filter(|attempt| !attempt.pending && attempt.attempted_at >= window_start)
            .map(|attempt| attempt.attempted_at)
            .collect::<Vec<_>>();
        in_window.sort_unstable();
        AttemptStats {
            count: u32::try_from(in_window.len()).unwrap(),
            latest_at: in_window.last().copied(),
        }
    }
}

#[async_trait]
impl LockoutStore for CoordinatedAttemptStore {
    async fn record_attempt_and_stats(
        &self,
        _identity: &str,
        at: DateTime<Utc>,
        _context: Option<&str>,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<AttemptStats> {
        if self.fail_records {
            return Err(magnetar::Error::Internal {
                message: "forced attempt persistence failure".to_owned(),
            });
        }
        let mut attempts = self.attempts.lock();
        let id = format!("ordinary-{}", attempts.len());
        attempts.push(CoordinatedAttempt {
            id,
            attempted_at: at,
            pending: false,
        });
        drop(attempts);
        Ok(self.stats(window_start))
    }

    async fn admit_attempt_and_stats(
        &self,
        _identity: &str,
        at: DateTime<Utc>,
        _context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: u32,
    ) -> magnetar::Result<AttemptReservation> {
        if self.fail_records {
            return Err(magnetar::Error::Internal {
                message: "forced attempt persistence failure".to_owned(),
            });
        }
        let reservation = {
            let mut attempts = self.attempts.lock();
            let current = u32::try_from(
                attempts
                    .iter()
                    .filter(|attempt| attempt.attempted_at >= window_start)
                    .count(),
            )
            .unwrap();
            let admitted = current < max_attempts;
            let reservation_id = format!("reservation-{}", attempts.len());
            if admitted {
                attempts.push(CoordinatedAttempt {
                    id: reservation_id.clone(),
                    attempted_at: at,
                    pending: true,
                });
            }
            let latest_at = attempts
                .iter()
                .filter(|attempt| !attempt.pending && attempt.attempted_at >= window_start)
                .map(|attempt| attempt.attempted_at)
                .max();
            AttemptReservation {
                admitted,
                stats: AttemptStats {
                    count: u32::try_from(
                        attempts
                            .iter()
                            .filter(|attempt| {
                                !attempt.pending && attempt.attempted_at >= window_start
                            })
                            .count(),
                    )
                    .unwrap(),
                    latest_at,
                },
                reservation_id: admitted.then_some(reservation_id),
                locked_event: false,
            }
        };
        if let Some(preflight) = &self.preflight {
            preflight.wait().await;
        }
        Ok(reservation)
    }

    async fn cancel_attempt_reservation(
        &self,
        _identity: &str,
        reservation_id: &str,
    ) -> magnetar::Result<bool> {
        let mut attempts = self.attempts.lock();
        let before = attempts.len();
        attempts.retain(|attempt| !(attempt.id == reservation_id && attempt.pending));
        Ok(attempts.len() + 1 == before)
    }

    async fn finalize_attempt_reservation(
        &self,
        _identity: &str,
        reservation_id: &str,
        _finalized_at: DateTime<Utc>,
        _context: Option<&str>,
        window_start: DateTime<Utc>,
        _max_attempts: u32,
    ) -> magnetar::Result<AttemptFinalization> {
        let mut attempts = self.attempts.lock();
        let Some(attempt) = attempts
            .iter_mut()
            .find(|attempt| attempt.id == reservation_id && attempt.pending)
        else {
            return Err(magnetar::Error::Conflict {
                resource: "attempt reservation".to_owned(),
                message: "missing pending reservation".to_owned(),
            });
        };
        attempt.pending = false;
        drop(attempts);
        Ok(AttemptFinalization {
            stats: self.stats(window_start),
            locked_event: false,
        })
    }

    async fn attempt_stats(
        &self,
        _identity: &str,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<AttemptStats> {
        let stats = self.stats(window_start);
        if let Some(preflight) = &self.preflight {
            preflight.wait().await;
        }
        Ok(stats)
    }

    async fn clear_attempts(&self, _identity: &str) -> magnetar::Result<u64> {
        let removed = self.attempts.lock().drain(..).count();
        Ok(u64::try_from(removed).unwrap())
    }

    async fn cleanup_attempts_before(&self, before: DateTime<Utc>) -> magnetar::Result<u64> {
        let mut attempts = self.attempts.lock();
        let before_count = attempts.len();
        attempts.retain(|attempt| attempt.attempted_at >= before);
        Ok(u64::try_from(before_count - attempts.len()).unwrap())
    }
}

struct FailingResetLockout {
    inner: Arc<dyn LockoutStore>,
}

struct ClaimFailingTwoFactorStore {
    inner: Arc<dyn TwoFactorStore>,
}

#[async_trait]
impl TwoFactorStore for ClaimFailingTwoFactorStore {
    async fn find_enrollment(&self, user_id: &str) -> magnetar::Result<Option<TwoFactorRow>> {
        self.inner.find_enrollment(user_id).await
    }

    async fn begin_enrollment(
        &self,
        actor: &CredentialActor,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
    ) -> magnetar::Result<bool> {
        self.inner
            .begin_enrollment(actor, secret, recovery_codes)
            .await
    }

    async fn set_confirmed(
        &self,
        actor: &CredentialActor,
        at: DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        self.inner.set_confirmed(actor, at).await
    }

    async fn claim_timestep(&self, _user_id: &str, _matched_step: i64) -> magnetar::Result<bool> {
        Err(magnetar::Error::Internal {
            message: "forced two-factor claim failure".to_owned(),
        })
    }

    async fn swap_recovery_codes(
        &self,
        user_id: &str,
        expected: &[u8],
        next: Option<&[u8]>,
    ) -> magnetar::Result<bool> {
        self.inner
            .swap_recovery_codes(user_id, expected, next)
            .await
    }

    async fn rotate_enrollment(
        &self,
        actor: &CredentialActor,
        claim: TwoFactorProofClaim,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
    ) -> magnetar::Result<bool> {
        self.inner
            .rotate_enrollment(actor, claim, secret, recovery_codes)
            .await
    }

    async fn regenerate_recovery_codes(
        &self,
        actor: &CredentialActor,
        claim: TwoFactorProofClaim,
        next: &[u8],
    ) -> magnetar::Result<bool> {
        self.inner
            .regenerate_recovery_codes(actor, claim, next)
            .await
    }

    async fn delete_enrollment(&self, actor: &CredentialActor) -> magnetar::Result<bool> {
        self.inner.delete_enrollment(actor).await
    }
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

    async fn admit_attempt_and_stats(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: u32,
    ) -> magnetar::Result<AttemptReservation> {
        self.inner
            .admit_attempt_and_stats(identity, at, context, window_start, max_attempts)
            .await
    }

    async fn cancel_attempt_reservation(
        &self,
        identity: &str,
        reservation_id: &str,
    ) -> magnetar::Result<bool> {
        self.inner
            .cancel_attempt_reservation(identity, reservation_id)
            .await
    }

    async fn finalize_attempt_reservation(
        &self,
        identity: &str,
        reservation_id: &str,
        finalized_at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: u32,
    ) -> magnetar::Result<AttemptFinalization> {
        self.inner
            .finalize_attempt_reservation(
                identity,
                reservation_id,
                finalized_at,
                context,
                window_start,
                max_attempts,
            )
            .await
    }

    async fn reset_admitted_attempts(
        &self,
        identity: &str,
        reservation_id: &str,
        context: Option<&str>,
    ) -> magnetar::Result<u64> {
        self.inner
            .reset_admitted_attempts(identity, reservation_id, context)
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

fn service_with_attempt_store(
    world: &FactorWorld,
    attempts: Arc<dyn LockoutStore>,
    max_failed_attempts: u32,
) -> Arc<TwoFactorService> {
    let lockout = Arc::new(LockoutService::new(
        attempts,
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts,
            ..LockoutConfig::default()
        },
    ));
    Arc::new(TwoFactorService::new(
        Arc::new(storage_schema::sql_two_factor::SqlTwoFactorStore(
            world.db.clone(),
        )),
        world.storage.clone(),
        lockout,
        Arc::new(AeadEncryptor::new([21; 32])),
        TwoFactorConfig::default(),
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn challenge_attempt_admission_caps_concurrent_proof_checks() {
    use magnetar::auth::{
        AuthenticationContext, FactorGate, OpaqueFactorGate, TWO_FACTOR_CHALLENGE_KIND,
    };
    use magnetar::sessions::SessionMetadata;
    use magnetar::storage::{CeremonyStore, NewCeremony};
    use serde::Serialize;

    const ATTEMPT_LIMIT: usize = 2;
    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrollment = confirmed_enrollment(&world, &user_id).await;
    let attempts = Arc::new(CoordinatedAttemptStore::racing(4));
    let service = service_with_attempt_store(&world, attempts, ATTEMPT_LIMIT as u32);
    let encryptor = Arc::new(AeadEncryptor::new([21; 32]));
    let gate = Arc::new(OpaqueFactorGate::new(
        world.storage.clone(),
        service,
        encryptor.clone(),
        world.sessions.clone(),
    ));
    let valid_code = totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + STEP_SECONDS,
    );
    let codes = [
        "invalid-one".to_owned(),
        "invalid-two".to_owned(),
        "invalid-three".to_owned(),
        valid_code,
    ];

    #[derive(Serialize)]
    struct ChallengePayload {
        user_id: String,
        context: AuthenticationContext,
    }

    let mut challenges = Vec::new();
    for index in 0..codes.len() {
        let selector = format!("attempt-admission-{index}");
        let plaintext = serde_json::to_vec(&ChallengePayload {
            user_id: user_id.clone(),
            context: AuthenticationContext::new(SessionMetadata::default(), 0, Utc::now()),
        })
        .unwrap();
        let payload = encryptor
            .encrypt(CryptoPurpose::CeremonyState, &plaintext)
            .unwrap();
        world
            .storage
            .create(NewCeremony {
                selector: selector.clone(),
                kind: TWO_FACTOR_CHALLENGE_KIND.to_owned(),
                state: "pending".to_owned(),
                payload,
                expires_at: Utc::now() + chrono::Duration::minutes(10),
            })
            .await
            .unwrap();
        challenges.push(selector);
    }

    let mut tasks = Vec::new();
    for (selector, code) in challenges.into_iter().zip(codes) {
        let gate = gate.clone();
        tasks.push(tokio::spawn(async move {
            let result = gate.complete_challenge(&selector, &code).await;
            (code, result)
        }));
    }

    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("challenge attempt task joins"));
    }

    assert_eq!(
        outcomes
            .iter()
            .filter(|(_, outcome)| matches!(outcome, Err(magnetar::Error::Conflict { resource, .. }) if resource == "account lockout"))
            .count(),
        2,
        "every request beyond the atomic admission budget must be rejected"
    );
}

#[tokio::test]
async fn challenge_attempt_admission_fails_when_its_write_fails() {
    use magnetar::auth::FactorVerifier;

    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    confirmed_enrollment(&world, &user_id).await;
    let service =
        service_with_attempt_store(&world, Arc::new(CoordinatedAttemptStore::failing()), 5);

    let error = service
        .prepare_code(&user_id, "invalid-code")
        .await
        .expect_err("proof evaluation must not start when admission persistence fails");

    assert!(
        matches!(error, magnetar::Error::Internal { message } if message == "forced attempt persistence failure")
    );
}

#[tokio::test]
async fn challenge_prepare_error_cancels_attempt_without_locking_user() {
    use magnetar::auth::FactorVerifier;

    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    confirmed_enrollment(&world, &user_id).await;
    let row = storage_schema::two_factor::Entity::find_by_id(user_id.clone())
        .one(&world.db)
        .await
        .unwrap()
        .unwrap();
    let mut row = row.into_active_model();
    row.secret = Set(vec![0_u8; 3]);
    row.update(&world.db).await.unwrap();
    let lockout = Arc::new(LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: 1,
            ..LockoutConfig::default()
        },
    ));
    let service = TwoFactorService::new(
        Arc::new(storage_schema::sql_two_factor::SqlTwoFactorStore(
            world.db.clone(),
        )),
        world.storage.clone(),
        lockout.clone(),
        Arc::new(AeadEncryptor::new([21; 32])),
        TwoFactorConfig::default(),
    );

    service
        .prepare_code(&user_id, "123456")
        .await
        .expect_err("corrupt stored proof material must fail preparation");

    assert_eq!(lockout.status(EMAIL).await.unwrap().failed_attempts, 0);
    assert!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .unwrap()
            .locked_at
            .is_none()
    );
}

#[tokio::test]
async fn challenge_claim_error_cancels_attempt_without_locking_user() {
    use magnetar::auth::{
        AuthenticationContext, FactorGate, OpaqueFactorGate, TWO_FACTOR_CHALLENGE_KIND,
    };
    use magnetar::sessions::SessionMetadata;
    use magnetar::storage::{CeremonyStore, NewCeremony};
    use serde::Serialize;

    let world = factor_world().await;
    let user_id = registered_user(&world).await;
    let enrollment = confirmed_enrollment(&world, &user_id).await;
    let lockout = Arc::new(LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: 1,
            ..LockoutConfig::default()
        },
    ));
    let factor_store = Arc::new(ClaimFailingTwoFactorStore {
        inner: Arc::new(storage_schema::sql_two_factor::SqlTwoFactorStore(
            world.db.clone(),
        )),
    });
    let service = Arc::new(TwoFactorService::new(
        factor_store,
        world.storage.clone(),
        lockout.clone(),
        Arc::new(AeadEncryptor::new([21; 32])),
        TwoFactorConfig::default(),
    ));
    let encryptor = Arc::new(AeadEncryptor::new([21; 32]));
    let gate = OpaqueFactorGate::new(
        world.storage.clone(),
        service,
        encryptor.clone(),
        world.sessions.clone(),
    );

    #[derive(Serialize)]
    struct ChallengePayload {
        user_id: String,
        context: AuthenticationContext,
    }

    let selector = "claim-failure-cancellation";
    let plaintext = serde_json::to_vec(&ChallengePayload {
        user_id: user_id.clone(),
        context: AuthenticationContext::new(SessionMetadata::default(), 0, Utc::now()),
    })
    .unwrap();
    let payload = encryptor
        .encrypt(CryptoPurpose::CeremonyState, &plaintext)
        .unwrap();
    world
        .storage
        .create(NewCeremony {
            selector: selector.to_owned(),
            kind: TWO_FACTOR_CHALLENGE_KIND.to_owned(),
            state: "pending".to_owned(),
            payload,
            expires_at: Utc::now() + chrono::Duration::minutes(10),
        })
        .await
        .unwrap();

    let error = gate
        .complete_challenge(
            selector,
            &totp_code_at(
                &enrollment.otpauth_url,
                Utc::now().timestamp() + STEP_SECONDS,
            ),
        )
        .await
        .expect_err("claim failure must propagate after cancellation");

    assert_eq!(lockout.status(EMAIL).await.unwrap().failed_attempts, 0);
    assert!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .unwrap()
            .locked_at
            .is_none()
    );
    assert!(matches!(
        error,
        magnetar::Error::Internal { message }
            if message == "forced two-factor claim failure"
    ));
}

#[tokio::test]
async fn finalized_attempt_retries_user_lock_transition_exactly_once() {
    let world = factor_world().await;
    registered_user(&world).await;
    let lockout = LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: 1,
            ..LockoutConfig::default()
        },
    );
    let admission = lockout
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .expect("reserve threshold attempt");
    world
        .db
        .execute_unprepared(
            "CREATE TRIGGER fail_lockout_user_stamp BEFORE UPDATE OF locked_at ON storage_users WHEN NEW.locked_at IS NOT NULL BEGIN SELECT RAISE(FAIL, 'forced user lock persistence failure'); END",
        )
        .await
        .unwrap();

    let error = lockout
        .finalize_failed_attempt(EMAIL, &admission)
        .await
        .expect_err("first user lock write is injected to fail");
    assert!(matches!(
        error,
        magnetar::Error::DependencyUnavailable { .. } | magnetar::Error::Internal { .. }
    ));
    assert_eq!(lockout.status(EMAIL).await.unwrap().failed_attempts, 0);
    assert!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .unwrap()
            .locked_at
            .is_none()
    );
    world
        .db
        .execute_unprepared("DROP TRIGGER fail_lockout_user_stamp")
        .await
        .unwrap();

    let repaired = lockout
        .finalize_failed_attempt(EMAIL, &admission)
        .await
        .expect("same exact reservation must repair its user lock transition");
    assert!(repaired.locked_event);
    assert_eq!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .unwrap()
            .locked_at,
        repaired
            .status
            .locked_until
            .map(|locked_until| locked_until - LockoutConfig::default().lockout_period),
        "a delayed same-token repair must preserve the original lock cycle timestamp"
    );

    let repeated = lockout
        .finalize_failed_attempt(EMAIL, &admission)
        .await
        .expect("idempotent retry after repair remains valid");
    assert!(!repeated.locked_event);
}

#[tokio::test]
async fn rejected_admission_repairs_a_finalized_user_lock_transition() {
    let world = factor_world().await;
    registered_user(&world).await;
    let lockout = LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: 1,
            ..LockoutConfig::default()
        },
    );
    let cycle_at = Utc::now();
    world
        .storage
        .record_attempt_and_stats(
            EMAIL,
            cycle_at,
            Some("legacy finalized attempt"),
            cycle_at - LockoutConfig::default().lockout_period,
        )
        .await
        .expect("seed finalized attempt without a user lock stamp");

    let repair = lockout
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .expect("later admission repairs committed failure state");
    assert!(!repair.admitted);
    assert!(repair.locked_event);
    assert_eq!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .unwrap()
            .locked_at,
        repair
            .status
            .locked_until
            .map(|locked_until| locked_until - LockoutConfig::default().lockout_period),
        "rejected-admission repair must preserve the finalized cycle timestamp"
    );

    let repeated = lockout
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .expect("repeated rejected admission is idempotent");
    assert!(!repeated.admitted);
    assert!(!repeated.locked_event);
}

#[tokio::test]
async fn delayed_repair_does_not_suppress_the_next_lock_cycle_transition() {
    let world = factor_world().await;
    registered_user(&world).await;
    let period = chrono::Duration::seconds(1);
    let lockout = LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: 1,
            lockout_period: period,
            ..LockoutConfig::default()
        },
    );
    let first_cycle_at = Utc::now() - chrono::Duration::milliseconds(750);
    world
        .storage
        .record_attempt_and_stats(
            EMAIL,
            first_cycle_at,
            Some("legacy finalized attempt"),
            first_cycle_at - period,
        )
        .await
        .unwrap();

    let repair = lockout
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .unwrap();
    assert!(!repair.admitted);
    assert!(repair.locked_event);
    assert_eq!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .unwrap()
            .locked_at,
        Some(first_cycle_at)
    );

    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    let next_admission = lockout
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .expect("the expired first-cycle attempt must free admission capacity");
    assert!(next_admission.admitted);
    let next_failure = lockout
        .finalize_failed_attempt(EMAIL, &next_admission)
        .await
        .expect("the next cycle must finalize normally");
    assert!(next_failure.locked_event);
}

#[tokio::test]
async fn seaorm_success_reset_preserves_other_pending_reservations() {
    let world = factor_world().await;
    registered_user(&world).await;
    let service = LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: 5,
            ..LockoutConfig::default()
        },
    );
    let prior_failure_at = Utc::now();
    world
        .storage
        .record_attempt_and_stats(
            EMAIL,
            prior_failure_at,
            Some("prior finalized failure"),
            prior_failure_at - chrono::Duration::minutes(15),
        )
        .await
        .unwrap();
    world
        .storage
        .set_locked_at_by_email(EMAIL, Some(prior_failure_at))
        .await
        .unwrap();
    let successful = service
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .unwrap();
    let later_failure = service
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .unwrap();
    let later_abort = service
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .unwrap();

    service
        .reset_admitted_attempts(EMAIL, &successful)
        .await
        .expect("successful proof resets its own reservation");
    assert_eq!(service.status(EMAIL).await.unwrap().failed_attempts, 0);
    assert!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .unwrap()
            .locked_at
            .is_none()
    );

    let finalized = service
        .finalize_failed_attempt(EMAIL, &later_failure)
        .await
        .expect("another admitted request keeps its exact reservation");
    assert_eq!(finalized.status.failed_attempts, 1);
    service
        .cancel_attempt(EMAIL, &later_abort)
        .await
        .expect("another admitted request may still cancel its reservation");
    assert_eq!(service.status(EMAIL).await.unwrap().failed_attempts, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seaorm_attempt_admission_never_exceeds_its_concurrent_budget() {
    const ATTEMPT_LIMIT: usize = 2;
    let world = factor_world().await;
    registered_user(&world).await;
    let service = Arc::new(LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: ATTEMPT_LIMIT as u32,
            ..LockoutConfig::default()
        },
    ));
    let start = Arc::new(Barrier::new(5));

    let mut tasks = Vec::new();
    for _ in 0..4 {
        let service = service.clone();
        let start = start.clone();
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            service
                .admit_attempt(EMAIL, Some("two-factor challenge"))
                .await
        }));
    }
    start.wait().await;

    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(
            task.await
                .expect("SeaORM admission task joins")
                .expect("SeaORM admission succeeds"),
        );
    }

    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.admitted).count(),
        ATTEMPT_LIMIT
    );
    assert_eq!(
        service.status(EMAIL).await.unwrap().failed_attempts,
        0,
        "pending reservations consume admission capacity but are not public failures"
    );
    assert!(
        !service
            .admit_attempt(EMAIL, Some("two-factor challenge"))
            .await
            .unwrap()
            .admitted,
        "pending reservations must continue to consume the bounded admission budget"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seaorm_finalize_and_distinct_admitted_reset_serialize_consistently() {
    let world = factor_world().await;
    registered_user(&world).await;
    let service = Arc::new(LockoutService::new(
        world.storage.clone(),
        world.storage.clone(),
        LockoutConfig {
            max_failed_attempts: 5,
            ..LockoutConfig::default()
        },
    ));
    let seeded_at = Utc::now();
    for offset in 1..=3 {
        world
            .storage
            .record_attempt_and_stats(
                EMAIL,
                seeded_at - chrono::Duration::milliseconds(offset),
                Some("earlier finalized failure"),
                seeded_at - chrono::Duration::minutes(15),
            )
            .await
            .unwrap();
    }
    let invalid_admission = service
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .expect("reserve the invalid real SeaORM attempt");
    let valid_admission = service
        .admit_attempt(EMAIL, Some("two-factor challenge"))
        .await
        .expect("reserve the valid real SeaORM attempt");
    assert!(invalid_admission.admitted && valid_admission.admitted);
    world
        .storage
        .record_attempt_and_stats(
            EMAIL,
            Utc::now(),
            Some("concurrent primary-auth failure"),
            Utc::now() - chrono::Duration::minutes(15),
        )
        .await
        .unwrap();
    assert_eq!(service.status(EMAIL).await.unwrap().failed_attempts, 4);
    let start = Arc::new(Barrier::new(3));

    let invalid_service = service.clone();
    let invalid_start = start.clone();
    let invalid = tokio::spawn(async move {
        invalid_start.wait().await;
        invalid_service
            .finalize_failed_attempt(EMAIL, &invalid_admission)
            .await
    });
    let valid_service = service.clone();
    let valid_start = start.clone();
    let valid = tokio::spawn(async move {
        valid_start.wait().await;
        valid_service
            .reset_admitted_attempts(EMAIL, &valid_admission)
            .await
    });
    start.wait().await;

    let invalid = invalid.await.unwrap();
    let valid = valid.await.unwrap();
    valid.expect("the valid admitted reset must serialize and clear finalized history");
    let failure = invalid.expect("the other admitted request keeps its reservation");
    let status = service.status(EMAIL).await.unwrap();
    let locked_at = world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .unwrap()
        .locked_at;
    assert_eq!(
        status.failed_attempts,
        u32::from(!failure.locked_event),
        "invalid-first is cleared by success; success-first leaves the later invalid failure"
    );
    assert!(!status.is_locked);
    assert!(locked_at.is_none());
}

#[tokio::test]
async fn legacy_null_locked_at_attempt_remains_a_finalized_public_failure() {
    let world = factor_world().await;
    let attempted_at = Utc::now();
    storage_schema::lockouts::ActiveModel {
        id: Set(42),
        identity: Set("legacy-null@example.test".to_owned()),
        attempted_at: Set(attempted_at),
        locked_at: Set(None),
        reason: Set(None),
    }
    .insert(&world.db)
    .await
    .expect("seed legacy lockout row");

    let stats = world
        .storage
        .attempt_stats(
            "legacy-null@example.test",
            attempted_at - chrono::Duration::minutes(1),
        )
        .await
        .expect("legacy lockout stats");

    assert_eq!(stats.count, 1);
    assert_eq!(stats.latest_at, Some(attempted_at));
}

#[tokio::test]
async fn abandoned_pending_reservation_releases_capacity_after_its_window() {
    let world = factor_world().await;
    let identity = "abandoned-pending@example.test";
    let fresh_at = Utc::now();
    let stale_at = fresh_at - chrono::Duration::hours(1);
    let stale = world
        .storage
        .admit_attempt_and_stats(
            identity,
            stale_at,
            Some("two-factor challenge"),
            stale_at - chrono::Duration::minutes(15),
            1,
        )
        .await
        .expect("seed abandoned pending reservation");
    assert!(stale.admitted);

    let fresh = world
        .storage
        .admit_attempt_and_stats(
            identity,
            fresh_at,
            Some("two-factor challenge"),
            fresh_at - chrono::Duration::minutes(15),
            1,
        )
        .await
        .expect("fresh reservation after stale window");

    assert!(fresh.admitted);
    assert_eq!(
        world
            .storage
            .attempt_stats(identity, fresh_at - chrono::Duration::minutes(15))
            .await
            .unwrap()
            .count,
        0,
        "neither abandoned nor fresh pending reservations are public failures"
    );
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
    // and verifies once, claiming its matched step - NOT server time.
    let forward = totp_code_at(&enrollment.otpauth_url, now + STEP_SECONDS);
    assert!(world.two_factor.verify(&user_id, &forward).await.unwrap());

    // The same forward code can never be accepted again - including when
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
    // code can verify again inside this timestep - exactly the hardened
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
