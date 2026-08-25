use std::sync::Arc;

use suprnova::{
    AbuseLimiter, AbusePolicy, Auth, AuthorizationRequestShape, AutoLinkPolicy,
    ClientAuthentication, ClientAuthenticationMaterial, Crypt, EncryptionKey, EndpointOverrides,
    GoogleOAuthProvider, GoogleProviderConfig, InvalidGrantMeaning, MagnetarConfig, MagnetarError,
    MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig, MagnetarResult, OAuthAuthorizationConfig,
    OAuthHttpRequest, OAuthHttpResponse, OAuthHttpTransport, OAuthProtocolError, OAuthProvider,
    OAuthResult, Permit, ProviderIdentity, ProviderResponse, RefreshPolicy, RevocationRequest,
    RevocationTransport, SecretString, TokenHint, TokenRequestShape, init_magnetar,
};

struct OfflineTransport;

#[suprnova::async_trait]
impl OAuthHttpTransport for OfflineTransport {
    async fn send(&self, _request: OAuthHttpRequest) -> MagnetarResult<OAuthHttpResponse> {
        Err(MagnetarError::DependencyUnavailable {
            dependency: "offline OAuth transport".to_owned(),
            message: "the registration e2e performs no token exchange".to_owned(),
        })
    }
}

#[suprnova::async_trait]
impl RevocationTransport for OfflineTransport {
    async fn send(&self, _request: RevocationRequest) -> OAuthResult<()> {
        Err(OAuthProtocolError::UpstreamUnavailable {
            provider: "google",
            message: "the registration e2e performs no revocation".to_owned(),
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
            message: "the registration e2e performs no identity exchange".to_owned(),
        })
    }

    async fn revoke(&self, _token: &str, _hint: TokenHint) -> OAuthResult<()> {
        Err(OAuthProtocolError::UpstreamUnavailable {
            provider: "community",
            message: "the registration e2e performs no revocation".to_owned(),
            retry_after_seconds: None,
        })
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
        None
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
async fn app_can_register_google_through_default_magnetar_config() {
    Crypt::init(EncryptionKey::generate());
    let database = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    let transport = Arc::new(OfflineTransport);
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "dogfood-google-client".to_owned(),
            client_secret: SecretString::from("dogfood-google-secret".to_owned()),
            redirect_uri: Some("https://app.test/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        transport.clone(),
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![
            MagnetarOAuthProviderConfig {
                provider,
                redirect_uri: "https://app.test/auth/google/callback".to_owned(),
                scopes: vec!["openid".to_owned(), "email".to_owned()],
            },
            MagnetarOAuthProviderConfig {
                provider: Arc::new(CommunityProvider),
                redirect_uri: "https://app.test/auth/community/callback".to_owned(),
                scopes: vec!["profile".to_owned()],
            },
        ],
        transport,
        Arc::new(AllowAll),
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("compose dogfood OAuth config");

    init_magnetar(MagnetarConfig::from_sea_orm(database).oauth(oauth))
        .await
        .expect("publish default engine with OAuth");

    let session = suprnova::session::new_session_slot_for_test();
    let kickoff = suprnova::session::session_scope_for_test(session, async {
        Auth::oauth("google").begin().await
    })
    .await
    .expect("start configured Google flow through the framework facade");

    assert!(kickoff.authorization_url.contains("dogfood-google-client"));
    assert!(!kickoff.state.is_empty());

    let community_session = suprnova::session::new_session_slot_for_test();
    let community = suprnova::session::session_scope_for_test(community_session, async {
        Auth::oauth("community").begin().await
    })
    .await
    .expect("start downstream custom provider through the framework facade");
    assert!(community.authorization_url.contains("community-client"));
}
