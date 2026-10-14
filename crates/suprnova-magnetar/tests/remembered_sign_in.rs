//! RED contracts for remembered primary authentication through the factor gate.

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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use magnetar::auth::{
    AuthenticationContext, FactorGate, SignInDecision, SignInMethod, VerifiedPrincipal,
};
use magnetar::sessions::{
    JwtEpochStore, RememberAnomaly, RememberAnomalyHook, RememberAnomalyKind, RememberCredential,
    RememberPostRotationDisposition, RememberRow, RememberService, RememberSignInAttempt,
    RememberSignInService, RememberStore, RememberTokenService, SessionMetadata, SessionQueries,
};
use magnetar::storage::{CredentialActor, NewUser, UserRecord, UserStore};
use magnetar::{Error, Result};
use parking_lot::Mutex;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use secrecy::{ExposeSecret, SecretString};

use factor::{credential_actor, factor_world, totp_code_now};

const EMAIL: &str = "remembered-primary@example.test";

struct RecordingFactorGate {
    inner: Arc<dyn FactorGate>,
    methods: Mutex<Vec<SignInMethod>>,
}

impl RecordingFactorGate {
    fn new(inner: Arc<dyn FactorGate>) -> Self {
        Self {
            inner,
            methods: Mutex::new(Vec::new()),
        }
    }

    fn methods(&self) -> Vec<SignInMethod> {
        self.methods.lock().clone()
    }
}

#[async_trait]
impl FactorGate for RecordingFactorGate {
    async fn complete_sign_in(
        &self,
        principal: VerifiedPrincipal,
        context: AuthenticationContext,
    ) -> Result<SignInDecision> {
        self.methods.lock().push(principal.method().clone());
        self.inner.complete_sign_in(principal, context).await
    }

    async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> Result<magnetar::sessions::SessionGrant> {
        self.inner.complete_challenge(selector, code).await
    }
}

#[derive(Default)]
struct RecordingRememberAnomalyHook {
    events: Mutex<Vec<RememberAnomaly>>,
}

impl RecordingRememberAnomalyHook {
    fn events(&self) -> Vec<RememberAnomaly> {
        self.events.lock().clone()
    }

    fn take(&self) -> Vec<RememberAnomaly> {
        std::mem::take(&mut *self.events.lock())
    }
}

#[async_trait]
impl RememberAnomalyHook for RecordingRememberAnomalyHook {
    async fn on_anomaly(&self, anomaly: RememberAnomaly) {
        self.events.lock().push(anomaly);
    }
}

type TestRememberStore = storage_schema::sql_stores::SqlRememberStore;

fn service_with_anomaly_hook(
    world: &factor::FactorWorld,
) -> (
    Arc<TestRememberStore>,
    Arc<RecordingRememberAnomalyHook>,
    TestRememberSignInService,
) {
    let store = Arc::new(storage_schema::sql_stores::SqlRememberStore(
        world.db.clone(),
    ));
    let hook = Arc::new(RecordingRememberAnomalyHook::default());
    let remember = Arc::new(
        RememberService::new(store.clone(), Duration::days(30))
            .expect("compose remember service")
            .with_anomaly_hook(hook.clone()),
    );
    let token_service: Arc<dyn RememberTokenService> = remember;
    let (_, service) = service_with_token_service(world, token_service);
    (store, hook, service)
}

async fn user(world: &factor::FactorWorld, email: &str) -> magnetar::storage::UserRecord {
    world
        .storage
        .create_user(NewUser {
            email: email.to_owned(),
            password_hash: None,
        })
        .await
        .expect("create remembered-sign-in user")
}

type TestRememberSignInService =
    RememberSignInService<magnetar::storage::SeaOrmStorage<storage_schema::StorageSchema>>;

fn service_with_token_service(
    world: &factor::FactorWorld,
    remember: Arc<dyn RememberTokenService>,
) -> (Arc<RecordingFactorGate>, TestRememberSignInService) {
    let recording = Arc::new(RecordingFactorGate::new(world.gate.clone()));
    let service = RememberSignInService::new(remember, world.storage.clone(), recording.clone());
    (recording, service)
}

fn service(world: &factor::FactorWorld) -> (Arc<RecordingFactorGate>, TestRememberSignInService) {
    let remember: Arc<dyn RememberTokenService> = world.remember.clone();
    service_with_token_service(world, remember)
}

struct FailOnceUserStore<U> {
    inner: Arc<U>,
    fail_next_id_lookup: AtomicBool,
}

impl<U> FailOnceUserStore<U> {
    fn new(inner: Arc<U>) -> Self {
        Self {
            inner,
            fail_next_id_lookup: AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl<U: UserStore> UserStore for FailOnceUserStore<U> {
    async fn find_by_email(&self, email: &str) -> Result<Option<UserRecord>> {
        self.inner.find_by_email(email).await
    }

    async fn find_by_id(&self, user_id: &str) -> Result<Option<UserRecord>> {
        if self.fail_next_id_lookup.swap(false, Ordering::SeqCst) {
            return Err(Error::DependencyUnavailable {
                dependency: "user store".to_owned(),
                message: "injected one-shot lookup failure".to_owned(),
            });
        }
        self.inner.find_by_id(user_id).await
    }

    async fn create_user(&self, input: NewUser) -> Result<UserRecord> {
        self.inner.create_user(input).await
    }

    async fn set_password_hash(&self, actor: &CredentialActor, password_hash: &str) -> Result<()> {
        self.inner.set_password_hash(actor, password_hash).await
    }

    async fn mark_email_verified(&self, user_id: &str, at: chrono::DateTime<Utc>) -> Result<()> {
        self.inner.mark_email_verified(user_id, at).await
    }

    async fn lock_if_unlocked_by_email(
        &self,
        email: &str,
        locked_at: chrono::DateTime<Utc>,
        window_start: chrono::DateTime<Utc>,
    ) -> Result<bool> {
        self.inner
            .lock_if_unlocked_by_email(email, locked_at, window_start)
            .await
    }

    async fn set_locked_at_by_email(
        &self,
        email: &str,
        locked_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        self.inner.set_locked_at_by_email(email, locked_at).await
    }
}

struct FailOnceFactorGate {
    inner: Arc<dyn FactorGate>,
    fail_next_sign_in: AtomicBool,
}

struct ExternalRememberTokenService {
    inner: Arc<RememberService<TestRememberStore>>,
    fail_after_rotation: AtomicBool,
    fail_revoke_all: AtomicBool,
    revoke_all_attempts: AtomicUsize,
}

impl ExternalRememberTokenService {
    fn new(inner: Arc<RememberService<TestRememberStore>>) -> Self {
        Self {
            inner,
            fail_after_rotation: AtomicBool::new(false),
            fail_revoke_all: AtomicBool::new(false),
            revoke_all_attempts: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl RememberTokenService for ExternalRememberTokenService {
    fn default_lifetime(&self) -> Duration {
        Duration::days(30)
    }

    async fn issue_at_epoch(
        &self,
        user_id: &str,
        auth_epoch: u64,
        now: chrono::DateTime<Utc>,
        lifetime: Duration,
    ) -> Result<RememberCredential> {
        self.inner
            .issue_at_epoch(user_id, auth_epoch, now, lifetime)
            .await
    }

    async fn rotate_at_epoch(
        &self,
        credential: &RememberCredential,
        now: chrono::DateTime<Utc>,
        replacement_lifetime: Duration,
    ) -> Result<(String, u64, RememberCredential)> {
        let rotation = self
            .inner
            .rotate_at_epoch(credential, now, replacement_lifetime)
            .await?;
        if self.fail_after_rotation.swap(false, Ordering::SeqCst) {
            return Err(Error::Internal {
                message: "external rotation outcome unavailable".to_owned(),
            });
        }
        Ok(rotation)
    }

    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        self.revoke_all_attempts.fetch_add(1, Ordering::SeqCst);
        if self.fail_revoke_all.load(Ordering::SeqCst) {
            return Err(Error::DependencyUnavailable {
                dependency: "external remember service".to_owned(),
                message: "injected broad cleanup failure".to_owned(),
            });
        }
        self.inner.revoke_all_for_user(user_id).await
    }
}

#[async_trait]
impl FactorGate for FailOnceFactorGate {
    async fn complete_sign_in(
        &self,
        principal: VerifiedPrincipal,
        context: AuthenticationContext,
    ) -> Result<SignInDecision> {
        if self.fail_next_sign_in.swap(false, Ordering::SeqCst) {
            return Err(Error::DependencyUnavailable {
                dependency: "factor gate".to_owned(),
                message: "injected one-shot gate failure".to_owned(),
            });
        }
        self.inner.complete_sign_in(principal, context).await
    }

    async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> Result<magnetar::sessions::SessionGrant> {
        self.inner.complete_challenge(selector, code).await
    }
}

#[tokio::test]
async fn attempt_returns_successor_when_user_lookup_fails_after_rotation() {
    let world = factor_world().await;
    let user = user(&world, "remembered-user-retry@example.test").await;
    let users = Arc::new(FailOnceUserStore::new(world.storage.clone()));
    let remember: Arc<dyn RememberTokenService> = world.remember.clone();
    let service = RememberSignInService::new(remember, users, world.gate.clone());
    let now = Utc::now();
    let original = world
        .remember
        .issue_at_epoch(&user.user_id, user.auth_epoch, now, Duration::days(30))
        .await
        .expect("issue original")
        .expose_once();
    let sessions_before = world
        .sessions
        .list_for_user(&user.user_id)
        .await
        .expect("list initial sessions")
        .len();

    let attempt = service
        .attempt_sign_in_with_lifetime(
            RememberCredential::from_host(original.clone()),
            SessionMetadata::default(),
            now,
            Duration::days(30),
        )
        .await
        .expect("rotation itself commits");
    let replacement = match attempt {
        RememberSignInAttempt::RotationCommitted {
            failure,
            replacement,
            ..
        } => {
            assert_eq!(
                failure.disposition,
                RememberPostRotationDisposition::Retryable
            );
            assert!(matches!(failure.error, Error::DependencyUnavailable { .. }));
            replacement.expose_once()
        }
        other => panic!("expected recoverable committed rotation, got {other:?}"),
    };

    assert!(
        service
            .attempt_sign_in_with_lifetime(
                RememberCredential::from_host(original),
                SessionMetadata::default(),
                now,
                Duration::days(30)
            )
            .await
            .is_err(),
        "the consumed original must not replay"
    );
    let retry = service
        .attempt_sign_in_with_lifetime(
            RememberCredential::from_host(replacement),
            SessionMetadata::default(),
            now,
            Duration::days(30),
        )
        .await
        .expect("successor retry reaches the service");
    assert!(matches!(retry, RememberSignInAttempt::Authenticated(_)));
    assert_eq!(
        world
            .sessions
            .list_for_user(&user.user_id)
            .await
            .unwrap()
            .len(),
        sessions_before + 1
    );
}

#[tokio::test]
async fn attempt_returns_successor_when_factor_gate_fails_after_rotation() {
    let world = factor_world().await;
    let user = user(&world, "remembered-factor-retry@example.test").await;
    let gate: Arc<dyn FactorGate> = Arc::new(FailOnceFactorGate {
        inner: world.gate.clone(),
        fail_next_sign_in: AtomicBool::new(true),
    });
    let remember: Arc<dyn RememberTokenService> = world.remember.clone();
    let service = RememberSignInService::new(remember, world.storage.clone(), gate);
    let now = Utc::now();
    let original = service
        .issue(&user.user_id, now)
        .await
        .expect("issue original")
        .expose_once();
    let sessions_before = world
        .sessions
        .list_for_user(&user.user_id)
        .await
        .unwrap()
        .len();

    let attempt = service
        .attempt_sign_in_with_lifetime(
            RememberCredential::from_host(original.clone()),
            SessionMetadata::default(),
            now,
            Duration::days(30),
        )
        .await
        .expect("rotation itself commits");
    let replacement = match attempt {
        RememberSignInAttempt::RotationCommitted {
            failure,
            replacement,
            ..
        } => {
            assert_eq!(
                failure.disposition,
                RememberPostRotationDisposition::Retryable
            );
            replacement.expose_once()
        }
        other => panic!("expected recoverable committed rotation, got {other:?}"),
    };

    assert!(
        service
            .sign_in(
                RememberCredential::from_host(original),
                SessionMetadata::default(),
                now
            )
            .await
            .is_err()
    );
    let retry = service
        .attempt_sign_in_with_lifetime(
            RememberCredential::from_host(replacement),
            SessionMetadata::default(),
            now,
            Duration::days(30),
        )
        .await
        .expect("successor retry reaches the service");
    assert!(matches!(retry, RememberSignInAttempt::Authenticated(_)));
    assert_eq!(
        world
            .sessions
            .list_for_user(&user.user_id)
            .await
            .unwrap()
            .len(),
        sessions_before + 1
    );
}

#[tokio::test]
async fn compatibility_sign_in_retires_successor_before_returning_post_rotation_error() {
    let world = factor_world().await;
    let user = user(&world, "remembered-compat-cleanup@example.test").await;
    let gate: Arc<dyn FactorGate> = Arc::new(FailOnceFactorGate {
        inner: world.gate.clone(),
        fail_next_sign_in: AtomicBool::new(true),
    });
    let remember: Arc<dyn RememberTokenService> = world.remember.clone();
    let service = RememberSignInService::new(remember, world.storage.clone(), gate);
    let now = Utc::now();
    let original = service
        .issue(&user.user_id, now)
        .await
        .expect("issue original");

    let error = service
        .sign_in(original, SessionMetadata::default(), now)
        .await
        .expect_err("compatibility adapter surfaces the gate failure");
    assert!(matches!(error, Error::DependencyUnavailable { .. }));
    assert_eq!(
        service
            .revoke_all_for_user(&user.user_id)
            .await
            .expect("inspect remaining remember rows"),
        0,
        "the compatibility adapter must not strand its unreturnable successor",
    );
}

#[tokio::test]
async fn external_token_service_default_cleanup_retires_successor_without_masking_error() {
    let world = factor_world().await;
    let user = user(&world, "remembered-external-cleanup@example.test").await;
    let store = Arc::new(storage_schema::sql_stores::SqlRememberStore(
        world.db.clone(),
    ));
    let inner = Arc::new(
        RememberService::new(store, Duration::days(30)).expect("compose concrete token service"),
    );
    let external = Arc::new(ExternalRememberTokenService::new(inner));
    let token_service: Arc<dyn RememberTokenService> = external.clone();
    let gate: Arc<dyn FactorGate> = Arc::new(FailOnceFactorGate {
        inner: world.gate.clone(),
        fail_next_sign_in: AtomicBool::new(true),
    });
    let service = RememberSignInService::new(token_service, world.storage.clone(), gate);
    let now = Utc::now();
    let original = service
        .issue(&user.user_id, now)
        .await
        .expect("issue original");

    let error = service
        .sign_in(original, SessionMetadata::default(), now)
        .await
        .expect_err("compatibility call surfaces the downstream error");
    assert!(matches!(
        error,
        Error::DependencyUnavailable { dependency, .. } if dependency == "factor gate"
    ));
    assert_eq!(external.revoke_all_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.revoke_all_for_user(&user.user_id).await.unwrap(),
        0,
        "the broad compatibility fallback must retire the successor",
    );
}

#[tokio::test]
async fn external_rotation_error_is_reported_as_unknown_commit_state() {
    let world = factor_world().await;
    let user = user(&world, "remembered-external-unknown@example.test").await;
    let store = Arc::new(storage_schema::sql_stores::SqlRememberStore(
        world.db.clone(),
    ));
    let inner = Arc::new(
        RememberService::new(store, Duration::days(30)).expect("compose concrete token service"),
    );
    let external = Arc::new(ExternalRememberTokenService::new(inner));
    external.fail_after_rotation.store(true, Ordering::SeqCst);
    let token_service: Arc<dyn RememberTokenService> = external.clone();
    let service =
        RememberSignInService::new(token_service, world.storage.clone(), world.gate.clone());
    let now = Utc::now();
    let original = service
        .issue(&user.user_id, now)
        .await
        .expect("issue original");

    let attempt = service
        .attempt_sign_in_with_lifetime(
            original,
            SessionMetadata::default(),
            now,
            Duration::days(30),
        )
        .await
        .expect("unknown commit state is an explicit attempt outcome");
    assert!(matches!(
        attempt,
        RememberSignInAttempt::RotationOutcomeUnknown {
            error: Error::Internal { message }
        } if message == "external rotation outcome unavailable"
    ));
    external
        .inner
        .revoke_all_for_user(&user.user_id)
        .await
        .expect("clean up committed fixture successor");
}

#[tokio::test]
async fn external_compatibility_cleanup_failure_does_not_mask_downstream_error() {
    let world = factor_world().await;
    let user = user(&world, "remembered-cleanup-error@example.test").await;
    let store = Arc::new(storage_schema::sql_stores::SqlRememberStore(
        world.db.clone(),
    ));
    let inner = Arc::new(
        RememberService::new(store, Duration::days(30)).expect("compose concrete token service"),
    );
    let external = Arc::new(ExternalRememberTokenService::new(inner));
    external.fail_revoke_all.store(true, Ordering::SeqCst);
    let token_service: Arc<dyn RememberTokenService> = external.clone();
    let gate: Arc<dyn FactorGate> = Arc::new(FailOnceFactorGate {
        inner: world.gate.clone(),
        fail_next_sign_in: AtomicBool::new(true),
    });
    let service = RememberSignInService::new(token_service, world.storage.clone(), gate);
    let now = Utc::now();
    let original = service
        .issue(&user.user_id, now)
        .await
        .expect("issue original");

    let error = service
        .sign_in(original, SessionMetadata::default(), now)
        .await
        .expect_err("compatibility call surfaces the downstream error");
    assert!(matches!(
        error,
        Error::DependencyUnavailable { dependency, .. } if dependency == "factor gate"
    ));
    assert_eq!(external.revoke_all_attempts.load(Ordering::SeqCst), 1);
    external
        .inner
        .revoke_all_for_user(&user.user_id)
        .await
        .expect("clean up successor after injected cleanup failure");
}

#[tokio::test]
async fn trusted_remember_proof_enters_factor_gate_and_mints_real_opaque_session() {
    let world = factor_world().await;
    let user = user(&world, EMAIL).await;
    let actor = credential_actor(&world, &user.user_id).await;
    let enrollment = world
        .two_factor
        .enroll(&actor)
        .await
        .expect("begin real two-factor enrollment");
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .expect("confirm real two-factor enrollment");

    let (gate, service) = service(&world);
    let now = Utc::now();
    let credential = service
        .issue(&user.user_id, now)
        .await
        .expect("issue epoch-bound remember credential");
    let original = credential.expose_once();
    let sessions_before = world
        .sessions
        .list_for_user(&user.user_id)
        .await
        .expect("list sessions before remembered sign-in");

    let outcome = service
        .sign_in(
            RememberCredential::from_host(original.clone()),
            SessionMetadata {
                user_agent: Some("remember-contract-agent".to_owned()),
                ip_address: Some("192.0.2.18".to_owned()),
            },
            now,
        )
        .await
        .expect("trusted remember proof signs in");

    assert_eq!(gate.methods(), vec![SignInMethod::Remembered]);
    assert_eq!(outcome.session.user_id(), user.user_id);
    assert_eq!(
        outcome.session.metadata().user_agent.as_deref(),
        Some("remember-contract-agent")
    );
    assert_ne!(
        outcome.session.session_id(),
        actor.opaque_session_id().expect("actor has opaque session")
    );
    assert_eq!(
        world
            .sessions
            .list_for_user(&user.user_id)
            .await
            .expect("list sessions after remembered sign-in")
            .len(),
        sessions_before.len() + 1,
        "the remembered path must mint a real opaque Magnetar session",
    );

    let replacement = outcome.replacement.expose_once();
    assert!(!replacement.expose_secret().is_empty());
    assert!(
        service
            .sign_in(
                RememberCredential::from_host(original),
                SessionMetadata::default(),
                now,
            )
            .await
            .is_err(),
        "the consumed remember credential must never replay",
    );
    service
        .sign_in(
            RememberCredential::from_host(replacement),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect("the rotated replacement remains the sole live credential");
}

#[tokio::test]
async fn replacement_insert_failure_preserves_original_credential_for_retry() {
    let world = factor_world().await;
    let user = user(&world, "remembered-rotation-failure@example.test").await;
    let store = Arc::new(storage_schema::sql_stores::SqlRememberStore(
        world.db.clone(),
    ));
    let hook = Arc::new(RecordingRememberAnomalyHook::default());
    let remember = Arc::new(
        RememberService::new(store.clone(), Duration::days(30))
            .expect("compose remember service")
            .with_anomaly_hook(hook.clone()),
    );
    let token_service: Arc<dyn RememberTokenService> = remember;
    let (gate, service) = service_with_token_service(&world, token_service);
    let now = Utc::now();
    let original = service
        .issue(&user.user_id, now)
        .await
        .expect("issue original remember credential")
        .expose_once();
    let original_plaintext = original.expose_secret().to_owned();
    let (original_selector, _) = credential_parts(&original);
    let original_row = store
        .find_for_rotation(&original_selector, now)
        .await
        .expect("read original remember row")
        .expect("original remember row exists");
    let sessions_before = world
        .sessions
        .list_for_user(&user.user_id)
        .await
        .expect("list sessions before failed rotation")
        .len();

    world
        .db
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!(
                "CREATE TRIGGER fail_remember_replacement
                 BEFORE INSERT ON storage_remembers
                 WHEN NEW.user_id = '{}'
                 BEGIN
                     SELECT RAISE(ABORT, 'injected remember replacement failure');
                 END",
                user.user_id
            ),
        ))
        .await
        .expect("install scoped replacement failure trigger");

    let error = service
        .sign_in(
            RememberCredential::from_host(SecretString::from(original_plaintext.clone())),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect_err("replacement insertion failure must surface");
    assert!(matches!(error, Error::Internal { .. }));
    assert_eq!(
        store
            .find_for_rotation(&original_selector, now)
            .await
            .expect("inspect original row after failed rotation"),
        Some(original_row),
        "failed replacement insertion must preserve the exact original row",
    );
    assert!(gate.methods().is_empty());
    assert!(hook.events().is_empty());
    assert_eq!(
        world
            .sessions
            .list_for_user(&user.user_id)
            .await
            .expect("list sessions after failed rotation")
            .len(),
        sessions_before,
    );

    world
        .db
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER fail_remember_replacement".to_owned(),
        ))
        .await
        .expect("remove replacement failure trigger before retry");

    let replacement = service
        .sign_in(
            RememberCredential::from_host(SecretString::from(original_plaintext.clone())),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect("retry with the preserved original credential succeeds")
        .replacement
        .expose_once();

    let replay_error = service
        .sign_in(
            RememberCredential::from_host(SecretString::from(original_plaintext)),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect_err("the successfully rotated original must not replay");
    assert!(matches!(
        replay_error,
        Error::NotFound {
            resource,
            identifier,
        } if resource == "remember token" && identifier == "expired or revoked"
    ));

    service
        .sign_in(
            RememberCredential::from_host(replacement),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect("the returned replacement remains usable");
    assert_eq!(
        gate.methods(),
        vec![SignInMethod::Remembered, SignInMethod::Remembered]
    );
    assert_eq!(
        world
            .sessions
            .list_for_user(&user.user_id)
            .await
            .expect("list sessions after successful rotations")
            .len(),
        sessions_before + 2,
    );
}

#[tokio::test]
async fn auth_epoch_change_fences_an_unconsumed_remember_credential() {
    let world = factor_world().await;
    let user = user(&world, "remembered-epoch@example.test").await;
    let (_, service) = service(&world);
    let now = Utc::now();
    let credential = service
        .issue(&user.user_id, now)
        .await
        .expect("issue epoch-bound remember credential");
    let sessions_before = world
        .sessions
        .list_for_user(&user.user_id)
        .await
        .expect("list sessions before epoch fence")
        .len();

    world
        .storage
        .bump_auth_epoch(&user.user_id)
        .await
        .expect("advance authentication epoch");

    assert!(
        service
            .sign_in(credential, SessionMetadata::default(), now)
            .await
            .is_err(),
        "a credential issued at the previous epoch must not hydrate",
    );
    assert_eq!(
        world
            .sessions
            .list_for_user(&user.user_id)
            .await
            .expect("list sessions after epoch fence")
            .len(),
        sessions_before,
    );
    assert_eq!(
        service
            .revoke_all_for_user(&user.user_id)
            .await
            .expect("inspect rows after rejected compatibility sign-in"),
        0,
        "the compatibility adapter must retire a rejected successor",
    );
}

#[tokio::test]
async fn explicit_revoke_prevents_remembered_hydration() {
    let world = factor_world().await;
    let user = user(&world, "remembered-revoke@example.test").await;
    let (_, service) = service(&world);
    let credential = service
        .issue(&user.user_id, Utc::now())
        .await
        .expect("issue remember credential");

    assert_eq!(
        world
            .remember
            .revoke_all_for_user(&user.user_id)
            .await
            .expect("revoke remember credentials"),
        1,
    );
    assert!(
        service
            .sign_in(credential, SessionMetadata::default(), Utc::now())
            .await
            .is_err(),
        "an explicitly revoked credential must not hydrate",
    );
}

#[tokio::test]
async fn password_reset_prevents_remembered_hydration() {
    let world = factor_world().await;
    let user = user(&world, "remembered-reset@example.test").await;
    let (_, service) = service(&world);
    let credential = service
        .issue(&user.user_id, Utc::now())
        .await
        .expect("issue remember credential");

    world
        .management
        .send_link(&user.email)
        .await
        .expect("send reset link");
    let reset_link = world
        .mail
        .sent
        .lock()
        .last()
        .and_then(|message| message.payload.get("reset_link"))
        .and_then(serde_json::Value::as_str)
        .expect("recorded reset link")
        .to_owned();
    let token = reset_link
        .split_once("token=")
        .map(|(_, token)| token)
        .expect("reset link contains token");
    world
        .management
        .complete(token, "replacement-password")
        .await
        .expect("complete real password reset");

    let error = service
        .sign_in(credential, SessionMetadata::default(), Utc::now())
        .await
        .expect_err("password reset must revoke remembered hydration");
    assert!(
        matches!(error, Error::NotFound { .. } | Error::InvalidInput { .. }),
        "reset rejection must be an authentication failure, got {error:?}",
    );
}

fn credential_parts(raw: &SecretString) -> (String, String) {
    let (selector, verifier) = raw
        .expose_secret()
        .split_once('.')
        .expect("remember credential has selector and verifier");
    (selector.to_owned(), verifier.to_owned())
}

#[tokio::test]
async fn erased_remember_facade_issues_at_the_current_nonzero_epoch() {
    let world = factor_world().await;
    let user = user(&world, "remembered-erased-epoch@example.test").await;
    let current_epoch = world
        .storage
        .bump_auth_epoch(&user.user_id)
        .await
        .expect("advance epoch before remember issuance");
    assert!(current_epoch > 0);

    let remember: Arc<dyn RememberTokenService> = world.remember.clone();
    let (_, service) = service_with_token_service(&world, remember);
    let now = Utc::now();
    let credential = service
        .issue(&user.user_id, now)
        .await
        .expect("erased facade issues at the loaded epoch");
    let raw = credential.expose_once();
    let (selector, _) = credential_parts(&raw);
    let store = storage_schema::sql_stores::SqlRememberStore(world.db.clone());
    let row = store
        .find_for_rotation(&selector, now)
        .await
        .expect("read issued remember row")
        .expect("issued remember row exists");
    assert_eq!(
        row.auth_epoch, current_epoch,
        "the erased facade must never fall back to epoch zero",
    );

    let outcome = service
        .sign_in(
            RememberCredential::from_host(raw),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect("current-epoch credential signs in through erased facade");
    assert_eq!(outcome.session.user_id(), user.user_id);
}

#[tokio::test]
async fn verifier_mismatch_revokes_the_users_rows_and_emits_one_redacted_anomaly() {
    let world = factor_world().await;
    let user = user(&world, "remembered-mismatch@example.test").await;
    let (store, hook, service) = service_with_anomaly_hook(&world);
    let now = Utc::now();
    let attacked = service
        .issue(&user.user_id, now)
        .await
        .expect("issue attacked credential")
        .expose_once();
    let sibling = service
        .issue(&user.user_id, now)
        .await
        .expect("issue sibling credential")
        .expose_once();
    let (attacked_selector, attacked_verifier) = credential_parts(&attacked);
    let (sibling_selector, _) = credential_parts(&sibling);
    let forged_verifier = "ff".repeat(32);
    let forged = RememberCredential::from_host(SecretString::from(format!(
        "{attacked_selector}.{forged_verifier}"
    )));

    assert!(
        service
            .sign_in(forged, SessionMetadata::default(), now)
            .await
            .is_err(),
        "a valid selector with the wrong verifier must fail",
    );
    let events = hook.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, RememberAnomalyKind::VerifierMismatch);
    assert_eq!(events[0].user_id.as_deref(), Some(user.user_id.as_str()));
    let event_debug = format!("{:?}", events[0]);
    assert!(!event_debug.contains(&attacked_selector));
    assert!(!event_debug.contains(&attacked_verifier));
    assert!(!event_debug.contains(&forged_verifier));
    assert!(
        store
            .find_for_rotation(&sibling_selector, now)
            .await
            .expect("inspect sibling row")
            .is_none(),
        "verifier mismatch revokes every remember row for the resolved user",
    );
}

#[tokio::test]
async fn unknown_and_replayed_selectors_are_redacted_without_cross_user_revocation() {
    let world = factor_world().await;
    let unrelated = user(&world, "remembered-unrelated@example.test").await;
    let replayed_user = user(&world, "remembered-replayed@example.test").await;
    let (store, hook, service) = service_with_anomaly_hook(&world);
    let now = Utc::now();
    let unrelated_credential = service
        .issue(&unrelated.user_id, now)
        .await
        .expect("issue unrelated credential")
        .expose_once();
    let (unrelated_selector, unrelated_verifier) = credential_parts(&unrelated_credential);
    let unknown_selector = "11".repeat(16);
    let unknown_verifier = "22".repeat(32);

    assert!(
        service
            .sign_in(
                RememberCredential::from_host(SecretString::from(format!(
                    "{unknown_selector}.{unknown_verifier}"
                ))),
                SessionMetadata::default(),
                now,
            )
            .await
            .is_err(),
    );
    let unknown_events = hook.take();
    assert_eq!(unknown_events.len(), 1);
    assert_eq!(
        unknown_events[0].kind,
        RememberAnomalyKind::UnknownOrReusedSelector,
    );
    assert_eq!(unknown_events[0].user_id, None);
    let unknown_debug = format!("{:?}", unknown_events[0]);
    assert!(!unknown_debug.contains(&unknown_selector));
    assert!(!unknown_debug.contains(&unknown_verifier));
    assert!(
        store
            .find_for_rotation(&unrelated_selector, now)
            .await
            .expect("inspect unrelated row after unknown selector")
            .is_some(),
    );

    let replayed = service
        .issue(&replayed_user.user_id, now)
        .await
        .expect("issue replay target")
        .expose_once();
    let replayed_plaintext = replayed.expose_secret().to_owned();
    service
        .sign_in(
            RememberCredential::from_host(SecretString::from(replayed_plaintext.clone())),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect("first presentation succeeds");
    assert!(hook.take().is_empty());
    assert!(
        service
            .sign_in(
                RememberCredential::from_host(SecretString::from(replayed_plaintext.clone())),
                SessionMetadata::default(),
                now,
            )
            .await
            .is_err(),
        "replayed selector fails",
    );
    let replay_events = hook.take();
    assert_eq!(replay_events.len(), 1);
    assert_eq!(
        replay_events[0].kind,
        RememberAnomalyKind::UnknownOrReusedSelector,
    );
    let replay_debug = format!("{:?}", replay_events[0]);
    assert!(!replay_debug.contains(&replayed_plaintext));
    assert!(!replay_debug.contains(&unrelated_selector));
    assert!(!replay_debug.contains(&unrelated_verifier));
    assert!(
        store
            .find_for_rotation(&unrelated_selector, now)
            .await
            .expect("inspect unrelated row after replay")
            .is_some(),
        "unknown or replayed selectors must not revoke unrelated users",
    );
    service
        .sign_in(
            RememberCredential::from_host(unrelated_credential),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect("unrelated user's credential remains usable");
}

#[tokio::test]
async fn new_verifiers_use_sha256_while_legacy_bcrypt_rotates_and_upgrades() {
    let world = factor_world().await;
    let user = user(&world, "remembered-hash-upgrade@example.test").await;
    let (store, hook, service) = service_with_anomaly_hook(&world);
    let now = Utc::now();

    let issued = service
        .issue(&user.user_id, now)
        .await
        .expect("issue modern remember credential")
        .expose_once();
    let (issued_selector, issued_verifier) = credential_parts(&issued);
    let issued_row = store
        .find_for_rotation(&issued_selector, now)
        .await
        .expect("read modern row")
        .expect("modern row exists");
    assert!(issued_row.verifier_hash.starts_with("sha256:"));
    assert!(!issued_row.verifier_hash.starts_with("$2"));
    let issued_debug = format!("{issued_row:?}");
    assert!(!issued_debug.contains(&issued_selector));
    assert!(!issued_debug.contains(&issued_verifier));

    let legacy_selector = "33".repeat(16);
    let legacy_verifier = "44".repeat(32);
    let legacy_hash = bcrypt::hash(&legacy_verifier, 4).expect("hash legacy verifier");
    store
        .insert_remember(RememberRow {
            id: "legacy-bcrypt-row".to_owned(),
            selector: legacy_selector.clone(),
            user_id: user.user_id.clone(),
            auth_epoch: user.auth_epoch,
            verifier_hash: legacy_hash,
            expires_at: now + Duration::days(30),
        })
        .await
        .expect("seed legacy bcrypt row");

    let replacement = service
        .sign_in(
            RememberCredential::from_host(SecretString::from(format!(
                "{legacy_selector}.{legacy_verifier}"
            ))),
            SessionMetadata::default(),
            now,
        )
        .await
        .expect("legacy bcrypt verifier rotates successfully")
        .replacement
        .expose_once();
    let (replacement_selector, replacement_verifier) = credential_parts(&replacement);
    let replacement_row = store
        .find_for_rotation(&replacement_selector, now)
        .await
        .expect("read upgraded replacement")
        .expect("upgraded replacement exists");
    assert!(replacement_row.verifier_hash.starts_with("sha256:"));
    assert!(!replacement_row.verifier_hash.starts_with("$2"));
    let replacement_debug = format!("{replacement_row:?}");
    assert!(!replacement_debug.contains(&replacement_selector));
    assert!(!replacement_debug.contains(&replacement_verifier));
    assert!(hook.events().is_empty());

    let credential_debug = format!(
        "{:?}",
        RememberCredential::from_host(SecretString::from(format!(
            "{legacy_selector}.{legacy_verifier}"
        )))
    );
    assert!(!credential_debug.contains(&legacy_selector));
    assert!(!credential_debug.contains(&legacy_verifier));
}
