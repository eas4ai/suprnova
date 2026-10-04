//! Shared second-factor gate and single-use challenge handling.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::primary::{AuthenticationContext, FactorGateApproval, SignInMethod, VerifiedPrincipal};
use crate::crypto::Encryptor;
use crate::sessions::opaque::{OpaqueSessionProvider, OpaqueSessionStore};
use crate::sessions::{SessionGrant, SessionIssuer, SessionMetadata};
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

/// A verifier-owned proof prepared without consuming one-time factor state.
///
/// The wrapper exposes only whether verification succeeded. Its inner value
/// is never formatted, so TOTP/recovery claim material and fake proof tokens
/// cannot leak through logs or assertion failures.
pub struct PreparedFactorProof<P> {
    inner: P,
    valid: bool,
}

impl<P> PreparedFactorProof<P> {
    /// Wrap proof material that verified during the read-only prepare phase.
    pub fn valid(inner: P) -> Self {
        Self { inner, valid: true }
    }

    /// Wrap verifier-owned material for an invalid submitted proof.
    ///
    /// The material is passed to the conditional claim phase so the verifier
    /// can apply its normal failed-attempt accounting without approving the
    /// challenge.
    pub fn invalid(inner: P) -> Self {
        Self {
            inner,
            valid: false,
        }
    }

    fn is_valid(&self) -> bool {
        self.valid
    }

    fn into_inner(self) -> P {
        self.inner
    }
}

impl<P> std::fmt::Debug for PreparedFactorProof<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedFactorProof")
            .field("valid", &self.valid)
            .field("inner", &"[REDACTED]")
            .finish()
    }
}

/// Host-owned second-factor verifier used by the shared gate.
///
/// Implementations prepare proof claim material without consuming it. Only
/// the caller that wins the challenge transition receives
/// [`FactorVerifier::claim_prepared`], so a losing concurrent completion
/// cannot burn a distinct valid TOTP or recovery proof.
#[async_trait]
pub trait FactorVerifier: Send + Sync {
    /// Verifier-owned claim material carried between preparation and claim.
    type PreparedProof: Send;

    /// Return whether this user has a confirmed second-factor enrollment.
    async fn has_confirmed_enrollment(&self, user_id: &str) -> Result<bool>;

    /// Return whether this user has a second-factor enrollment at all,
    /// confirmed or waiting for its confirmation.
    ///
    /// A host that keeps a second factor of its own asks this before it
    /// enrolls one, so an account never holds both. The default answers for
    /// confirmed enrollments only; verifiers that keep pending enrollments
    /// override it.
    async fn has_enrollment(&self, user_id: &str) -> Result<bool> {
        self.has_confirmed_enrollment(user_id).await
    }

    /// Read and verify a submitted code without consuming one-time state.
    async fn prepare_code(
        &self,
        user_id: &str,
        code: &str,
    ) -> Result<PreparedFactorProof<Self::PreparedProof>>;

    /// Conditionally consume prepared proof material.
    ///
    /// Invalid prepared material is also routed here to complete the verifier
    /// protocol, but the gate never approves its challenge. Implementations
    /// reserve attempt capacity in [`Self::prepare_code`] before reading proof.
    async fn claim_prepared(&self, user_id: &str, proof: Self::PreparedProof) -> Result<bool>;

    /// Cancel a prepared proof when the surrounding ceremony cannot commit.
    ///
    /// The default fails closed so legacy verifier implementations remain
    /// source compatible without silently leaking reserved attempt capacity.
    async fn cancel_prepared(&self, user_id: &str, proof: Self::PreparedProof) -> Result<()> {
        let _ = (user_id, proof);
        Err(Error::DependencyUnavailable {
            dependency: "factor verifier".to_owned(),
            message: "prepared-proof cancellation is unavailable".to_owned(),
        })
    }
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
            .map_err(map_opaque_issuance_error)
    }

    fn selector() -> String {
        format!("challenge-{:032x}", rand::random::<u128>())
    }

    /// Refuse a host sign-in that would bypass this gate's factor policy.
    ///
    /// Hosts call this before they evaluate a proof of their own, so a
    /// refusal neither consumes the proof nor fires the host's login
    /// events. [`Self::complete_host_sign_in`] repeats the check when it
    /// issues.
    ///
    /// # Errors
    ///
    /// [`Error::Conflict`] when the user has a confirmed second factor in
    /// this gate's verifier, and the verifier's storage errors.
    pub async fn check_host_sign_in(&self, user_id: &str) -> Result<()> {
        if user_id.is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        if self.factors.has_confirmed_enrollment(user_id).await? {
            return Err(Error::Conflict {
                resource: "host sign-in".to_owned(),
                message: "the user has a second factor the host did not verify".to_owned(),
            });
        }
        Ok(())
    }

    /// Issue a session for a user whose sign-in the host application
    /// completed itself, outside every Magnetar provider.
    ///
    /// **Host-trusted.** This mints a session from a bare user id, as
    /// Laravel's `Auth::loginUsingId` does: the caller asserts that it
    /// authenticated the user. Only the host that composed this concrete
    /// gate can call it. Providers and plugins receive an
    /// `Arc<dyn FactorGate>`, whose trait has no such method, so no plugin
    /// context can reach it. Never expose it to code that does not own the
    /// application's authentication decision.
    ///
    /// A host can keep a credential check of its own - a session guard over
    /// its own user table, or its own TOTP store - and still route its
    /// sessions through this store, because revocation, auth epochs and web
    /// bindings answer to the store. The issuance stays behind the gate's
    /// factor policy (see [`Self::check_host_sign_in`]).
    ///
    /// `auth_epoch` is the user's epoch as the host read it when it checked
    /// the user's credential. Issuance fails once it is no longer current,
    /// so a password reset or a sign-out-everywhere between that check and
    /// this call cancels the sign-in.
    ///
    /// # Errors
    ///
    /// [`Error::Conflict`] when the user has a confirmed second factor,
    /// [`Error::InvalidInput`] for an empty user id or a stale epoch, and the
    /// session store's issuance errors.
    pub async fn complete_host_sign_in(
        &self,
        user_id: &str,
        auth_epoch: u64,
        metadata: SessionMetadata,
    ) -> Result<SessionGrant> {
        self.check_host_sign_in(user_id).await?;
        let approval = FactorGateApproval {
            user_id: user_id.to_owned(),
            context: AuthenticationContext::new(metadata, auth_epoch, Utc::now()),
        };
        self.issue(approval).await
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
        mut context: AuthenticationContext,
    ) -> Result<SignInDecision> {
        if principal.user_id().is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        context.auth_epoch = principal.context().auth_epoch;
        let factor_satisfied = matches!(principal.method(), SignInMethod::Remembered);
        if factor_satisfied
            || !self
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
                message: format!(
                    "invalid challenge state: {}",
                    crate::crypto::decode_failure(&error)
                ),
            })?;
        let prepared = self.factors.prepare_code(&payload.user_id, code).await?;
        if !prepared.is_valid() {
            self.factors
                .claim_prepared(&payload.user_id, prepared.into_inner())
                .await?;
            return Err(invalid("code", "invalid or expired second-factor code"));
        }
        let proof = prepared.into_inner();
        let transition = self
            .ceremonies
            .transition(
                selector,
                TWO_FACTOR_CHALLENGE_KIND,
                CHALLENGE_PENDING,
                CHALLENGE_APPROVED,
            )
            .await;
        let transitioned = match transition {
            Ok(transitioned) => transitioned,
            Err(transition_error) => {
                if let Err(cancel_error) =
                    self.factors.cancel_prepared(&payload.user_id, proof).await
                {
                    tracing::error!(
                        error = %cancel_error,
                        original_error = %transition_error,
                        "failed to cancel second-factor proof after ceremony transition aborted"
                    );
                    return Err(cancel_error);
                }
                return Err(transition_error);
            }
        };
        if !transitioned {
            self.factors
                .cancel_prepared(&payload.user_id, proof)
                .await?;
            return Err(Error::Conflict {
                resource: "two-factor challenge".to_owned(),
                message: "challenge was already completed".to_owned(),
            });
        }
        let proof_claimed = match self.factors.claim_prepared(&payload.user_id, proof).await {
            Ok(claimed) => claimed,
            Err(error) => {
                let restored = self
                    .ceremonies
                    .transition(
                        selector,
                        TWO_FACTOR_CHALLENGE_KIND,
                        CHALLENGE_APPROVED,
                        CHALLENGE_PENDING,
                    )
                    .await;
                if !matches!(restored, Ok(true)) {
                    tracing::error!(
                        ?restored,
                        "failed to restore challenge after proof claim error"
                    );
                }
                return Err(error);
            }
        };
        if !proof_claimed {
            match self
                .ceremonies
                .transition(
                    selector,
                    TWO_FACTOR_CHALLENGE_KIND,
                    CHALLENGE_APPROVED,
                    CHALLENGE_PENDING,
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("failed to restore unclaimed second-factor challenge")
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to restore unclaimed second-factor challenge")
                }
            }
            return Err(invalid("code", "invalid or expired second-factor code"));
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

fn map_opaque_issuance_error(error: Error) -> Error {
    match error {
        Error::NotFound {
            resource,
            identifier,
        } if resource == "credential actor" && identifier == "expired or revoked" => {
            invalid("credentials", "invalid credentials")
        }
        error => error,
    }
}

fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_issuance_maps_only_stale_actor_to_invalid_credentials() {
        let stale = Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        };
        assert_eq!(
            map_opaque_issuance_error(stale),
            Error::InvalidInput {
                field: "credentials".to_owned(),
                message: "invalid credentials".to_owned(),
            }
        );

        let other = Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "different failure".to_owned(),
        };
        assert_eq!(
            map_opaque_issuance_error(other),
            Error::NotFound {
                resource: "credential actor".to_owned(),
                identifier: "different failure".to_owned(),
            }
        );
    }
}
