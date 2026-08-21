//! The `MagicLinkAuthProvider` plugin: passwordless sign-in by emailed
//! single-use link.
//!
//! Ported from torii's magic-link service and the frozen Suprnova facade:
//! open-policy minting runs `get_or_create` (a first-time email is a
//! sign-up), the service surface returns the freshly minted plaintext to
//! app-owned delivery exactly once, and the mounted route mails the link
//! through the typed mail driver. Consume is atomic single-use, stamps
//! `email_verified_at` (FLAGGED hardening: clicking a link mailed to the
//! address is the same ownership proof as 05's verification), and passes
//! the principal through the shared factor gate — never straight to a
//! session.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;

use crate::abuse::AbusePolicy;
use crate::auth::{
    AuthenticationContext, FactorGate, SignInDecision, SignInMethod, VerifiedPrincipal,
};
use crate::first_email_proof::{FirstEmailProofMutation, FirstEmailProofStore};
use crate::password::normalize_email;
use crate::plugin::{
    Effect, EffectResponse, LinkGenerator, MailDriver, Method, Plugin, PluginResult,
    RequestContext, RouteDescriptor, WireResponse,
};
use crate::schema::AuthSchema;
use crate::sessions::SessionMetadata;
use crate::storage::{IssueToken, NewUser, PresentedToken, TokenStore, UserStore};
use crate::{Error, Result};

use super::{Gate, acquire, bad_request, body_string, generic_ok, request_metadata};

/// Purpose namespace for magic-link tokens in the unified store.
pub const MAGIC_LINK_PURPOSE: &str = "magic-link";

/// Grounded default link lifetime (torii's 15 minutes).
pub const MAGIC_LINK_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Who may receive a magic link.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RegistrationPolicy {
    /// Torii's grounded default: every syntactically valid email yields a
    /// user (created passwordless on first use) and a token.
    #[default]
    Open,
    /// FLAGGED new capability: no user is created; an absent email mints
    /// nothing and mails nothing behind the same generic outcome.
    ExistingOnly,
}

/// The outcome of one send, surfaced to the trusted app only.
///
/// The `Debug` form never exposes the minted plaintext.
///
/// Route responses stay generic for both variants; app-owned delivery
/// (the frozen facade posture) consumes `Minted` to build and mail its own
/// link and treats `Suppressed` as the same generic success.
pub enum MagicLinkIssued {
    /// A token was minted; the plaintext is exposed exactly once here.
    Minted(SecretString),
    /// Existing-only policy with an absent email: nothing was minted.
    Suppressed,
}

impl std::fmt::Debug for MagicLinkIssued {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Minted(_) => formatter.write_str("Minted([redacted])"),
            Self::Suppressed => formatter.write_str("Suppressed"),
        }
    }
}

/// Magic-link operations shared by the route surface and the Rust API.
pub struct MagicLinkService {
    users: Arc<dyn UserStore>,
    tokens: Arc<dyn TokenStore>,
    first_proof: Arc<dyn FirstEmailProofStore>,
    gate: Arc<dyn FactorGate>,
    policy: RegistrationPolicy,
}

impl MagicLinkService {
    /// Bind the service to user storage, token issuance, the atomic first-proof
    /// store, the shared factor gate, and a registration policy.
    pub fn new(
        users: Arc<dyn UserStore>,
        tokens: Arc<dyn TokenStore>,
        first_proof: Arc<dyn FirstEmailProofStore>,
        gate: Arc<dyn FactorGate>,
        policy: RegistrationPolicy,
    ) -> Self {
        Self {
            users,
            tokens,
            first_proof,
            gate,
            policy,
        }
    }

    /// Mint a magic-link token per the registration policy.
    ///
    /// The plaintext leaves storage exactly once, inside the returned
    /// [`MagicLinkIssued::Minted`]; it is never logged or persisted.
    pub async fn issue(&self, email: &str) -> Result<MagicLinkIssued> {
        let normalized = normalize_email(email);
        if normalized.is_empty() {
            return Err(Error::InvalidInput {
                field: "email".to_owned(),
                message: "must not be empty".to_owned(),
            });
        }
        let user = match self.users.find_by_email(&normalized).await? {
            Some(user) => user,
            None => match self.policy {
                RegistrationPolicy::ExistingOnly => return Ok(MagicLinkIssued::Suppressed),
                RegistrationPolicy::Open => {
                    // Torii's get-or-create: a first-time email is a signup.
                    let created = self
                        .users
                        .create_user(NewUser {
                            email: normalized.clone(),
                            password_hash: None,
                        })
                        .await?;
                    // Open mode requires a binding that can represent a
                    // passwordless account. A binding that surfaces a
                    // credential where none was stored could let an
                    // attacker-chosen password authenticate later; refuse
                    // loudly instead of persisting that row silently.
                    if created.password_hash.is_some() {
                        return Err(Error::Internal {
                            message: "user binding cannot represent passwordless accounts; \
                                      open magic-link registration is unavailable"
                                .to_owned(),
                        });
                    }
                    created
                }
            },
        };
        let issued = self
            .tokens
            .issue(IssueToken {
                user_id: user.user_id,
                purpose: MAGIC_LINK_PURPOSE.to_owned(),
                ttl: MAGIC_LINK_TTL,
            })
            .await?;
        Ok(MagicLinkIssued::Minted(issued.plaintext))
    }

    /// Atomically consume a magic link, apply any first-proof cleanup, and
    /// pass the committed principal through the shared factor gate.
    pub async fn consume(&self, token: &str, metadata: SessionMetadata) -> Result<SignInDecision> {
        let commit = self
            .first_proof
            .apply(FirstEmailProofMutation::MagicLink {
                token: PresentedToken::new(token),
            })
            .await
            .map_err(|error| match error {
                Error::NotFound { .. } | Error::Conflict { .. } => invalid_link(),
                other => other,
            })?;
        let principal = VerifiedPrincipal::new(
            commit.user_id,
            SignInMethod::MagicLink,
            AuthenticationContext::new(metadata, commit.auth_epoch, Utc::now()),
        )?;
        let context = principal.context().clone();
        self.gate.complete_sign_in(principal, context).await
    }
}

fn invalid_link() -> Error {
    Error::InvalidInput {
        field: "token".to_owned(),
        message: "invalid or expired magic link".to_owned(),
    }
}

/// Route-level configuration for the magic-link plugin.
#[derive(Clone, Copy, Debug)]
pub struct MagicLinkPluginConfig {
    /// Abuse budget for the send route.
    pub send_policy: AbusePolicy,
}

impl Default for MagicLinkPluginConfig {
    fn default() -> Self {
        Self {
            send_policy: AbusePolicy {
                max_requests: 3,
                window: std::time::Duration::from_secs(3600),
            },
        }
    }
}

/// The magic-link route plugin: `magic-link` and `magic-link/verify`.
pub struct MagicLinkPlugin {
    service: Arc<MagicLinkService>,
    mail: Arc<dyn MailDriver>,
    links: Arc<dyn LinkGenerator>,
    config: MagicLinkPluginConfig,
}

impl MagicLinkPlugin {
    /// Compose the plugin over the shared service and delivery drivers.
    pub fn new(
        service: Arc<MagicLinkService>,
        mail: Arc<dyn MailDriver>,
        links: Arc<dyn LinkGenerator>,
        config: MagicLinkPluginConfig,
    ) -> Self {
        Self {
            service,
            mail,
            links,
            config,
        }
    }
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for MagicLinkPlugin {
    fn name(&self) -> &str {
        "magic-link"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![
            RouteDescriptor::new(Method::Post, "/magic-link", "magic-link.send")
                .with_feature("magic-link"),
            RouteDescriptor::new(Method::Get, "/magic-link/verify", "magic-link.verify")
                .with_feature("magic-link"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        match context.request.path.trim_matches('/') {
            "magic-link" => {
                let Some(email) = body_string(context.request, "email") else {
                    return Ok(bad_request("email is required"));
                };
                let identity = normalize_email(&email);
                match acquire(
                    &context,
                    "magic-link.send",
                    &identity,
                    self.config.send_policy,
                )
                .await
                {
                    Gate::Proceed => {}
                    Gate::Respond(response) => return Ok(response),
                }
                // Plugin-owned delivery: mint, build the link, and dispatch
                // exactly one typed message. Suppressed sends mint and mail
                // nothing; the response stays byte-identical.
                if let MagicLinkIssued::Minted(plaintext) = self.service.issue(&email).await? {
                    let link = self
                        .links
                        .url_for(
                            "magic-link.verify",
                            &[("token".to_owned(), plaintext.expose_secret().to_owned())],
                        )
                        .await?;
                    self.mail
                        .send(crate::mail::magic_link(&identity, &link))
                        .await?;
                }
                Ok(WireResponse::from_effects(EffectResponse::json(
                    generic_ok(),
                )))
            }
            "magic-link/verify" => {
                let Some(token) = context.request.query.get("token").cloned() else {
                    return Ok(bad_request("token is required"));
                };
                let metadata = request_metadata(context.request);
                match self.service.consume(&token, metadata).await {
                    Ok(SignInDecision::SessionAllowed(grant)) => Ok(WireResponse::from_effects(
                        EffectResponse::json(generic_ok())
                            .with_effect(Effect::EstablishSession(grant)),
                    )),
                    Ok(SignInDecision::FactorRequired { challenge_selector }) => {
                        Ok(WireResponse::from_effects(EffectResponse::json(json!({
                            "two_factor_required": true,
                            "challenge_selector": challenge_selector,
                        }))))
                    }
                    Err(Error::InvalidInput { .. }) => {
                        let mut response = EffectResponse::json(json!({
                            "message": "invalid or expired magic link",
                        }));
                        response.status = 401;
                        Ok(WireResponse::from_effects(response))
                    }
                    Err(other) => Err(other.into()),
                }
            }
            other => Err(crate::plugin::PluginError::RouteNotFound {
                path: other.to_owned(),
            }),
        }
    }
}
