//! Shared second-factor gate and single-use challenge handling.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::primary::{AuthenticationContext, FactorGateApproval, VerifiedPrincipal};
use crate::crypto::Encryptor;
use crate::sessions::opaque::{OpaqueSessionProvider, OpaqueSessionStore};
use crate::sessions::{SessionGrant, SessionIssuer};
use crate::storage::{CeremonyStore, NewCeremony};
use crate::{Error, Result};

/// Ceremony namespace used for all primary-auth second-factor challenges.
pub const TWO_FACTOR_CHALLENGE_KIND: &str = "two-factor.challenge";
const CHALLENGE_PENDING: &str = "pending";
const CHALLENGE_APPROVED: &str = "approved";
const CHALLENGE_EXPIRY_MINUTES: i64 = 10;

/// Outcome of passing a verified primary principal through the factor gate.
#[derive(Debug)]
pub enum SignInDecision {
    /// A session was issued because no confirmed second factor is required.
    SessionAllowed(SessionGrant),
    /// A one-time challenge was created; no session exists yet.
    FactorRequired {
        /// Selector used to submit the second-factor proof.
        challenge_selector: String,
    },
}

/// Host-owned second-factor verifier used by the shared gate.
///
/// Implementations may verify TOTP or recovery codes, but they do not receive
/// a session issuer and cannot mint a [`SessionGrant`].
#[async_trait]
pub trait FactorVerifier: Send + Sync {
    /// Return whether this user has a confirmed second-factor enrollment.
    async fn has_confirmed_enrollment(&self, user_id: &str) -> Result<bool>;

    /// Verify a submitted one-time code for the user.
    async fn verify_code(&self, user_id: &str, code: &str) -> Result<bool>;
}

/// The only shared sign-in and challenge-completion boundary.
#[async_trait]
pub trait FactorGate: Send + Sync {
    /// Complete primary sign-in, issuing directly only when no factor
    /// ceremony is required.
    async fn complete_sign_in(
        &self,
        principal: VerifiedPrincipal,
        context: AuthenticationContext,
    ) -> Result<SignInDecision>;

    /// Complete one pending challenge and issue exactly one session on success.
    async fn complete_challenge(&self, selector: &str, code: &str) -> Result<SessionGrant>;
}

/// Concrete factor gate backed by the existing opaque session issuer.
///
/// This type is intentionally provider-neutral: password, magic-link,
/// passkey, OAuth, and device providers all pass the same principal through
/// [`FactorGate::complete_sign_in`].
pub struct OpaqueFactorGate<S, F, O>
where
    S: CeremonyStore,
    F: FactorVerifier,
    O: OpaqueSessionStore,
{
    ceremonies: Arc<S>,
    factors: Arc<F>,
    encryptor: Arc<dyn Encryptor>,
    sessions: Arc<OpaqueSessionProvider<O>>,
}

impl<S, F, O> Clone for OpaqueFactorGate<S, F, O>
where
    S: CeremonyStore,
    F: FactorVerifier,
    O: OpaqueSessionStore,
{
    fn clone(&self) -> Self {
        Self {
            ceremonies: Arc::clone(&self.ceremonies),
            factors: Arc::clone(&self.factors),
            encryptor: Arc::clone(&self.encryptor),
            sessions: Arc::clone(&self.sessions),
        }
    }
}

impl<S, F, O> OpaqueFactorGate<S, F, O>
where
    S: CeremonyStore,
    F: FactorVerifier,
    O: OpaqueSessionStore,
{
    /// Construct a gate from ceremony, factor-verification, encryption, and
    /// opaque-session boundaries owned by the host.
    pub fn new(
        ceremonies: Arc<S>,
        factors: Arc<F>,
        encryptor: Arc<dyn Encryptor>,
        sessions: Arc<OpaqueSessionProvider<O>>,
    ) -> Self {
        Self {
            ceremonies,
            factors,
            encryptor,
            sessions,
        }
    }

    async fn issue(&self, approval: FactorGateApproval) -> Result<SessionGrant> {
        let user_id = approval.user_id.clone();
        let metadata = approval.context.metadata.clone();
        let gate_approval = SessionIssuer::approval_from_factor(approval);
        SessionIssuer
            .issue_opaque(&self.sessions, gate_approval, user_id, metadata, Utc::now())
            .await
    }

    fn selector() -> String {
        format!("challenge-{:032x}", rand::random::<u128>())
    }
}

#[async_trait]
impl<S, F, O> FactorGate for OpaqueFactorGate<S, F, O>
where
    S: CeremonyStore,
    F: FactorVerifier,
    O: OpaqueSessionStore,
{
    async fn complete_sign_in(
        &self,
        principal: VerifiedPrincipal,
        context: AuthenticationContext,
    ) -> Result<SignInDecision> {
        if principal.user_id().is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        if !self
            .factors
            .has_confirmed_enrollment(principal.user_id())
            .await?
        {
            let approval = FactorGateApproval {
                user_id: principal.user_id().to_owned(),
                context: context.clone(),
            };
            let grant = self.issue(approval).await?;
            return Ok(SignInDecision::SessionAllowed(grant));
        }

        let payload = ChallengePayload {
            user_id: principal.user_id().to_owned(),
            context,
        };
        let plaintext = serde_json::to_vec(&payload).map_err(|error| Error::Internal {
            message: format!("challenge payload serialization failed: {error}"),
        })?;
        let payload = self
            .encryptor
            .encrypt(crate::crypto::CryptoPurpose::CeremonyState, &plaintext)?;
        let selector = Self::selector();
        self.ceremonies
            .create(NewCeremony {
                selector: selector.clone(),
                kind: TWO_FACTOR_CHALLENGE_KIND.to_owned(),
                state: CHALLENGE_PENDING.to_owned(),
                payload,
                expires_at: Utc::now() + chrono::Duration::minutes(CHALLENGE_EXPIRY_MINUTES),
            })
            .await?;
        Ok(SignInDecision::FactorRequired {
            challenge_selector: selector,
        })
    }

    async fn complete_challenge(&self, selector: &str, code: &str) -> Result<SessionGrant> {
        if selector.is_empty() {
            return Err(invalid("selector", "must not be empty"));
        }
        if code.is_empty() {
            return Err(invalid("code", "must not be empty"));
        }
        let ceremony = self
            .ceremonies
            .peek(selector, TWO_FACTOR_CHALLENGE_KIND)
            .await?
            .ok_or_else(|| Error::NotFound {
                resource: "two-factor challenge".to_owned(),
                identifier: selector.to_owned(),
            })?;
        if ceremony.state != CHALLENGE_PENDING {
            return Err(Error::Conflict {
                resource: "two-factor challenge".to_owned(),
                message: "challenge is no longer pending".to_owned(),
            });
        }
        let plaintext = self.encryptor.decrypt(
            crate::crypto::CryptoPurpose::CeremonyState,
            &ceremony.payload,
        )?;
        let payload: ChallengePayload =
            serde_json::from_slice(&plaintext).map_err(|error| Error::InvalidInput {
                field: "challenge".to_owned(),
                message: format!("invalid challenge state: {error}"),
            })?;
        if !self.factors.verify_code(&payload.user_id, code).await? {
            return Err(invalid("code", "invalid or expired second-factor code"));
        }
        if !self
            .ceremonies
            .transition(
                selector,
                TWO_FACTOR_CHALLENGE_KIND,
                CHALLENGE_PENDING,
                CHALLENGE_APPROVED,
            )
            .await?
        {
            return Err(Error::Conflict {
                resource: "two-factor challenge".to_owned(),
                message: "challenge was already completed".to_owned(),
            });
        }
        let approval = FactorGateApproval {
            user_id: payload.user_id,
            context: payload.context,
        };
        self.issue(approval).await
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChallengePayload {
    user_id: String,
    context: AuthenticationContext,
}

fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.to_owned(),
    }
}
