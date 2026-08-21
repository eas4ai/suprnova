//! Email completion: a Magnetar-minted, single-use mailed token proving
//! ownership of an email address for providers whose identity carries no
//! trusted email (`docs/specs/suprnova-magnetar/09-oauth-engine.md`).
//!
//! [`EmailCompletionService::resend`] (the only mail-emitting entry point --
//! [`request`](EmailCompletionService::request) is a thin gated alias) mints
//! a fresh [`TokenStore`] token per attempt, keyed by the pending record's
//! stable `sibling_key` in its `user_id` slot so that consuming any one of
//! them stamps every other outstanding token for that pending identity as
//! used (`TokenStore`'s sibling invalidation, restored deliberately -- see
//! the doc comment on [`EmailCompletionService::resend`]). `sibling_key` is
//! a separate, decimal-numeric handle from `pending_id` -- `pending_id`
//! itself is a high-entropy, caller-visible hex selector unsuitable for a
//! numeric-FK `user_id` column (see [`super::identity::PendingIdentityPayload::sibling_key`]).
//!
//! The submitted email is bound to a specific token via an encrypted
//! [`CeremonyStore`] record keyed by the token's own `token_id` -- not a
//! client-supplied link parameter, which would let an attacker mail
//! themselves a real token and then swap in a victim's address before
//! consuming it. `consume` finalizes atomically: the token, then the email
//! binding, then the pending identity are each consumed exactly once, so a
//! stale second click always fails generically once the pending identity is
//! gone -- nothing is ever created twice.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::authorization::encrypt;
use super::identity::{IdentityOutcome, peek_pending_identity};
use crate::abuse::{AbuseLimiter, AbusePolicy, Permit};
use crate::crypto::Encryptor;
use crate::first_email_proof::{
    FirstEmailProofMutation, FirstEmailProofOutcome, FirstEmailProofStore,
};
use crate::mail;
use crate::password::normalize_email;
use crate::plugin::{LinkGenerator, MailDriver};
use crate::storage::{CeremonyStore, IssueToken, NewCeremony, PresentedToken, TokenStore};
use crate::{Error, Result};

/// Purpose namespace for email-completion tokens in the unified token
/// store.
pub const OAUTH_EMAIL_COMPLETION_PURPOSE: &str = "oauth-email-completion";

/// Grounded token lifetime (spec 09: "a 24-hour TTL").
pub const OAUTH_EMAIL_COMPLETION_TTL: StdDuration = StdDuration::from_secs(24 * 60 * 60);

/// Ceremony kind namespace for the per-token `(pending_id, email)` binding.
pub(crate) const OAUTH_EMAIL_COMPLETION_BINDING_KIND: &str = "oauth.email-completion";

/// Route name resolved through [`LinkGenerator`] for the mailed completion
/// link.
const OAUTH_EMAIL_COMPLETION_ROUTE: &str = "oauth.email-completion.verify";

/// Purpose namespace for the resend abuse limiter.
const OAUTH_EMAIL_COMPLETION_RESEND_PURPOSE: &str = "oauth-email-completion-resend";

/// What a specific token is bound to. Keyed by the token's own `token_id`.
/// `pending_id` rides here (rather than in `IssueToken::user_id`, which
/// carries the stable `sibling_key` instead) so `consume` can still reach
/// the pending record after a sibling-invalidated token is rejected.
#[derive(Serialize, Deserialize)]
pub(crate) struct BindingPayload {
    pub(crate) pending_id: String,
    pub(crate) normalized_email: String,
}

/// Route-level configuration for [`EmailCompletionService`].
#[derive(Clone, Copy, Debug)]
pub struct EmailCompletionConfig {
    /// Abuse budget consulted before every send.
    pub resend_policy: AbusePolicy,
}

impl Default for EmailCompletionConfig {
    fn default() -> Self {
        Self {
            resend_policy: AbusePolicy {
                max_requests: 5,
                window: StdDuration::from_secs(3600),
            },
        }
    }
}

/// Email-completion operations: gated send, anti-enumeration resend, and
/// consuming completion.
pub struct EmailCompletionService {
    ceremonies: Arc<dyn CeremonyStore>,
    tokens: Arc<dyn TokenStore>,
    first_proof: Arc<dyn FirstEmailProofStore>,
    encryptor: Arc<dyn Encryptor>,
    mail: Arc<dyn MailDriver>,
    links: Arc<dyn LinkGenerator>,
    limiter: Arc<dyn AbuseLimiter>,
    config: EmailCompletionConfig,
}

impl EmailCompletionService {
    /// Bind the service to ceremony/token storage, the atomic first-proof
    /// boundary, ceremony-state encryption, mail/link drivers, and limiter.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ceremonies: Arc<dyn CeremonyStore>,
        tokens: Arc<dyn TokenStore>,
        first_proof: Arc<dyn FirstEmailProofStore>,
        encryptor: Arc<dyn Encryptor>,
        mail: Arc<dyn MailDriver>,
        links: Arc<dyn LinkGenerator>,
        limiter: Arc<dyn AbuseLimiter>,
        config: EmailCompletionConfig,
    ) -> Self {
        Self {
            ceremonies,
            tokens,
            first_proof,
            encryptor,
            mail,
            links,
            limiter,
            config,
        }
    }

    /// Mint and mail a completion token binding `email` to the pending
    /// provider identity `pending_id`. A thin alias for [`Self::resend`] --
    /// there is no unlimited first-send path: `pending_id` is
    /// caller-visible material (handed back in
    /// [`IdentityOutcome::EmailCompletionRequired`], alive for
    /// [`super::identity::OAUTH_PENDING_IDENTITY_TTL`]) that could
    /// otherwise be replayed against arbitrary victim addresses.
    ///
    /// # Errors
    ///
    /// Same as [`Self::resend`].
    pub async fn request(&self, pending_id: &str, email: &str) -> Result<()> {
        self.resend(pending_id, email).await
    }

    /// Mint and mail a fresh completion token, gated by the resend abuse
    /// budget. The exact same acquisition and generic `Ok` outcome run
    /// whether or not `pending_id` still has a live pending identity or
    /// `email` matches anything -- a limiter observation or backend
    /// failure never reveals which. A successful consume of any token
    /// minted for one `pending_id` invalidates every other outstanding
    /// token for that same pending identity (sibling invalidation, keyed
    /// on the pending record's stable `sibling_key`) -- an old, unclicked
    /// resend link stops working the moment a newer one is redeemed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the abuse budget is exhausted and
    /// [`Error::DependencyUnavailable`] when the limiter backend fails
    /// (fail closed: no token is minted, no mail is sent).
    pub async fn resend(&self, pending_id: &str, email: &str) -> Result<()> {
        let key = resend_key(pending_id, email);
        match self.limiter.acquire(&key, self.config.resend_policy).await {
            Ok(Permit::Allowed { .. }) => {}
            Ok(Permit::Rejected { .. }) => {
                return Err(Error::Conflict {
                    resource: "oauth.email-completion.resend".to_owned(),
                    message: "too many resend attempts, retry later".to_owned(),
                });
            }
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "oauth email-completion resend abuse limiter unavailable; failing closed"
                );
                return Err(Error::DependencyUnavailable {
                    dependency: "abuse-limiter".to_owned(),
                    message: error.to_string(),
                });
            }
        }
        self.send(pending_id, email).await
    }

    async fn send(&self, pending_id: &str, email: &str) -> Result<()> {
        let normalized = normalize_email(email);
        if normalized.is_empty() {
            return Err(Error::InvalidInput {
                field: "email".to_owned(),
                message: "must not be empty".to_owned(),
            });
        }
        // Anti-enumeration: an unknown/expired pending_id mints and mails
        // nothing but still returns Ok below.
        let Some(pending) = peek_pending_identity(
            self.ceremonies.as_ref(),
            self.encryptor.as_ref(),
            pending_id,
        )
        .await?
        else {
            return Ok(());
        };

        // `sibling_key` (not `pending_id`, which is a high-entropy hex
        // selector) is reused as `IssueToken::user_id`: it is a stable,
        // decimal-numeric handle minted once with the pending record and
        // read back on every send/resend, so `TokenStore::consume_in`'s
        // sibling invalidation stamps every other outstanding token for
        // this pending identity used in the same transaction -- exactly
        // the "resend kills prior links" property spec 02 assigns to the
        // primitive itself.
        let issued = self
            .tokens
            .issue(IssueToken {
                user_id: pending.sibling_key.clone(),
                purpose: OAUTH_EMAIL_COMPLETION_PURPOSE.to_owned(),
                ttl: OAUTH_EMAIL_COMPLETION_TTL,
            })
            .await?;

        // The submitted email must be cryptographically bound to *this*
        // token; keying the encrypted binding on the token's own
        // `token_id` keeps that binding independent of the sibling
        // invalidation above: an invalidated sibling token's binding
        // simply becomes unreachable (its token can never be consumed
        // again) and expires naturally with the ceremony's own TTL.
        let binding = BindingPayload {
            pending_id: pending_id.to_owned(),
            normalized_email: normalized.clone(),
        };
        let ciphertext = encrypt(self.encryptor.as_ref(), &binding)?;
        self.ceremonies
            .create(NewCeremony {
                selector: issued.token_id.clone(),
                kind: OAUTH_EMAIL_COMPLETION_BINDING_KIND.to_owned(),
                state: "pending".to_owned(),
                payload: ciphertext,
                expires_at: chrono::Utc::now()
                    + chrono::Duration::from_std(OAUTH_EMAIL_COMPLETION_TTL)
                        .expect("OAUTH_EMAIL_COMPLETION_TTL fits in chrono::Duration"),
            })
            .await?;

        let link = self
            .links
            .url_for(
                OAUTH_EMAIL_COMPLETION_ROUTE,
                &[(
                    "token".to_owned(),
                    issued.plaintext.expose_secret().to_owned(),
                )],
            )
            .await?;
        self.mail
            .send(mail::oauth_email_completion(&normalized, &link))
            .await
    }

    /// Consume a mailed completion token through the atomic first-proof
    /// boundary. A verified collision remains explicit-link-only.
    pub async fn consume(&self, token: &str) -> Result<IdentityOutcome> {
        match self
            .first_proof
            .apply(FirstEmailProofMutation::OAuthEmailCompletion {
                token: PresentedToken::new(token),
            })
            .await
            .map_err(|error| match error {
                Error::NotFound { .. } | Error::Conflict { .. } => invalid_completion(),
                other => other,
            })? {
            FirstEmailProofOutcome::ExplicitLinkRequired { normalized_email } => {
                Ok(IdentityOutcome::ExplicitLinkRequired { normalized_email })
            }
            FirstEmailProofOutcome::Committed(commit) => {
                let provider_account_id =
                    commit.provider_account_id.ok_or_else(invalid_completion)?;
                if commit.first_proof {
                    Ok(IdentityOutcome::Link {
                        actor_user_id: commit.user_id,
                        provider_account_id,
                    })
                } else {
                    Ok(IdentityOutcome::Create {
                        user_id: commit.user_id,
                        provider_account_id,
                    })
                }
            }
        }
    }
}

/// Keyed only on `pending_id`, deliberately: the budget must be shared
/// across every address an attacker rotates through for one pending
/// identity, not reset per address. Keying on `(pending_id, email)` would
/// let one live `pending_id` buy `max_requests` mails per distinct
/// address, unbounded overall -- exactly the vector I2 exists to close.
fn resend_key(pending_id: &str, _email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pending_id.as_bytes());
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{OAUTH_EMAIL_COMPLETION_RESEND_PURPOSE}:{digest}")
}

fn invalid_completion() -> Error {
    Error::InvalidInput {
        field: "token".to_owned(),
        message: "invalid or expired email-completion token".to_owned(),
    }
}
