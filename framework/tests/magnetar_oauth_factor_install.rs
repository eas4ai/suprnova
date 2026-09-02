#![cfg(feature = "magnetar-oauth")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use magnetar::sessions::{
    HostSessionApproval, OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata,
    SessionQueries, SessionSummary, StoredSession, WebSessionBinding,
};
use sha2::{Digest, Sha256};
use suprnova::magnetar_integration::engine::{
    HostOAuthError, MagnetarFactorAuthEngine, MagnetarIssuedSession, MagnetarOAuthAuthEngine,
    MagnetarOAuthBegin, MagnetarOAuthCallback, MagnetarOAuthCompletion, MagnetarOAuthKickoff,
};
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{Auth, Crypt, EncryptionKey, FrameworkError, Session, SessionToken, User, UserId};

const USER_ID: &str = "custom-oauth-user";
const SELECTOR: &str = "custom-oauth-factor";
const FACTOR_CODE: &str = "246810";

#[derive(Default)]
struct FactorState {
    challenge_pending: bool,
    sessions: HashMap<String, StoredSession>,
}

#[derive(Clone, Default)]
struct FactorEngine {
    state: Arc<Mutex<FactorState>>,
}

impl FactorEngine {
    fn user() -> User {
        User::builder()
            .id(UserId::new(USER_ID))
            .email("custom-oauth@example.test".to_owned())
            .build()
            .expect("custom OAuth fixture user")
    }

    fn require_factor(&self) {
        self.state.lock().expect("factor state").challenge_pending = true;
    }

    fn provider(&self) -> OpaqueSessionProvider<Self> {
        OpaqueSessionProvider::new(Arc::new(self.clone()), OpaqueConfig::default())
    }
}

#[async_trait]
impl MagnetarFactorAuthEngine for FactorEngine {
    async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> magnetar::Result<MagnetarIssuedSession> {
        let mut state = self.state.lock().expect("factor state");
        if selector != SELECTOR || code != FACTOR_CODE || !state.challenge_pending {
            return Err(magnetar::Error::InvalidInput {
                field: "factor".to_owned(),
                message: "invalid custom OAuth factor proof".to_owned(),
            });
        }
        state.challenge_pending = false;
        let session_id = format!("opaque-{}", uuid::Uuid::new_v4().simple());
        let token = SessionToken::new_random();
        let token_digest: [u8; 32] = Sha256::digest(token.expose_secret().as_bytes()).into();
        let session = Session::builder()
            .token(token)
            .user_id(UserId::new(USER_ID))
            .build()
            .expect("custom OAuth opaque session");
        state.sessions.insert(
            session_id.clone(),
            StoredSession {
                session_id: session_id.clone(),
                user_id: USER_ID.to_owned(),
                auth_epoch: 0,
                token_hash: token_digest,
                token_digest,
                expires_at: suprnova::chrono::Utc::now() + suprnova::chrono::Duration::hours(1),
                revoked_at: None,
                metadata: SessionMetadata::default(),
            },
        );
        Ok(MagnetarIssuedSession {
            session_id: session_id.clone(),
            web_binding: WebSessionBinding {
                session_id,
                token_digest,
            },
            session,
        })
    }

    async fn user_by_id(&self, user_id: &str) -> magnetar::Result<Option<User>> {
        Ok((user_id == USER_ID).then(Self::user))
    }

    async fn resolve_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
        self.provider()
            .resolve_web_binding(binding, &HostSessionApproval::authenticated())
            .await
    }

    async fn bearer_user_id(&self, token: &str) -> magnetar::Result<Option<String>> {
        match self.provider().verify_bearer(token).await {
            Ok(session) => Ok(Some(session.user_id().to_owned())),
            Err(magnetar::Error::NotFound { .. } | magnetar::Error::InvalidInput { .. }) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn revoke_session(&self, session_id: &str) -> magnetar::Result<bool> {
        self.provider().revoke_session(session_id).await
    }

    async fn revoke_all_sessions(&self, user_id: &str) -> magnetar::Result<u64> {
        self.provider().revoke_all_for_user(user_id).await
    }

    async fn list_sessions(&self, user_id: &str) -> magnetar::Result<Vec<SessionSummary>> {
        self.provider().list_for_user(user_id).await
    }
}

#[async_trait]
impl OpaqueSessionStore for FactorEngine {
    async fn insert_session_if_epoch_current(
        &self,
        session: StoredSession,
    ) -> magnetar::Result<()> {
        self.state
            .lock()
            .expect("factor state")
            .sessions
            .insert(session.session_id.clone(), session);
        Ok(())
    }

    async fn find_by_token_hash(
        &self,
        token_hash: [u8; 32],
    ) -> magnetar::Result<Option<StoredSession>> {
        Ok(self
            .state
            .lock()
            .expect("factor state")
            .sessions
            .values()
            .find(|session| session.token_hash == token_hash)
            .cloned())
    }

    async fn find_by_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> magnetar::Result<Option<StoredSession>> {
        Ok(self
            .state
            .lock()
            .expect("factor state")
            .sessions
            .get(&binding.session_id)
            .filter(|session| session.token_digest == binding.token_digest)
            .cloned())
    }

    async fn revoke_all_sessions(
        &self,
        user_id: &str,
        at: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> magnetar::Result<u64> {
        let mut state = self.state.lock().expect("factor state");
        let mut revoked = 0;
        for session in state.sessions.values_mut() {
            if session.user_id == user_id && session.revoked_at.is_none() {
                session.revoked_at = Some(at);
                revoked += 1;
            }
        }
        Ok(revoked)
    }

    async fn revoke_session(
        &self,
        session_id: &str,
        at: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> magnetar::Result<bool> {
        let mut state = self.state.lock().expect("factor state");
        let Some(session) = state.sessions.get_mut(session_id) else {
            return Ok(false);
        };
        if session.revoked_at.is_some() {
            return Ok(false);
        }
        session.revoked_at = Some(at);
        Ok(true)
    }

    async fn list_active_sessions(
        &self,
        user_id: &str,
        now: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> magnetar::Result<Vec<StoredSession>> {
        Ok(self
            .state
            .lock()
            .expect("factor state")
            .sessions
            .values()
            .filter(|session| {
                session.user_id == user_id
                    && session.revoked_at.is_none()
                    && session.expires_at > now
            })
            .cloned()
            .collect())
    }
}

struct OAuthEngine {
    factor: FactorEngine,
    completion_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl MagnetarOAuthAuthEngine for OAuthEngine {
    fn oauth_supports_provider(&self, provider: &str) -> bool {
        provider == "custom"
    }

    async fn oauth_begin(
        &self,
        _: MagnetarOAuthBegin,
    ) -> Result<MagnetarOAuthKickoff, HostOAuthError> {
        Ok(MagnetarOAuthKickoff {
            authorization_url: "https://custom.example/authorize".to_owned(),
            state: "custom-oauth-state".to_owned(),
        })
    }

    async fn oauth_complete(
        &self,
        _: MagnetarOAuthCallback,
    ) -> Result<MagnetarOAuthCompletion, HostOAuthError> {
        self.completion_calls.fetch_add(1, Ordering::SeqCst);
        self.factor.require_factor();
        Ok(MagnetarOAuthCompletion::FactorRequired {
            challenge_selector: SELECTOR.to_owned(),
        })
    }

    async fn oauth_verify_identity(
        &self,
        _: MagnetarOAuthCallback,
    ) -> Result<magnetar::oauth::VerifiedProviderIdentity, HostOAuthError> {
        Err(HostOAuthError::Auth(
            magnetar::Error::DependencyUnavailable {
                dependency: "unused custom OAuth operation".to_owned(),
                message: "identity verification is outside this fixture".to_owned(),
            },
        ))
    }
}

#[derive(Default)]
struct MemoryStore {
    sessions: Mutex<HashMap<String, SessionData>>,
}

#[async_trait]
impl SessionStore for MemoryStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self
            .sessions
            .lock()
            .expect("framework session store")
            .get(id)
            .cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.sessions
            .lock()
            .expect("framework session store")
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions
            .lock()
            .expect("framework session store")
            .remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.sessions.lock().expect("framework session store");
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok(u64::try_from(before - sessions.len()).unwrap_or(u64::MAX))
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

async fn request(cookie: Option<&str>) -> suprnova::Request {
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
    let bytes =
        format!("GET /oauth HTTP/1.1\r\nHost: localhost\r\n{cookie}Content-Length: 0\r\n\r\n")
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
        .expect("write in-memory OAuth request");
    drop(client);
    request_rx.await.expect("capture in-memory OAuth request")
}

#[tokio::test]
async fn atomic_custom_oauth_and_factor_install_completes_a_cookie_round_trip() {
    Crypt::init(EncryptionKey::generate());
    let factor = FactorEngine::default();
    let completion_calls = Arc::new(AtomicUsize::new(0));
    suprnova::install_magnetar_oauth_engine_with_factor(
        Arc::new(OAuthEngine {
            factor: factor.clone(),
            completion_calls: completion_calls.clone(),
        }),
        Arc::new(factor),
    )
    .expect("atomically install custom OAuth and factor/session authority");

    let outside_middleware = Auth::oauth("custom")
        .complete_outcome("code", "outside-middleware-state")
        .await;
    assert_eq!(
        completion_calls.load(Ordering::SeqCst),
        0,
        "scope rejection must happen before the OAuth callback is consumed",
    );
    let outside_middleware =
        outside_middleware.expect_err("OAuth completion requires active SessionMiddleware scopes");
    assert!(
        outside_middleware
            .to_string()
            .contains("requires active SessionMiddleware scopes"),
        "unexpected unscoped completion error: {outside_middleware}",
    );

    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let cookie_name = config.cookie_name.clone();
    let middleware = SessionMiddleware::with_store(config, Arc::new(MemoryStore::default()));
    let complete: Next = Arc::new(|_| {
        Box::pin(async move {
            let kickoff = Auth::oauth("custom").begin().await?;
            let outcome = Auth::oauth("custom")
                .complete_outcome("code", &kickoff.state)
                .await?;
            let suprnova::SignInOutcome::FactorRequired { challenge_selector } = outcome else {
                panic!("custom OAuth fixture must require its factor authority");
            };
            let (user, _) = Auth::factor()
                .complete_challenge(&challenge_selector, FACTOR_CODE)
                .await?;
            assert_eq!(user.id.as_str(), USER_ID);
            Ok(suprnova::HttpResponse::text("completed"))
        })
    });
    let response = middleware.handle(request(None).await, complete).await;
    let response = match response {
        Ok(response) => response.into_hyper(),
        Err(error) => panic!("custom OAuth completion returned {}", error.status_code()),
    };
    let prefix = format!("{cookie_name}=");
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&prefix))
        .and_then(|value| value.split(';').next())
        .expect("custom OAuth factor completion emits a cookie")
        .to_owned();

    let authenticated: Next = Arc::new(|_| {
        Box::pin(async move {
            assert_eq!(Auth::id().as_deref(), Some(USER_ID));
            Ok(suprnova::HttpResponse::text("authenticated"))
        })
    });
    let next_request = middleware
        .handle(request(Some(&cookie)).await, authenticated)
        .await;
    if let Err(error) = next_request {
        panic!(
            "custom OAuth cookie request returned {}",
            error.status_code()
        );
    }
}
