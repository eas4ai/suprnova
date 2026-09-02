#![cfg(feature = "magnetar-oauth")]

#[cfg(feature = "testing")]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "testing")]
use std::sync::Mutex;

#[cfg(feature = "testing")]
use suprnova::FrameworkError;
#[cfg(feature = "testing")]
use suprnova::middleware::{Middleware, Next};
#[cfg(feature = "testing")]
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    AbuseLimiter, AbusePolicy, Auth, AuthorizationRequestShape, AutoLinkPolicy,
    ClientAuthentication, ClientAuthenticationMaterial, Crypt, EncryptionKey, InvalidGrantMeaning,
    MagnetarError, MagnetarOAuthHostConfig, MagnetarOAuthOnlyConfig, MagnetarOAuthProviderConfig,
    MagnetarResult, OAuthAuthorizationConfig, OAuthHttpRequest, OAuthHttpResponse,
    OAuthHttpTransport, OAuthProtocolError, OAuthProvider, OAuthResult, Permit, ProviderIdentity,
    ProviderResponse, RefreshPolicy, RevocationRequest, RevocationTransport, TokenHint,
    TokenRequestShape, init_magnetar_oauth_only,
};

#[cfg(feature = "testing")]
#[derive(Default)]
struct MemorySessionStore {
    sessions: Mutex<HashMap<String, SessionData>>,
}

#[cfg(feature = "testing")]
#[suprnova::async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self
            .sessions
            .lock()
            .expect("session store")
            .get(id)
            .cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.sessions
            .lock()
            .expect("session store")
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().expect("session store").remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.sessions.lock().expect("session store");
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok(u64::try_from(before - sessions.len()).unwrap_or(u64::MAX))
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

struct OfflineTransport;

#[suprnova::async_trait]
impl OAuthHttpTransport for OfflineTransport {
    async fn send(&self, _request: OAuthHttpRequest) -> MagnetarResult<OAuthHttpResponse> {
        Err(MagnetarError::DependencyUnavailable {
            dependency: "offline OAuth transport".to_owned(),
            message: "the initialization proof performs no callback".to_owned(),
        })
    }
}

#[suprnova::async_trait]
impl RevocationTransport for OfflineTransport {
    async fn send(&self, _request: RevocationRequest) -> OAuthResult<()> {
        Err(OAuthProtocolError::UpstreamUnavailable {
            provider: "community",
            message: "the initialization proof performs no revocation".to_owned(),
            retry_after_seconds: None,
        })
    }
}

struct AllowAll;

#[suprnova::async_trait]
impl AbuseLimiter for AllowAll {
    async fn acquire(&self, _key: &str, _policy: AbusePolicy) -> MagnetarResult<Permit> {
        Ok(Permit::Allowed { retry_after: None })
    }
}

struct CommunityProvider;

#[suprnova::async_trait]
impl OAuthProvider for CommunityProvider {
    fn name(&self) -> &'static str {
        "community"
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        AuthorizationRequestShape::default()
    }

    fn token_shape(&self) -> TokenRequestShape {
        TokenRequestShape::default()
    }

    async fn resolve_identity(&self, _response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        Err(OAuthProtocolError::ProviderConfiguration {
            provider: "community",
            message: "identity exchange is outside this test".to_owned(),
        })
    }

    async fn revoke(&self, _token: &str, _hint: TokenHint) -> OAuthResult<()> {
        Ok(())
    }

    fn client_id(&self) -> &str {
        "community-client"
    }

    fn token_endpoint(&self) -> String {
        "https://community.test/token".to_owned()
    }

    fn authorization_endpoint(&self) -> String {
        "https://community.test/authorize".to_owned()
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        Some("https://community.test/user".to_owned())
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: false,
            token_client_authentication: ClientAuthentication::RequestBody,
            extra_authorization_params: Vec::new(),
            required_scopes: Vec::new(),
            requires_reconsent_for_reissue: false,
            invalid_grant_meaning: InvalidGrantMeaning::OrdinaryRevocation,
        }
    }

    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial> {
        Ok(ClientAuthenticationMaterial::default())
    }
}

#[tokio::test]
async fn oauth_only_initialization_leaves_legacy_session_authority_active() {
    Crypt::init(EncryptionKey::generate());
    let database = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    let transport = Arc::new(OfflineTransport);
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider: Arc::new(CommunityProvider),
            redirect_uri: "https://app.test/auth/community/callback".to_owned(),
            scopes: vec!["profile".to_owned()],
        }],
        transport,
        Arc::new(AllowAll),
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("OAuth host configuration");

    init_magnetar_oauth_only(MagnetarOAuthOnlyConfig::from_sea_orm(database, oauth))
        .await
        .expect("install only OAuth");

    let session = suprnova::session::new_session_slot_for_test();
    let kickoff = suprnova::session::session_scope_for_test(session, async {
        Auth::oauth("community").begin().await
    })
    .await
    .expect("OAuth provider is installed");
    assert!(kickoff.authorization_url.contains("community-client"));

    let password_error = Auth::password()
        .authenticate("user@example.com", "password", None, None)
        .await
        .expect_err("OAuth-only initialization must not install password authority");
    assert!(
        password_error
            .to_string()
            .contains("engine is not installed")
    );

    #[cfg(feature = "testing")]
    {
        let mut session_config = SessionConfig::default();
        session_config.cookie_secure = false;
        let middleware =
            SessionMiddleware::with_store(session_config, Arc::new(MemorySessionStore::default()));
        let next: Next = Arc::new(|_| {
            Box::pin(async {
                let error = Auth::factor()
                    .complete_challenge("missing-selector", "000000")
                    .await
                    .expect_err("unknown selector must be rejected");
                assert!(
                    matches!(
                        error,
                        FrameworkError::Domain {
                            status_code: 401,
                            ..
                        }
                    ),
                    "OAuth-only boot must install its shared factor owner: {error}"
                );
                Ok(suprnova::HttpResponse::text("checked"))
            })
        });
        let result = middleware
            .handle(suprnova::Request::for_test("GET", "/factor"), next)
            .await;
        if let Err(error) = result {
            panic!(
                "OAuth-only factor check returned status {}",
                error.status_code()
            );
        }
    }
}
