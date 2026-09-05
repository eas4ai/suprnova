use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use suprnova::{
    Auth, Crypt, EncryptionKey, MagnetarConfig, RateLimiterDriver, SlidingWindowConfig,
    init_magnetar,
};

#[cfg(feature = "magnetar-oauth")]
use suprnova::{
    AbuseLimiter as MagnetarAbuseLimiter, AbusePolicy, AutoLinkPolicy, EndpointOverrides,
    GoogleOAuthProvider, GoogleProviderConfig, MagnetarError, MagnetarOAuthHostConfig,
    MagnetarOAuthProviderConfig, MagnetarResult, OAuthAuthorizationConfig, OAuthHttpRequest,
    OAuthHttpResponse, OAuthHttpTransport, Permit, RevocationRequest, RevocationTransport,
    SecretString,
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

#[cfg(feature = "magnetar-oauth")]
struct OfflineOAuthTransport;

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl OAuthHttpTransport for OfflineOAuthTransport {
    async fn send(&self, _request: OAuthHttpRequest) -> MagnetarResult<OAuthHttpResponse> {
        Err(MagnetarError::DependencyUnavailable {
            dependency: "offline OAuth transport".to_owned(),
            message: "the default-engine registration test performs no exchange".to_owned(),
        })
    }
}

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl RevocationTransport for OfflineOAuthTransport {
    async fn send(&self, _request: RevocationRequest) -> suprnova::OAuthResult<()> {
        Err(suprnova::OAuthProtocolError::UpstreamUnavailable {
            provider: "google",
            message: "the default-engine registration test performs no revocation".to_owned(),
            retry_after_seconds: None,
        })
    }
}

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl MagnetarAbuseLimiter for AllowingLimiter {
    async fn acquire(&self, _key: &str, _policy: AbusePolicy) -> MagnetarResult<Permit> {
        Ok(Permit::Allowed { retry_after: None })
    }
}

#[cfg(feature = "magnetar-oauth")]
fn google_config(
    connection: sea_orm::DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn MagnetarAbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.test/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.test/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("compose OAuth host config");
    MagnetarConfig::from_sea_orm(connection).oauth(oauth)
}

#[tokio::test]
async fn default_installer_runs_password_session_and_lockout_flows() {
    Crypt::init(EncryptionKey::generate());
    suprnova::App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    #[cfg(feature = "magnetar-oauth")]
    let config = {
        let transport = Arc::new(OfflineOAuthTransport);
        google_config(
            connection,
            transport.clone(),
            transport,
            Arc::new(AllowingLimiter),
        )
    };
    #[cfg(not(feature = "magnetar-oauth"))]
    let config = MagnetarConfig::from_sea_orm(connection);
    init_magnetar(config)
        .await
        .expect("install default Magnetar engine");

    #[cfg(feature = "magnetar-oauth")]
    {
        let session = suprnova::session::new_session_slot_for_test();
        let kickoff = suprnova::session::session_scope_for_test(session, async {
            Auth::oauth("google").begin().await
        })
        .await
        .expect("configured Google provider is reachable through Auth::oauth");
        assert!(kickoff.authorization_url.contains("google-client"));
        assert!(!kickoff.state.is_empty());
    }

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
