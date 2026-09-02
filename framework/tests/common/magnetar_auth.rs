use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use magnetar::sessions::{SessionSummary, WebSessionBinding};
use secrecy::{ExposeSecret, SecretString};
use suprnova::magnetar_integration::engine::{
    HostSignInDecision, MagnetarIssuedSession, MagnetarPasswordAuthEngine,
};
use suprnova::{LockoutStatus, Session, SessionToken, User, UserId};

static FAIL_NEXT_REMEMBER_ISSUE: AtomicBool = AtomicBool::new(false);

pub fn fail_next_remember_issue() {
    FAIL_NEXT_REMEMBER_ISSUE.store(true, Ordering::SeqCst);
}

pub fn take_unconsumed_remember_issue_failure() -> bool {
    FAIL_NEXT_REMEMBER_ISSUE.swap(false, Ordering::SeqCst)
}

struct AllowingLimiter;

#[async_trait]
impl suprnova::RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(
        &self,
        _: &str,
        _: &suprnova::SlidingWindowConfig,
    ) -> Result<bool, suprnova::FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _: &str,
        _: &suprnova::SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, suprnova::FrameworkError> {
        Ok(None)
    }
}

#[derive(Default)]
struct State {
    users_by_email: HashMap<String, (User, String)>,
    users_by_id: HashMap<String, User>,
    bearer_users: HashMap<String, String>,
    sessions: HashMap<String, SessionSummary>,
    magic_links: HashMap<String, String>,
    failures: HashMap<String, u32>,
    locked_until: HashMap<String, suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
}

#[derive(Default)]
struct TestEngine {
    state: Mutex<State>,
}

impl TestEngine {
    fn issue_session(&self, user: &User) -> MagnetarIssuedSession {
        let session_id = format!("ses_{}", uuid::Uuid::new_v4());
        let token = SessionToken::new_random();
        let token_value = token.expose_secret().to_owned();
        let session = Session::builder()
            .token(token)
            .user_id(user.id.clone())
            .build()
            .expect("test session");
        let summary = SessionSummary {
            session_id: session_id.clone(),
            user_id: user.id.to_string(),
            expires_at: session.expires_at,
            metadata: magnetar::sessions::SessionMetadata::default(),
        };
        let mut state = self.state.lock().expect("test engine state");
        state.bearer_users.insert(token_value, user.id.to_string());
        state.sessions.insert(session_id.clone(), summary);
        MagnetarIssuedSession {
            session_id: session_id.clone(),
            web_binding: WebSessionBinding {
                session_id,
                token_digest: [0; 32],
            },
            session,
        }
    }

    fn user_by_email(&self, email: &str) -> Option<User> {
        self.state
            .lock()
            .expect("test engine state")
            .users_by_email
            .get(&email.trim().to_ascii_lowercase())
            .map(|(user, _)| user.clone())
    }
}

#[async_trait]
impl MagnetarPasswordAuthEngine for TestEngine {
    async fn password_sign_in(
        &self,
        input: magnetar::plugins::password::PasswordAttempt,
    ) -> magnetar::Result<(User, HostSignInDecision)> {
        let email = input.email.trim().to_ascii_lowercase();
        let user = {
            let state = self.state.lock().expect("test engine state");
            let Some((user, password)) = state.users_by_email.get(&email) else {
                return Err(magnetar::Error::NotFound {
                    resource: "user".to_owned(),
                    identifier: email,
                });
            };
            if password != input.password.expose_secret() {
                return Err(magnetar::Error::InvalidInput {
                    field: "credentials".to_owned(),
                    message: "invalid credentials".to_owned(),
                });
            }
            user.clone()
        };
        let issued = self.issue_session(&user);
        Ok((user, HostSignInDecision::SessionAllowed(Box::new(issued))))
    }

    async fn password_register(
        &self,
        input: magnetar::plugins::password::RegisterInput,
    ) -> magnetar::Result<User> {
        let email = input.email.trim().to_ascii_lowercase();
        if let Some(user) = self.user_by_email(&email) {
            return Ok(user);
        }
        let user_id = format!("usr_{}", uuid::Uuid::new_v4().simple());
        let user = User::builder()
            .id(UserId::new(&user_id))
            .email(email.clone())
            .build()
            .map_err(|error| magnetar::Error::Internal {
                message: error.to_string(),
            })?;
        let mut state = self.state.lock().expect("test engine state");
        state.users_by_id.insert(user.id.to_string(), user.clone());
        state.users_by_email.insert(
            email,
            (user.clone(), input.password.expose_secret().to_owned()),
        );
        Ok(user)
    }

    async fn issue_password_reset(
        &self,
        _email: &str,
    ) -> magnetar::Result<Option<suprnova::magnetar_integration::engine::HostPasswordResetIssued>>
    {
        Ok(None)
    }

    async fn check_password_reset(&self, _token: SecretString) -> magnetar::Result<bool> {
        Ok(false)
    }

    async fn complete_password_reset(
        &self,
        _token: SecretString,
        _password: SecretString,
    ) -> magnetar::Result<magnetar::plugins::password_management::PasswordResetFlowOutcome> {
        Err(magnetar::Error::InvalidInput {
            field: "password reset".to_owned(),
            message: "not configured in shared auth fixture".to_owned(),
        })
    }

    async fn bearer_user_id(&self, token: &str) -> magnetar::Result<Option<String>> {
        Ok(self
            .state
            .lock()
            .expect("test engine state")
            .bearer_users
            .get(token)
            .cloned())
    }

    async fn issue_remember(
        &self,
        user_id: &str,
        _lifetime: chrono::Duration,
    ) -> magnetar::Result<magnetar::sessions::RememberCredential> {
        if FAIL_NEXT_REMEMBER_ISSUE.swap(false, Ordering::SeqCst) {
            return Err(magnetar::Error::Internal {
                message: "scripted remember issuance failure".to_owned(),
            });
        }
        Ok(magnetar::sessions::RememberCredential::from_host(
            SecretString::from(format!("test-selector-{user_id}.test-verifier")),
        ))
    }

    async fn remember_sign_in(
        &self,
        _credential: magnetar::sessions::RememberCredential,
        _metadata: magnetar::sessions::SessionMetadata,
        _replacement_lifetime: chrono::Duration,
    ) -> magnetar::Result<suprnova::magnetar_integration::engine::MagnetarRememberSignIn> {
        Err(magnetar::Error::Internal {
            message: "remember sign-in is not configured in this test engine".to_owned(),
        })
    }

    async fn resolve_web_binding(
        &self,
        _binding: &WebSessionBinding,
    ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
        Err(magnetar::Error::NotFound {
            resource: "session".to_owned(),
            identifier: "web binding".to_owned(),
        })
    }

    async fn revoke_remember(&self, _user_id: &str) -> magnetar::Result<u64> {
        Ok(0)
    }

    async fn user_by_id(&self, user_id: &str) -> magnetar::Result<Option<User>> {
        Ok(self
            .state
            .lock()
            .expect("test engine state")
            .users_by_id
            .get(user_id)
            .cloned())
    }

    async fn revoke_session(&self, session_id: &str) -> magnetar::Result<bool> {
        Ok(self
            .state
            .lock()
            .expect("test engine state")
            .sessions
            .remove(session_id)
            .is_some())
    }

    async fn revoke_all_sessions(&self, user_id: &str) -> magnetar::Result<u64> {
        let mut state = self.state.lock().expect("test engine state");
        let before = state.sessions.len();
        state
            .sessions
            .retain(|_, session| session.user_id != user_id);
        Ok((before - state.sessions.len()) as u64)
    }

    async fn list_sessions(&self, user_id: &str) -> magnetar::Result<Vec<SessionSummary>> {
        Ok(self
            .state
            .lock()
            .expect("test engine state")
            .sessions
            .values()
            .filter(|session| session.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn record_failed_attempt(
        &self,
        email: &str,
        _: Option<&str>,
    ) -> magnetar::Result<LockoutStatus> {
        let mut state = self.state.lock().expect("test engine state");
        let attempts = {
            let attempts = state.failures.entry(email.to_owned()).or_default();
            *attempts += 1;
            *attempts
        };
        if attempts >= 5 {
            state
                .locked_until
                .entry(email.to_owned())
                .or_insert_with(|| {
                    suprnova::chrono::Utc::now() + suprnova::chrono::Duration::minutes(15)
                });
        }
        Ok(LockoutStatus {
            email: email.to_owned(),
            failed_attempts: attempts,
            is_locked: attempts >= 5,
            locked_until: state.locked_until.get(email).copied(),
        })
    }

    async fn lockout_status(&self, email: &str) -> magnetar::Result<LockoutStatus> {
        let state = self.state.lock().expect("test engine state");
        let attempts = state.failures.get(email).copied().unwrap_or_default();
        Ok(LockoutStatus {
            email: email.to_owned(),
            failed_attempts: attempts,
            is_locked: attempts >= 5,
            locked_until: state.locked_until.get(email).copied(),
        })
    }

    async fn reset_attempts(&self, email: &str) -> magnetar::Result<()> {
        let mut state = self.state.lock().expect("test engine state");
        state.failures.remove(email);
        state.locked_until.remove(email);
        Ok(())
    }

    async fn unlock_account(&self, email: &str) -> magnetar::Result<bool> {
        let mut state = self.state.lock().expect("test engine state");
        let was_locked = state.locked_until.remove(email).is_some();
        state.failures.remove(email);
        Ok(was_locked)
    }

    async fn magic_link_send(&self, email: &str) -> magnetar::Result<String> {
        let email = email.trim().to_ascii_lowercase();
        let user = if let Some(user) = self.user_by_email(&email) {
            user
        } else {
            self.password_register(magnetar::plugins::password::RegisterInput {
                email: email.clone(),
                password: secrecy::SecretString::from(uuid::Uuid::new_v4().to_string()),
            })
            .await?
        };
        let token = uuid::Uuid::new_v4().to_string();
        self.state
            .lock()
            .expect("test engine state")
            .magic_links
            .insert(token.clone(), user.id.to_string());
        Ok(token)
    }

    async fn magic_link_consume(
        &self,
        token: &str,
        _: magnetar::sessions::SessionMetadata,
    ) -> magnetar::Result<HostSignInDecision> {
        let user =
            {
                let mut state = self.state.lock().expect("test engine state");
                let user_id =
                    state
                        .magic_links
                        .remove(token)
                        .ok_or_else(|| magnetar::Error::NotFound {
                            resource: "magic link".to_owned(),
                            identifier: "expired or used".to_owned(),
                        })?;
                state.users_by_id.get(&user_id).cloned().ok_or_else(|| {
                    magnetar::Error::NotFound {
                        resource: "user".to_owned(),
                        identifier: user_id,
                    }
                })?
            };
        Ok(HostSignInDecision::SessionAllowed(Box::new(
            self.issue_session(&user),
        )))
    }
}

pub async fn install() {
    suprnova::App::bind::<dyn suprnova::RateLimiterDriver>(Arc::new(AllowingLimiter));
    let _ = suprnova::magnetar_integration::install_magnetar_password_engine_for_test(Arc::new(
        TestEngine::default(),
    ));
}
