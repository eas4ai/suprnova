//! Password recovery: `forgot-password` and `reset-password`.
//!
//! The service keeps reset-link issuance anti-enumerating and delegates
//! completion to the host-owned [`crate::first_email_proof::FirstEmailProofStore`].
//! That boundary consumes the token, rotates the credential, advances the
//! authentication epoch, and revokes opaque sessions and remember-me rows in
//! one transaction. Lockout clearing and the changed-password notification
//! remain explicit post-commit follow-ups.

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde_json::json;

use crate::abuse::AbusePolicy;
use crate::first_email_proof::{FirstEmailProofMutation, FirstEmailProofStore};
use crate::password::{LockoutService, PasswordVerifier, normalize_email, validate_password};
use crate::plugin::{
    EffectResponse, LinkGenerator, MailDriver, MailMessage, Method, Plugin, PluginResult,
    RequestContext, RouteDescriptor, WireResponse,
};
use crate::schema::AuthSchema;
use crate::storage::{IssueToken, PASSWORD_RESET_PURPOSE, PresentedToken, TokenStore, UserStore};
use crate::{Error, Result};

use super::{Gate, acquire, bad_request, body_string, generic_ok};

/// Grounded default reset-link lifetime.
pub const PASSWORD_RESET_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Outcome of one completed reset.
///
/// By the time this value exists the credential rotation, epoch bump, and
/// opaque-session revocation have already committed atomically. The
/// remaining fields report the post-commit follow-ups so a caller that needs
/// to alert or retry does not have to scrape logs.
#[derive(Debug)]
pub struct PasswordResetFlowOutcome {
    /// The user whose password rotated.
    pub user_id: String,
    /// The authentication epoch after the committed rotation.
    pub auth_epoch: u64,
    /// Opaque sessions revoked inside the committed transaction.
    pub revoked_sessions: u64,
    /// Remember-me rows revoked inside the committed transaction.
    pub remember_rows_revoked: u64,
    /// Whether the lockout clear ran; `Ok(true)` reports a true
    /// locked-to-unlocked transition (the recovery path out of lockout).
    pub lockout_cleared: Result<bool>,
}

/// Password-recovery operations shared by routes and the Rust API.
pub struct PasswordManagementService {
    users: Arc<dyn UserStore>,
    tokens: Arc<dyn TokenStore>,
    first_proof: Arc<dyn FirstEmailProofStore>,
    verifier: Arc<PasswordVerifier>,
    lockout: Arc<LockoutService>,
    mail: Arc<dyn MailDriver>,
    links: Arc<dyn LinkGenerator>,
}

impl PasswordManagementService {
    /// Bind the service to its storage, hashing, lockout, and driver
    /// boundaries.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        users: Arc<dyn UserStore>,
        tokens: Arc<dyn TokenStore>,
        first_proof: Arc<dyn FirstEmailProofStore>,
        verifier: Arc<PasswordVerifier>,
        lockout: Arc<LockoutService>,
        mail: Arc<dyn MailDriver>,
        links: Arc<dyn LinkGenerator>,
    ) -> Self {
        Self {
            users,
            tokens,
            first_proof,
            verifier,
            lockout,
            mail,
            links,
        }
    }

    /// Anti-enumeration reset-link entry point: an unknown email mints no
    /// token, dispatches no mail, and still returns `Ok`, so neither the
    /// caller nor a mail-count observer can distinguish absent addresses.
    pub async fn send_link(&self, email: &str) -> Result<()> {
        let normalized = normalize_email(email);
        let Some(user) = self.users.find_by_email(&normalized).await? else {
            return Ok(());
        };
        let issued = self
            .tokens
            .issue(IssueToken {
                user_id: user.user_id.clone(),
                purpose: PASSWORD_RESET_PURPOSE.to_owned(),
                ttl: PASSWORD_RESET_TTL,
            })
            .await?;
        let link = self
            .links
            .url_for(
                "password.reset",
                &[(
                    "token".to_owned(),
                    issued.plaintext.expose_secret().to_owned(),
                )],
            )
            .await?;
        // The reset link is the primary action the user is waiting on, so a
        // transport failure propagates instead of being swallowed.
        self.mail
            .send(MailMessage {
                name: "password_reset".to_owned(),
                recipient: user.email.clone(),
                payload: json!({
                    "email": user.email,
                    "reset_link": link,
                }),
            })
            .await
    }

    /// Non-consuming liveness check for reset landing pages.
    pub async fn check(&self, token: &str) -> Result<bool> {
        self.tokens
            .check(PresentedToken::new(token), PASSWORD_RESET_PURPOSE)
            .await
    }

    /// Consume the token and rotate the credential, returning the user id.
    /// See [`PasswordManagementService::complete_with_outcome`] for the
    /// variant that surfaces the post-commit follow-up results.
    pub async fn complete(&self, token: &str, new_password: &str) -> Result<String> {
        self.complete_with_outcome(token, new_password)
            .await
            .map(|outcome| outcome.user_id)
    }

    /// The full reset delegates token consumption, password rotation, epoch
    /// advance, opaque-session revocation, and remember-me revocation to one
    /// host-owned transaction. Lockout clearing and the changed-password
    /// notification remain post-commit and cannot un-reset the password.
    pub async fn complete_with_outcome(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<PasswordResetFlowOutcome> {
        validate_password(new_password)?;
        let hash = self
            .verifier
            .mint_target(&secrecy::SecretString::from(new_password.to_owned()))?;
        let commit = self
            .first_proof
            .apply(FirstEmailProofMutation::PasswordReset {
                token: PresentedToken::new(token),
                expected_user_id: None,
                new_password_hash: secrecy::SecretString::from(hash),
            })
            .await
            .map_err(|error| match error {
                Error::NotFound { .. } => invalid_token(),
                other => other,
            })?;

        let user = self.users.find_by_id(&commit.user_id).await;
        let lockout_cleared = match &user {
            Ok(Some(user)) => {
                let result = self
                    .lockout
                    .unlock_account(&normalize_email(&user.email))
                    .await;
                if let Err(error) = &result {
                    tracing::warn!(
                        user_id = %commit.user_id,
                        error = %error,
                        "lockout clear failed after password reset"
                    );
                }
                result
            }
            Ok(None) => Err(Error::NotFound {
                resource: "user".to_owned(),
                identifier: commit.user_id.clone(),
            }),
            Err(error) => Err(error.clone()),
        };

        // Fire-and-forget security notification: a vanished user or a mail
        // failure is logged and never rolls back the committed reset.
        match user {
            Ok(Some(user)) => {
                if let Err(error) = self
                    .mail
                    .send(MailMessage {
                        name: "password_changed".to_owned(),
                        recipient: user.email.clone(),
                        payload: json!({"email": user.email}),
                    })
                    .await
                {
                    tracing::warn!(
                        user_id = %commit.user_id,
                        error = %error,
                        "password-changed notification failed"
                    );
                }
            }
            Ok(None) => tracing::warn!(
                user_id = %commit.user_id,
                "password-changed notification skipped: user not found after reset"
            ),
            Err(error) => tracing::warn!(
                user_id = %commit.user_id,
                error = %error,
                "password-changed notification skipped: lookup failed"
            ),
        }

        Ok(PasswordResetFlowOutcome {
            user_id: commit.user_id,
            auth_epoch: commit.auth_epoch,
            revoked_sessions: commit.revoked_sessions,
            remember_rows_revoked: commit.revoked_remember_rows,
            lockout_cleared,
        })
    }
}

fn invalid_token() -> Error {
    Error::InvalidInput {
        field: "token".to_owned(),
        message: "invalid or expired reset token".to_owned(),
    }
}

/// Route-level configuration for the recovery plugin.
#[derive(Clone, Copy, Debug)]
pub struct PasswordManagementPluginConfig {
    /// Abuse budget for `forgot-password`.
    pub forgot_policy: AbusePolicy,
}

impl Default for PasswordManagementPluginConfig {
    fn default() -> Self {
        Self {
            forgot_policy: AbusePolicy {
                max_requests: 3,
                window: std::time::Duration::from_secs(3600),
            },
        }
    }
}

/// The recovery route plugin: `forgot-password` and `reset-password`.
pub struct PasswordManagementPlugin {
    service: Arc<PasswordManagementService>,
    config: PasswordManagementPluginConfig,
}

impl PasswordManagementPlugin {
    /// Compose the plugin over the shared service.
    pub fn new(
        service: Arc<PasswordManagementService>,
        config: PasswordManagementPluginConfig,
    ) -> Self {
        Self { service, config }
    }
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for PasswordManagementPlugin {
    fn name(&self) -> &str {
        "password-management"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![
            RouteDescriptor::new(Method::Post, "/forgot-password", "password.forgot")
                .with_feature("password-management"),
            RouteDescriptor::new(Method::Post, "/reset-password", "password.reset")
                .with_feature("password-management"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        match context.request.path.trim_matches('/') {
            "forgot-password" => {
                let Some(email) = body_string(context.request, "email") else {
                    return Ok(bad_request("email is required"));
                };
                let identity = normalize_email(&email);
                match acquire(
                    &context,
                    "password.forgot",
                    &identity,
                    self.config.forgot_policy,
                )
                .await
                {
                    Gate::Proceed => {}
                    Gate::Respond(response) => return Ok(response),
                }
                self.service.send_link(&email).await?;
                Ok(WireResponse::from_effects(EffectResponse::json(
                    generic_ok(),
                )))
            }
            "reset-password" => {
                let Some(token) = body_string(context.request, "token") else {
                    return Ok(bad_request("token is required"));
                };
                let Some(password) = body_string(context.request, "password") else {
                    return Ok(bad_request("password is required"));
                };
                match self.service.complete_with_outcome(&token, &password).await {
                    Ok(_) => Ok(WireResponse::from_effects(EffectResponse::json(
                        generic_ok(),
                    ))),
                    Err(Error::InvalidInput { message, .. }) => Ok(bad_request(&message)),
                    Err(other) => Err(other.into()),
                }
            }
            other => Err(crate::plugin::PluginError::RouteNotFound {
                path: other.to_owned(),
            }),
        }
    }
}
