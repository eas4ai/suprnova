//! Magnetar's second-factor flows on live PostgreSQL and MySQL/MariaDB.
//!
//! The SQLite suites prove the attempt reservation with a coordinated fake
//! attempt store. These run the shipped default schema, the SeaORM lockout
//! store and its row lock on real engines instead: every parallel request
//! holds a connection of its own, so the requests contend in the database
//! the way production traffic does. They cover the challenge, `confirm`,
//! `re_enroll` and `regenerate_recovery_codes` under a burst of wrong
//! proofs, a correct proof raced against itself, and the second factor's
//! counter surviving a password success.
//!
//! PostgreSQL and MySQL are manual, ignored qualification tests against the
//! live backends named by `MAGNETAR_POSTGRES_TEST_URL` and
//! `MAGNETAR_MYSQL_TEST_URL`. Every test creates users of its own, so
//! repeated runs against one database never collide.

#![cfg(all(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "two-factor",
    any(feature = "seaorm-postgres", feature = "seaorm-mysql")
))]

#[path = "fixtures/password_harness.rs"]
mod harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use magnetar::auth::{FactorGate, OpaqueFactorGate};
use magnetar::crypto::AeadEncryptor;
use magnetar::crypto::{CryptoPurpose, Encryptor};
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::default_schema::sql_two_factor::SqlTwoFactorStore;
use magnetar::default_schema::{DefaultAuthSchema, two_factor};
use magnetar::password::{
    LockoutConfig, LockoutService, PasswordVerifier, StandardPasswordHashDriver,
};
use magnetar::plugin::{PluginContext, PluginRegistry, WireRequest};
use magnetar::plugins::password::{PasswordAuthService, PasswordPlugin, PasswordPluginConfig};
use magnetar::sessions::{OpaqueConfig, OpaqueSessionProvider, SessionQueries};
use magnetar::storage::{CredentialActor, SeaOrmStorage};
use magnetar::two_factor::{
    EnrollmentResponse, TwoFactorConfig, TwoFactorProofClaim, TwoFactorRow, TwoFactorService,
    TwoFactorStore,
};
use parking_lot::Mutex;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectOptions, Database, DatabaseConnection};
use secrecy::{ExposeSecret, SecretString};

use harness::{
    CountingLimiter, IdentityEncryptor, NoTransport, RecordingMail, Reply, TestLinks,
    fast_hash_config, login_request, register_request, split,
};

const PASSWORD: &str = "orange tabby cat";
/// The second factor's attempt threshold in the wrong-proof tests.
const THRESHOLD: u32 = 3;
/// Parallel requests per flow, well past the threshold.
const PARALLEL: usize = 8;

type Gate = OpaqueFactorGate<SeaOrmStorage<DefaultAuthSchema>, TwoFactorService, SqlSessionStore>;

/// The default schema's stores, the real lockout service, the two-factor
/// service as the factor gate's verifier, and the password plugin that
/// signs users in and opens challenges.
struct LiveWorld {
    db: DatabaseConnection,
    storage: Arc<SeaOrmStorage<DefaultAuthSchema>>,
    sessions: Arc<OpaqueSessionProvider<SqlSessionStore>>,
    lockout: Arc<LockoutService>,
    two_factor: Arc<TwoFactorService>,
    gate: Arc<Gate>,
    registry: PluginRegistry<DefaultAuthSchema>,
}

async fn live_world(url: &str, max_failed_attempts: u32) -> LiveWorld {
    let mut options = ConnectOptions::new(url.to_owned());
    // A connection for every parallel request, so they contend for the
    // lockout row lock rather than for the pool.
    options
        .max_connections(u32::try_from(PARALLEL).unwrap() + 4)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(30))
        .sqlx_logging(false);
    let db = Database::connect(options)
        .await
        .expect("connect live backend");
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create default auth tables");
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let sessions = Arc::new(OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(db.clone())),
        OpaqueConfig::default(),
    ));
    let lockout = Arc::new(LockoutService::new(
        storage.clone(),
        storage.clone(),
        LockoutConfig {
            max_failed_attempts,
            ..LockoutConfig::default()
        },
    ));
    let crypto = Arc::new(AeadEncryptor::new([21; 32]));
    let two_factor = Arc::new(TwoFactorService::new(
        Arc::new(SqlTwoFactorStore(db.clone())),
        storage.clone(),
        lockout.clone(),
        crypto.clone(),
        TwoFactorConfig::default(),
    ));
    let gate = Arc::new(OpaqueFactorGate::new(
        storage.clone(),
        two_factor.clone(),
        crypto,
        sessions.clone(),
    ));
    let verifier = Arc::new(
        PasswordVerifier::new(Arc::new(StandardPasswordHashDriver), fast_hash_config())
            .expect("dummy warmup succeeds"),
    );
    let provider = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        verifier,
    ));
    let context = PluginContext::new(
        storage.clone(),
        sessions.clone(),
        gate.clone(),
        Arc::new(IdentityEncryptor),
        Arc::new(CountingLimiter::default()),
        Arc::new(RecordingMail::default()),
        Arc::new(NoTransport),
        Arc::new(TestLinks),
    );
    let registry = PluginRegistry::new(context)
        .register(PasswordPlugin::new(
            provider,
            lockout.clone(),
            None,
            None,
            PasswordPluginConfig::default(),
        ))
        .build()
        .await
        .expect("plugin composition is valid");
    LiveWorld {
        db,
        storage,
        sessions,
        lockout,
        two_factor,
        gate,
        registry,
    }
}

async fn send(world: &LiveWorld, request: WireRequest) -> Reply {
    split(
        world
            .registry
            .handle(request)
            .await
            .expect("route dispatch succeeds"),
    )
}

fn unique_email(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    format!(
        "tf-live-{label}-{nanos}-{:016x}@example.test",
        rand::random::<u64>()
    )
}

fn totp_code_at(otpauth_url: &SecretString, unix_seconds: i64) -> String {
    let totp = totp_rs::TOTP::from_url_unchecked(otpauth_url.expose_secret())
        .expect("enrollment otpauth URL parses");
    totp.generate(u64::try_from(unix_seconds).expect("clock is after the epoch"))
}

fn totp_code_now(otpauth_url: &SecretString) -> String {
    totp_code_at(otpauth_url, Utc::now().timestamp())
}

/// A registered user, signed in with the password before any enrollment.
struct LiveUser {
    user_id: String,
    email: String,
    actor: CredentialActor,
}

async fn signed_in_user(world: &LiveWorld, label: &str) -> LiveUser {
    let email = unique_email(label);
    let registered = send(world, register_request(&email, PASSWORD)).await;
    assert_eq!(registered.status, 200, "registration succeeds");
    let login = send(world, login_request(&email, PASSWORD)).await;
    assert_eq!(login.status, 200, "the password is right");
    let grant = login
        .grant
        .expect("a user without a second factor gets a session");
    let user_id = grant.user_id().to_owned();
    let token = grant.into_bearer().expose_token_once();
    let session = world
        .sessions
        .verify_bearer(token.expose_secret())
        .await
        .expect("the issued session verifies");
    LiveUser {
        user_id,
        email,
        actor: CredentialActor::from_session(&session),
    }
}

async fn confirmed_enrollment(world: &LiveWorld, user: &LiveUser) -> EnrollmentResponse {
    let enrollment = world.two_factor.enroll(&user.actor).await.unwrap();
    world
        .two_factor
        .confirm(&user.actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();
    enrollment
}

/// Sign in with the password and return the challenge it opens.
async fn challenge(world: &LiveWorld, user: &LiveUser) -> String {
    let login = send(world, login_request(&user.email, PASSWORD)).await;
    assert_eq!(login.status, 200, "the password is right");
    login.body.expect("challenge body")["challenge_selector"]
        .as_str()
        .expect("a confirmed second factor opens a challenge")
        .to_owned()
}

/// A second-factor flow that checks a proof.
#[derive(Clone, Copy, Debug)]
enum Flow {
    Challenge,
    Confirm,
    ReEnroll,
    Regenerate,
}

/// What one request needs to submit a proof through `flow`.
#[derive(Clone)]
struct Target {
    two_factor: Arc<TwoFactorService>,
    gate: Arc<Gate>,
    actor: CredentialActor,
    selector: Option<String>,
}

async fn submit(target: Target, flow: Flow, proof: String) -> magnetar::Result<()> {
    match flow {
        Flow::Challenge => target
            .gate
            .complete_challenge(target.selector.as_deref().expect("challenge"), &proof)
            .await
            .map(|_| ()),
        Flow::Confirm => target.two_factor.confirm(&target.actor, &proof).await,
        Flow::ReEnroll => target
            .two_factor
            .re_enroll(&target.actor, &proof)
            .await
            .map(|_| ()),
        Flow::Regenerate => target
            .two_factor
            .regenerate_recovery_codes(&target.actor, &proof)
            .await
            .map(|_| ()),
    }
}

/// Submit every `(target, proof)` pair at once and collect the outcomes.
async fn race(flow: Flow, requests: Vec<(Target, String)>) -> Vec<magnetar::Result<()>> {
    let tasks = requests
        .into_iter()
        .map(|(target, proof)| tokio::spawn(submit(target, flow, proof)))
        .collect::<Vec<_>>();
    let mut outcomes = Vec::with_capacity(tasks.len());
    for task in tasks {
        outcomes.push(task.await.expect("proof task joins"));
    }
    outcomes
}

fn is_wrong_proof(outcome: &magnetar::Result<()>) -> bool {
    matches!(outcome, Err(magnetar::Error::InvalidInput { field, .. }) if field == "code" || field == "proof")
}

fn is_lockout(outcome: &magnetar::Result<()>) -> bool {
    matches!(outcome, Err(magnetar::Error::Conflict { resource, .. }) if resource == "account lockout")
}

/// A burst of wrong proofs on each flow: the threshold of them is evaluated
/// and counted, and every other one is refused before it is read.
async fn parallel_wrong_proofs_evaluate_at_most_the_threshold(url: &str) {
    let world = live_world(url, THRESHOLD).await;
    let mut problems = Vec::new();
    for flow in [
        Flow::Challenge,
        Flow::Confirm,
        Flow::ReEnroll,
        Flow::Regenerate,
    ] {
        let user = signed_in_user(&world, "wrong").await;
        let selector = match flow {
            Flow::Confirm => {
                world.two_factor.enroll(&user.actor).await.unwrap();
                None
            }
            Flow::ReEnroll | Flow::Regenerate => {
                confirmed_enrollment(&world, &user).await;
                None
            }
            Flow::Challenge => {
                confirmed_enrollment(&world, &user).await;
                Some(challenge(&world, &user).await)
            }
        };
        let target = Target {
            two_factor: world.two_factor.clone(),
            gate: world.gate.clone(),
            actor: user.actor.clone(),
            selector,
        };
        let requests = (0..PARALLEL)
            .map(|index| (target.clone(), format!("wrong-proof-{index}")))
            .collect();

        let outcomes = race(flow, requests).await;

        let evaluated = outcomes
            .iter()
            .filter(|outcome| is_wrong_proof(outcome))
            .count();
        let refused = outcomes
            .iter()
            .filter(|outcome| is_lockout(outcome))
            .count();
        let counted = world
            .lockout
            .status(&magnetar::two_factor::lockout_identity(&user.user_id))
            .await
            .expect("lockout status reads")
            .failed_attempts;
        if evaluated != THRESHOLD as usize
            || refused != PARALLEL - THRESHOLD as usize
            || counted != THRESHOLD
        {
            problems.push(format!(
                "{flow:?}: {evaluated} evaluated, {refused} refused, {counted} counted; \
                 expected {THRESHOLD}, {}, {THRESHOLD}: {outcomes:?}",
                PARALLEL - THRESHOLD as usize
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The same correct proof raced against itself on each flow: exactly one
/// request is accepted.
async fn a_correct_proof_under_contention_is_accepted_once(url: &str) {
    // A threshold no burst reaches, so every request gets to the claim.
    let world = live_world(url, 100).await;
    let mut problems = Vec::new();
    for flow in [
        Flow::Challenge,
        Flow::Confirm,
        Flow::ReEnroll,
        Flow::Regenerate,
    ] {
        let user = signed_in_user(&world, "correct").await;
        let enrollment = if matches!(flow, Flow::Confirm) {
            world.two_factor.enroll(&user.actor).await.unwrap()
        } else {
            confirmed_enrollment(&world, &user).await
        };
        let target = Target {
            two_factor: world.two_factor.clone(),
            gate: world.gate.clone(),
            actor: user.actor.clone(),
            selector: None,
        };
        // One code for every request: codes from two timesteps would each
        // be valid once. A confirmed enrollment used the current code to
        // confirm, so the proofs after it use the next one.
        let proof = match flow {
            Flow::Regenerate => enrollment.recovery_codes[0].clone(),
            Flow::Confirm => totp_code_now(&enrollment.otpauth_url),
            Flow::Challenge | Flow::ReEnroll => totp_code_at(
                &enrollment.otpauth_url,
                Utc::now().timestamp() + magnetar::two_factor::totp::STEP_SECONDS,
            ),
        };
        let mut requests = Vec::with_capacity(PARALLEL);
        for _ in 0..PARALLEL {
            let mut target = target.clone();
            if matches!(flow, Flow::Challenge) {
                // A challenge of its own for every request, so the code is
                // the only thing they share.
                target.selector = Some(challenge(&world, &user).await);
            }
            requests.push((target, proof.clone()));
        }

        let outcomes = race(flow, requests).await;

        let accepted = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        if accepted != 1 {
            problems.push(format!(
                "{flow:?}: {accepted} requests accepted one proof: {outcomes:?}"
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// Wrong codes split across two password sign-ins still lock the second
/// factor at its threshold, and never touch the password's own counter.
async fn a_password_success_does_not_clear_second_factor_failures(url: &str) {
    let world = live_world(url, THRESHOLD).await;
    let user = signed_in_user(&world, "password").await;
    confirmed_enrollment(&world, &user).await;
    let second_factor = magnetar::two_factor::lockout_identity(&user.user_id);

    let mut outcomes = Vec::new();
    for _sign_in in 0..2 {
        let selector = challenge(&world, &user).await;
        for _ in 0..2 {
            outcomes.push(
                world
                    .gate
                    .complete_challenge(&selector, "000000")
                    .await
                    .map(|_| ()),
            );
        }
    }

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| is_wrong_proof(outcome))
            .count(),
        THRESHOLD as usize,
        "the second-factor counter locks at its threshold: {outcomes:?}"
    );
    assert!(
        is_lockout(outcomes.last().expect("four attempts")),
        "the next wrong code is refused: {outcomes:?}"
    );
    let status = world.lockout.status(&second_factor).await.unwrap();
    assert_eq!(status.failed_attempts, THRESHOLD);
    assert!(status.is_locked);
    let password = world
        .lockout
        .status(&magnetar::password::normalize_email(&user.email))
        .await
        .unwrap();
    assert_eq!(
        password.failed_attempts, 0,
        "wrong codes leave the password counter alone"
    );
    let login = send(&world, login_request(&user.email, PASSWORD)).await;
    assert_eq!(login.status, 200, "password sign-in is not locked");
}

/// The shipped two-factor store, except that another request changes the
/// enrollment row right after this request reads it: the read returns the
/// old row, and the row in the database is already the replacement.
struct ReplacedAfterRead {
    inner: SqlTwoFactorStore,
    db: DatabaseConnection,
    replacement: Mutex<Option<two_factor::ActiveModel>>,
}

#[async_trait::async_trait]
impl TwoFactorStore for ReplacedAfterRead {
    async fn find_enrollment(&self, user_id: &str) -> magnetar::Result<Option<TwoFactorRow>> {
        let row = self.inner.find_enrollment(user_id).await?;
        let replacement = self.replacement.lock().take();
        if let Some(replacement) = replacement {
            replacement
                .update(&self.db)
                .await
                .expect("the concurrent write commits");
        }
        Ok(row)
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
        expected_secret: &[u8],
        matched_step: i64,
        at: chrono::DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        self.inner
            .set_confirmed(actor, expected_secret, matched_step, at)
            .await
    }

    async fn claim_timestep(&self, user_id: &str, matched_step: i64) -> magnetar::Result<bool> {
        self.inner.claim_timestep(user_id, matched_step).await
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

/// A confirmation whose enrollment another request replaces between the
/// code check and the stamp: a restarted enrollment of a pending secret,
/// and a re-enrollment of a confirmed one. Neither replacement was proven,
/// so neither is confirmed.
async fn a_confirmation_racing_a_replacement_confirms_neither_secret(url: &str) {
    let world = live_world(url, THRESHOLD).await;
    let mut problems = Vec::new();
    for re_enrollment in [false, true] {
        let user = signed_in_user(&world, "race").await;
        let enrollment = if re_enrollment {
            confirmed_enrollment(&world, &user).await
        } else {
            world.two_factor.enroll(&user.actor).await.unwrap()
        };
        let replacement_secret = AeadEncryptor::new([21; 32])
            .encrypt(CryptoPurpose::TwoFactorSecret, b"JBSWY3DPEHPK3PXP")
            .unwrap();
        let store = ReplacedAfterRead {
            inner: SqlTwoFactorStore(world.db.clone()),
            db: world.db.clone(),
            // Written by the same session, so only the secret tells the two
            // enrollments apart.
            replacement: Mutex::new(Some(two_factor::ActiveModel {
                user_id: Set(user.user_id.clone()),
                secret: Set(replacement_secret),
                enrollment_auth_epoch: Set(i64::try_from(user.actor.issuance_epoch()).unwrap()),
                enrollment_session_id: Set(user.actor.opaque_session_id().map(str::to_owned)),
                enrollment_expires_at: Set(user.actor.expires_at()),
                confirmed_at: Set(None),
                rotation_pending: Set(re_enrollment),
                last_used_timestep: Set(None),
                ..Default::default()
            })),
        };
        let service = TwoFactorService::new(
            Arc::new(store),
            world.storage.clone(),
            world.lockout.clone(),
            Arc::new(AeadEncryptor::new([21; 32])),
            TwoFactorConfig::default(),
        );

        let outcome = service
            .confirm(&user.actor, &totp_code_now(&enrollment.otpauth_url))
            .await;

        let row = SqlTwoFactorStore(world.db.clone())
            .find_enrollment(&user.user_id)
            .await
            .unwrap()
            .expect("the replacement row exists");
        if outcome.is_ok() || row.confirmed_at.is_some() {
            problems.push(format!(
                "re-enrollment {re_enrollment}: {outcome:?}, the unproven replacement confirmed at {:?}",
                row.confirmed_at
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_parallel_wrong_second_factor_proofs_evaluate_at_most_the_threshold() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    parallel_wrong_proofs_evaluate_at_most_the_threshold(&url).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_a_correct_second_factor_proof_under_contention_is_accepted_once() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    a_correct_proof_under_contention_is_accepted_once(&url).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_a_password_success_does_not_clear_second_factor_failures() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    a_password_success_does_not_clear_second_factor_failures(&url).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_a_confirmation_racing_a_replacement_confirms_neither_secret() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    a_confirmation_racing_a_replacement_confirms_neither_secret(&url).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_parallel_wrong_second_factor_proofs_evaluate_at_most_the_threshold() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    parallel_wrong_proofs_evaluate_at_most_the_threshold(&url).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_a_correct_second_factor_proof_under_contention_is_accepted_once() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    a_correct_proof_under_contention_is_accepted_once(&url).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_a_password_success_does_not_clear_second_factor_failures() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    a_password_success_does_not_clear_second_factor_failures(&url).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_a_confirmation_racing_a_replacement_confirms_neither_secret() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    a_confirmation_racing_a_replacement_confirms_neither_secret(&url).await;
}
