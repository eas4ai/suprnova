//! Shared composition harness for the password-domain suites.
//!
//! Composes the real SeaORM stores over in-memory SQLite, the real opaque
//! session provider and factor gate, recording mail/limiter fakes, and the
//! three password-domain plugins — the same shape the example host uses.

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use magnetar::Result;
use magnetar::auth::{FactorVerifier, OpaqueFactorGate};
use magnetar::crypto::AeadEncryptor;
use magnetar::password::{
    LockoutConfig, LockoutService, PasswordHashConfig, PasswordHashDriver, PasswordVerifier,
    StandardPasswordHashDriver,
};
use magnetar::plugin::{
    Effect, HttpRequest, HttpResponse, HttpTransport, Method, PluginContext, PluginRegistry,
    WireBody, WireRequest, WireResponse,
};
use magnetar::plugins::email_verification::{
    EmailVerificationPlugin, EmailVerificationPluginConfig, EmailVerificationService,
};
use magnetar::plugins::password::{
    PasswordAuthService, PasswordPlugin, PasswordPluginConfig, RegistrationVerification,
};
use magnetar::plugins::password_management::{
    PasswordManagementPlugin, PasswordManagementPluginConfig, PasswordManagementService,
};
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, RememberFacade, RememberService, SessionGrant,
};
use magnetar::storage::SeaOrmStorage;
use serde_json::{Value, json};

use super::storage_schema::sql_stores::{SqlRememberStore, SqlSessionStore};
use super::storage_schema::{StorageSchema, database};

#[path = "fakes.rs"]
mod fakes;
#[allow(unused_imports)]
pub use fakes::{
    CountingLimiter, LimiterMode, RecordingMail, SequentialFirstProofStore, TestLinks,
};

/// A fast, real-work hash profile so flow tests stay quick while keeping the
/// exact dual-format semantics. Corpus tests use the deployed default.
pub fn fast_hash_config() -> PasswordHashConfig {
    PasswordHashConfig {
        bcrypt_cost: 4,
        argon2_memory_kib: 8,
        argon2_iterations: 1,
        argon2_parallelism: 1,
    }
}

/// Plugin-context encryptor fake (plugin-owned opaque data only).
pub struct IdentityEncryptor;

#[async_trait]
impl magnetar::plugin::Encryptor for IdentityEncryptor {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }
    async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        Ok(ciphertext.to_vec())
    }
}

/// Outbound transport fake; the password domain sends no HTTP.
pub struct NoTransport;

#[async_trait]
impl HttpTransport for NoTransport {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse> {
        Err(magnetar::Error::Internal {
            message: "password domain sends no outbound HTTP".into(),
        })
    }
}

/// No user has a confirmed second factor in these suites.
pub struct NoSecondFactor;

#[async_trait]
impl FactorVerifier for NoSecondFactor {
    async fn has_confirmed_enrollment(&self, _user_id: &str) -> Result<bool> {
        Ok(false)
    }
    async fn verify_code(&self, _user_id: &str, _code: &str) -> Result<bool> {
        Ok(false)
    }
}

/// The composed world one suite operates in.
pub struct Harness {
    pub db: sea_orm::DatabaseConnection,
    pub storage: Arc<SeaOrmStorage<StorageSchema>>,
    pub sessions: Arc<OpaqueSessionProvider<SqlSessionStore>>,
    pub remember: Arc<RememberService<SqlRememberStore>>,
    pub verifier: Arc<PasswordVerifier>,
    pub lockout: Arc<LockoutService>,
    pub provider: Arc<PasswordAuthService>,
    pub verification: Arc<EmailVerificationService>,
    pub management: Arc<PasswordManagementService>,
    pub mail: Arc<RecordingMail>,
    pub limiter: Arc<CountingLimiter>,
    pub registry: PluginRegistry<StorageSchema>,
}

/// Compose the harness with an explicit hash driver and lockout config.
pub async fn harness_with(
    driver: Arc<dyn PasswordHashDriver>,
    hash_config: PasswordHashConfig,
    lockout_config: LockoutConfig,
) -> Harness {
    let db = database().await;
    let storage = Arc::new(SeaOrmStorage::<StorageSchema>::new(db.clone()));
    let session_store = Arc::new(SqlSessionStore(db.clone()));
    let sessions = Arc::new(OpaqueSessionProvider::new(
        session_store,
        OpaqueConfig::default(),
    ));
    let remember = Arc::new(
        RememberService::new(
            Arc::new(SqlRememberStore(db.clone())),
            chrono::Duration::days(30),
        )
        .expect("remember lifetime is positive"),
    );
    let verifier =
        Arc::new(PasswordVerifier::new(driver, hash_config).expect("dummy warmup succeeds"));
    let lockout = Arc::new(LockoutService::new(
        storage.clone(),
        storage.clone(),
        lockout_config,
    ));
    let mail = Arc::new(RecordingMail::default());
    let limiter = Arc::new(CountingLimiter::default());
    let links = Arc::new(TestLinks);

    let provider = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        verifier.clone(),
    ));
    let verification = Arc::new(EmailVerificationService::new(
        storage.clone(),
        storage.clone(),
        mail.clone(),
        links.clone(),
    ));
    let first_proof = Arc::new(SequentialFirstProofStore::new(
        storage.clone(),
        storage.clone(),
        storage.clone(),
        remember.clone(),
    ));
    let management = Arc::new(PasswordManagementService::new(
        storage.clone(),
        storage.clone(),
        first_proof,
        verifier.clone(),
        lockout.clone(),
        mail.clone(),
        links.clone(),
    ));

    let encryptor = Arc::new(AeadEncryptor::new([7; 32]));
    let gate = Arc::new(OpaqueFactorGate::new(
        storage.clone(),
        Arc::new(NoSecondFactor),
        encryptor,
        sessions.clone(),
    ));

    let context = PluginContext::new(
        storage.clone(),
        sessions.clone(),
        gate,
        Arc::new(IdentityEncryptor),
        limiter.clone(),
        mail.clone(),
        Arc::new(NoTransport),
        links,
    );

    let registry = PluginRegistry::new(context)
        .register(PasswordPlugin::new(
            provider.clone(),
            lockout.clone(),
            Some(verification.clone() as Arc<dyn RegistrationVerification>),
            Some(remember.clone() as Arc<dyn RememberFacade>),
            PasswordPluginConfig::default(),
        ))
        .register(EmailVerificationPlugin::new(
            verification.clone(),
            EmailVerificationPluginConfig::default(),
        ))
        .register(PasswordManagementPlugin::new(
            management.clone(),
            PasswordManagementPluginConfig::default(),
        ))
        .build()
        .await
        .expect("plugin composition is valid");

    Harness {
        db,
        storage,
        sessions,
        remember,
        verifier,
        lockout,
        provider,
        verification,
        management,
        mail,
        limiter,
        registry,
    }
}

/// Compose the default harness: real hashing at the fast profile.
pub async fn harness() -> Harness {
    harness_with(
        Arc::new(StandardPasswordHashDriver),
        fast_hash_config(),
        LockoutConfig::default(),
    )
    .await
}

/// Build a JSON POST request for a route path.
pub fn post_json(path: &str, body: Value) -> WireRequest {
    let mut request = WireRequest::new(Method::Post, path);
    request.body = WireBody::Json(body);
    request
        .headers
        .insert("user-agent".into(), "harness-agent".into());
    request
        .headers
        .insert("x-client-ip".into(), "203.0.113.7".into());
    request
}

/// Build the register request body.
pub fn register_request(email: &str, password: &str) -> WireRequest {
    post_json("/register", json!({"email": email, "password": password}))
}

/// Build the login request body.
pub fn login_request(email: &str, password: &str) -> WireRequest {
    post_json("/login", json!({"email": email, "password": password}))
}

/// Decompose a response into comparable parts and extract any session grant.
pub struct Reply {
    pub status: u16,
    pub body: Option<Value>,
    pub grant: Option<SessionGrant>,
    pub cleared_session: bool,
    pub remember_issued: bool,
    pub headers: Vec<(String, String)>,
}

pub fn split(response: WireResponse) -> Reply {
    let effects = response.into_effects();
    let mut grant = None;
    let mut cleared_session = false;
    let mut remember_issued = false;
    let mut headers = Vec::new();
    for effect in effects.effects {
        match effect {
            Effect::EstablishSession(value) => grant = Some(value),
            Effect::ClearSession => cleared_session = true,
            Effect::IssueRemember(_) => remember_issued = true,
            Effect::SetHeader { name, value } => headers.push((name, value)),
            _ => {}
        }
    }
    Reply {
        status: effects.status,
        body: effects.body,
        grant,
        cleared_session,
        remember_issued,
        headers,
    }
}

/// Dispatch one request through the registry without a bound session.
pub async fn dispatch(harness: &Harness, request: WireRequest) -> Reply {
    split(
        harness
            .registry
            .handle(request)
            .await
            .expect("route dispatch succeeds"),
    )
}
