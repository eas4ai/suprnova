#![cfg(feature = "testing")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use magnetar::sessions::{
    HostSessionApproval, OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata,
    SessionQueries, SessionSummary, StoredSession, WebSessionBinding,
};
use secrecy::SecretString;
#[cfg(feature = "magnetar-oauth")]
use suprnova::magnetar_integration::engine::{
    HostOAuthError, MagnetarOAuthAuthEngine, MagnetarOAuthBegin, MagnetarOAuthCallback,
    MagnetarOAuthCompletion, MagnetarOAuthKickoff,
};
use suprnova::magnetar_integration::engine::{
    HostPasswordResetIssued, HostSignInDecision, MagnetarIssuedSession, MagnetarPasskeyAuthEngine,
    MagnetarPasswordAuthEngine, MagnetarRememberSignIn, MagnetarRememberSignInAttempt,
};
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{FrameworkError, LockoutStatus, Session, SessionToken, SignInOutcome, User, UserId};

const USER_ID: &str = "factor-user";
const FACTOR_CODE: &str = "654321";

#[derive(Clone, Copy, Debug)]
enum Origin {
    Password,
    MagicLink,
    Passkey,
    #[cfg(feature = "magnetar-oauth")]
    OAuth,
}

impl Origin {
    fn name(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::MagicLink => "magic-link",
            Self::Passkey => "passkey",
            #[cfg(feature = "magnetar-oauth")]
            Self::OAuth => "oauth",
        }
    }
}

#[derive(Default)]
struct EngineState {
    pending: HashMap<String, User>,
    sessions: HashMap<String, StoredSession>,
}

#[derive(Clone, Default)]
struct FactorEngine {
    state: Arc<Mutex<EngineState>>,
}

impl FactorEngine {
    fn user() -> User {
        User::builder()
            .id(UserId::new(USER_ID))
            .email("factor@example.test".to_owned())
            .build()
            .expect("fixture user")
    }

    fn require_factor(&self, origin: &str) -> HostSignInDecision {
        let selector = format!("factor-{origin}-{}", uuid::Uuid::new_v4().simple());
        self.state
            .lock()
            .expect("factor engine state")
            .pending
            .insert(selector.clone(), Self::user());
        HostSignInDecision::FactorRequired {
            challenge_selector: selector,
        }
    }

    fn unsupported<T>() -> magnetar::Result<T> {
        Err(magnetar::Error::DependencyUnavailable {
            dependency: "unused test operation".to_owned(),
            message: "operation is outside this fixture".to_owned(),
        })
    }
}

#[async_trait]
impl MagnetarPasswordAuthEngine for FactorEngine {
    async fn password_sign_in(
        &self,
        _: magnetar::plugins::password::PasswordAttempt,
    ) -> magnetar::Result<(User, HostSignInDecision)> {
        Ok((Self::user(), self.require_factor("password")))
    }

    async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> magnetar::Result<MagnetarIssuedSession> {
        if code != FACTOR_CODE {
            return Err(magnetar::Error::InvalidInput {
                field: "code".to_owned(),
                message: "invalid factor code".to_owned(),
            });
        }
        let user = self
            .state
            .lock()
            .expect("factor engine state")
            .pending
            .remove(selector)
            .ok_or_else(|| magnetar::Error::NotFound {
                resource: "factor challenge".to_owned(),
                identifier: "invalid or expired selector".to_owned(),
            })?;
        let session_id = format!("opaque-{}", uuid::Uuid::new_v4().simple());
        let token_digest = [9; 32];
        let session = Session::builder()
            .token(SessionToken::new_random())
            .user_id(user.id)
            .build()
            .expect("fixture opaque session");
        self.state
            .lock()
            .expect("factor engine state")
            .sessions
            .insert(
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

    async fn issue_password_reset(
        &self,
        _: &str,
    ) -> magnetar::Result<Option<HostPasswordResetIssued>> {
        Self::unsupported()
    }

    async fn check_password_reset(&self, _: SecretString) -> magnetar::Result<bool> {
        Self::unsupported()
    }

    async fn complete_password_reset(
        &self,
        _: SecretString,
        _: SecretString,
    ) -> magnetar::Result<magnetar::plugins::password_management::PasswordResetFlowOutcome> {
        Self::unsupported()
    }

    async fn password_register(
        &self,
        _: magnetar::plugins::password::RegisterInput,
    ) -> magnetar::Result<User> {
        Self::unsupported()
    }

    async fn bearer_user_id(&self, _: &str) -> magnetar::Result<Option<String>> {
        Self::unsupported()
    }

    async fn issue_remember(
        &self,
        _: &str,
        _: suprnova::chrono::Duration,
    ) -> magnetar::Result<magnetar::sessions::RememberCredential> {
        Self::unsupported()
    }

    async fn remember_sign_in(
        &self,
        _: magnetar::sessions::RememberCredential,
        _: SessionMetadata,
        _: suprnova::chrono::Duration,
    ) -> magnetar::Result<MagnetarRememberSignIn> {
        Self::unsupported()
    }

    async fn remember_sign_in_attempt(
        &self,
        _: magnetar::sessions::RememberCredential,
        _: SessionMetadata,
        _: suprnova::chrono::Duration,
    ) -> magnetar::Result<MagnetarRememberSignInAttempt> {
        Self::unsupported()
    }

    async fn resolve_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
        OpaqueSessionProvider::new(Arc::new(self.clone()), OpaqueConfig::default())
            .resolve_web_binding(binding, &HostSessionApproval::authenticated())
            .await
    }

    async fn revoke_remember(&self, _: &str) -> magnetar::Result<u64> {
        Self::unsupported()
    }

    async fn user_by_id(&self, user_id: &str) -> magnetar::Result<Option<User>> {
        Ok((user_id == USER_ID).then(Self::user))
    }

    async fn revoke_session(&self, _: &str) -> magnetar::Result<bool> {
        Self::unsupported()
    }

    async fn revoke_all_sessions(&self, _: &str) -> magnetar::Result<u64> {
        Self::unsupported()
    }

    async fn list_sessions(&self, _: &str) -> magnetar::Result<Vec<SessionSummary>> {
        Self::unsupported()
    }

    async fn record_failed_attempt(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> magnetar::Result<LockoutStatus> {
        Self::unsupported()
    }

    async fn lockout_status(&self, _: &str) -> magnetar::Result<LockoutStatus> {
        Self::unsupported()
    }

    async fn reset_attempts(&self, _: &str) -> magnetar::Result<()> {
        Self::unsupported()
    }

    async fn unlock_account(&self, _: &str) -> magnetar::Result<bool> {
        Self::unsupported()
    }

    async fn magic_link_send(&self, _: &str) -> magnetar::Result<String> {
        Ok("magic-token".to_owned())
    }

    async fn magic_link_consume(
        &self,
        _: &str,
        _: SessionMetadata,
    ) -> magnetar::Result<HostSignInDecision> {
        Ok(self.require_factor("magic-link"))
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
            .expect("factor engine state")
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
            .expect("factor engine state")
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
            .expect("factor engine state")
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
        let mut state = self.state.lock().expect("factor engine state");
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
        let mut state = self.state.lock().expect("factor engine state");
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
            .expect("factor engine state")
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

#[async_trait]
impl MagnetarPasskeyAuthEngine for FactorEngine {
    async fn passkey_begin_registration(
        &self,
        _: magnetar::passkey::RegistrationIntent,
    ) -> magnetar::Result<magnetar::passkey::BegunRegistration> {
        Self::unsupported()
    }

    async fn passkey_finish_registration(
        &self,
        _: &str,
        _: &str,
        _: &webauthn_rs::prelude::RegisterPublicKeyCredential,
    ) -> magnetar::Result<User> {
        Self::unsupported()
    }

    async fn passkey_begin_authentication(
        &self,
        _: &str,
    ) -> magnetar::Result<magnetar::passkey::BegunAuthentication> {
        Self::unsupported()
    }

    async fn passkey_finish_authentication(
        &self,
        _: &str,
        _: &str,
        _: &webauthn_rs::prelude::PublicKeyCredential,
        _: SessionMetadata,
    ) -> magnetar::Result<HostSignInDecision> {
        Ok(self.require_factor("passkey"))
    }

    async fn passkey_user_by_id(&self, _: &str) -> magnetar::Result<User> {
        Ok(Self::user())
    }
}

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl MagnetarOAuthAuthEngine for FactorEngine {
    fn oauth_supports_provider(&self, provider: &str) -> bool {
        provider == "fixture"
    }

    async fn oauth_begin(
        &self,
        _: MagnetarOAuthBegin,
    ) -> Result<MagnetarOAuthKickoff, HostOAuthError> {
        Ok(MagnetarOAuthKickoff {
            authorization_url: "https://provider.example/authorize".to_owned(),
            state: "oauth-state".to_owned(),
        })
    }

    async fn oauth_complete(
        &self,
        _: MagnetarOAuthCallback,
    ) -> Result<MagnetarOAuthCompletion, HostOAuthError> {
        let HostSignInDecision::FactorRequired { challenge_selector } =
            self.require_factor("oauth")
        else {
            unreachable!("fixture always requires a factor")
        };
        Ok(MagnetarOAuthCompletion::FactorRequired { challenge_selector })
    }

    async fn oauth_verify_identity(
        &self,
        _: MagnetarOAuthCallback,
    ) -> Result<magnetar::oauth::identity::VerifiedProviderIdentity, HostOAuthError> {
        Err(HostOAuthError::Auth(
            magnetar::Error::DependencyUnavailable {
                dependency: "unused test operation".to_owned(),
                message: "operation is outside this fixture".to_owned(),
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

struct AllowingLimiter;

#[async_trait]
impl suprnova::RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(
        &self,
        _: &str,
        _: &suprnova::SlidingWindowConfig,
    ) -> Result<bool, FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _: &str,
        _: &suprnova::SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, FrameworkError> {
        Ok(None)
    }
}

fn factor_selector(outcome: SignInOutcome) -> String {
    match outcome {
        SignInOutcome::FactorRequired { challenge_selector } => challenge_selector,
        SignInOutcome::Authenticated { .. } => panic!("fixture must require a factor"),
        _ => panic!("unexpected sign-in outcome"),
    }
}

fn dummy_passkey() -> webauthn_rs::prelude::PublicKeyCredential {
    serde_json::from_value(serde_json::json!({
        "id": "Y3JlZA",
        "rawId": "Y3JlZA",
        "response": {
            "authenticatorData": "",
            "clientDataJSON": "",
            "signature": "",
            "userHandle": null
        },
        "type": "public-key"
    }))
    .expect("syntactically valid browser credential")
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
        format!("GET /factor HTTP/1.1\r\nHost: localhost\r\n{cookie}Content-Length: 0\r\n\r\n")
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

async fn outcome_for(origin: Origin) -> Result<SignInOutcome, FrameworkError> {
    match origin {
        Origin::Password => {
            suprnova::Auth::password()
                .authenticate_outcome("factor@example.test", "password", None, None)
                .await
        }
        Origin::MagicLink => {
            let token = suprnova::Auth::magic_link()
                .send("factor@example.test", "https://app.example/magic")
                .await?;
            suprnova::Auth::magic_link().consume_outcome(&token).await
        }
        Origin::Passkey => {
            suprnova::session::session_mut(|session| {
                session.put("passkey_auth", "passkey-ceremony")
            });
            suprnova::Auth::passkey()
                .finish_authentication_outcome("factor@example.test", dummy_passkey())
                .await
        }
        #[cfg(feature = "magnetar-oauth")]
        Origin::OAuth => {
            suprnova::Auth::oauth("fixture")
                .complete_outcome("code", "state")
                .await
        }
    }
}

#[tokio::test]
async fn every_sign_in_origin_completes_through_the_installed_factor_facade() {
    suprnova::testing::install_test_encryption_key();
    let engine = Arc::new(FactorEngine::default());
    suprnova::magnetar_integration::install_magnetar_engines(engine.clone(), engine.clone())
        .expect("install one shared password, factor, and passkey engine");
    #[cfg(feature = "magnetar-oauth")]
    suprnova::magnetar_integration::install_magnetar_oauth_engine(engine)
        .expect("install the same engine for OAuth");

    let origins = [
        Origin::Password,
        Origin::MagicLink,
        Origin::Passkey,
        #[cfg(feature = "magnetar-oauth")]
        Origin::OAuth,
    ];
    for origin in origins {
        let store = Arc::new(MemoryStore::default());
        let mut config = SessionConfig::default();
        config.cookie_name = format!("factor_{}", origin.name().replace('-', "_"));
        config.cookie_secure = false;
        let middleware = SessionMiddleware::with_store(config.clone(), store);
        let next: Next = Arc::new(move |_| {
            Box::pin(async move {
                let before = suprnova::session::session().expect("session scope before completion");
                let selector = factor_selector(outcome_for(origin).await?);
                let wrong = suprnova::Auth::factor()
                    .complete_challenge("wrong-selector", FACTOR_CODE)
                    .await;
                assert!(matches!(
                    wrong,
                    Err(FrameworkError::Domain {
                        status_code: 401,
                        ..
                    })
                ));
                let (user, issued) = suprnova::Auth::factor()
                    .complete_challenge(&selector, FACTOR_CODE)
                    .await?;
                assert_eq!(user.id.as_str(), USER_ID);
                assert!(issued.token.is_some());
                let after = suprnova::session::session().expect("session scope after completion");
                assert_ne!(
                    before.id,
                    after.id,
                    "{} must rotate session id",
                    origin.name()
                );
                assert_ne!(
                    before.csrf_token,
                    after.csrf_token,
                    "{} must rotate CSRF state",
                    origin.name()
                );
                let replay = suprnova::Auth::factor()
                    .complete_challenge(&selector, FACTOR_CODE)
                    .await;
                assert!(matches!(
                    replay,
                    Err(FrameworkError::Domain {
                        status_code: 401,
                        ..
                    })
                ));
                Ok(suprnova::HttpResponse::text("completed"))
            })
        });
        let response = suprnova::testing::TestContainer::scope(async {
            suprnova::testing::TestContainer::bind::<dyn suprnova::RateLimiterDriver>(Arc::new(
                AllowingLimiter,
            ));
            middleware.handle(request(None).await, next).await
        })
        .await;
        let response = match response {
            Ok(response) => response.into_hyper(),
            Err(error) => panic!(
                "factor-completion request returned status {}",
                error.status_code()
            ),
        };
        let session_prefix = format!("{}=", config.cookie_name);
        let cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with(&session_prefix))
            .and_then(|value| value.split(';').next())
            .expect("factor completion emits the rotated session cookie")
            .to_owned();

        let second_next: Next = Arc::new(move |_| {
            Box::pin(async move {
                assert_eq!(
                    suprnova::Auth::id().as_deref(),
                    Some(USER_ID),
                    "{} cookie must authenticate the next request",
                    origin.name()
                );
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let second = middleware
            .handle(request(Some(&cookie)).await, second_next)
            .await;
        if let Err(error) = second {
            panic!(
                "cookie-bearing request returned status {}",
                error.status_code()
            );
        }
    }
}
