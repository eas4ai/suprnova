//! Identity resolution and account linking
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s "Identity resolution
//! and linking" section).
//!
//! [`VerifiedProviderIdentity`] is the input seam this task owns in place of
//! a real `OAuthProvider` trait (Task 3): a provider plugin verifies a
//! callback's userinfo/ID-token and hands this crate a
//! `VerifiedProviderIdentity` value. [`IdentityResolver::resolve`] never
//! performs I/O beyond the storage/ceremony boundaries below -- no HTTP,
//! no provider-name branch.
//!
//! Five distinct outcomes ([`IdentityOutcome`]): known `(provider, subject)`
//! resolves to `SignIn` with a bare [`VerifiedPrincipal`] -- passing it
//! through [`crate::auth::FactorGate`] is the caller's job, not this
//! module's, so identity resolution stays testable without session
//! machinery. Unknown identity with a *provider-verified* matching email
//! either links (policy-gated, default deny -> `ExplicitLinkRequired`) or
//! creates. An **unverified** provider email is deliberately treated
//! identically to a **missing** one: neither is ever used to search for or
//! attach to an existing account (the takeover vector spec 09 requires
//! dead), so both fall through to `EmailCompletionRequired`, mirroring the
//! no-email providers (X, TikTok, withheld-Facebook) this outcome was
//! designed for.

use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::authorization::{OAuthIntent, decrypt, encrypt, new_selector};
use crate::auth::{AuthenticationContext, SignInMethod, VerifiedPrincipal};
use crate::crypto::Encryptor;
use crate::password::normalize_email;
use crate::sessions::SessionMetadata;
use crate::storage::{
    CeremonyStore, LinkedAccountStore, NewCeremony, NewLinkedAccount, NewUser, UserStore,
};
use crate::{Error, Result};

/// Ceremony kind namespace for the pending record minted when a provider
/// identity has no trusted email (`Providers that return no email ...
/// shall resolve to a fifth outcome`).
pub const OAUTH_PENDING_IDENTITY_KIND: &str = "oauth.pending-identity";

/// Pending-identity lifetime: generous enough to cover several
/// [`super::email_completion::EmailCompletionService::resend`] attempts, well
/// beyond any single mailed token's 24-hour TTL.
pub const OAUTH_PENDING_IDENTITY_TTL: chrono::Duration = chrono::Duration::days(7);

/// A provider-verified identity, already authenticated by an `OAuthProvider`
/// plugin (Task 3). Task 2 defines this seam and consumes it purely as
/// data; tests drive [`IdentityResolver`] with fakes constructed directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedProviderIdentity {
    /// Provider key (the `{provider}` route segment).
    pub provider: String,
    /// The provider's stable account identifier (`sub` for OIDC, `id` for
    /// GitHub-style providers).
    pub subject: String,
    /// The provider's reported email address, if any.
    pub email: Option<String>,
    /// Whether the provider asserts `email` is verified. Ignored (treated
    /// as absent) whenever `email` is `None`.
    pub email_verified: bool,
    /// The provider's reported display name, if any.
    pub display_name: Option<String>,
}

/// Whether an unknown identity with a provider-verified matching email may
/// auto-link instead of requiring an explicit, authenticated link flow.
///
/// [`Self::ExplicitLinkRequired`] is the grounded safe default (ux.md's
/// approved policy; FLAGGED hardening over the unconditional legacy path).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AutoLinkPolicy {
    /// Never auto-link; a matching verified email always fails safe.
    #[default]
    ExplicitLinkRequired,
    /// Auto-link a verified matching email to the existing user.
    AutoLink,
}

/// The five distinct identity-resolution outcomes.
pub enum IdentityOutcome {
    /// A known `(provider, subject)` resolved to an existing user. Passing
    /// this through [`crate::auth::FactorGate::complete_sign_in`] is the
    /// caller's responsibility.
    SignIn(VerifiedPrincipal),
    /// Unknown identity with no usable email match created a new user and
    /// linked account.
    Create {
        /// The newly created user's identifier.
        user_id: String,
        /// The provider's account identifier (echoed input).
        provider_account_id: String,
    },
    /// An identity was attached to an existing user: either an explicit
    /// link intent, or an unknown sign-in identity whose verified email
    /// matched under [`AutoLinkPolicy::AutoLink`].
    Link {
        /// The user the identity was attached to.
        actor_user_id: String,
        /// The provider's account identifier (echoed input).
        provider_account_id: String,
    },
    /// A verified email matched an existing user but policy refused to
    /// auto-link (the safe default). No account state changed. An
    /// unverified email never reaches this variant -- it is routed to
    /// [`Self::EmailCompletionRequired`] before any lookup runs (see the
    /// module doc's "unverified == absent" invariant).
    ExplicitLinkRequired {
        /// The normalized email that matched.
        normalized_email: String,
    },
    /// The identity carries no trusted email; a mailed proof-of-ownership
    /// token must be completed before any account changes.
    EmailCompletionRequired {
        /// Selector of the pending-identity ceremony
        /// ([`OAUTH_PENDING_IDENTITY_KIND`]) that
        /// [`super::email_completion::EmailCompletionService`] consumes.
        pending_id: String,
    },
}

/// Payload persisted for an [`IdentityOutcome::EmailCompletionRequired`]
/// pending record. `pub(crate)` so [`super::email_completion`] can decrypt
/// and finalize it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PendingIdentityPayload {
    pub(crate) provider: String,
    pub(crate) subject: String,
    pub(crate) display_name: Option<String>,
    /// The provider's claimed (untrusted: absent or unverified) email, kept
    /// only as a completion-form pre-fill hint. Never used for matching.
    pub(crate) claimed_email: Option<String>,
    /// A stable, decimal-numeric identifier minted once alongside the
    /// pending record and read back on every send/resend. Used (not
    /// `pending_id`) as `IssueToken::user_id` so `TokenStore`'s sibling
    /// invalidation groups every completion token minted for this pending
    /// identity together, regardless of `pending_id`'s hex shape (which a
    /// numeric-FK token schema cannot store as `user_id`).
    pub(crate) sibling_key: String,
}

/// Attempt to create a linked-account row; on a driver-level unique
/// violation, re-read the winning row instead of failing. Concurrent
/// identical `(provider, provider_account_id)` writes are expected under
/// spec 01's driver-enforced uniqueness -- "someone else won" is a normal
/// outcome here, not an error.
async fn create_or_read_linked_account(
    accounts: &dyn LinkedAccountStore,
    input: NewLinkedAccount,
) -> Result<crate::storage::LinkedAccountRecord> {
    match accounts.create(input.clone()).await {
        Ok(record) => Ok(record),
        Err(Error::Conflict { .. }) => accounts
            .find_by_provider_subject(&input.provider, &input.provider_account_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: "linked-account create conflicted but no winning row exists".to_owned(),
            }),
        Err(other) => Err(other),
    }
}

/// Create a user and attach a provider identity to it. Shared by
/// [`IdentityResolver::resolve`]'s `Create` branch and
/// [`super::email_completion::EmailCompletionService::consume`]'s finalize
/// step; both call sites already hold a *trusted* (provider-verified, or
/// app-mailed-and-clicked) email by the time they reach here.
///
/// If a concurrent resolution already linked this exact
/// `(provider, provider_account_id)` to a different user between this
/// call's user creation and its link attempt, the just-created user row is
/// left orphaned (no linked account) and the *winning* user id is returned
/// instead -- "someone else won, continue as them" per spec 01's
/// driver-enforced uniqueness.
pub(crate) async fn create_user_and_link(
    users: &dyn UserStore,
    accounts: &dyn LinkedAccountStore,
    provider: &str,
    provider_account_id: &str,
    email: &str,
) -> Result<String> {
    let user = users
        .create_user(NewUser {
            email: email.to_owned(),
            password_hash: None,
        })
        .await?;
    let record = create_or_read_linked_account(
        accounts,
        NewLinkedAccount {
            user_id: user.user_id.clone(),
            provider: provider.to_owned(),
            provider_account_id: provider_account_id.to_owned(),
        },
    )
    .await?;
    Ok(record.user_id)
}

/// Identity resolution and linking, driven by a [`VerifiedProviderIdentity`]
/// and the ceremony's [`OAuthIntent`].
pub struct IdentityResolver {
    users: Arc<dyn UserStore>,
    accounts: Arc<dyn LinkedAccountStore>,
    ceremonies: Arc<dyn CeremonyStore>,
    encryptor: Arc<dyn Encryptor>,
    policy: AutoLinkPolicy,
}

impl IdentityResolver {
    /// Bind the resolver to user/linked-account/ceremony storage,
    /// ceremony-state encryption, and the auto-link policy.
    pub fn new(
        users: Arc<dyn UserStore>,
        accounts: Arc<dyn LinkedAccountStore>,
        ceremonies: Arc<dyn CeremonyStore>,
        encryptor: Arc<dyn Encryptor>,
        policy: AutoLinkPolicy,
    ) -> Self {
        Self {
            users,
            accounts,
            ceremonies,
            encryptor,
            policy,
        }
    }

    /// Resolve a verified provider identity against the given ceremony
    /// intent, returning exactly one of the five [`IdentityOutcome`]
    /// variants.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for an empty provider/subject or
    /// empty link actor id, [`Error::NotFound`] when a link intent's actor
    /// no longer exists, and [`Error::Conflict`] when a link intent's
    /// identity is already attached to a *different* user.
    pub async fn resolve(
        &self,
        identity: VerifiedProviderIdentity,
        intent: OAuthIntent,
        metadata: SessionMetadata,
    ) -> Result<IdentityOutcome> {
        if identity.provider.is_empty() {
            return Err(invalid("provider", "must not be empty"));
        }
        if identity.subject.is_empty() {
            return Err(invalid("subject", "must not be empty"));
        }

        let existing = self
            .accounts
            .find_by_provider_subject(&identity.provider, &identity.subject)
            .await?;

        match intent {
            OAuthIntent::Link { actor_user_id } => {
                self.resolve_link(identity, actor_user_id, existing).await
            }
            OAuthIntent::SignIn => self.resolve_sign_in(identity, existing, metadata).await,
        }
    }

    async fn resolve_link(
        &self,
        identity: VerifiedProviderIdentity,
        actor_user_id: String,
        existing: Option<crate::storage::LinkedAccountRecord>,
    ) -> Result<IdentityOutcome> {
        if actor_user_id.is_empty() {
            return Err(invalid(
                "actor_user_id",
                "must not be empty for a link intent",
            ));
        }
        if let Some(account) = existing {
            return if account.user_id == actor_user_id {
                // Idempotent: already linked to the same actor.
                Ok(IdentityOutcome::Link {
                    actor_user_id,
                    provider_account_id: identity.subject,
                })
            } else {
                Err(Error::Conflict {
                    resource: "linked-account".to_owned(),
                    message: "provider identity is already linked to a different account"
                        .to_owned(),
                })
            };
        }
        self.users
            .find_by_id(&actor_user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                resource: "user".to_owned(),
                identifier: actor_user_id.clone(),
            })?;
        let record = create_or_read_linked_account(
            self.accounts.as_ref(),
            NewLinkedAccount {
                user_id: actor_user_id.clone(),
                provider: identity.provider,
                provider_account_id: identity.subject.clone(),
            },
        )
        .await?;
        if record.user_id != actor_user_id {
            return Err(Error::Conflict {
                resource: "linked-account".to_owned(),
                message: "provider identity is already linked to a different account".to_owned(),
            });
        }
        Ok(IdentityOutcome::Link {
            actor_user_id,
            provider_account_id: identity.subject,
        })
    }

    async fn resolve_sign_in(
        &self,
        identity: VerifiedProviderIdentity,
        existing: Option<crate::storage::LinkedAccountRecord>,
        metadata: SessionMetadata,
    ) -> Result<IdentityOutcome> {
        if let Some(account) = existing {
            let user = self
                .users
                .find_by_id(&account.user_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    resource: "user".to_owned(),
                    identifier: account.user_id.clone(),
                })?;
            let principal = VerifiedPrincipal::new(
                user.user_id,
                SignInMethod::OAuth,
                AuthenticationContext::new(metadata, user.auth_epoch, Utc::now()),
            )?;
            return Ok(IdentityOutcome::SignIn(principal));
        }

        // Unverified provider email is deliberately treated as absent: it
        // is never used to search for or attach to an existing account.
        let trusted_email = match (&identity.email, identity.email_verified) {
            (Some(email), true) => Some(normalize_email(email)),
            _ => None,
        };

        let Some(normalized) = trusted_email else {
            let pending_id = self.create_pending_identity(&identity).await?;
            return Ok(IdentityOutcome::EmailCompletionRequired { pending_id });
        };

        match self.users.find_by_email(&normalized).await? {
            Some(user) => match self.policy {
                AutoLinkPolicy::ExplicitLinkRequired => Ok(IdentityOutcome::ExplicitLinkRequired {
                    normalized_email: normalized,
                }),
                AutoLinkPolicy::AutoLink => {
                    let record = create_or_read_linked_account(
                        self.accounts.as_ref(),
                        NewLinkedAccount {
                            user_id: user.user_id.clone(),
                            provider: identity.provider,
                            provider_account_id: identity.subject.clone(),
                        },
                    )
                    .await?;
                    Ok(IdentityOutcome::Link {
                        actor_user_id: record.user_id,
                        provider_account_id: identity.subject,
                    })
                }
            },
            None => {
                let user_id = create_user_and_link(
                    self.users.as_ref(),
                    self.accounts.as_ref(),
                    &identity.provider,
                    &identity.subject,
                    &normalized,
                )
                .await?;
                Ok(IdentityOutcome::Create {
                    user_id,
                    provider_account_id: identity.subject,
                })
            }
        }
    }

    async fn create_pending_identity(&self, identity: &VerifiedProviderIdentity) -> Result<String> {
        let selector = new_selector("oauth-pending");
        let payload = PendingIdentityPayload {
            provider: identity.provider.clone(),
            subject: identity.subject.clone(),
            display_name: identity.display_name.clone(),
            claimed_email: identity.email.clone(),
            sibling_key: crate::storage::random_id(),
        };
        let ciphertext = encrypt(self.encryptor.as_ref(), &payload)?;
        self.ceremonies
            .create(NewCeremony {
                selector: selector.clone(),
                kind: OAUTH_PENDING_IDENTITY_KIND.to_owned(),
                state: "pending".to_owned(),
                payload: ciphertext,
                expires_at: Utc::now() + OAUTH_PENDING_IDENTITY_TTL,
            })
            .await?;
        Ok(selector)
    }
}

/// Peek and decrypt a still-live pending-identity record without consuming
/// it. `pub(crate)` for [`super::email_completion`].
pub(crate) async fn peek_pending_identity(
    ceremonies: &dyn CeremonyStore,
    encryptor: &dyn Encryptor,
    pending_id: &str,
) -> Result<Option<PendingIdentityPayload>> {
    let Some(record) = ceremonies
        .peek(pending_id, OAUTH_PENDING_IDENTITY_KIND)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(decrypt(encryptor, &record.payload)?))
}

/// Consume and decrypt a pending-identity record. `pub(crate)` for
/// [`super::email_completion`].
pub(crate) async fn consume_pending_identity(
    ceremonies: &dyn CeremonyStore,
    encryptor: &dyn Encryptor,
    pending_id: &str,
) -> Result<Option<PendingIdentityPayload>> {
    let Some(record) = ceremonies
        .consume(pending_id, OAUTH_PENDING_IDENTITY_KIND)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(decrypt(encryptor, &record.payload)?))
}

fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.to_owned(),
    }
}
