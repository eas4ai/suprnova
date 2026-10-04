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

/// A user store whose first two email lookups miss, as two concurrent
/// registrations see before either inserts. Later lookups and every write
/// reach the real default-schema store.
struct RacingLookups {
    store: std::sync::Arc<
        magnetar::storage::SeaOrmStorage<magnetar::default_schema::DefaultAuthSchema>,
    >,
    misses_left: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl magnetar::storage::UserStore for RacingLookups {
    async fn find_by_email(
        &self,
        email: &str,
    ) -> magnetar::Result<Option<magnetar::storage::UserRecord>> {
        let missed = self
            .misses_left
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |left| left.checked_sub(1),
            )
            .is_ok();
        if missed {
            return Ok(None);
        }
        self.store.find_by_email(email).await
    }

    async fn find_by_id(
        &self,
        user_id: &str,
    ) -> magnetar::Result<Option<magnetar::storage::UserRecord>> {
        self.store.find_by_id(user_id).await
    }

    async fn create_user(
        &self,
        input: magnetar::storage::NewUser,
    ) -> magnetar::Result<magnetar::storage::UserRecord> {
        self.store.create_user(input).await
    }

    async fn set_password_hash(
        &self,
        actor: &magnetar::storage::CredentialActor,
        password_hash: &str,
    ) -> magnetar::Result<()> {
        self.store.set_password_hash(actor, password_hash).await
    }

    async fn mark_email_verified(
        &self,
        user_id: &str,
        at: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> magnetar::Result<()> {
        self.store.mark_email_verified(user_id, at).await
    }

    async fn lock_if_unlocked_by_email(
        &self,
        email: &str,
        locked_at: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
        window_start: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> magnetar::Result<bool> {
        self.store
            .lock_if_unlocked_by_email(email, locked_at, window_start)
            .await
    }

    async fn set_locked_at_by_email(
        &self,
        email: &str,
        locked_at: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    ) -> magnetar::Result<()> {
        self.store.set_locked_at_by_email(email, locked_at).await
    }
}

/// IDENTITY-037: the fresh default schema holds one account per email.
/// Two registrations for one new address both miss the lookup, the way two
/// concurrent requests do. The second must not create a second account: it
/// reports the address as already registered, and one row remains.
#[tokio::test]
async fn racing_registrations_for_one_email_create_one_account() {
    use magnetar::password::{PasswordHashConfig, PasswordVerifier, StandardPasswordHashDriver};
    use magnetar::plugins::password::{
        PasswordAuthProvider, PasswordAuthService, RegisterInput, RegistrationOutcome,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create the default auth tables");
    let storage = std::sync::Arc::new(magnetar::storage::SeaOrmStorage::<
        magnetar::default_schema::DefaultAuthSchema,
    >::new(database.clone()));
    let verifier = PasswordVerifier::new(
        std::sync::Arc::new(StandardPasswordHashDriver),
        PasswordHashConfig {
            bcrypt_cost: 4,
            argon2_memory_kib: 8,
            argon2_iterations: 1,
            argon2_parallelism: 1,
        },
    )
    .expect("fast test verifier");
    let service = PasswordAuthService::new(
        std::sync::Arc::new(RacingLookups {
            store: storage.clone(),
            misses_left: std::sync::atomic::AtomicUsize::new(2),
        }),
        storage,
        std::sync::Arc::new(verifier),
    );
    let register = || RegisterInput {
        email: "racing@example.test".to_owned(),
        password: secrecy::SecretString::from("correct horse battery staple".to_owned()),
    };

    let first = service
        .register(register())
        .await
        .expect("first registration");
    let RegistrationOutcome::Created { user_id, .. } = first else {
        panic!("the first registration creates the account");
    };
    let second = service
        .register(register())
        .await
        .expect("a racing registration answers like any registration of a known address");
    assert!(
        matches!(&second, RegistrationOutcome::Existing { user_id: existing } if *existing == user_id),
        "the racing registration must report the existing account, not create a second one"
    );
    let accounts = magnetar::default_schema::users::Entity::find()
        .filter(magnetar::default_schema::users::Column::Email.eq("racing@example.test"))
        .all(&database)
        .await
        .expect("read accounts");
    assert_eq!(accounts.len(), 1, "one email, one account");
}
