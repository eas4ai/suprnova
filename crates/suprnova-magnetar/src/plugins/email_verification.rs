//! Email verification: send, anti-enumeration resend, non-consuming check,
//! and consuming verify.
//!
//! Ported from `auth_flows::email_verify`: `resend` mints and mails nothing
//! for an unknown address while returning the same generic outcome; `verify`
//! consumes the single-use token first and then stamps the verification
//! timestamp, so a failed stamp burns the token rather than leaving a
//! reusable one behind. Verified state gates nothing inside Magnetar; the
//! host middleware remains the enforcement point.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::abuse::AbusePolicy;
use crate::password::normalize_email;
use crate::plugin::{
    EffectResponse, LinkGenerator, MailDriver, MailMessage, Method, Plugin, PluginResult,
    RequestContext, RouteDescriptor, WireResponse,
};
use crate::schema::AuthSchema;
use crate::storage::{IssueToken, PresentedToken, TokenStore, UserStore};
use crate::{Error, Result};

use super::{Gate, acquire, bad_request, body_string, generic_ok};

/// Purpose namespace for verification tokens in the unified token store.
pub const EMAIL_VERIFICATION_PURPOSE: &str = "email-verification";

/// Grounded default verification-link lifetime.
pub const EMAIL_VERIFICATION_TTL: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);

/// Email-verification operations shared by routes and registration.
pub struct EmailVerificationService {
    users: Arc<dyn UserStore>,
    tokens: Arc<dyn TokenStore>,
    mail: Arc<dyn MailDriver>,
    links: Arc<dyn LinkGenerator>,
}

impl EmailVerificationService {
    /// Bind the service to user storage, the token store, and mail/link
    /// drivers. Hosts pass the same driver instances they hand the plugin
    /// context.
    pub fn new(
        users: Arc<dyn UserStore>,
        tokens: Arc<dyn TokenStore>,
        mail: Arc<dyn MailDriver>,
        links: Arc<dyn LinkGenerator>,
    ) -> Self {
        Self {
            users,
            tokens,
            mail,
            links,
        }
    }

    /// Mint a verification token for a known user and dispatch the mail.
    ///
    /// The plaintext token exists only between issuance and the outgoing
    /// link; it is never logged or persisted.
    pub async fn send_link(&self, user_id: &str, email: &str) -> Result<()> {
        let issued = self
            .tokens
            .issue(IssueToken {
                user_id: user_id.to_owned(),
                purpose: EMAIL_VERIFICATION_PURPOSE.to_owned(),
                ttl: EMAIL_VERIFICATION_TTL,
            })
            .await?;
        let link = self
            .links
            .url_for(
                "verification.verify",
                &[
                    ("id".to_owned(), user_id.to_owned()),
                    (
                        "hash".to_owned(),
                        secrecy::ExposeSecret::expose_secret(&issued.plaintext).to_owned(),
                    ),
                ],
            )
            .await?;
        self.mail
            .send(MailMessage {
                name: "email_verification".to_owned(),
                recipient: email.to_owned(),
                payload: json!({
                    "email": email,
                    "verification_link": link,
                }),
            })
            .await
    }

    /// Anti-enumeration resend: an unknown address mints no token, sends no
    /// mail, and still returns `Ok`.
    pub async fn resend(&self, email: &str) -> Result<()> {
        let normalized = normalize_email(email);
        let Some(user) = self.users.find_by_email(&normalized).await? else {
            return Ok(());
        };
        self.send_link(&user.user_id, &user.email).await
    }

    /// Non-consuming liveness check for landing pages, so a page refresh
    /// does not burn the token.
    pub async fn check(&self, token: &str) -> Result<bool> {
        self.tokens
            .check(PresentedToken::new(token), EMAIL_VERIFICATION_PURPOSE)
            .await
    }

    /// Consume the token, require it to belong to the presented user, and
    /// stamp `email_verified_at`. Single-use under concurrency through the
    /// store's conditional consume.
    pub async fn verify(&self, presented_user_id: &str, token: &str) -> Result<String> {
        let consumed = self
            .tokens
            .consume(PresentedToken::new(token), EMAIL_VERIFICATION_PURPOSE)
            .await
            .map_err(|error| match error {
                Error::NotFound { .. } | Error::Conflict { .. } => invalid_token(),
                other => other,
            })?;
        let user_id = consumed.user_id.ok_or_else(invalid_token)?;
        if user_id != presented_user_id {
            return Err(invalid_token());
        }
        self.users.mark_email_verified(&user_id, Utc::now()).await?;
        Ok(user_id)
    }
}

fn invalid_token() -> Error {
    Error::InvalidInput {
        field: "token".to_owned(),
        message: "invalid or expired verification token".to_owned(),
    }
}

#[cfg(feature = "password")]
#[async_trait]
impl super::password::RegistrationVerification for EmailVerificationService {
    async fn send_for_new_user(&self, user_id: &str, email: &str) -> Result<()> {
        self.send_link(user_id, email).await
    }
}

/// Route-level configuration for the verification plugin.
#[derive(Clone, Copy, Debug)]
pub struct EmailVerificationPluginConfig {
    /// Abuse budget for the resend route.
    pub resend_policy: AbusePolicy,
}

impl Default for EmailVerificationPluginConfig {
    fn default() -> Self {
        Self {
            resend_policy: AbusePolicy {
                max_requests: 3,
                window: std::time::Duration::from_secs(3600),
            },
        }
    }
}

/// The verification route plugin: `email/verify/{id}/{hash}` and
/// `email/verification-notification`.
pub struct EmailVerificationPlugin {
    service: Arc<EmailVerificationService>,
    config: EmailVerificationPluginConfig,
}

impl EmailVerificationPlugin {
    /// Compose the plugin over the shared service.
    pub fn new(
        service: Arc<EmailVerificationService>,
        config: EmailVerificationPluginConfig,
    ) -> Self {
        Self { service, config }
    }
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for EmailVerificationPlugin {
    fn name(&self) -> &str {
        "email-verification"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![
            RouteDescriptor::new(
                Method::Get,
                "/email/verify/{id}/{hash}",
                "verification.verify",
            )
            .with_feature("email-verification"),
            RouteDescriptor::new(
                Method::Post,
                "/email/verification-notification",
                "verification.send",
            )
            .with_feature("email-verification"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        if context.request.path.trim_matches('/') == "email/verification-notification" {
            let Some(email) = body_string(context.request, "email") else {
                return Ok(bad_request("email is required"));
            };
            let identity = normalize_email(&email);
            match acquire(
                &context,
                "email.verification-resend",
                &identity,
                self.config.resend_policy,
            )
            .await
            {
                Gate::Proceed => {}
                Gate::Respond(response) => return Ok(response),
            }
            self.service.resend(&email).await?;
            return Ok(WireResponse::from_effects(EffectResponse::json(
                generic_ok(),
            )));
        }

        let (Some(id), Some(hash)) = (
            context.request.path_params.get("id").cloned(),
            context.request.path_params.get("hash").cloned(),
        ) else {
            return Ok(bad_request("missing verification parameters"));
        };
        let Some(session) = context.session else {
            return Ok(bad_request("authenticated verification is required"));
        };
        if session.user_id() != id {
            return Ok(bad_request("invalid or expired verification token"));
        }
        match self.service.verify(session.user_id(), &hash).await {
            Ok(_) => Ok(WireResponse::from_effects(EffectResponse::json(json!({
                "status": "verified",
            })))),
            Err(Error::InvalidInput { message, .. }) => Ok(bad_request(&message)),
            Err(other) => Err(other.into()),
        }
    }
}
