use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use suprnova::{
    Auth, Crypt, EncryptionKey, MagnetarConfig, RateLimiterDriver, SlidingWindowConfig,
    init_magnetar,
};

struct AllowingLimiter;

#[async_trait]
impl RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(
        &self,
        _: &str,
        _: &SlidingWindowConfig,
    ) -> Result<bool, suprnova::FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _: &str,
        _: &SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, suprnova::FrameworkError> {
        Ok(None)
    }
}

#[tokio::test]
async fn default_installer_runs_password_session_and_lockout_flows() {
    Crypt::init(EncryptionKey::generate());
    suprnova::App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    init_magnetar(MagnetarConfig::from_sea_orm(connection))
        .await
        .expect("install default Magnetar engine");

    let user = Auth::password()
        .register("default-engine@example.test", "correct-password")
        .await
        .expect("register user");
    let (authenticated, session) = Auth::password()
        .authenticate(
            "DEFAULT-ENGINE@example.test",
            "correct-password",
            None,
            None,
        )
        .await
        .expect("authenticate user");
    assert_eq!(authenticated.id, user.id);
    assert!(session.token.is_some());
    assert_eq!(
        suprnova::magnetar_integration::find_user_by_id(user.id.as_str())
            .await
            .expect("lookup user")
            .expect("user exists")
            .id,
        user.id
    );
    assert_eq!(
        suprnova::magnetar_integration::list_sessions(user.id.as_str())
            .await
            .expect("list active sessions")
            .len(),
        1
    );

    let rejected_connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect rejected SQLite");
    let error = init_magnetar(MagnetarConfig::from_sea_orm(rejected_connection.clone()))
        .await
        .expect_err("second engine installation must be rejected");
    assert!(error.to_string().contains("already installed"));
    let app_users = rejected_connection
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'app_users'",
        ))
        .await
        .expect("inspect rejected database");
    assert!(
        app_users.is_none(),
        "rejected initialization must not mutate schema"
    );
}
