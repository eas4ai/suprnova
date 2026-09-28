//! End-to-end password-domain flows over the real stores and plugins.
//!
//! Ports the observable behavior of torii's password service and the
//! deployed Suprnova reset/verification/lockout flows: idempotent
//! registration, indistinguishable failures, upgrade-only rehash, the atomic
//! reset composite (epoch bump + all-session revocation in one commit), and
//! lockout as the recovery-gated account state.

#![cfg(all(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "seaorm-sqlite"
))]

#[path = "fixtures/password_harness.rs"]
mod harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Duration;
use hmac::{Hmac, Mac};
use magnetar::password::{
    HashParameters, HashWorkProfile, LockoutConfig, PasswordHashConfig, PasswordHashDriver,
    StandardPasswordHashDriver, VerificationCall,
};
use magnetar::plugin::BearerCredential;
use magnetar::plugins::password::{
    PasswordAttempt, PasswordAuthProvider, RegisterInput, RegistrationOutcome, RehashReport,
};
use magnetar::sessions::{JwtConfig, JwtSessionProvider, SessionMetadata, SessionQueries};
use magnetar::storage::{CredentialActor, UserStore};
use parking_lot::Mutex;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;

use harness::{
    Harness, dispatch, fast_hash_config, harness, harness_with, login_request, post_json,
    register_request, split,
};
use storage_schema::StorageSchema;

const EMAIL: &str = "carol@example.test";
const PASSWORD: &str = "orange tabby cat";
#[derive(Default)]
struct WorkSpyDriver {
    mints: Mutex<Vec<HashWorkProfile>>,
    verifies: Mutex<Vec<HashWorkProfile>>,
}

impl WorkSpyDriver {
    fn drain_mints(&self) -> Vec<HashWorkProfile> {
        std::mem::take(&mut *self.mints.lock())
    }

    fn drain_verifies(&self) -> Vec<HashWorkProfile> {
        std::mem::take(&mut *self.verifies.lock())
    }
}

fn spy_password_hash(profile: &HashWorkProfile, password: &str) -> String {
    match profile.parameters {
        HashParameters::Bcrypt { cost } => format!("$2b${cost:02}$spy:{password}"),
        HashParameters::Argon2 {
            memory_kib,
            iterations,
            parallelism,
        } => format!("$argon2id$v=19$m={memory_kib},t={iterations},p={parallelism}$spy:{password}"),
    }
}

impl PasswordHashDriver for WorkSpyDriver {
    fn verify(&self, call: &VerificationCall<'_>) -> magnetar::Result<bool> {
        self.verifies.lock().push(call.profile);
        Ok(call.hash == spy_password_hash(&call.profile, call.password.expose_secret()))
    }

    fn mint(&self, profile: &HashWorkProfile, password: &SecretString) -> magnetar::Result<String> {
        self.mints.lock().push(*profile);
        Ok(spy_password_hash(profile, password.expose_secret()))
    }
}

fn assert_configured_dual_work(work: &[HashWorkProfile], config: PasswordHashConfig) {
    assert_eq!(
        work,
        [config.bcrypt_profile(), config.argon2_target()],
        "every rejected login must execute the configured bcrypt and Argon2 verifier lanes"
    );
}

fn query_param(link: &str, name: &str) -> String {
    let query = link.split('?').nth(1).expect("link has a query string");
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if parts.next() == Some(name) {
            return parts.next().expect("parameter has a value").to_owned();
        }
    }
    panic!("link {link} is missing parameter {name}");
}

async fn register(world: &Harness) -> String {
    let reply = dispatch(world, register_request(EMAIL, PASSWORD)).await;
    assert_eq!(reply.status, 200);
    world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .expect("registration created the user")
        .user_id
}

#[tokio::test]
async fn register_is_idempotent_and_existing_email_stays_generic() {
    let world = harness().await;
    let first = dispatch(&world, register_request(EMAIL, PASSWORD)).await;
    assert_eq!(first.status, 200);
    assert_eq!(first.body, Some(json!({"status": "ok"})));
    assert!(
        first.grant.is_none(),
        "register does not establish a session"
    );
    assert_eq!(world.mail.names(), vec!["email_verification"]);
    let stored = world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .expect("user exists");

    // Re-registering the same email with an attacker's password: the
    // response body is byte-identical, no session, no second verification
    // mail, no credential change, and no lockout mutation.
    let second = dispatch(&world, register_request(EMAIL, "attacker password")).await;
    assert_eq!(second.status, first.status);
    assert_eq!(second.body, first.body);
    assert!(second.grant.is_none());
    assert_eq!(world.mail.count(), 1, "no second verification mail");
    let after = world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .expect("user still exists");
    assert_eq!(after.password_hash, stored.password_hash);
    assert_eq!(after.user_id, stored.user_id);
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        0,
        "existing-email registration moves no lockout counter"
    );

    // The provider surfaces the created/existing split internally while the
    // route stays generic.
    let outcome = world
        .provider
        .register(RegisterInput {
            email: EMAIL.into(),
            password: SecretString::from(PASSWORD),
        })
        .await
        .unwrap();
    assert!(
        matches!(&outcome, RegistrationOutcome::Existing { user_id } if user_id == &stored.user_id)
    );
}

#[tokio::test]
async fn registration_mints_exactly_one_target_hash_for_created_and_existing_email() {
    let config = fast_hash_config();
    let driver = Arc::new(WorkSpyDriver::default());
    let world = harness_with(driver.clone(), config, LockoutConfig::default()).await;
    driver.drain_mints();

    let created = dispatch(&world, register_request(EMAIL, PASSWORD)).await;
    assert_eq!(created.status, 200);
    assert_eq!(created.body, Some(json!({"status": "ok"})));
    assert_eq!(
        driver.drain_mints(),
        [config.argon2_target()],
        "new-email registration must mint the target credential exactly once"
    );

    let before_user = world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .expect("new-email registration creates the user");
    let before_mail = world.mail.count();
    let before_lockout = world.lockout.status(EMAIL).await.unwrap();

    let existing = dispatch(
        &world,
        register_request("  CAROL@example.test ", "attacker password"),
    )
    .await;

    assert_eq!(
        driver.drain_mints(),
        [config.argon2_target()],
        "existing-email registration must pay exactly the same target-cost mint as creation"
    );
    assert_eq!(existing.status, created.status);
    assert_eq!(
        serde_json::to_vec(&existing.body).unwrap(),
        serde_json::to_vec(&created.body).unwrap(),
        "created and existing registration responses must be byte-identical"
    );
    assert!(existing.grant.is_none());
    assert_eq!(
        world
            .storage
            .find_by_email(EMAIL)
            .await
            .unwrap()
            .expect("existing user remains present"),
        before_user,
        "the equalized mint must never replace the existing credential or user state"
    );
    assert_eq!(
        world.mail.count(),
        before_mail,
        "existing-email registration must not send another verification message"
    );
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap(),
        before_lockout,
        "existing-email registration must not mutate lockout state"
    );
}

#[tokio::test]
async fn email_verification_round_trip_is_single_use() {
    let world = harness().await;
    let user_id = register(&world).await;
    let link = world.mail.last_payload().unwrap()["verification_link"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = query_param(&link, "hash");
    assert_eq!(query_param(&link, "id"), user_id);

    // Non-consuming check leaves the token alive.
    assert!(world.verification.check(&token).await.unwrap());

    let mut verify = magnetar::plugin::WireRequest::new(
        magnetar::plugin::Method::Get,
        format!("/email/verify/{user_id}/{token}"),
    );
    verify.headers.insert("user-agent".into(), "harness".into());
    let logged_out = split(world.registry.handle(verify.clone()).await.unwrap());
    assert_eq!(logged_out.status, 400);
    assert!(
        world.verification.check(&token).await.unwrap(),
        "logged-out verification must not consume the token"
    );

    let login = dispatch(&world, login_request(EMAIL, PASSWORD)).await;
    let bearer = login
        .grant
        .expect("password sign-in issues a session")
        .into_bearer()
        .expose_token_once();
    let bearer_value = bearer.expose_secret().to_owned();
    let reply = split(
        world
            .registry
            .handle_bound(
                verify.clone(),
                Some(BearerCredential::new(bearer_value.clone())),
            )
            .await
            .unwrap(),
    );
    assert_eq!(reply.status, 200);
    let stamped = world
        .storage
        .find_by_id(&user_id)
        .await
        .unwrap()
        .expect("user exists");
    assert!(
        stamped.email_verified_at.is_some(),
        "verify stamps the timestamp"
    );

    // Single use: the same authenticated link answers 400 the second time.
    let replay = split(
        world
            .registry
            .handle_bound(verify, Some(BearerCredential::new(bearer_value)))
            .await
            .unwrap(),
    );
    assert_eq!(replay.status, 400);
}

#[tokio::test]
async fn verification_resend_is_anti_enumeration() {
    let world = harness().await;
    register(&world).await;
    let baseline = world.mail.count();

    let present = dispatch(
        &world,
        post_json("/email/verification-notification", json!({"email": EMAIL})),
    )
    .await;
    let absent = dispatch(
        &world,
        post_json(
            "/email/verification-notification",
            json!({"email": "nobody@example.test"}),
        ),
    )
    .await;
    assert_eq!(present.status, absent.status);
    assert_eq!(present.body, absent.body);
    assert_eq!(
        world.mail.count(),
        baseline + 1,
        "present resends exactly one mail; absent mints and mails nothing"
    );
}

#[tokio::test]
async fn mismatched_authenticated_actor_does_not_consume_verification_token() {
    let world = harness().await;
    let user_id = register(&world).await;
    let link = world.mail.last_payload().unwrap()["verification_link"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = query_param(&link, "hash");
    world
        .provider
        .register(RegisterInput {
            email: "other@example.test".to_owned(),
            password: SecretString::from("other honest password"),
        })
        .await
        .unwrap();
    let login = dispatch(
        &world,
        login_request("other@example.test", "other honest password"),
    )
    .await;
    let bearer = login
        .grant
        .expect("other user signs in")
        .into_bearer()
        .expose_token_once();
    let request = magnetar::plugin::WireRequest::new(
        magnetar::plugin::Method::Get,
        format!("/email/verify/{user_id}/{token}"),
    );
    let mismatched = split(
        world
            .registry
            .handle_bound(
                request,
                Some(BearerCredential::new(bearer.expose_secret().to_owned())),
            )
            .await
            .unwrap(),
    );
    assert_eq!(mismatched.status, 400);
    let user = world.storage.find_by_id(&user_id).await.unwrap().unwrap();
    assert!(user.email_verified_at.is_none(), "mismatch must not stamp");
    assert!(
        world.verification.check(&token).await.unwrap(),
        "actor mismatch must not consume the token"
    );
}

#[tokio::test]
async fn login_failures_are_indistinguishable_and_recorded() {
    let world = harness().await;
    register(&world).await;
    // A passwordless account alongside the credentialed one.
    world
        .storage
        .create_user(magnetar::storage::NewUser {
            email: "passwordless@example.test".into(),
            password_hash: None,
        })
        .await
        .unwrap();

    let wrong = dispatch(&world, login_request(EMAIL, "wrong password")).await;
    let unknown = dispatch(
        &world,
        login_request("nobody@example.test", "wrong password"),
    )
    .await;
    let passwordless = dispatch(
        &world,
        login_request("passwordless@example.test", "wrong password"),
    )
    .await;
    for reply in [&wrong, &unknown, &passwordless] {
        assert_eq!(reply.status, 401);
        assert_eq!(reply.body, wrong.body);
        assert!(reply.grant.is_none());
    }
    // Every failure recorded one lockout attempt under its own identity.
    for identity in [EMAIL, "nobody@example.test", "passwordless@example.test"] {
        assert_eq!(
            world
                .lockout
                .status(identity)
                .await
                .unwrap()
                .failed_attempts,
            1,
            "{identity} must have one recorded attempt"
        );
    }
}

#[tokio::test]
async fn failed_attempt_storage_failure_returns_service_unavailable() {
    use sea_orm::{ConnectionTrait as _, EntityTrait as _, PaginatorTrait as _};

    let hash_config = fast_hash_config();
    let driver = Arc::new(WorkSpyDriver::default());
    let world = harness_with(driver.clone(), hash_config, LockoutConfig::default()).await;
    register(&world).await;
    driver.drain_mints();
    assert!(
        world.storage.find_by_email(EMAIL).await.unwrap().is_some(),
        "the trigger must not interfere with user lookup"
    );

    world
        .db
        .execute_unprepared(
            "CREATE TRIGGER fail_failed_attempt_insert \
             BEFORE INSERT ON storage_lockouts \
             WHEN NEW.reason = '203.0.113.7' \
             BEGIN \
                 SELECT RAISE(ABORT, 'injected failed-attempt write failure'); \
             END",
        )
        .await
        .unwrap();

    let reply = dispatch(&world, login_request(EMAIL, "wrong password")).await;

    assert_configured_dual_work(&driver.drain_verifies(), hash_config);
    assert_eq!(reply.status, 503);
    assert_eq!(reply.body, Some(json!({"message": "service unavailable"})));
    assert!(reply.grant.is_none(), "failed accounting grants no session");
    assert_eq!(
        storage_schema::sessions::Entity::find()
            .count(&world.db)
            .await
            .unwrap(),
        0,
        "failed accounting persists no session"
    );
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        0,
        "the aborted failed-attempt transaction leaves no partial accounting row"
    );
}

#[tokio::test]
async fn successful_login_passes_the_gate_and_upgrades_legacy_bcrypt() {
    let world = harness().await;
    // Seed a legacy bcrypt credential exactly as the framework minted it.
    let legacy = StandardPasswordHashDriver
        .mint(
            &fast_hash_config().bcrypt_profile(),
            &SecretString::from(PASSWORD),
        )
        .unwrap();
    let user = world
        .storage
        .create_user(magnetar::storage::NewUser {
            email: EMAIL.into(),
            password_hash: Some(legacy.clone()),
        })
        .await
        .unwrap();

    let reply = dispatch(&world, login_request(EMAIL, PASSWORD)).await;
    assert_eq!(reply.status, 200);
    let grant = reply.grant.expect("session established through the gate");
    assert_eq!(grant.user_id(), user.user_id);

    // Upgrade-only rehash: the stored credential is now the Argon2id target.
    let upgraded = world
        .storage
        .find_by_id(&user.user_id)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .expect("credential retained");
    assert_ne!(upgraded, legacy);
    assert!(upgraded.starts_with("$argon2id$"));

    // Second login verifies through the Argon2 lane and does not rewrite.
    let again = dispatch(&world, login_request(EMAIL, PASSWORD)).await;
    assert_eq!(again.status, 200);
    let unchanged = world
        .storage
        .find_by_id(&user.user_id)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .unwrap();
    assert_eq!(
        unchanged, upgraded,
        "a target-strength hash is never rewritten"
    );

    // Counters cleared on success.
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        0
    );

    // The bearer token from the grant verifies against the live store.
    let bearer = grant.into_bearer().expose_token_once();
    let session = world
        .sessions
        .verify_bearer(bearer.expose_secret())
        .await
        .unwrap();
    assert_eq!(session.user_id(), user.user_id);
    assert_eq!(
        session.metadata().user_agent.as_deref(),
        Some("harness-agent")
    );
    assert_eq!(
        session.metadata().ip_address.as_deref(),
        Some("203.0.113.7")
    );
}

#[tokio::test]
async fn long_passwords_flow_through_the_argon2_lane() {
    let world = harness().await;
    let long_password = "p".repeat(128);
    let reply = dispatch(&world, register_request(EMAIL, &long_password)).await;
    assert_eq!(reply.status, 200);
    let login = dispatch(&world, login_request(EMAIL, &long_password)).await;
    assert_eq!(login.status, 200, "128-byte passwords authenticate");
    assert!(login.grant.is_some());
}

/// A driver that fails target minting on demand, after warmup.
struct FailingMint {
    inner: StandardPasswordHashDriver,
    fail: AtomicBool,
}

impl PasswordHashDriver for FailingMint {
    fn verify(&self, call: &VerificationCall<'_>) -> magnetar::Result<bool> {
        self.inner.verify(call)
    }
    fn mint(&self, profile: &HashWorkProfile, password: &SecretString) -> magnetar::Result<String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(magnetar::Error::Internal {
                message: "mint outage".into(),
            });
        }
        self.inner.mint(profile, password)
    }
}

#[tokio::test]
async fn rehash_failure_is_a_post_login_outcome_not_an_auth_failure() {
    let driver = Arc::new(FailingMint {
        inner: StandardPasswordHashDriver,
        fail: AtomicBool::new(false),
    });
    let world = harness_with(driver.clone(), fast_hash_config(), LockoutConfig::default()).await;
    let legacy = StandardPasswordHashDriver
        .mint(
            &fast_hash_config().bcrypt_profile(),
            &SecretString::from(PASSWORD),
        )
        .unwrap();
    let user = world
        .storage
        .create_user(magnetar::storage::NewUser {
            email: EMAIL.into(),
            password_hash: Some(legacy.clone()),
        })
        .await
        .unwrap();

    driver.fail.store(true, Ordering::SeqCst);
    let (principal, report) = world
        .provider
        .authenticate_with_outcome(PasswordAttempt {
            email: EMAIL.into(),
            password: SecretString::from(PASSWORD),
            metadata: SessionMetadata::default(),
        })
        .await
        .expect("a valid credential authenticates even when rehash fails");
    assert_eq!(principal.user_id(), user.user_id);
    assert!(matches!(&report, RehashReport::Failed { .. }));
    let unchanged = world
        .storage
        .find_by_id(&user.user_id)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .unwrap();
    assert_eq!(
        unchanged, legacy,
        "a failed rehash leaves the credential intact"
    );
}

#[tokio::test]
async fn locked_unknown_and_wrong_password_logins_are_generic_and_work_equivalent() {
    const OTHER_EMAIL: &str = "other@example.test";
    let hash_config = fast_hash_config();
    let driver = Arc::new(WorkSpyDriver::default());
    let lockout_config = LockoutConfig {
        max_failed_attempts: 1,
        ..LockoutConfig::default()
    };
    let world = harness_with(driver.clone(), hash_config, lockout_config).await;
    driver.drain_mints();

    let first_registration = dispatch(&world, register_request(EMAIL, PASSWORD)).await;
    let second_registration = dispatch(
        &world,
        register_request(OTHER_EMAIL, "other honest password"),
    )
    .await;
    assert_eq!(first_registration.status, 200);
    assert_eq!(second_registration.status, 200);
    driver.drain_mints();

    let threshold = dispatch(&world, login_request(EMAIL, "wrong before lock")).await;
    assert_eq!(threshold.status, 401);
    assert_configured_dual_work(&driver.drain_verifies(), hash_config);
    assert!(world.lockout.status(EMAIL).await.unwrap().is_locked);

    let locked = dispatch(&world, login_request(EMAIL, PASSWORD)).await;
    let locked_work = driver.drain_verifies();
    let unknown = dispatch(
        &world,
        login_request("nobody@example.test", "irrelevant password"),
    )
    .await;
    let unknown_work = driver.drain_verifies();
    let wrong = dispatch(&world, login_request(OTHER_EMAIL, "not the other password")).await;
    let wrong_work = driver.drain_verifies();

    for (label, reply) in [
        ("locked known account", &locked),
        ("unknown account", &unknown),
        ("known account with wrong password", &wrong),
    ] {
        assert_eq!(reply.status, 401, "{label} must return generic 401");
        assert!(
            !reply
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("retry-after")),
            "{label} must not expose retry timing"
        );
    }
    let generic_body = serde_json::to_vec(&unknown.body).unwrap();
    assert_eq!(
        serde_json::to_vec(&locked.body).unwrap(),
        generic_body,
        "locked and unknown responses must be byte-identical"
    );
    assert_eq!(
        serde_json::to_vec(&wrong.body).unwrap(),
        generic_body,
        "wrong-password and unknown responses must be byte-identical"
    );
    assert_configured_dual_work(&locked_work, hash_config);
    assert_configured_dual_work(&unknown_work, hash_config);
    assert_configured_dual_work(&wrong_work, hash_config);
}

/// Counting wrapper proving locked attempts retain fixed verifier work.
struct CountingDriver {
    inner: StandardPasswordHashDriver,
    verifies: AtomicUsize,
}

impl PasswordHashDriver for CountingDriver {
    fn verify(&self, call: &VerificationCall<'_>) -> magnetar::Result<bool> {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        self.inner.verify(call)
    }
    fn mint(&self, profile: &HashWorkProfile, password: &SecretString) -> magnetar::Result<String> {
        self.inner.mint(profile, password)
    }
}

#[tokio::test]
async fn lockout_locks_after_threshold_and_reset_is_the_recovery_path() {
    let driver = Arc::new(CountingDriver {
        inner: StandardPasswordHashDriver,
        verifies: AtomicUsize::new(0),
    });
    let config = LockoutConfig {
        max_failed_attempts: 3,
        lockout_period: Duration::minutes(15),
        ..LockoutConfig::default()
    };
    let world = harness_with(driver.clone(), fast_hash_config(), config).await;
    let user_id = register(&world).await;

    let mut generic_failure = None;
    for _ in 0..3 {
        let reply = dispatch(&world, login_request(EMAIL, "wrong password")).await;
        assert_eq!(
            reply.status, 401,
            "pre-lock failures stay indistinguishable"
        );
        generic_failure = Some(serde_json::to_vec(&reply.body).unwrap());
    }
    let status = world.lockout.status(EMAIL).await.unwrap();
    assert!(status.is_locked);
    assert!(status.retry_after_seconds().unwrap() > 0);
    // The threshold crossing stamped the user row.
    let locked_user = world.storage.find_by_id(&user_id).await.unwrap().unwrap();
    assert!(locked_user.locked_at.is_some());

    // Lockout remains recovery-gated state, but it is not exposed as a
    // distinguishable response or a zero-work verifier branch.
    let before = driver.verifies.load(Ordering::SeqCst);
    let locked = dispatch(&world, login_request(EMAIL, PASSWORD)).await;
    assert_eq!(locked.status, 401);
    assert_eq!(
        serde_json::to_vec(&locked.body).unwrap(),
        generic_failure.expect("threshold failures produced a generic body")
    );
    assert!(
        !locked
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
    );
    assert_eq!(
        driver.verifies.load(Ordering::SeqCst),
        before + 2,
        "locked accounts must execute both configured verifier lanes"
    );

    // Unlock fires only on a true locked -> unlocked transition.
    // Password reset is the recovery path out of lockout.
    let forgot = dispatch(
        &world,
        post_json("/forgot-password", json!({"email": EMAIL})),
    )
    .await;
    assert_eq!(forgot.status, 200);
    let reset_link = world.mail.last_payload().unwrap()["reset_link"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = query_param(&reset_link, "token");
    let outcome = world
        .management
        .complete_with_outcome(&token, "fresh honest password")
        .await
        .unwrap();
    assert!(
        outcome.lockout_cleared.unwrap(),
        "reset reports the true unlock transition"
    );
    let status = world.lockout.status(EMAIL).await.unwrap();
    assert!(!status.is_locked);
    assert_eq!(status.failed_attempts, 0, "reset clears the counters");
    let unlocked_user = world.storage.find_by_id(&user_id).await.unwrap().unwrap();
    assert!(unlocked_user.locked_at.is_none(), "reset clears locked_at");

    // A second unlock is idempotent and reports no transition.
    assert!(!world.lockout.unlock_account(EMAIL).await.unwrap());

    // The account signs in again with the new credential.
    let login = dispatch(&world, login_request(EMAIL, "fresh honest password")).await;
    assert_eq!(login.status, 200);
}

fn craft_jwt(user_id: &str, issuer: &str, key: &str, auth_epoch: u64) -> String {
    const HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
    let claims = json!({
        "sub": user_id,
        "iss": issuer,
        "exp": (chrono::Utc::now() + Duration::hours(1)).timestamp(),
        "sid": "crafted-session",
        "auth_epoch": auth_epoch,
        "metadata": {"user_agent": null, "ip_address": null},
    });
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    let message = format!("{HEADER}.{payload}");
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(message.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{message}.{signature}")
}

#[tokio::test]
async fn reset_commits_epoch_sessions_and_credential_atomically() {
    let world = harness().await;
    let user_id = register(&world).await;

    // Two live sessions: one to survive-check later, one bound for logout
    // comparisons; plus one remember-me row.
    let mut login = login_request(EMAIL, PASSWORD);
    if let magnetar::plugin::WireBody::Json(body) = &mut login.body {
        body["remember"] = json!(true);
    }
    let first = dispatch(&world, login.clone()).await;
    assert_eq!(first.status, 200);
    assert!(first.remember_issued, "remember-me issued on request");
    let second = dispatch(&world, login).await;
    assert_eq!(second.status, 200);
    let bearer = second
        .grant
        .expect("session grant")
        .into_bearer()
        .expose_token_once();
    assert_eq!(
        world.sessions.list_for_user(&user_id).await.unwrap().len(),
        2
    );

    // An outstanding JWT bound to epoch 0 verifies before the reset.
    let epoch_store: Arc<magnetar::storage::SeaOrmStorage<StorageSchema>> = world.storage.clone();
    let jwt_provider = JwtSessionProvider::new(
        JwtConfig::new(
            "magnetar-tests",
            SecretString::from("test-signing-key"),
            Duration::hours(1),
        ),
        epoch_store,
    )
    .unwrap();
    let outstanding = craft_jwt(&user_id, "magnetar-tests", "test-signing-key", 0);
    assert_eq!(
        jwt_provider
            .verify_bearer(&outstanding)
            .await
            .unwrap()
            .user_id(),
        user_id
    );

    // J8: forgot -> reset with the mailed token.
    let forgot = dispatch(
        &world,
        post_json("/forgot-password", json!({"email": EMAIL})),
    )
    .await;
    assert_eq!(forgot.status, 200);
    let token = query_param(
        world.mail.last_payload().unwrap()["reset_link"]
            .as_str()
            .unwrap(),
        "token",
    );
    let mails_before = world.mail.count();
    let reset = dispatch(
        &world,
        post_json(
            "/reset-password",
            json!({"token": token, "password": "fresh honest password"}),
        ),
    )
    .await;
    assert_eq!(reset.status, 200);
    assert_eq!(reset.body, Some(json!({"status": "ok"})));

    // The committed mutation: epoch bumped, every opaque session revoked,
    // remember rows retired, credential rotated.
    let user = world.storage.find_by_id(&user_id).await.unwrap().unwrap();
    assert_eq!(user.auth_epoch, 1);
    assert!(
        world
            .sessions
            .list_for_user(&user_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        world
            .sessions
            .verify_bearer(bearer.expose_secret())
            .await
            .is_err(),
        "pre-reset opaque sessions are dead"
    );
    assert!(
        jwt_provider.verify_bearer(&outstanding).await.is_err(),
        "pre-reset JWTs fail epoch verification immediately"
    );
    let rotate = world
        .remember
        .rotate_at_epoch(
            &magnetar::sessions::RememberCredential::from_host(SecretString::from(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbb",
            )),
            chrono::Utc::now(),
            chrono::Duration::days(30),
        )
        .await;
    assert!(rotate.is_err(), "remember rows are gone after reset");

    // The changed-password notification went out post-commit.
    assert_eq!(world.mail.count(), mails_before + 1);
    assert!(world.mail.names().contains(&"password_changed".to_owned()));

    // Single use: replaying the consumed token answers 400.
    let replay = dispatch(
        &world,
        post_json(
            "/reset-password",
            json!({"token": token, "password": "another password"}),
        ),
    )
    .await;
    assert_eq!(replay.status, 400);

    // Old credential fails; the new one signs in.
    assert_eq!(
        dispatch(&world, login_request(EMAIL, PASSWORD))
            .await
            .status,
        401
    );
    assert_eq!(
        dispatch(&world, login_request(EMAIL, "fresh honest password"))
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn forgot_password_is_anti_enumeration() {
    let world = harness().await;
    register(&world).await;
    let baseline = world.mail.count();

    let present = dispatch(
        &world,
        post_json("/forgot-password", json!({"email": EMAIL})),
    )
    .await;
    let absent = dispatch(
        &world,
        post_json("/forgot-password", json!({"email": "nobody@example.test"})),
    )
    .await;
    assert_eq!(present.status, absent.status);
    assert_eq!(present.body, absent.body);
    assert_eq!(
        world.mail.count(),
        baseline + 1,
        "absent addresses mint no token and send no mail"
    );
}

#[tokio::test]
async fn logout_revokes_only_the_presented_session_and_all_remember_rows() {
    let world = harness().await;
    let user_id = register(&world).await;
    let mut login = login_request(EMAIL, PASSWORD);
    if let magnetar::plugin::WireBody::Json(body) = &mut login.body {
        body["remember"] = json!(true);
    }
    let kept = dispatch(&world, login.clone()).await;
    let leaving = dispatch(&world, login).await;
    let kept_grant = kept.grant.expect("first session");
    let leaving_grant = leaving.grant.expect("second session");
    let epoch_before = world
        .storage
        .find_by_id(&user_id)
        .await
        .unwrap()
        .unwrap()
        .auth_epoch;

    let logout = split(
        world
            .registry
            .handle_web_binding(
                post_json("/logout", json!({})),
                &leaving_grant.web_binding(),
            )
            .await
            .unwrap(),
    );
    assert_eq!(logout.status, 200);
    assert!(logout.cleared_session);

    let remaining = world.sessions.list_for_user(&user_id).await.unwrap();
    assert_eq!(remaining.len(), 1, "ordinary logout leaves other sessions");
    assert_eq!(remaining[0].session_id, kept_grant.session_id());
    let epoch_after = world
        .storage
        .find_by_id(&user_id)
        .await
        .unwrap()
        .unwrap()
        .auth_epoch;
    assert_eq!(
        epoch_before, epoch_after,
        "ordinary logout never bumps the epoch"
    );

    // Remember-me rows are retired wholesale, as the guard chains today.
    let rotate = world
        .remember
        .rotate_at_epoch(
            &magnetar::sessions::RememberCredential::from_host(SecretString::from(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbb",
            )),
            chrono::Utc::now(),
            chrono::Duration::days(30),
        )
        .await;
    assert!(rotate.is_err());

    // Logout without a bound session is a 401.
    let anonymous = dispatch(&world, post_json("/logout", json!({}))).await;
    assert_eq!(anonymous.status, 401);
}

#[tokio::test]
async fn change_set_and_census_guarded_remove() {
    let world = harness().await;
    let user_id = register(&world).await;

    // Wrong current password fails with the indistinguishable error.
    let wrong = world
        .provider
        .change_password(
            &user_id,
            SecretString::from("not the password"),
            SecretString::from("brand new password"),
        )
        .await
        .unwrap_err();
    assert!(matches!(&wrong,
        magnetar::Error::InvalidInput { field, .. } if field == "credentials"
    ));

    // Correct current password rotates the credential.
    world
        .provider
        .change_password(
            &user_id,
            SecretString::from(PASSWORD),
            SecretString::from("brand new password"),
        )
        .await
        .unwrap();
    assert_eq!(
        dispatch(&world, login_request(EMAIL, PASSWORD))
            .await
            .status,
        401
    );
    let login = dispatch(&world, login_request(EMAIL, "brand new password")).await;
    assert_eq!(login.status, 200);
    let grant = login.grant.expect("password sign-in issues a session");
    let token = grant.into_bearer().expose_token_once();
    let session = world
        .sessions
        .verify_bearer(token.expose_secret())
        .await
        .expect("official opaque provider verifies password session");
    let actor = CredentialActor::from_session(&session);

    // Administrative set requires no current password.
    world
        .provider
        .set_password(&actor, SecretString::from("administratively set"))
        .await
        .unwrap();
    assert_eq!(
        dispatch(&world, login_request(EMAIL, "administratively set"))
            .await
            .status,
        200
    );

    // Removing the last sign-in method is refused and leaves the credential.
    assert!(world.provider.has_password(&user_id).await.unwrap());
    assert!(!world.provider.remove_password(&actor).await.unwrap());
    assert!(world.provider.has_password(&user_id).await.unwrap());

    // With a second method on file the removal wins and the census updates.
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};
    storage_schema::methods::ActiveModel {
        id: Set(41),
        user_id: Set(user_id.parse().unwrap()),
        ..Default::default()
    }
    .insert(&world.db)
    .await
    .unwrap();
    assert!(world.provider.remove_password(&actor).await.unwrap());
    assert!(!world.provider.has_password(&user_id).await.unwrap());

    // The account is now passwordless: logins fail indistinguishably.
    assert_eq!(
        dispatch(&world, login_request(EMAIL, "administratively set"))
            .await
            .status,
        401
    );
}

#[tokio::test]
async fn weak_and_oversized_passwords_are_rejected_at_registration() {
    let world = harness().await;
    for bad in ["short", "        ", &"p".repeat(129)] {
        let reply = dispatch(&world, register_request(EMAIL, bad)).await;
        assert_eq!(reply.status, 400, "{bad:?} must be rejected");
    }
    assert!(
        world.storage.find_by_email(EMAIL).await.unwrap().is_none(),
        "no user row is created for rejected passwords"
    );
}
