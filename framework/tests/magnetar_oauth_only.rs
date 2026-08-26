#![cfg(feature = "magnetar-oauth")]

use std::sync::Arc;

use suprnova::{
    AbuseLimiter, AbusePolicy, Auth, AuthorizationRequestShape, AutoLinkPolicy,
    ClientAuthentication, ClientAuthenticationMaterial, Crypt, EncryptionKey, InvalidGrantMeaning,
    MagnetarError, MagnetarOAuthHostConfig, MagnetarOAuthOnlyConfig, MagnetarOAuthProviderConfig,
    MagnetarResult, OAuthAuthorizationConfig, OAuthHttpRequest, OAuthHttpResponse,
    OAuthHttpTransport, OAuthProtocolError, OAuthProvider, OAuthResult, Permit, ProviderIdentity,
    ProviderResponse, RefreshPolicy, RevocationRequest, RevocationTransport, TokenHint,
    TokenRequestShape, init_magnetar_oauth_only,
};

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
}
