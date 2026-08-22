//! Shared recording/counting fakes used by every domain harness
//! (`password_harness.rs`, `oauth_harness.rs`, ...). Touches only
//! unconditional modules (`magnetar::plugin`, `magnetar::abuse`), so it is
//! safe to include from suites that enable no plugin feature at all.

#![allow(dead_code)]

use async_trait::async_trait;
use std::sync::Arc;

use magnetar::Result;
use magnetar::abuse::{AbuseLimiter, AbusePolicy, Permit};
use magnetar::crypto::{CryptoPurpose, Encryptor};
use magnetar::first_email_proof::{
    FirstEmailProofCommit, FirstEmailProofKind, FirstEmailProofMutation, FirstEmailProofOutcome,
    FirstEmailProofStore, NewVerifiedProviderAccount, VerifiedProviderAccountCommit,
};
use magnetar::plugin::{LinkGenerator, MailDriver, MailMessage};
use magnetar::sessions::RememberFacade;
use magnetar::storage::{
    CeremonyStore, LinkedAccountInitializer, NewLinkedAccount, NewUser, PasswordResetInput,
    PasswordResetStore, TokenStore, UserStore,
};
use parking_lot::Mutex;
use secrecy::ExposeSecret;
use serde::Deserialize;
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
#[cfg(feature = "oauth")]
#[derive(Deserialize)]
struct FixtureOAuthBinding {
    pending_id: String,
    normalized_email: String,
}

#[cfg(feature = "oauth")]
#[derive(Deserialize)]
struct FixturePendingIdentity {
    provider: String,
    subject: String,
    sibling_key: String,
}

pub struct SequentialFirstProofStore {
    users: Arc<dyn UserStore>,
    tokens: Arc<dyn TokenStore>,
    accounts: Arc<dyn LinkedAccountInitializer>,
    reset: Option<Arc<dyn PasswordResetStore>>,
    remember: Option<Arc<dyn RememberFacade>>,
    ceremonies: Option<Arc<dyn CeremonyStore>>,
    encryptor: Option<Arc<dyn Encryptor>>,
}

impl SequentialFirstProofStore {
    pub fn new(
        users: Arc<dyn UserStore>,
        tokens: Arc<dyn TokenStore>,
        accounts: Arc<dyn LinkedAccountInitializer>,
        reset: Arc<dyn PasswordResetStore>,
        remember: Arc<dyn RememberFacade>,
    ) -> Self {
        Self {
            users,
            tokens,
            accounts,
            reset: Some(reset),
            remember: Some(remember),
            ceremonies: None,
            encryptor: None,
        }
    }

    pub fn for_oauth(
        users: Arc<dyn UserStore>,
        tokens: Arc<dyn TokenStore>,
        accounts: Arc<dyn LinkedAccountInitializer>,
        ceremonies: Arc<dyn CeremonyStore>,
        encryptor: Arc<dyn Encryptor>,
    ) -> Self {
        Self {
            users,
            tokens,
            accounts,
            reset: None,
            remember: None,
            ceremonies: Some(ceremonies),
            encryptor: Some(encryptor),
        }
    }
}

#[async_trait]
impl FirstEmailProofStore for SequentialFirstProofStore {
    async fn apply(&self, mutation: FirstEmailProofMutation) -> Result<FirstEmailProofOutcome> {
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
                let reset = self
                    .reset
                    .as_ref()
                    .ok_or_else(|| magnetar::Error::Internal {
                        message: "fixture password-reset boundary is unavailable".to_owned(),
                    })?;
                let remember = self
                    .remember
                    .as_ref()
                    .ok_or_else(|| magnetar::Error::Internal {
                        message: "fixture remember boundary is unavailable".to_owned(),
                    })?;
                let commit = reset.apply_password_reset(input).await?;
                let revoked_remember_rows = remember.revoke_all(&commit.user_id).await?;
                Ok(FirstEmailProofOutcome::Committed(FirstEmailProofCommit {
                    user_id: commit.user_id,
                    kind: FirstEmailProofKind::PasswordReset,
                    first_proof,
                    auth_epoch: commit.auth_epoch,
                    provider_account_id: None,
                    revoked_sessions: commit.revoked_sessions,
                    revoked_remember_rows,
                }))
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
                Ok(FirstEmailProofOutcome::Committed(FirstEmailProofCommit {
                    user_id,
                    kind: FirstEmailProofKind::MagicLink,
                    first_proof,
                    auth_epoch: user.auth_epoch,
                    provider_account_id: None,
                    revoked_sessions: 0,
                    revoked_remember_rows: 0,
                }))
            }
            FirstEmailProofMutation::OAuthEmailCompletion { token } => {
                #[cfg(feature = "oauth")]
                {
                    let ceremonies =
                        self.ceremonies
                            .as_ref()
                            .ok_or_else(|| magnetar::Error::Internal {
                                message: "fixture ceremony boundary is unavailable".to_owned(),
                            })?;
                    let encryptor =
                        self.encryptor
                            .as_ref()
                            .ok_or_else(|| magnetar::Error::Internal {
                                message: "fixture encryptor boundary is unavailable".to_owned(),
                            })?;
                    let consumed = self.tokens.consume(token, "oauth-email-completion").await?;
                    let binding_record = ceremonies
                        .consume(&consumed.token_id, "oauth.email-completion")
                        .await?
                        .ok_or_else(|| magnetar::Error::NotFound {
                            resource: "fixture OAuth binding".to_owned(),
                            identifier: consumed.token_id,
                        })?;
                    let binding_plaintext =
                        encryptor.decrypt(CryptoPurpose::CeremonyState, &binding_record.payload)?;
                    let binding: FixtureOAuthBinding = serde_json::from_slice(&binding_plaintext)
                        .map_err(|error| {
                        magnetar::Error::Internal {
                            message: format!("decode fixture OAuth binding: {error}"),
                        }
                    })?;
                    let pending_record = ceremonies
                        .consume(&binding.pending_id, "oauth.pending-identity")
                        .await?
                        .ok_or_else(|| magnetar::Error::NotFound {
                            resource: "fixture pending identity".to_owned(),
                            identifier: binding.pending_id,
                        })?;
                    let pending_plaintext =
                        encryptor.decrypt(CryptoPurpose::CeremonyState, &pending_record.payload)?;
                    let pending: FixturePendingIdentity =
                        serde_json::from_slice(&pending_plaintext).map_err(|error| {
                            magnetar::Error::Internal {
                                message: format!("decode fixture pending identity: {error}"),
                            }
                        })?;
                    if consumed.user_id.as_deref() != Some(pending.sibling_key.as_str()) {
                        return Err(magnetar::Error::Conflict {
                            resource: "fixture OAuth completion".to_owned(),
                            message: "token does not match pending identity".to_owned(),
                        });
                    }
                    if self
                        .users
                        .find_by_email(&binding.normalized_email)
                        .await?
                        .is_some()
                    {
                        return Ok(FirstEmailProofOutcome::ExplicitLinkRequired {
                            normalized_email: binding.normalized_email,
                        });
                    }
                    let user = self
                        .users
                        .create_user(NewUser {
                            email: binding.normalized_email,
                            password_hash: None,
                        })
                        .await?;
                    self.users
                        .mark_email_verified(&user.user_id, chrono::Utc::now())
                        .await?;
                    self.accounts
                        .initialize(NewLinkedAccount {
                            user_id: user.user_id.clone(),
                            provider: pending.provider,
                            provider_account_id: pending.subject.clone(),
                        })
                        .await?;
                    Ok(FirstEmailProofOutcome::Committed(FirstEmailProofCommit {
                        user_id: user.user_id,
                        kind: FirstEmailProofKind::OAuthEmailCompletion,
                        first_proof: false,
                        auth_epoch: user.auth_epoch,
                        provider_account_id: Some(pending.subject),
                        revoked_sessions: 0,
                        revoked_remember_rows: 0,
                    }))
                }
                #[cfg(not(feature = "oauth"))]
                {
                    let _ = token;
                    Err(magnetar::Error::InvalidInput {
                        field: "first-email-proof mutation".to_owned(),
                        message: "fixture OAuth feature is disabled".to_owned(),
                    })
                }
            }
        }
    }

    async fn create_verified_provider_account(
        &self,
        input: NewVerifiedProviderAccount,
    ) -> Result<VerifiedProviderAccountCommit> {
        if let Some(existing) = self
            .accounts
            .find_by_provider_subject(&input.provider, &input.provider_account_id)
            .await?
        {
            let user = self
                .users
                .find_by_id(&existing.user_id)
                .await?
                .ok_or_else(|| magnetar::Error::Conflict {
                    resource: "fixture provider account".to_owned(),
                    message: "linked provider has no user".to_owned(),
                })?;
            return Ok(VerifiedProviderAccountCommit {
                user_id: user.user_id,
                auth_epoch: user.auth_epoch,
            });
        }
        let provider = input.provider;
        let provider_account_id = input.provider_account_id;
        let user = self
            .users
            .create_user(NewUser {
                email: input.email,
                password_hash: None,
            })
            .await?;
        self.users
            .mark_email_verified(&user.user_id, chrono::Utc::now())
            .await?;
        match self
            .accounts
            .initialize(NewLinkedAccount {
                user_id: user.user_id.clone(),
                provider: provider.clone(),
                provider_account_id: provider_account_id.clone(),
            })
            .await
        {
            Ok(_) => Ok(VerifiedProviderAccountCommit {
                user_id: user.user_id,
                auth_epoch: user.auth_epoch,
            }),
            Err(magnetar::Error::Conflict { .. }) => {
                let winner = self
                    .accounts
                    .find_by_provider_subject(&provider, &provider_account_id)
                    .await?
                    .ok_or_else(|| magnetar::Error::Conflict {
                        resource: "fixture provider account".to_owned(),
                        message: "provider create conflicted without a winner".to_owned(),
                    })?;
                let winner = self
                    .users
                    .find_by_id(&winner.user_id)
                    .await?
                    .ok_or_else(|| magnetar::Error::Conflict {
                        resource: "fixture provider account".to_owned(),
                        message: "winning provider link has no user".to_owned(),
                    })?;
                Ok(VerifiedProviderAccountCommit {
                    user_id: winner.user_id,
                    auth_epoch: winner.auth_epoch,
                })
            }
            Err(error) => Err(error),
        }
    }
}
