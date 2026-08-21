//! Shared recording/counting fakes used by every domain harness
//! (`password_harness.rs`, `oauth_harness.rs`, ...). Touches only
//! unconditional modules (`magnetar::plugin`, `magnetar::abuse`), so it is
//! safe to include from suites that enable no plugin feature at all.

#![allow(dead_code)]

use async_trait::async_trait;
use std::sync::Arc;

use magnetar::Result;
use magnetar::abuse::{AbuseLimiter, AbusePolicy, Permit};
use magnetar::first_email_proof::{
    FirstEmailProofCommit, FirstEmailProofKind, FirstEmailProofMutation, FirstEmailProofStore,
    NewVerifiedProviderAccount, VerifiedProviderAccountCommit,
};
use magnetar::plugin::{LinkGenerator, MailDriver, MailMessage};
use magnetar::sessions::RememberFacade;
use magnetar::storage::{
    PasswordResetInput, PasswordResetStore, PresentedToken, TokenStore, UserStore,
};
use parking_lot::Mutex;
use secrecy::ExposeSecret;
use serde_json::Value;

/// Recording mail driver.
#[derive(Default)]
pub struct RecordingMail {
    /// Every message handed to the driver, in order.
    pub sent: Mutex<Vec<MailMessage>>,
    /// When set, every send fails (notification-failure paths).
    pub fail: Mutex<bool>,
}

#[async_trait]
impl MailDriver for RecordingMail {
    async fn send(&self, message: MailMessage) -> Result<()> {
        if *self.fail.lock() {
            return Err(magnetar::Error::DependencyUnavailable {
                dependency: "mail".into(),
                message: "harness failure".into(),
            });
        }
        self.sent.lock().push(message);
        Ok(())
    }
}

impl RecordingMail {
    pub fn count(&self) -> usize {
        self.sent.lock().len()
    }
    pub fn last_payload(&self) -> Option<Value> {
        self.sent
            .lock()
            .last()
            .map(|message| message.payload.clone())
    }
    pub fn names(&self) -> Vec<String> {
        self.sent
            .lock()
            .iter()
            .map(|message| message.name.clone())
            .collect()
    }
}

/// Limiter behavior selected per test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimiterMode {
    Allow,
    Reject,
    Error,
}

/// Counting abuse-limiter fake.
pub struct CountingLimiter {
    /// Every `(key, max_requests)` acquisition, in order.
    pub acquired: Mutex<Vec<(String, u32)>>,
    /// Behavior applied to every acquisition.
    pub mode: Mutex<LimiterMode>,
}

impl Default for CountingLimiter {
    fn default() -> Self {
        Self {
            acquired: Mutex::new(Vec::new()),
            mode: Mutex::new(LimiterMode::Allow),
        }
    }
}

impl CountingLimiter {
    pub fn count(&self) -> usize {
        self.acquired.lock().len()
    }
    pub fn keys(&self) -> Vec<String> {
        self.acquired
            .lock()
            .iter()
            .map(|(key, _)| key.clone())
            .collect()
    }
    pub fn set_mode(&self, mode: LimiterMode) {
        *self.mode.lock() = mode;
    }
}

#[async_trait]
impl AbuseLimiter for CountingLimiter {
    async fn acquire(&self, key: &str, policy: AbusePolicy) -> Result<Permit> {
        self.acquired
            .lock()
            .push((key.to_owned(), policy.max_requests));
        match *self.mode.lock() {
            LimiterMode::Allow => Ok(Permit::Allowed { retry_after: None }),
            LimiterMode::Reject => Ok(Permit::Rejected {
                retry_after: std::time::Duration::from_secs(30),
            }),
            LimiterMode::Error => Err(magnetar::Error::DependencyUnavailable {
                dependency: "limiter".into(),
                message: "harness outage".into(),
            }),
        }
    }
}

/// Deterministic link generator.
pub struct TestLinks;

#[async_trait]
impl LinkGenerator for TestLinks {
    async fn url_for(&self, route_name: &str, params: &[(String, String)]) -> Result<String> {
        let query = params
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        Ok(format!("https://app.test/{route_name}?{query}"))
    }
}

/// Test-only adapter for verified-account reset flows over custom schemas.
///
/// Security takeover tests use the real default atomic store. This adapter
/// exists only so schema-binding fixtures can exercise already-verified
/// behavior without pretending to be a production transaction boundary.
pub struct SequentialFirstProofStore {
    users: Arc<dyn UserStore>,
    tokens: Arc<dyn TokenStore>,
    reset: Arc<dyn PasswordResetStore>,
    remember: Arc<dyn RememberFacade>,
}

impl SequentialFirstProofStore {
    pub fn new(
        users: Arc<dyn UserStore>,
        tokens: Arc<dyn TokenStore>,
        reset: Arc<dyn PasswordResetStore>,
        remember: Arc<dyn RememberFacade>,
    ) -> Self {
        Self {
            users,
            tokens,
            reset,
            remember,
        }
    }
}

#[async_trait]
impl FirstEmailProofStore for SequentialFirstProofStore {
    async fn apply(&self, mutation: FirstEmailProofMutation) -> Result<FirstEmailProofCommit> {
        match mutation {
            FirstEmailProofMutation::PasswordReset {
                token,
                expected_user_id,
                new_password_hash,
            } => {
                let first_proof = match &expected_user_id {
                    Some(user_id) => self
                        .users
                        .find_by_id(user_id)
                        .await?
                        .is_some_and(|user| user.email_verified_at.is_none()),
                    None => false,
                };
                let mut input =
                    PasswordResetInput::new(token, new_password_hash.expose_secret().to_owned());
                if let Some(user_id) = expected_user_id {
                    input = input.expecting_user(user_id);
                }
                let commit = self.reset.apply_password_reset(input).await?;
                let revoked_remember_rows = self.remember.revoke_all(&commit.user_id).await?;
                Ok(FirstEmailProofCommit {
                    user_id: commit.user_id,
                    kind: FirstEmailProofKind::PasswordReset,
                    first_proof,
                    auth_epoch: commit.auth_epoch,
                    revoked_sessions: commit.revoked_sessions,
                    revoked_remember_rows,
                })
            }
            FirstEmailProofMutation::MagicLink { token } => {
                let consumed = self.tokens.consume(token, "magic-link").await?;
                let user_id = consumed.user_id.ok_or_else(|| magnetar::Error::Conflict {
                    resource: "magic-link".to_owned(),
                    message: "fixture token carries no owner".to_owned(),
                })?;
                let user = self.users.find_by_id(&user_id).await?.ok_or_else(|| {
                    magnetar::Error::NotFound {
                        resource: "user".to_owned(),
                        identifier: user_id.clone(),
                    }
                })?;
                let first_proof = user.email_verified_at.is_none();
                if first_proof {
                    self.users
                        .mark_email_verified(&user_id, chrono::Utc::now())
                        .await?;
                }
                Ok(FirstEmailProofCommit {
                    user_id,
                    kind: FirstEmailProofKind::MagicLink,
                    first_proof,
                    auth_epoch: user.auth_epoch,
                    revoked_sessions: 0,
                    revoked_remember_rows: 0,
                })
            }
            FirstEmailProofMutation::OAuthEmailCompletion {
                token: PresentedToken(_),
            } => Err(magnetar::Error::InvalidInput {
                field: "first-email-proof mutation".to_owned(),
                message: "fixture OAuth completion is not composed".to_owned(),
            }),
        }
    }

    async fn create_verified_provider_account(
        &self,
        _input: NewVerifiedProviderAccount,
    ) -> Result<VerifiedProviderAccountCommit> {
        Err(magnetar::Error::InvalidInput {
            field: "verified provider account".to_owned(),
            message: "fixture adapter does not create provider accounts".to_owned(),
        })
    }
}
