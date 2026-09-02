#![cfg(feature = "magnetar-oauth")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use suprnova::magnetar_integration::engine::{
    HostOAuthError, MagnetarOAuthAuthEngine, MagnetarOAuthBegin, MagnetarOAuthCallback,
    MagnetarOAuthCompletion, MagnetarOAuthKickoff,
};
use suprnova::{
    Auth, Crypt, EncryptionKey, MagnetarConfig, RateLimiterDriver, SlidingWindowConfig,
    init_magnetar,
};

struct AllowingLimiter;

#[suprnova::async_trait]
impl RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<bool, suprnova::FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, suprnova::FrameworkError> {
        Ok(None)
    }
}

#[derive(Default)]
struct ExistingOAuthEngine {
    completion_calls: AtomicUsize,
    identity_calls: AtomicUsize,
}

#[suprnova::async_trait]
impl MagnetarOAuthAuthEngine for ExistingOAuthEngine {
    fn oauth_supports_provider(&self, provider: &str) -> bool {
        provider == "existing"
    }

    async fn oauth_begin(
        &self,
        _input: MagnetarOAuthBegin,
    ) -> Result<MagnetarOAuthKickoff, HostOAuthError> {
        Err(HostOAuthError::Auth(magnetar::Error::Internal {
            message: "unused preinstalled OAuth engine".to_owned(),
        }))
    }

    async fn oauth_complete(
        &self,
        _input: MagnetarOAuthCallback,
    ) -> Result<MagnetarOAuthCompletion, HostOAuthError> {
        self.completion_calls.fetch_add(1, Ordering::SeqCst);
        Err(HostOAuthError::Auth(magnetar::Error::Internal {
            message: "unused preinstalled OAuth engine".to_owned(),
        }))
    }

    async fn oauth_verify_identity(
        &self,
        _input: MagnetarOAuthCallback,
    ) -> Result<magnetar::oauth::VerifiedProviderIdentity, HostOAuthError> {
        self.identity_calls.fetch_add(1, Ordering::SeqCst);
        Ok(magnetar::oauth::VerifiedProviderIdentity {
            provider: "existing".to_owned(),
            subject: "identity-only-subject".to_owned(),
            email: Some("identity-only@example.test".to_owned()),
            email_verified: true,
            display_name: Some("Identity Only".to_owned()),
        })
    }
}

#[tokio::test]
async fn oauth_conflict_publishes_no_default_engine_or_schema() {
    Crypt::init(EncryptionKey::generate());
    suprnova::App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
    let existing_oauth = Arc::new(ExistingOAuthEngine::default());
    suprnova::magnetar_integration::install_magnetar_oauth_engine(existing_oauth.clone())
        .expect("preinstall OAuth engine");

    let identity = Auth::oauth("existing")
        .verify_oauth_identity("code", "state")
        .await
        .expect("legacy OAuth install retains identity verification");
    assert_eq!(identity.subject, "identity-only-subject");
    assert_eq!(existing_oauth.identity_calls.load(Ordering::SeqCst), 1);

    let sign_in = Auth::oauth("existing")
        .complete_outcome("code", "state")
        .await
        .expect_err("an OAuth-only legacy install cannot complete sign-in");
    assert_eq!(
        existing_oauth.completion_calls.load(Ordering::SeqCst),
        0,
        "missing factor/session authority must fail before consuming OAuth completion",
    );
    assert_eq!(
        sign_in.to_string(),
        "Internal server error: Magnetar factor/session engine is not installed",
    );
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");

    let error = init_magnetar(MagnetarConfig::from_sea_orm(database.clone()))
        .await
        .expect_err("OAuth conflict must reject the complete default installation");
    assert!(error.to_string().contains("already installed"));

    let app_users = database
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'app_users'",
        ))
        .await
        .expect("inspect rejected database");
    assert!(
        app_users.is_none(),
        "rejected installation must not mutate schema"
    );

    let password_error = Auth::password()
        .register("not-installed@example.test", "correct-password")
        .await
        .expect_err("password engine must remain unpublished");
    assert!(
        password_error.to_string().contains("not installed"),
        "unexpected password error: {password_error}"
    );
}
