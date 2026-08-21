//! Full-composition harness: every 002 plugin mounted, with the real
//! two-factor service wired in as the factor gate's verifier so the
//! universal promotion path is exercised exactly as a host composes it.

#![allow(dead_code)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use magnetar::Result;
use magnetar::auth::OpaqueFactorGate;
use magnetar::crypto::AeadEncryptor;
use magnetar::passkey::{PasskeyAuthService, PasskeyConfig};
use magnetar::password::{
    LockoutConfig, LockoutService, PasswordVerifier, StandardPasswordHashDriver,
};
use magnetar::plugin::{PluginContext, PluginRegistry};
use magnetar::plugins::email_verification::{
    EmailVerificationPlugin, EmailVerificationPluginConfig, EmailVerificationService,
};
use magnetar::plugins::magic_link::{
    MagicLinkPlugin, MagicLinkPluginConfig, MagicLinkService, RegistrationPolicy,
};
use magnetar::plugins::passkey::{PasskeyPlugin, PasskeyPluginConfig, ReauthSource};
use magnetar::plugins::password::{
    PasswordAuthService, PasswordPlugin, PasswordPluginConfig, RegistrationVerification,
};
use magnetar::plugins::password_management::{
    PasswordManagementPlugin, PasswordManagementPluginConfig, PasswordManagementService,
};
use magnetar::plugins::two_factor::TwoFactorPlugin;
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, RememberFacade, RememberService, VerifiedSession,
};
use magnetar::storage::SeaOrmStorage;
use magnetar::two_factor::{TwoFactorConfig, TwoFactorService};
use parking_lot::Mutex;
use secrecy::ExposeSecret;

use super::harness::{
    CountingLimiter, IdentityEncryptor, NoTransport, RecordingMail, SequentialFirstProofStore,
    TestLinks, fast_hash_config,
};
use super::storage_schema::sql_stores::{SqlRememberStore, SqlSessionStore};
use super::storage_schema::sql_two_factor::SqlTwoFactorStore;
use super::storage_schema::{StorageSchema, database};

/// A host reauth boundary with a test-settable stamp.
#[derive(Default)]
pub struct StubReauth(pub Mutex<Option<DateTime<Utc>>>);

#[async_trait::async_trait]
impl ReauthSource for StubReauth {
    async fn password_confirmed_at(
        &self,
        _session: &VerifiedSession,
    ) -> Result<Option<DateTime<Utc>>> {
        Ok(*self.0.lock())
    }
}

/// The fully composed world.
pub struct FactorWorld {
    pub db: sea_orm::DatabaseConnection,
    pub gate: Arc<dyn magnetar::auth::FactorGate>,
    pub storage: Arc<SeaOrmStorage<StorageSchema>>,
    pub sessions: Arc<OpaqueSessionProvider<SqlSessionStore>>,
    pub remember: Arc<RememberService<SqlRememberStore>>,
    pub lockout: Arc<LockoutService>,
    pub two_factor: Arc<TwoFactorService>,
    pub magic: Arc<MagicLinkService>,
    pub passkeys: Arc<PasskeyAuthService>,
    pub provider: Arc<PasswordAuthService>,
    pub verification: Arc<EmailVerificationService>,
    pub management: Arc<PasswordManagementService>,
    pub first_proof: Arc<SequentialFirstProofStore>,
    pub mail: Arc<RecordingMail>,
    pub limiter: Arc<CountingLimiter>,
    pub reauth: Arc<StubReauth>,
    pub registry: PluginRegistry<StorageSchema>,
}

/// Compose everything with an explicit magic-link policy and lockout
/// configuration.
pub async fn factor_world_with(
    policy: RegistrationPolicy,
    lockout_config: LockoutConfig,
) -> FactorWorld {
    let db = database().await;
    let storage = Arc::new(SeaOrmStorage::<StorageSchema>::new(db.clone()));
    let sessions = Arc::new(OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(db.clone())),
        OpaqueConfig::default(),
    ));
    let remember = Arc::new(
        RememberService::new(
            Arc::new(SqlRememberStore(db.clone())),
            chrono::Duration::days(30),
        )
        .expect("remember lifetime is positive"),
    );
    let verifier = Arc::new(
        PasswordVerifier::new(Arc::new(StandardPasswordHashDriver), fast_hash_config())
            .expect("dummy warmup succeeds"),
    );
    let lockout = Arc::new(LockoutService::new(
        storage.clone(),
        storage.clone(),
        lockout_config,
    ));
    let mail = Arc::new(RecordingMail::default());
    let limiter = Arc::new(CountingLimiter::default());
    let links = Arc::new(TestLinks);
    let reauth = Arc::new(StubReauth::default());
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
        crypto.clone(),
        sessions.clone(),
    ));

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
        first_proof.clone(),
        verifier,
        lockout.clone(),
        mail.clone(),
        links.clone(),
    ));
    let magic = Arc::new(MagicLinkService::new(
        storage.clone(),
        storage.clone(),
        first_proof.clone(),
        gate.clone(),
        policy,
    ));
    let passkeys = Arc::new(
        PasskeyAuthService::new(
            &PasskeyConfig::default(),
            storage.clone(),
            storage.clone(),
            storage.clone(),
            crypto.clone(),
            gate.clone(),
        )
        .expect("localhost relying party is valid"),
    );

    let context = PluginContext::new(
        storage.clone(),
        sessions.clone(),
        gate.clone(),
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
        .register(MagicLinkPlugin::new(
            magic.clone(),
            mail.clone(),
            Arc::new(TestLinks),
            MagicLinkPluginConfig::default(),
        ))
        .register(PasskeyPlugin::new(
            passkeys.clone(),
            reauth.clone(),
            PasskeyPluginConfig::default(),
        ))
        .register(TwoFactorPlugin::new(
            two_factor.clone(),
            Some(remember.clone() as Arc<dyn RememberFacade>),
        ))
        .build()
        .await
        .expect("plugin composition is valid");

    FactorWorld {
        db,
        gate,
        storage,
        sessions,
        remember,
        lockout,
        two_factor,
        magic,
        passkeys,
        provider,
        verification,
        first_proof,
        management,
        mail,
        limiter,
        reauth,
        registry,
    }
}

/// Compose the default world: open magic-link policy, default lockout.
pub async fn factor_world() -> FactorWorld {
    factor_world_with(RegistrationPolicy::Open, LockoutConfig::default()).await
}

/// Generate the current TOTP code from an enrollment's otpauth URL.
pub fn totp_code_now(otpauth_url: &secrecy::SecretString) -> String {
    totp_code_at(otpauth_url, Utc::now().timestamp())
}

/// Generate the TOTP code for an arbitrary Unix time.
pub fn totp_code_at(otpauth_url: &secrecy::SecretString, unix_seconds: i64) -> String {
    let totp = totp_rs::TOTP::from_url_unchecked(otpauth_url.expose_secret())
        .expect("enrollment otpauth URL parses");
    totp.generate(unix_seconds as u64)
}

/// The registration-time origin the software authenticator stamps into
/// `clientDataJSON`; must agree with [`PasskeyConfig::default`].
pub fn passkey_test_origin() -> webauthn_authenticator_rs::prelude::Url {
    webauthn_authenticator_rs::prelude::Url::parse("http://localhost").expect("test origin")
}

/// A software authenticator that claims user verification.
pub fn soft_authenticator() -> webauthn_authenticator_rs::WebauthnAuthenticator<
    webauthn_authenticator_rs::softpasskey::SoftPasskey,
> {
    webauthn_authenticator_rs::WebauthnAuthenticator::new(
        webauthn_authenticator_rs::softpasskey::SoftPasskey::new(true),
    )
}

/// Dispatch one request through the composed registry.
pub async fn send(
    world: &FactorWorld,
    request: magnetar::plugin::WireRequest,
) -> super::harness::Reply {
    super::harness::split(
        world
            .registry
            .handle(request)
            .await
            .expect("route dispatch succeeds"),
    )
}

/// Dispatch one request with a host-verified web binding.
pub async fn send_bound(
    world: &FactorWorld,
    request: magnetar::plugin::WireRequest,
    binding: &magnetar::sessions::WebSessionBinding,
) -> super::harness::Reply {
    super::harness::split(
        world
            .registry
            .handle_web_binding(request, binding)
            .await
            .expect("route dispatch succeeds"),
    )
}
