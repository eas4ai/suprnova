#![cfg(feature = "testing")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use magnetar::sessions::{
    HostSessionApproval, OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata,
    SessionQueries, SessionSummary, StoredSession, WebSessionBinding,
};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
#[cfg(feature = "magnetar-oauth")]
use suprnova::magnetar_integration::engine::{
    HostOAuthError, MagnetarOAuthAuthEngine, MagnetarOAuthBegin, MagnetarOAuthCallback,
    MagnetarOAuthCompletion, MagnetarOAuthKickoff,
};
use suprnova::magnetar_integration::engine::{
    HostPasswordResetIssued, HostSignInDecision, MagnetarFactorAuthEngine, MagnetarIssuedSession,
    MagnetarPasskeyAuthEngine, MagnetarPasswordAuthEngine, MagnetarRememberSignIn,
    MagnetarRememberSignInAttempt,
};
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{FrameworkError, LockoutStatus, Session, SessionToken, SignInOutcome, User, UserId};
use tokio::sync::Notify;

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

#[derive(Clone, Copy)]
enum StoreFailure {
    Read,
    Write,
    Destroy,
    Cookie,
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
    revocations: HashMap<String, usize>,
    last_issued_id: Option<String>,
    completion_calls: usize,
    direct_allowed: bool,
    passkey_lookup_fails: bool,
    block_passkey_lookup: bool,
    magic_lookup_missing: bool,
    remember_returns_malformed_replacement: bool,
    revoke_failures_remaining: usize,
    block_next_revoke: bool,
}

#[derive(Clone, Default)]
struct FactorEngine {
    state: Arc<Mutex<EngineState>>,
    revoke_started: Arc<Notify>,
    revoke_release: Arc<Notify>,
    passkey_lookup_started: Arc<Notify>,
    passkey_lookup_release: Arc<Notify>,
    revoke_attempts: Arc<AtomicUsize>,
    hold_revocations: Arc<AtomicBool>,
}

struct PasswordEngine {
    factors: Arc<FactorEngine>,
    binding_resolution_calls: Arc<AtomicUsize>,
}

struct PasskeyEngine {
    factors: Arc<FactorEngine>,
}

#[cfg(feature = "magnetar-oauth")]
struct OAuthEngine {
    factors: Arc<FactorEngine>,
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

    fn issue_session(&self, user: &User) -> MagnetarIssuedSession {
        let session_id = format!("opaque-{}", uuid::Uuid::new_v4().simple());
        let token = SessionToken::new_random();
        let token_digest: [u8; 32] = Sha256::digest(token.expose_secret().as_bytes()).into();
        let session = Session::builder()
            .token(token)
            .user_id(user.id.clone())
            .build()
            .expect("fixture opaque session");
        let mut state = self.state.lock().expect("factor engine state");
        state.last_issued_id = Some(session_id.clone());
        state.sessions.insert(
            session_id.clone(),
            StoredSession {
                session_id: session_id.clone(),
                user_id: user.id.to_string(),
                auth_epoch: 0,
                token_hash: token_digest,
                token_digest,
                expires_at: suprnova::chrono::Utc::now() + suprnova::chrono::Duration::hours(1),
                revoked_at: None,
                metadata: SessionMetadata::default(),
            },
        );
        MagnetarIssuedSession {
            session_id: session_id.clone(),
            web_binding: WebSessionBinding {
                session_id,
                token_digest,
            },
            session,
        }
    }

    fn set_direct_allowed(&self, allowed: bool) {
        self.state
            .lock()
            .expect("factor engine state")
            .direct_allowed = allowed;
    }

    fn set_malformed_remember_replacement(&self, enabled: bool) {
        self.state
            .lock()
            .expect("factor engine state")
            .remember_returns_malformed_replacement = enabled;
    }

    fn fail_next_revocations(&self, count: usize) {
        self.state
            .lock()
            .expect("factor engine state")
            .revoke_failures_remaining = count;
    }

    fn block_next_revoke(&self) {
        self.state
            .lock()
            .expect("factor engine state")
            .block_next_revoke = true;
    }

    fn hold_revocations(&self, hold: bool) {
        self.hold_revocations.store(hold, Ordering::SeqCst);
    }

    fn block_next_passkey_lookup(&self) {
        self.state
            .lock()
            .expect("factor engine state")
            .block_passkey_lookup = true;
    }

    fn revoke_attempts(&self) -> usize {
        self.revoke_attempts.load(Ordering::SeqCst)
    }

    fn active_session_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("factor engine state")
            .sessions
            .values()
            .filter(|session| session.revoked_at.is_none())
            .map(|session| session.session_id.clone())
            .collect()
    }

    fn last_issued_id(&self) -> String {
        self.state
            .lock()
            .expect("factor engine state")
            .last_issued_id
            .clone()
            .expect("fixture issued a session")
    }

    fn revocation_count(&self, session_id: &str) -> usize {
        self.state
            .lock()
            .expect("factor engine state")
            .revocations
            .get(session_id)
            .copied()
            .unwrap_or_default()
    }

    fn unsupported<T>() -> magnetar::Result<T> {
        Err(magnetar::Error::DependencyUnavailable {
            dependency: "unused test operation".to_owned(),
            message: "operation is outside this fixture".to_owned(),
        })
    }
}

#[async_trait]
impl MagnetarFactorAuthEngine for FactorEngine {
    async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> magnetar::Result<MagnetarIssuedSession> {
        self.state
            .lock()
            .expect("factor engine state")
            .completion_calls += 1;
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
        Ok(self.issue_session(&user))
    }

    async fn user_by_id(&self, user_id: &str) -> magnetar::Result<Option<User>> {
        Ok((user_id == USER_ID).then(FactorEngine::user))
    }

    async fn resolve_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
        OpaqueSessionProvider::new(Arc::new(self.clone()), OpaqueConfig::default())
            .resolve_web_binding(binding, &HostSessionApproval::authenticated())
            .await
    }

    async fn bearer_user_id(&self, token: &str) -> magnetar::Result<Option<String>> {
        match OpaqueSessionProvider::new(Arc::new(self.clone()), OpaqueConfig::default())
            .verify_bearer(token)
            .await
        {
            Ok(session) => Ok(Some(session.user_id().to_owned())),
            Err(magnetar::Error::NotFound { .. } | magnetar::Error::InvalidInput { .. }) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn revoke_session(&self, session_id: &str) -> magnetar::Result<bool> {
        self.revoke_attempts.fetch_add(1, Ordering::SeqCst);
        if self.hold_revocations.load(Ordering::SeqCst) {
            self.revoke_started.notify_waiters();
            self.revoke_release.notified().await;
        }
        let block = {
            let mut state = self.state.lock().expect("factor engine state");
            std::mem::take(&mut state.block_next_revoke)
        };
        if block {
            self.revoke_started.notify_waiters();
            self.revoke_release.notified().await;
        }
        {
            let mut state = self.state.lock().expect("factor engine state");
            if state.revoke_failures_remaining > 0 {
                state.revoke_failures_remaining -= 1;
                return Err(magnetar::Error::DependencyUnavailable {
                    dependency: "fixture opaque session store".to_owned(),
                    message: "injected transient revocation failure".to_owned(),
                });
            }
        }
        let revoked = OpaqueSessionProvider::new(Arc::new(self.clone()), OpaqueConfig::default())
            .revoke_session(session_id)
            .await?;
        *self
            .state
            .lock()
            .expect("factor engine state")
            .revocations
            .entry(session_id.to_owned())
            .or_default() += 1;
        Ok(revoked)
    }

    async fn revoke_all_sessions(&self, user_id: &str) -> magnetar::Result<u64> {
        OpaqueSessionProvider::new(Arc::new(self.clone()), OpaqueConfig::default())
            .revoke_all_for_user(user_id)
            .await
    }

    async fn list_sessions(&self, user_id: &str) -> magnetar::Result<Vec<SessionSummary>> {
        OpaqueSessionProvider::new(Arc::new(self.clone()), OpaqueConfig::default())
            .list_for_user(user_id)
            .await
    }
}

#[async_trait]
impl MagnetarPasswordAuthEngine for PasswordEngine {
    async fn password_sign_in(
        &self,
        _: magnetar::plugins::password::PasswordAttempt,
    ) -> magnetar::Result<(User, HostSignInDecision)> {
        let user = FactorEngine::user();
        let decision = if self
            .factors
            .state
            .lock()
            .expect("factor engine state")
            .direct_allowed
        {
            HostSignInDecision::SessionAllowed(Box::new(self.factors.issue_session(&user)))
        } else {
            self.factors.require_factor("password")
        };
        Ok((user, decision))
    }

    async fn complete_challenge(
        &self,
        _: &str,
        _: &str,
    ) -> magnetar::Result<MagnetarIssuedSession> {
        FactorEngine::unsupported()
    }

    async fn issue_password_reset(
        &self,
        _: &str,
    ) -> magnetar::Result<Option<HostPasswordResetIssued>> {
        FactorEngine::unsupported()
    }

    async fn check_password_reset(&self, _: SecretString) -> magnetar::Result<bool> {
        FactorEngine::unsupported()
    }

    async fn complete_password_reset(
        &self,
        _: SecretString,
        _: SecretString,
    ) -> magnetar::Result<magnetar::plugins::password_management::PasswordResetFlowOutcome> {
        FactorEngine::unsupported()
    }

    async fn password_register(
        &self,
        _: magnetar::plugins::password::RegisterInput,
    ) -> magnetar::Result<User> {
        FactorEngine::unsupported()
    }

    async fn bearer_user_id(&self, _: &str) -> magnetar::Result<Option<String>> {
        FactorEngine::unsupported()
    }

    async fn issue_remember(
        &self,
        _: &str,
        _: suprnova::chrono::Duration,
    ) -> magnetar::Result<magnetar::sessions::RememberCredential> {
        FactorEngine::unsupported()
    }

    async fn remember_sign_in(
        &self,
        _: magnetar::sessions::RememberCredential,
        _: SessionMetadata,
        _: suprnova::chrono::Duration,
    ) -> magnetar::Result<MagnetarRememberSignIn> {
        FactorEngine::unsupported()
    }

    async fn remember_sign_in_attempt(
        &self,
        _: magnetar::sessions::RememberCredential,
        _: SessionMetadata,
        _: suprnova::chrono::Duration,
    ) -> magnetar::Result<MagnetarRememberSignInAttempt> {
        if self
            .factors
            .state
            .lock()
            .expect("factor engine state")
            .remember_returns_malformed_replacement
        {
            let user = FactorEngine::user();
            return Ok(MagnetarRememberSignInAttempt::Authenticated(
                MagnetarRememberSignIn {
                    session: Box::new(self.factors.issue_session(&user)),
                    replacement: magnetar::sessions::RememberCredential::from_host(
                        SecretString::from("malformed-replacement"),
                    ),
                },
            ));
        }
        FactorEngine::unsupported()
    }

    async fn resolve_web_binding(
        &self,
        _: &WebSessionBinding,
    ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
        self.binding_resolution_calls.fetch_add(1, Ordering::SeqCst);
        Err(magnetar::Error::NotFound {
            resource: "password-owned opaque session".to_owned(),
            identifier: "factor sessions use an independent store".to_owned(),
        })
    }

    async fn revoke_remember(&self, _: &str) -> magnetar::Result<u64> {
        FactorEngine::unsupported()
    }

    async fn user_by_id(&self, user_id: &str) -> magnetar::Result<Option<User>> {
        let missing = self
            .factors
            .state
            .lock()
            .expect("factor engine state")
            .magic_lookup_missing;
        Ok((!missing && user_id == USER_ID).then(FactorEngine::user))
    }

    async fn revoke_session(&self, _: &str) -> magnetar::Result<bool> {
        FactorEngine::unsupported()
    }

    async fn revoke_all_sessions(&self, _: &str) -> magnetar::Result<u64> {
        FactorEngine::unsupported()
    }

    async fn list_sessions(&self, _: &str) -> magnetar::Result<Vec<SessionSummary>> {
        FactorEngine::unsupported()
    }

    async fn record_failed_attempt(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> magnetar::Result<LockoutStatus> {
        FactorEngine::unsupported()
    }

    async fn lockout_status(&self, _: &str) -> magnetar::Result<LockoutStatus> {
        FactorEngine::unsupported()
    }

    async fn reset_attempts(&self, _: &str) -> magnetar::Result<()> {
        FactorEngine::unsupported()
    }

    async fn unlock_account(&self, _: &str) -> magnetar::Result<bool> {
        FactorEngine::unsupported()
    }

    async fn magic_link_send(&self, _: &str) -> magnetar::Result<String> {
        Ok("magic-token".to_owned())
    }

    async fn magic_link_consume(
        &self,
        _: &str,
        _: SessionMetadata,
    ) -> magnetar::Result<HostSignInDecision> {
        if self
            .factors
            .state
            .lock()
            .expect("factor engine state")
            .direct_allowed
        {
            let user = FactorEngine::user();
            Ok(HostSignInDecision::SessionAllowed(Box::new(
                self.factors.issue_session(&user),
            )))
        } else {
            Ok(self.factors.require_factor("magic-link"))
        }
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
impl MagnetarPasskeyAuthEngine for PasskeyEngine {
    async fn passkey_begin_registration(
        &self,
        _: magnetar::passkey::RegistrationIntent,
    ) -> magnetar::Result<magnetar::passkey::BegunRegistration> {
        FactorEngine::unsupported()
    }

    async fn passkey_finish_registration(
        &self,
        _: &str,
        _: &str,
        _: &webauthn_rs::prelude::RegisterPublicKeyCredential,
    ) -> magnetar::Result<User> {
        FactorEngine::unsupported()
    }

    async fn passkey_begin_authentication(
        &self,
        _: &str,
    ) -> magnetar::Result<magnetar::passkey::BegunAuthentication> {
        FactorEngine::unsupported()
    }

    async fn passkey_finish_authentication(
        &self,
        _: &str,
        _: &str,
        _: &webauthn_rs::prelude::PublicKeyCredential,
        _: SessionMetadata,
    ) -> magnetar::Result<HostSignInDecision> {
        if self
            .factors
            .state
            .lock()
            .expect("factor engine state")
            .direct_allowed
        {
            let user = FactorEngine::user();
            Ok(HostSignInDecision::SessionAllowed(Box::new(
                self.factors.issue_session(&user),
            )))
        } else {
            Ok(self.factors.require_factor("passkey"))
        }
    }

    async fn passkey_user_by_id(&self, _: &str) -> magnetar::Result<User> {
        let (block, fails) = {
            let mut state = self.factors.state.lock().expect("factor engine state");
            (
                std::mem::take(&mut state.block_passkey_lookup),
                state.passkey_lookup_fails,
            )
        };
        if block {
            self.factors.passkey_lookup_started.notify_waiters();
            self.factors.passkey_lookup_release.notified().await;
        }
        if fails {
            return Err(magnetar::Error::DependencyUnavailable {
                dependency: "fixture host user lookup".to_owned(),
                message: "injected lookup failure".to_owned(),
            });
        }
        Ok(FactorEngine::user())
    }
}

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl MagnetarOAuthAuthEngine for OAuthEngine {
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
        if self
            .factors
            .state
            .lock()
            .expect("factor engine state")
            .direct_allowed
        {
            let user = FactorEngine::user();
            return Ok(MagnetarOAuthCompletion::SessionAllowed {
                session: Box::new(self.factors.issue_session(&user)),
                user,
            });
        }
        let HostSignInDecision::FactorRequired { challenge_selector } =
            self.factors.require_factor("oauth")
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
    fail_next_read: AtomicBool,
    fail_next_write: AtomicBool,
    fail_next_destroy: AtomicBool,
}

impl MemoryStore {
    fn fail_next_read(&self) {
        self.fail_next_read.store(true, Ordering::SeqCst);
    }

    fn fail_next_write(&self) {
        self.fail_next_write.store(true, Ordering::SeqCst);
    }

    fn fail_next_destroy(&self) {
        self.fail_next_destroy.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl SessionStore for MemoryStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        if self.fail_next_read.swap(false, Ordering::SeqCst) {
            return Err(FrameworkError::internal("injected session read failure"));
        }
        Ok(self
            .sessions
            .lock()
            .expect("session store")
            .get(id)
            .cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        if self.fail_next_write.swap(false, Ordering::SeqCst) {
            return Err(FrameworkError::internal("injected session write failure"));
        }
        self.sessions
            .lock()
            .expect("session store")
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        if self.fail_next_destroy.swap(false, Ordering::SeqCst) {
            return Err(FrameworkError::internal("injected session destroy failure"));
        }
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

async fn request(cookie: Option<&str>, authorization: Option<&str>) -> suprnova::Request {
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

fn response_cookie(headers: &hyper::HeaderMap, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&prefix))
        .and_then(|value| value.split(';').next())
        .map(ToOwned::to_owned)
}

async fn seed_anonymous_session(middleware: &SessionMiddleware, cookie_name: &str) -> String {
    let next: Next = Arc::new(|_| {
        Box::pin(async move {
            suprnova::session::session_mut(|session| session.put("seed", true));
            Ok(suprnova::HttpResponse::text("seeded"))
        })
    });
    let response = match middleware.handle(request(None, None).await, next).await {
        Ok(response) => response.into_hyper(),
        Err(_) => panic!("seed request succeeds"),
    };
    response_cookie(response.headers(), cookie_name).expect("seed request emits session cookie")
}

async fn assert_lookup_failure_does_not_bind(origin: Origin, factors: &FactorEngine) -> String {
    let store = Arc::new(MemoryStore::default());
    let mut config = SessionConfig::default();
    config.cookie_name = format!("lookup_failure_{}", origin.name().replace('-', "_"));
    config.cookie_secure = false;
    let middleware = SessionMiddleware::with_store(config.clone(), store);
    let next: Next = Arc::new(move |_| {
        Box::pin(async move {
            let result = outcome_for(origin).await;
            assert!(result.is_err(), "{} lookup must fail", origin.name());
            assert_eq!(suprnova::Auth::id(), None);
            assert!(
                suprnova::session::session()
                    .and_then(|session| session.magnetar_web_binding())
                    .is_none(),
                "{} lookup failure must not bind an opaque carrier",
                origin.name()
            );
            Ok(suprnova::HttpResponse::text("lookup failed cleanly"))
        })
    });
    let response = match middleware.handle(request(None, None).await, next).await {
        Ok(response) => response.into_hyper(),
        Err(_) => panic!("{} lookup failure should be handler-visible", origin.name()),
    };
    if let Some(cookie) = response_cookie(response.headers(), &config.cookie_name) {
        let next: Next = Arc::new(|_| {
            Box::pin(async {
                assert_eq!(suprnova::Auth::id(), None);
                Ok(suprnova::HttpResponse::text("anonymous"))
            })
        });
        if middleware
            .handle(request(Some(&cookie), None).await, next)
            .await
            .is_err()
        {
            panic!("lookup-failure cookie must remain anonymous and readable");
        }
    }
    let issued_id = factors.last_issued_id();
    assert!(
        factors.active_session_ids().is_empty(),
        "{} lookup failure must retire its opaque session",
        origin.name()
    );
    assert_eq!(factors.revocation_count(&issued_id), 1);
    issued_id
}

async fn assert_persistence_failure_retires(
    origin: Origin,
    failure: StoreFailure,
    factors: &FactorEngine,
) {
    let store = Arc::new(MemoryStore::default());
    let mut config = SessionConfig::default();
    config.cookie_name = format!("handoff_failure_{}", origin.name().replace('-', "_"));
    config.cookie_secure = false;
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
    let old_cookie = match failure {
        StoreFailure::Read | StoreFailure::Destroy => {
            Some(seed_anonymous_session(&middleware, &config.cookie_name).await)
        }
        StoreFailure::Write | StoreFailure::Cookie => None,
    };
    match failure {
        StoreFailure::Read => store.fail_next_read(),
        StoreFailure::Write => store.fail_next_write(),
        StoreFailure::Destroy => store.fail_next_destroy(),
        StoreFailure::Cookie => {
            suprnova::session::middleware::fail_next_session_cookie_construction_for_test();
        }
    }

    let next: Next = Arc::new(move |_| {
        Box::pin(async move {
            let outcome = outcome_for(origin)
                .await
                .expect("provider issues a fresh authenticated session");
            assert!(matches!(outcome, SignInOutcome::Authenticated { .. }));
            assert_eq!(suprnova::Auth::id().as_deref(), Some(USER_ID));
            Ok(suprnova::HttpResponse::text("authenticated"))
        })
    });
    let response = middleware
        .handle(request(old_cookie.as_deref(), None).await, next)
        .await;
    let response = match response {
        Err(response) if response.status_code() == 500 => response.into_hyper(),
        _ => panic!("{} persistence failure must fail closed", origin.name()),
    };
    let returned_session_cookie = response_cookie(response.headers(), &config.cookie_name);
    assert!(
        returned_session_cookie
            .as_ref()
            .is_none_or(|cookie| cookie.ends_with('=')),
        "{} failure must not attach a live replacement carrier",
        origin.name()
    );
    let issued_id = factors.last_issued_id();
    assert!(
        factors.active_session_ids().is_empty(),
        "{} persistence failure must retire its opaque session",
        origin.name()
    );
    assert_eq!(
        factors.revocation_count(&issued_id),
        1,
        "{} opaque session must be retired exactly once",
        origin.name()
    );

    if let Some(old_cookie) = old_cookie {
        let next: Next = Arc::new(|_| {
            Box::pin(async {
                assert_eq!(suprnova::Auth::id(), None);
                Ok(suprnova::HttpResponse::text("anonymous"))
            })
        });
        if middleware
            .handle(request(Some(&old_cookie), None).await, next)
            .await
            .is_err()
        {
            panic!("failed handoff must leave the old anonymous session readable");
        }
    }
}

async fn assert_successful_handoff(origin: Origin, factors: &FactorEngine) {
    let store = Arc::new(MemoryStore::default());
    let mut config = SessionConfig::default();
    config.cookie_name = format!("handoff_success_{}", origin.name().replace('-', "_"));
    config.cookie_secure = false;
    let middleware = SessionMiddleware::with_store(config.clone(), store);
    let next: Next = Arc::new(move |_| {
        Box::pin(async move {
            let outcome = outcome_for(origin)
                .await
                .expect("provider authenticates successfully");
            assert!(matches!(outcome, SignInOutcome::Authenticated { .. }));
            assert_eq!(suprnova::Auth::id().as_deref(), Some(USER_ID));
            Ok(suprnova::HttpResponse::text("authenticated"))
        })
    });
    let response = match middleware.handle(request(None, None).await, next).await {
        Ok(response) => response.into_hyper(),
        Err(_) => panic!("{} handoff must persist", origin.name()),
    };
    let cookie = response_cookie(response.headers(), &config.cookie_name)
        .expect("successful handoff emits a framework session carrier");
    let next: Next = Arc::new(|_| {
        Box::pin(async {
            assert_eq!(suprnova::Auth::id().as_deref(), Some(USER_ID));
            Ok(suprnova::HttpResponse::text("authenticated again"))
        })
    });
    if middleware
        .handle(request(Some(&cookie), None).await, next)
        .await
        .is_err()
    {
        panic!(
            "{} carrier must authenticate the next request",
            origin.name()
        );
    }
    let issued_id = factors.last_issued_id();
    assert_eq!(factors.revocation_count(&issued_id), 0);
    assert!(
        suprnova::magnetar_integration::revoke_session(&issued_id)
            .await
            .expect("clean up successful fixture session")
    );
}

async fn assert_commit_releases_only_current_handoff(factors: &FactorEngine) {
    let store = Arc::new(MemoryStore::default());
    let mut config = SessionConfig::default();
    config.cookie_name = "selective_handoff_commit".to_owned();
    config.cookie_secure = false;
    let middleware = SessionMiddleware::with_store(config, store);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let observed_for_handler = observed.clone();
    factors.fail_next_revocations(1);
    let next: Next = Arc::new(move |_| {
        let observed = observed_for_handler.clone();
        Box::pin(async move {
            for _ in 0..2 {
                assert!(matches!(
                    outcome_for(Origin::Password).await?,
                    SignInOutcome::Authenticated { .. }
                ));
                let binding = suprnova::session::session()
                    .and_then(|session| session.magnetar_web_binding())
                    .expect("fresh framework binding");
                observed
                    .lock()
                    .expect("handoff observations")
                    .push(binding.session_id);
            }
            Ok(suprnova::HttpResponse::text("committed"))
        })
    });
    assert!(
        middleware
            .handle(request(None, None).await, next)
            .await
            .is_ok(),
        "the current handoff must commit"
    );
    let observed = observed.lock().expect("handoff observations").clone();
    let [superseded, current] = observed.as_slice() else {
        panic!("fixture must issue exactly two sessions")
    };
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while factors.active_session_ids().contains(superseded) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("superseded handoff remains queued for Drop retry");
    assert!(factors.active_session_ids().contains(current));
    assert_eq!(factors.revocation_count(current), 0);
    assert!(
        suprnova::magnetar_integration::revoke_session(current)
            .await
            .expect("clean up current fixture session")
    );
}

async fn assert_scoped_cancellation_has_one_cleanup_owner(factors: &FactorEngine) {
    let store = Arc::new(MemoryStore::default());
    let mut config = SessionConfig::default();
    config.cookie_name = "cancelled_handoff".to_owned();
    config.cookie_secure = false;
    let middleware = SessionMiddleware::with_store(config, store);
    factors.block_next_passkey_lookup();
    factors.hold_revocations(true);
    let attempts_before = factors.revoke_attempts();
    let lookup_started = factors.passkey_lookup_started.notified();
    let next: Next = Arc::new(|_| {
        Box::pin(async move {
            let _ = outcome_for(Origin::Passkey).await?;
            Ok(suprnova::HttpResponse::text("unreachable"))
        })
    });
    let mut request = Box::pin(middleware.handle(request(None, None).await, next));
    tokio::pin!(lookup_started);
    tokio::select! {
        () = &mut lookup_started => {}
        _ = &mut request => panic!("middleware completed before cancellation"),
    }
    let session_id = factors.last_issued_id();
    drop(request);
    let revoke_started = factors.revoke_started.notified();
    tokio::pin!(revoke_started);
    tokio::time::timeout(std::time::Duration::from_secs(1), &mut revoke_started)
        .await
        .expect("cancellation starts cleanup");
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        factors.revoke_attempts() - attempts_before,
        1,
        "SessionMiddleware must be the only cleanup owner for scoped handoffs"
    );
    factors.hold_revocations(false);
    factors.revoke_release.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while factors.active_session_ids().contains(&session_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the single cleanup owner retires the session");
    assert_eq!(factors.revocation_count(&session_id), 1);
}

async fn outcome_for(origin: Origin) -> Result<SignInOutcome, FrameworkError> {
    match origin {
        Origin::Password => {
            suprnova::Auth::password()
                .authenticate_outcome("factor@example.test", "password", None, None)
                .await
        }
        Origin::MagicLink => {
            suprnova::Auth::magic_link()
                .consume_outcome("magic-token")
                .await
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

fn encrypted_remember_carrier(credential: &str) -> String {
    let plaintext = format!(
        "suprnova.remember.v1:{}",
        serde_json::json!({
            "guard": suprnova::Auth::default_guard_name(),
            "credential": credential,
        })
    );
    suprnova::Cookie::encrypted(suprnova::auth::remember::COOKIE_NAME, &plaintext)
        .expect("encrypt remember fixture")
        .value()
        .to_owned()
}

#[tokio::test]
async fn every_sign_in_origin_completes_through_the_installed_factor_facade() {
    suprnova::testing::install_test_encryption_key();
    let factors = Arc::new(FactorEngine::default());
    let password_binding_resolution_calls = Arc::new(AtomicUsize::new(0));
    let password = Arc::new(PasswordEngine {
        factors: factors.clone(),
        binding_resolution_calls: password_binding_resolution_calls.clone(),
    });
    let passkey = Arc::new(PasskeyEngine {
        factors: factors.clone(),
    });
    suprnova::magnetar_integration::install_magnetar_engines_with_factor(
        password,
        passkey,
        factors.clone(),
    )
    .expect("install distinct provider engines with one factor owner");
    #[cfg(feature = "magnetar-oauth")]
    suprnova::magnetar_integration::install_magnetar_oauth_engine(Arc::new(OAuthEngine {
        factors: factors.clone(),
    }))
    .expect("install a distinct OAuth provider engine");

    factors.set_direct_allowed(false);

    let HostSignInDecision::FactorRequired {
        challenge_selector: headless_selector,
    } = factors.require_factor("headless")
    else {
        unreachable!("fixture always requires a factor")
    };
    let headless = suprnova::Auth::factor()
        .complete_challenge(&headless_selector, FACTOR_CODE)
        .await;
    assert!(
        headless.is_err(),
        "factor completion must require active SessionMiddleware scopes"
    );
    {
        let state = factors.state.lock().expect("factor engine state");
        assert_eq!(
            state.completion_calls, 0,
            "scope rejection must happen before the factor proof is consumed"
        );
        assert!(
            state.pending.contains_key(&headless_selector),
            "scope rejection must leave the factor selector available"
        );
    }

    let origins = [
        Origin::Password,
        Origin::MagicLink,
        Origin::Passkey,
        #[cfg(feature = "magnetar-oauth")]
        Origin::OAuth,
    ];
    let issued_sessions = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    for origin in origins {
        let store = Arc::new(MemoryStore::default());
        let mut config = SessionConfig::default();
        config.cookie_name = format!("factor_{}", origin.name().replace('-', "_"));
        config.cookie_secure = false;
        let middleware = SessionMiddleware::with_store(config.clone(), store);
        let issued_sessions_for_handler = issued_sessions.clone();
        let next: Next = Arc::new(move |_| {
            let issued_sessions = issued_sessions_for_handler.clone();
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
                let binding = after
                    .magnetar_web_binding()
                    .expect("factor completion stores its opaque binding");
                let token = issued
                    .token
                    .as_ref()
                    .expect("fresh factor session has a bearer token")
                    .expose_secret()
                    .to_owned();
                issued_sessions
                    .lock()
                    .expect("issued session observations")
                    .push((binding.session_id, token));
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
            middleware.handle(request(None, None).await, next).await
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
            .handle(request(Some(&cookie), None).await, second_next)
            .await;
        if let Err(error) = second {
            panic!(
                "cookie-bearing request returned status {}",
                error.status_code()
            );
        }
    }
    assert_eq!(
        password_binding_resolution_calls.load(Ordering::SeqCst),
        0,
        "factor-issued bindings must be resolved by the installed factor/session authority",
    );

    let issued_sessions = issued_sessions
        .lock()
        .expect("issued session observations")
        .clone();
    let (first_session_id, first_token) = issued_sessions
        .first()
        .expect("at least one factor session was issued")
        .clone();
    let observed_bearer = Arc::new(Mutex::new(None));
    let observed_bearer_for_handler = observed_bearer.clone();
    let bearer_next: Next = Arc::new(move |_| {
        let observed = observed_bearer_for_handler.clone();
        Box::pin(async move {
            *observed.lock().expect("bearer observation") = suprnova::Auth::id();
            Ok(suprnova::HttpResponse::text("bearer checked"))
        })
    });
    let bearer_response = suprnova::auth::request_state::request_state_scope_for_test(async {
        suprnova::magnetar_integration::middleware::BearerTokenMiddleware
            .handle(request(None, Some(&first_token)).await, bearer_next)
            .await
    })
    .await;
    if let Err(error) = bearer_response {
        panic!(
            "factor bearer middleware returned status {}",
            error.status_code()
        );
    }
    assert_eq!(
        *observed_bearer.lock().expect("bearer observation"),
        Some(USER_ID.to_owned()),
        "factor-issued bearer tokens must use the factor/session authority",
    );

    let listed = suprnova::magnetar_integration::list_sessions(USER_ID)
        .await
        .expect("list factor-owned sessions");
    assert_eq!(listed.len(), issued_sessions.len());
    assert!(
        suprnova::magnetar_integration::revoke_session(&first_session_id)
            .await
            .expect("revoke one factor-owned session")
    );
    assert_eq!(
        suprnova::magnetar_integration::list_sessions(USER_ID)
            .await
            .expect("list after single revoke")
            .len(),
        issued_sessions.len() - 1,
    );
    assert_eq!(
        suprnova::magnetar_integration::revoke_all_sessions(USER_ID)
            .await
            .expect("revoke remaining factor-owned sessions"),
        u64::try_from(issued_sessions.len() - 1).expect("fixture count fits u64"),
    );
    assert!(
        suprnova::magnetar_integration::list_sessions(USER_ID)
            .await
            .expect("list after all-session revoke")
            .is_empty()
    );

    factors.set_direct_allowed(true);

    factors.set_malformed_remember_replacement(true);
    let malformed_remember = encrypted_remember_carrier("incoming-selector.incoming-secret");
    let malformed_cookie = format!("remember_me={malformed_remember}");
    let store = Arc::new(MemoryStore::default());
    let mut remember_config = SessionConfig::default();
    remember_config.cookie_secure = false;
    let remember_middleware = SessionMiddleware::with_store(remember_config, store);
    let unexpected_next: Next = Arc::new(|_| {
        Box::pin(async move { panic!("malformed rotated credential must fail before the handler") })
    });
    assert!(
        remember_middleware
            .handle(
                request(Some(&malformed_cookie), None).await,
                unexpected_next,
            )
            .await
            .is_err(),
        "malformed rotated remember credential must fail closed"
    );
    let remembered_session_id = factors.last_issued_id();
    assert!(
        !factors
            .active_session_ids()
            .contains(&remembered_session_id),
        "the installed factor/session authority must retire an early-aborted remembered session"
    );
    assert_eq!(factors.revocation_count(&remembered_session_id), 1);
    factors.set_malformed_remember_replacement(false);

    for origin in [Origin::Password, Origin::MagicLink] {
        let headless = outcome_for(origin).await.unwrap_or_else(|error| {
            panic!(
                "direct {} authentication remains supported without SessionMiddleware: {error}",
                origin.name()
            )
        });
        let SignInOutcome::Authenticated {
            session: headless_session,
            ..
        } = headless
        else {
            panic!("headless direct authentication must return its opaque session")
        };
        assert!(headless_session.token.is_some());
        let headless_id = factors.last_issued_id();
        assert!(factors.active_session_ids().contains(&headless_id));
        assert!(
            suprnova::magnetar_integration::revoke_session(&headless_id)
                .await
                .expect("clean up headless fixture session")
        );
    }

    factors
        .state
        .lock()
        .expect("factor engine state")
        .magic_lookup_missing = true;
    factors.fail_next_revocations(1);
    assert!(
        outcome_for(Origin::MagicLink).await.is_err(),
        "headless lookup failure must remain an authentication error"
    );
    let failed_headless_id = factors.last_issued_id();
    assert!(
        !factors.active_session_ids().contains(&failed_headless_id),
        "headless cleanup must survive one transient retirement failure"
    );
    assert_eq!(factors.revocation_count(&failed_headless_id), 1);
    factors
        .state
        .lock()
        .expect("factor engine state")
        .magic_lookup_missing = false;

    assert_scoped_cancellation_has_one_cleanup_owner(factors.as_ref()).await;
    assert_commit_releases_only_current_handoff(factors.as_ref()).await;

    factors
        .state
        .lock()
        .expect("factor engine state")
        .magic_lookup_missing = true;
    factors.block_next_revoke();
    let revoke_started = factors.revoke_started.notified();
    let mut cancelled_handoff = Box::pin(outcome_for(Origin::MagicLink));
    tokio::pin!(revoke_started);
    tokio::select! {
        () = &mut revoke_started => {}
        result = &mut cancelled_handoff => panic!("handoff completed before cleanup cancellation: {result:?}"),
    }
    let cancelled_session_id = factors.last_issued_id();
    drop(cancelled_handoff);
    factors.revoke_release.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while factors.active_session_ids().contains(&cancelled_session_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("detached headless cleanup completes after caller cancellation");
    assert_eq!(factors.revocation_count(&cancelled_session_id), 1);
    factors
        .state
        .lock()
        .expect("factor engine state")
        .magic_lookup_missing = false;

    assert_persistence_failure_retires(Origin::Password, StoreFailure::Read, factors.as_ref())
        .await;
    assert_persistence_failure_retires(Origin::MagicLink, StoreFailure::Destroy, factors.as_ref())
        .await;
    assert_persistence_failure_retires(Origin::Passkey, StoreFailure::Cookie, factors.as_ref())
        .await;
    #[cfg(feature = "magnetar-oauth")]
    assert_persistence_failure_retires(Origin::OAuth, StoreFailure::Write, factors.as_ref()).await;

    factors
        .state
        .lock()
        .expect("factor engine state")
        .magic_lookup_missing = true;
    assert_lookup_failure_does_not_bind(Origin::MagicLink, factors.as_ref()).await;
    factors
        .state
        .lock()
        .expect("factor engine state")
        .magic_lookup_missing = false;

    factors
        .state
        .lock()
        .expect("factor engine state")
        .passkey_lookup_fails = true;
    assert_lookup_failure_does_not_bind(Origin::Passkey, factors.as_ref()).await;
    factors
        .state
        .lock()
        .expect("factor engine state")
        .passkey_lookup_fails = false;

    for origin in [
        Origin::Password,
        Origin::MagicLink,
        Origin::Passkey,
        #[cfg(feature = "magnetar-oauth")]
        Origin::OAuth,
    ] {
        assert_successful_handoff(origin, factors.as_ref()).await;
    }
}
