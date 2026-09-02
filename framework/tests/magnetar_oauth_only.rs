#![cfg(feature = "magnetar-oauth")]

#[cfg(feature = "testing")]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "testing")]
use std::sync::Mutex;

#[cfg(feature = "testing")]
use base64::Engine as _;
#[cfg(feature = "testing")]
use magnetar::sessions::WebSessionBinding;
#[cfg(feature = "testing")]
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
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
impl MemorySessionStore {
    fn seed(&self, session: SessionData) {
        self.sessions
            .lock()
            .expect("session store")
            .insert(session.id.clone(), session);
    }

    fn corrupt_only_binding(&self) {
        let mut sessions = self.sessions.lock().expect("session store");
        let session = sessions
            .values_mut()
            .find(|session| session.magnetar_web_binding().is_some())
            .expect("completed OAuth session was persisted");
        session.set_magnetar_web_binding(WebSessionBinding {
            session_id: "missing-factor-session".to_owned(),
            token_digest: [0; 32],
        });
    }
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
    async fn send(&self, request: OAuthHttpRequest) -> MagnetarResult<OAuthHttpResponse> {
        let body = if request.url.ends_with("/token") {
            br#"{"access_token":"offline-token","token_type":"Bearer"}"#.to_vec()
        } else if request.url.ends_with("/user") {
            b"verified".to_vec()
        } else {
            return Err(MagnetarError::DependencyUnavailable {
                dependency: "offline OAuth transport".to_owned(),
                message: format!("unexpected fixture URL: {}", request.url),
            });
        };
        Ok(OAuthHttpResponse {
            status: 200,
            headers: Vec::new(),
            body,
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

    async fn resolve_identity(&self, response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        let ProviderResponse::UserInfo { body } = response else {
            return Err(OAuthProtocolError::MalformedProviderResponse {
                provider: "community",
                message: "fixture requires a userinfo response".to_owned(),
            });
        };
        if body != "verified" {
            return Err(OAuthProtocolError::MalformedProviderResponse {
                provider: "community",
                message: "unexpected userinfo fixture".to_owned(),
            });
        }
        Ok(ProviderIdentity {
            provider: "community".to_owned(),
            subject: "community-subject".to_owned(),
            email: Some("oauth-only@example.test".to_owned()),
            email_verified: true,
            display_name: Some("OAuth Only".to_owned()),
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

#[cfg(feature = "testing")]
async fn request_with_headers(
    cookie: Option<&str>,
    authorization: Option<&str>,
) -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let cookie = cookie
        .map(|value| format!("Cookie: {value}\r\n"))
        .unwrap_or_default();
    let authorization = authorization
        .map(|value| format!("Authorization: Bearer {value}\r\n"))
        .unwrap_or_default();
    let bytes = format!(
        "GET /factor HTTP/1.1\r\nHost: localhost\r\n{cookie}{authorization}Content-Length: 0\r\n\r\n"
    )
    .into_bytes();
    let (request_tx, request_rx) = oneshot::channel();
    let request_tx = Mutex::new(Some(request_tx));
    let (client_io, server_io) = tokio::io::duplex(bytes.len() + 4096);
    tokio::spawn(async move {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            if let Ok(mut sender) = request_tx.lock()
                && let Some(sender) = sender.take()
            {
                let _ = sender.send(suprnova::Request::new(request));
            }
            async {
                Ok::<_, Infallible>(hyper::Response::new(
                    http_body_util::Full::new(Bytes::new()),
                ))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .await;
    });
    let mut client = client_io;
    client
        .write_all(&bytes)
        .await
        .expect("write in-memory request");
    drop(client);
    request_rx.await.expect("capture in-memory request")
}

#[tokio::test]
async fn oauth_only_initialization_leaves_legacy_session_authority_active() {
    Crypt::init(EncryptionKey::generate());
    let database = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    #[cfg(feature = "testing")]
    {
        const RECOVERY_CODE: &str = "oauth-only-recovery";
        let now = suprnova::chrono::Utc::now();
        magnetar::default_schema::migrate(&database)
            .await
            .expect("migrate OAuth-only fixture");
        magnetar::default_schema::users::ActiveModel {
            id: Set(1),
            email: Set("oauth-only@example.test".to_owned()),
            email_verified_at: Set(Some(now)),
            auth_epoch: Set(0),
            ..Default::default()
        }
        .insert(&database)
        .await
        .expect("seed OAuth-only user");
        magnetar::default_schema::accounts::ActiveModel {
            id: Set(1),
            user_id: Set(1),
            provider: Set("community".to_owned()),
            provider_account_id: Set("community-subject".to_owned()),
            created_at: Set(Some(now)),
            updated_at: Set(Some(now)),
        }
        .insert(&database)
        .await
        .expect("seed linked OAuth account");
        let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP");
        let secret = Crypt::encrypt_string(suprnova::CryptPurpose::TwoFactorSecret, &secret)
            .expect("encrypt factor secret")
            .into_bytes();
        let recovery =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(RECOVERY_CODE.as_bytes());
        let recovery = Crypt::encrypt_string(suprnova::CryptPurpose::TwoFactorRecovery, &recovery)
            .expect("encrypt factor recovery code")
            .into_bytes();
        magnetar::default_schema::two_factor::ActiveModel {
            user_id: Set("1".to_owned()),
            secret: Set(secret),
            recovery_codes: Set(Some(recovery)),
            enrollment_auth_epoch: Set(0),
            enrollment_session_id: Set(None),
            enrollment_expires_at: Set(None),
            rotation_pending: Set(false),
            confirmed_at: Set(Some(now)),
            last_used_timestep: Set(None),
            created_at: Set(Some(now)),
            updated_at: Set(Some(now)),
        }
        .insert(&database)
        .await
        .expect("seed confirmed factor enrollment");
    }
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

    let config = MagnetarOAuthOnlyConfig::from_sea_orm(database, oauth);
    #[cfg(feature = "testing")]
    let config = config.apply_migrations(false);
    init_magnetar_oauth_only(config)
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
    assert_eq!(
        password_error.to_string(),
        "Internal server error: Magnetar password engine is not installed",
    );

    #[cfg(feature = "testing")]
    {
        const RECOVERY_CODE: &str = "oauth-only-recovery";
        let mut session_config = SessionConfig::default();
        session_config.cookie_secure = false;
        let cookie_name = session_config.cookie_name.clone();
        let sessions = Arc::new(MemorySessionStore::default());
        let middleware = SessionMiddleware::with_store(session_config, sessions.clone());

        let legacy_session_id = "l".repeat(40);
        let mut legacy_session = SessionData::new(
            legacy_session_id.clone(),
            "legacy-oauth-only-csrf".to_owned(),
        );
        legacy_session.user_id = Some("legacy-user".to_owned());
        sessions.seed(legacy_session);
        let legacy_cookie =
            suprnova::http::cookie::Cookie::encrypted(&cookie_name, &legacy_session_id)
                .expect("encrypt legacy framework session cookie");
        let legacy_cookie = format!("{cookie_name}={}", legacy_cookie.value());
        let legacy_authenticated: Next = Arc::new(|_| {
            Box::pin(async move {
                assert_eq!(
                    Auth::id().as_deref(),
                    Some("legacy-user"),
                    "OAuth-only mode must retain binding-less legacy sessions"
                );
                Ok(suprnova::HttpResponse::text("legacy authenticated"))
            })
        });
        let legacy_result = middleware
            .handle(
                request_with_headers(Some(&legacy_cookie), None).await,
                legacy_authenticated,
            )
            .await;
        if let Err(error) = legacy_result {
            panic!(
                "legacy OAuth-only session request returned status {}",
                error.status_code()
            );
        }

        let malformed_session_id = "m".repeat(40);
        let mut malformed_session = SessionData::new(
            malformed_session_id.clone(),
            "malformed-oauth-only-csrf".to_owned(),
        );
        malformed_session.user_id = Some("malformed-user".to_owned());
        malformed_session.put(
            "auth.magnetar_web_binding",
            serde_json::json!({ "session_id": 7, "token_digest": "not-a-digest" }),
        );
        sessions.seed(malformed_session);
        let malformed_cookie =
            suprnova::http::cookie::Cookie::encrypted(&cookie_name, &malformed_session_id)
                .expect("encrypt malformed framework session cookie");
        let malformed_cookie = format!("{cookie_name}={}", malformed_cookie.value());
        let rejects_malformed_binding: Next = Arc::new(|_| {
            Box::pin(async move {
                assert_eq!(
                    Auth::id(),
                    None,
                    "an explicit malformed binding must not use the OAuth-only legacy exception",
                );
                Ok(suprnova::HttpResponse::text("malformed binding rejected"))
            })
        });
        let malformed_result = middleware
            .handle(
                request_with_headers(Some(&malformed_cookie), None).await,
                rejects_malformed_binding,
            )
            .await;
        if let Err(error) = malformed_result {
            panic!(
                "malformed OAuth-only binding request returned status {}",
                error.status_code()
            );
        }

        let issued_session = Arc::new(Mutex::new(None::<(String, String)>));
        let issued_session_for_handler = issued_session.clone();
        let next: Next = Arc::new(move |_| {
            let issued_session = issued_session_for_handler.clone();
            Box::pin(async move {
                let kickoff = Auth::oauth("community")
                    .begin()
                    .await
                    .expect("begin real OAuth-only ceremony");
                let outcome = Auth::oauth("community")
                    .complete_outcome("code", &kickoff.state)
                    .await
                    .unwrap_or_else(|error| panic!("complete OAuth-only callback: {error}"));
                let suprnova::SignInOutcome::FactorRequired { challenge_selector } = outcome else {
                    panic!("confirmed enrollment must require factor completion");
                };
                let (user, issued) = Auth::factor()
                    .complete_challenge(&challenge_selector, RECOVERY_CODE)
                    .await
                    .unwrap_or_else(|error| panic!("complete OAuth-only factor: {error}"));
                assert_eq!(user.id.as_str(), "1");
                let token = issued
                    .token
                    .as_ref()
                    .expect("OAuth-only factor session has a bearer token")
                    .expose_secret()
                    .to_owned();
                let session_id = suprnova::session::session()
                    .expect("OAuth-only factor session scope")
                    .magnetar_web_binding()
                    .expect("OAuth-only factor binding")
                    .session_id;
                *issued_session.lock().expect("issued OAuth-only session") =
                    Some((session_id, token));
                Ok(suprnova::HttpResponse::text("completed"))
            })
        });
        let response = middleware
            .handle(suprnova::Request::for_test("GET", "/factor"), next)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "OAuth-only factor completion returned status {}",
                    error.status_code()
                )
            })
            .into_hyper();
        let cookie_prefix = format!("{cookie_name}=");
        let cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with(&cookie_prefix))
            .and_then(|value| value.split(';').next())
            .expect("factor completion emits a framework session cookie")
            .to_owned();

        let authenticated: Next = Arc::new(|_| {
            Box::pin(async {
                assert_eq!(Auth::id().as_deref(), Some("1"));
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let valid = middleware
            .handle(
                request_with_headers(Some(&cookie), None).await,
                authenticated,
            )
            .await;
        if let Err(error) = valid {
            panic!(
                "factor-owned binding request returned status {}",
                error.status_code()
            );
        }

        let (session_id, bearer_token) = issued_session
            .lock()
            .expect("issued OAuth-only session")
            .clone()
            .expect("factor completion captured its opaque session");
        assert!(
            suprnova::magnetar_integration::list_sessions("1")
                .await
                .expect("list OAuth-only sessions")
                .iter()
                .any(|session| session.session_id == session_id),
            "the factor-issued OAuth-only session must be listed",
        );
        let observed_bearer = Arc::new(Mutex::new(None));
        let observed_bearer_for_handler = observed_bearer.clone();
        let bearer_next: Next = Arc::new(move |_| {
            let observed = observed_bearer_for_handler.clone();
            Box::pin(async move {
                *observed.lock().expect("OAuth-only bearer observation") = Auth::id();
                Ok(suprnova::HttpResponse::text("bearer authenticated"))
            })
        });
        let bearer_result = suprnova::auth::request_state::request_state_scope_for_test(async {
            suprnova::magnetar_integration::middleware::BearerTokenMiddleware
                .handle(
                    request_with_headers(None, Some(&bearer_token)).await,
                    bearer_next,
                )
                .await
        })
        .await;
        if let Err(error) = bearer_result {
            panic!(
                "OAuth-only bearer request returned status {}",
                error.status_code()
            );
        }
        assert_eq!(
            *observed_bearer
                .lock()
                .expect("OAuth-only bearer observation"),
            Some("1".to_owned()),
            "OAuth-only bearer tokens must use the installed factor/session authority",
        );
        sessions.corrupt_only_binding();
        let rejects_corrupt_binding: Next = Arc::new(|_| {
            Box::pin(async {
                assert_eq!(
                    Auth::id(),
                    None,
                    "OAuth-only middleware must validate through its installed factor/session authority"
                );
                Ok(suprnova::HttpResponse::text("anonymous"))
            })
        });
        let invalid = middleware
            .handle(
                request_with_headers(Some(&cookie), None).await,
                rejects_corrupt_binding,
            )
            .await;
        if let Err(error) = invalid {
            panic!(
                "invalid factor-owned binding request returned status {}",
                error.status_code()
            );
        }
        assert!(
            suprnova::magnetar_integration::revoke_session(&session_id)
                .await
                .expect("revoke OAuth-only session")
        );
        assert!(
            suprnova::magnetar_integration::list_sessions("1")
                .await
                .expect("list OAuth-only sessions after revoke")
                .is_empty()
        );
        assert_eq!(
            suprnova::magnetar_integration::revoke_all_sessions("1")
                .await
                .expect("revoke all OAuth-only sessions after single revoke"),
            0,
        );
    }
}
