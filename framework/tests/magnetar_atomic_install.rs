#![cfg(feature = "magnetar-oauth")]

use std::sync::Arc;

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

struct ExistingOAuthEngine;

#[suprnova::async_trait]
impl MagnetarOAuthAuthEngine for ExistingOAuthEngine {
    fn oauth_supports_provider(&self, _provider: &str) -> bool {
        false
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
        Err(HostOAuthError::Auth(magnetar::Error::Internal {
            message: "unused preinstalled OAuth engine".to_owned(),
        }))
    }

    async fn oauth_verify_identity(
        &self,
        _input: MagnetarOAuthCallback,
    ) -> Result<magnetar::oauth::VerifiedProviderIdentity, HostOAuthError> {
        Err(HostOAuthError::Auth(magnetar::Error::Internal {
            message: "unused preinstalled OAuth engine".to_owned(),
        }))
    }
}

#[tokio::test]
async fn oauth_conflict_publishes_no_default_engine_or_schema() {
    Crypt::init(EncryptionKey::generate());
    suprnova::App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
    suprnova::magnetar_integration::install_magnetar_oauth_engine(Arc::new(ExistingOAuthEngine))
        .expect("preinstall OAuth engine");
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
