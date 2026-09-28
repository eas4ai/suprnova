//! TOTP two-factor authentication: enrollment, confirmation, matched-step
//! verification, recovery codes, and the factor-gate wiring.
//!
//! A near-whole adoption of the deployed `auth_flows::two_factor`:
//! enrollment is inactive until confirmed, secrets and recovery codes are
//! ciphertext under their distinct purposes, every code-checking path is
//! gated on 05's lockout accounting, and rotation paths demand proof of
//! possession. The one FLAGGED deviation is replay protection: the
//! verifier records the timestep that actually matched and rejects
//! `matched_step <= last_used_timestep`, closing the forward-edge replay
//! the deployed `current + skew` stamp permitted. 2FA is a factor, never a
//! sign-in method: it cannot start a session alone and never counts in the
//! census.

pub mod recovery;
pub mod store;
pub mod totp;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};

use crate::auth::{FactorVerifier, PreparedFactorProof};
use crate::crypto::{CryptoPurpose, Encryptor};
use crate::password::{AttemptAdmission, LockoutService, normalize_email};
use crate::storage::{CredentialActor, UserStore};
use crate::{Error, Result};

pub use store::{TwoFactorProofClaim, TwoFactorRow, TwoFactorStore};

/// Two-factor configuration (the `APP_NAME` lineage).
#[derive(Clone, Debug)]
pub struct TwoFactorConfig {
    /// Issuer label rendered by authenticator apps.
    pub issuer: String,
}

impl Default for TwoFactorConfig {
    /// The deployed default issuer.
    fn default() -> Self {
        Self {
            issuer: "Suprnova".to_owned(),
        }
    }
}

/// Successful enrollment payload. The recovery codes and otpauth URL are
/// shown to the user exactly once; there is no API for retrieving them
/// again.
#[derive(Clone)]
pub struct EnrollmentResponse {
    /// The otpauth URL (carries the raw secret in its query string).
    pub otpauth_url: SecretString,
    /// Inline-SVG QR code of the otpauth payload.
    pub qr_code_svg: String,
    /// Plaintext recovery codes, single display.
    pub recovery_codes: Vec<String>,
}

struct PreparedEnrollment {
    response: EnrollmentResponse,
    secret_ciphertext: Vec<u8>,
    recovery_ciphertext: Vec<u8>,
}

/// Hand-written so a stray `dbg!` or traced response cannot leak the
/// secret-bearing URL or the plaintext codes (the deployed discipline).
impl std::fmt::Debug for EnrollmentResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnrollmentResponse")
            .field("otpauth_url", &"[redacted]")
            .field("qr_code_svg", &"[svg]")
            .field("recovery_codes", &"[redacted]")
            .finish()
    }
}

enum ProofMaterial {
    Invalid,
    Totp {
        matched_step: i64,
    },
    Recovery {
        expected_ciphertext: Vec<u8>,
        remaining_codes: Vec<String>,
    },
}

enum PreparedTwoFactorClaim {
    Invalid,
    Totp {
        matched_step: i64,
    },
    Recovery {
        expected_ciphertext: Vec<u8>,
        next_ciphertext: Option<Vec<u8>>,
    },
}

/// Redacted claim material prepared by [`TwoFactorService`] for the factor
/// gate's challenge-owner CAS.
pub struct PreparedTwoFactorProof {
    user_id: String,
    lockout_identity: String,
    admission: AttemptAdmission,
    claim: PreparedTwoFactorClaim,
}

impl std::fmt::Debug for PreparedTwoFactorProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedTwoFactorProof")
            .field("user_id", &"[REDACTED]")
            .field("lockout_identity", &"[REDACTED]")
            .field("admission", &"[REDACTED]")
            .field("claim", &"[REDACTED]")
            .finish()
    }
}

/// The two-factor lifecycle service.
pub struct TwoFactorService {
    store: Arc<dyn TwoFactorStore>,
    users: Arc<dyn UserStore>,
    lockout: Arc<LockoutService>,
    encryptor: Arc<dyn Encryptor>,
    config: TwoFactorConfig,
}

impl TwoFactorService {
    /// Bind the service to its storage, lockout, and encryption boundaries.
    pub fn new(
        store: Arc<dyn TwoFactorStore>,
        users: Arc<dyn UserStore>,
        lockout: Arc<LockoutService>,
        encryptor: Arc<dyn Encryptor>,
        config: TwoFactorConfig,
    ) -> Self {
        Self {
            store,
            users,
            lockout,
            encryptor,
            config,
        }
    }

    /// Begin enrollment: mint a secret and ten recovery codes, persist
    /// them encrypted, and return the one-time artifacts. Refused when a
    /// confirmed enrollment already exists - a session-hijacked attacker
    /// must not pivot from "I have a session" to "I own 2FA"; rotation
    /// goes through [`TwoFactorService::re_enroll`] with proof.
    pub async fn enroll(&self, actor: &CredentialActor) -> Result<EnrollmentResponse> {
        let prepared = self.prepare_enrollment(actor.user_id()).await?;
        if !self
            .store
            .begin_enrollment(
                actor,
                &prepared.secret_ciphertext,
                Some(&prepared.recovery_ciphertext),
            )
            .await?
        {
            return Err(Error::Conflict {
                resource: "two-factor enrollment".to_owned(),
                message: "2FA enrollment already exists; rotation requires proof via re_enroll"
                    .to_owned(),
            });
        }
        Ok(prepared.response)
    }

    /// Rotate the secret of a confirmed enrollment. Requires a current
    /// TOTP code or an unused recovery code as proof of possession, and is
    /// refused while the account is locked out.
    pub async fn re_enroll(
        &self,
        actor: &CredentialActor,
        proof: &str,
    ) -> Result<EnrollmentResponse> {
        let user_id = actor.user_id();
        if !self.is_enabled(user_id).await? {
            return Err(Error::InvalidInput {
                field: "enrollment".to_owned(),
                message: "no confirmed 2FA enrollment to rotate; enroll first".to_owned(),
            });
        }
        let identity = self.lockout_identity(user_id).await?;
        self.require_unlocked(&identity).await?;
        let claim = self.prepare_proof(user_id, proof).await?;
        let prepared = self.prepare_enrollment(user_id).await?;
        if !self
            .store
            .rotate_enrollment(
                actor,
                claim,
                &prepared.secret_ciphertext,
                Some(&prepared.recovery_ciphertext),
            )
            .await?
        {
            let _ = self
                .lockout
                .record_failed_attempt(&identity, Some("two-factor re-enroll"))
                .await;
            return Err(invalid_proof("re-enrollment"));
        }
        if self.lockout.reset_attempts(&identity).await.is_err() {
            tracing::warn!("lockout reset failed after committed two-factor rotation");
        }
        Ok(prepared.response)
    }

    /// Confirm a pending enrollment with a live code; 2FA is inactive
    /// until this succeeds.
    pub async fn confirm(&self, actor: &CredentialActor, code: &str) -> Result<()> {
        let user_id = actor.user_id();
        let identity = self.lockout_identity(user_id).await?;
        self.require_unlocked(&identity).await?;
        let Some(row) = self.store.find_enrollment(user_id).await? else {
            return Err(Error::InvalidInput {
                field: "enrollment".to_owned(),
                message: "no pending 2FA enrollment".to_owned(),
            });
        };
        let secret = self.decrypt_secret(&row)?;
        if totp::matched_step(&secret, code, Utc::now())?.is_none() {
            let _ = self
                .lockout
                .record_failed_attempt(&identity, Some("two-factor confirm"))
                .await;
            return Err(Error::InvalidInput {
                field: "code".to_owned(),
                message: "invalid 2FA code".to_owned(),
            });
        }
        if !self.store.set_confirmed(actor, Utc::now()).await? {
            return Err(Error::Internal {
                message: "two-factor enrollment vanished mid-confirm".to_owned(),
            });
        }
        if self.lockout.reset_attempts(&identity).await.is_err() {
            tracing::warn!("lockout reset failed after committed two-factor confirmation");
        }
        Ok(())
    }

    /// Silent matched-step verification: no lockout accounting. The claim
    /// and the success result are one atomic decision - the conditional
    /// timestep write is the authority, so a code matched at any window
    /// edge can never be accepted again once its step is claimed.
    pub async fn verify(&self, user_id: &str, code: &str) -> Result<bool> {
        let Some(row) = self.store.find_enrollment(user_id).await? else {
            return Ok(false);
        };
        if row.confirmed_at.is_none() {
            return Ok(false);
        }
        let secret = self.decrypt_secret(&row)?;
        let Some(matched) = totp::matched_step(&secret, code, Utc::now())? else {
            return Ok(false);
        };
        if let Some(last) = row.last_used_timestep
            && matched <= last
        {
            return Ok(false);
        }
        self.store.claim_timestep(user_id, matched).await
    }

    /// Silent single-use recovery-code consumption through the blob CAS;
    /// two concurrent consumes of one code have exactly one winner.
    pub async fn consume_recovery_code(&self, user_id: &str, code: &str) -> Result<bool> {
        const MAX_RETRIES: u32 = 4;
        let mut attempt = 0_u32;
        loop {
            let Some(row) = self.store.find_enrollment(user_id).await? else {
                return Ok(false);
            };
            if row.confirmed_at.is_none() {
                return Ok(false);
            }
            let Some(expected) = row.recovery_codes else {
                return Ok(false);
            };
            let plaintext = self
                .encryptor
                .decrypt(CryptoPurpose::TwoFactorRecovery, &expected)?;
            let plaintext = String::from_utf8(plaintext).map_err(|_| Error::Internal {
                message: "stored recovery blob is not UTF-8".to_owned(),
            })?;
            let mut codes: Vec<String> = plaintext.lines().map(String::from).collect();
            let Some(index) = recovery::find_constant_time(&codes, code) else {
                return Ok(false);
            };
            codes.remove(index);
            let next = if codes.is_empty() {
                None
            } else {
                Some(self.encryptor.encrypt(
                    CryptoPurpose::TwoFactorRecovery,
                    codes.join("\n").as_bytes(),
                )?)
            };
            if self
                .store
                .swap_recovery_codes(user_id, &expected, next.as_deref())
                .await?
            {
                return Ok(true);
            }
            attempt += 1;
            if attempt >= MAX_RETRIES {
                return Err(Error::Conflict {
                    resource: "recovery codes".to_owned(),
                    message: "recovery-code consume lost the race repeatedly".to_owned(),
                });
            }
        }
    }

    /// Rotate the recovery codes of a confirmed enrollment; the secret and
    /// confirmation stay untouched. Requires proof of possession and an
    /// unlocked account - without proof, a hijacked session could destroy
    /// the legitimate user's recovery path.
    pub async fn regenerate_recovery_codes(
        &self,
        actor: &CredentialActor,
        proof: &str,
    ) -> Result<Vec<String>> {
        let user_id = actor.user_id();
        if !self.is_enabled(user_id).await? {
            return Err(Error::InvalidInput {
                field: "enrollment".to_owned(),
                message: "no confirmed 2FA enrollment; cannot regenerate recovery codes".to_owned(),
            });
        }
        let identity = self.lockout_identity(user_id).await?;
        self.require_unlocked(&identity).await?;
        let claim = self.prepare_proof(user_id, proof).await?;
        let codes = recovery::generate(recovery::RECOVERY_CODE_COUNT);
        let ciphertext = self.encryptor.encrypt(
            CryptoPurpose::TwoFactorRecovery,
            codes.join("\n").as_bytes(),
        )?;
        if !self
            .store
            .regenerate_recovery_codes(actor, claim, &ciphertext)
            .await?
        {
            let _ = self
                .lockout
                .record_failed_attempt(&identity, Some("two-factor recovery-rotate"))
                .await;
            return Err(invalid_proof("recovery-code regeneration"));
        }
        if self.lockout.reset_attempts(&identity).await.is_err() {
            tracing::warn!("lockout reset failed after committed recovery-code rotation");
        }
        Ok(codes)
    }

    /// Disable 2FA. Idempotent; returns whether a row was actually
    /// removed so hosts fire their disabled notification only on a true
    /// transition.
    pub async fn disable(&self, actor: &CredentialActor) -> Result<bool> {
        self.store.delete_enrollment(actor).await
    }

    /// Whether an active (confirmed) enrollment exists.
    pub async fn is_enabled(&self, user_id: &str) -> Result<bool> {
        Ok(self
            .store
            .find_enrollment(user_id)
            .await?
            .is_some_and(|row| row.confirmed_at.is_some()))
    }

    async fn prepare_enrollment(&self, user_id: &str) -> Result<PreparedEnrollment> {
        let account = self.lockout_identity(user_id).await?;
        let provisioned = totp::provision(&self.config.issuer, &account)?;
        let recovery_codes = recovery::generate(recovery::RECOVERY_CODE_COUNT);
        let secret_ciphertext = self.encryptor.encrypt(
            CryptoPurpose::TwoFactorSecret,
            provisioned.secret_b32.expose_secret().as_bytes(),
        )?;
        let recovery_ciphertext = self.encryptor.encrypt(
            CryptoPurpose::TwoFactorRecovery,
            recovery_codes.join("\n").as_bytes(),
        )?;
        Ok(PreparedEnrollment {
            response: EnrollmentResponse {
                otpauth_url: provisioned.otpauth_url,
                qr_code_svg: provisioned.qr_code_svg,
                recovery_codes,
            },
            secret_ciphertext,
            recovery_ciphertext,
        })
    }

    async fn inspect_proof(&self, user_id: &str, proof: &str) -> Result<ProofMaterial> {
        let Some(row) = self.store.find_enrollment(user_id).await? else {
            return Ok(ProofMaterial::Invalid);
        };
        if row.confirmed_at.is_none() {
            return Ok(ProofMaterial::Invalid);
        }
        let secret = self.decrypt_secret(&row)?;
        if let Some(matched_step) = totp::matched_step(&secret, proof, Utc::now())?
            && row
                .last_used_timestep
                .is_none_or(|last| matched_step > last)
        {
            return Ok(ProofMaterial::Totp { matched_step });
        }
        let Some(expected_ciphertext) = row.recovery_codes else {
            return Ok(ProofMaterial::Invalid);
        };
        let plaintext = self
            .encryptor
            .decrypt(CryptoPurpose::TwoFactorRecovery, &expected_ciphertext)?;
        let plaintext = String::from_utf8(plaintext).map_err(|_| Error::Internal {
            message: "stored recovery blob is not UTF-8".to_owned(),
        })?;
        let mut codes: Vec<String> = plaintext.lines().map(String::from).collect();
        let Some(index) = recovery::find_constant_time(&codes, proof) else {
            return Ok(ProofMaterial::Invalid);
        };
        codes.remove(index);
        Ok(ProofMaterial::Recovery {
            expected_ciphertext,
            remaining_codes: codes,
        })
    }

    async fn prepare_proof(&self, user_id: &str, proof: &str) -> Result<TwoFactorProofClaim> {
        match self.inspect_proof(user_id, proof).await? {
            ProofMaterial::Invalid => Ok(TwoFactorProofClaim::Invalid),
            ProofMaterial::Totp { matched_step } => Ok(TwoFactorProofClaim::Totp { matched_step }),
            ProofMaterial::Recovery {
                expected_ciphertext,
                ..
            } => Ok(TwoFactorProofClaim::Recovery {
                expected_ciphertext,
            }),
        }
    }

    async fn prepare_factor_proof(
        &self,
        user_id: &str,
        code: &str,
        lockout_identity: String,
        admission: AttemptAdmission,
    ) -> Result<PreparedFactorProof<PreparedTwoFactorProof>> {
        let (valid, claim) = match self.inspect_proof(user_id, code).await? {
            ProofMaterial::Invalid => (false, PreparedTwoFactorClaim::Invalid),
            ProofMaterial::Totp { matched_step } => {
                (true, PreparedTwoFactorClaim::Totp { matched_step })
            }
            ProofMaterial::Recovery {
                expected_ciphertext,
                remaining_codes,
            } => {
                let next_ciphertext = if remaining_codes.is_empty() {
                    None
                } else {
                    Some(self.encryptor.encrypt(
                        CryptoPurpose::TwoFactorRecovery,
                        remaining_codes.join("\n").as_bytes(),
                    )?)
                };
                (
                    true,
                    PreparedTwoFactorClaim::Recovery {
                        expected_ciphertext,
                        next_ciphertext,
                    },
                )
            }
        };
        let prepared = PreparedTwoFactorProof {
            user_id: user_id.to_owned(),
            lockout_identity,
            admission,
            claim,
        };
        Ok(if valid {
            PreparedFactorProof::valid(prepared)
        } else {
            PreparedFactorProof::invalid(prepared)
        })
    }

    fn decrypt_secret(&self, row: &TwoFactorRow) -> Result<SecretString> {
        let plaintext = self
            .encryptor
            .decrypt(CryptoPurpose::TwoFactorSecret, &row.secret)?;
        String::from_utf8(plaintext)
            .map(SecretString::from)
            .map_err(|_| Error::Internal {
                message: "stored 2FA secret is not UTF-8".to_owned(),
            })
    }

    /// Lockout is keyed by normalized email, shared with 05's accounting.
    async fn lockout_identity(&self, user_id: &str) -> Result<String> {
        let user = self
            .users
            .find_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                resource: "user".to_owned(),
                identifier: user_id.to_owned(),
            })?;
        Ok(normalize_email(&user.email))
    }

    async fn require_unlocked(&self, identity: &str) -> Result<()> {
        let status = self.lockout.guarded_status(identity).await?;
        if status.is_locked {
            return Err(Error::Conflict {
                resource: "account lockout".to_owned(),
                message: format!(
                    "account is locked due to too many failed attempts; retry in {} seconds",
                    status.retry_after_seconds().unwrap_or(0)
                ),
            });
        }
        Ok(())
    }
}

/// The gate wiring: a confirmed enrollment interrupts every primary
/// sign-in, and challenge proof accepts a TOTP code or a recovery code after
/// one canonical pre-verification attempt reservation.
#[async_trait]
impl FactorVerifier for TwoFactorService {
    type PreparedProof = PreparedTwoFactorProof;

    async fn has_confirmed_enrollment(&self, user_id: &str) -> Result<bool> {
        self.is_enabled(user_id).await
    }

    async fn prepare_code(
        &self,
        user_id: &str,
        code: &str,
    ) -> Result<PreparedFactorProof<Self::PreparedProof>> {
        let identity = self.lockout_identity(user_id).await?;
        let admission = self
            .lockout
            .admit_attempt(&identity, Some("two-factor challenge"))
            .await?;
        if !admission.admitted {
            return Err(Error::Conflict {
                resource: "account lockout".to_owned(),
                message: format!(
                    "account is locked due to too many failed attempts; retry in {} seconds",
                    admission.status.retry_after_seconds().unwrap_or(0)
                ),
            });
        }
        match self
            .prepare_factor_proof(user_id, code, identity.clone(), admission.clone())
            .await
        {
            Ok(proof) => Ok(proof),
            Err(proof_error) => {
                if let Err(cancel_error) = self.lockout.cancel_attempt(&identity, &admission).await
                {
                    tracing::error!(
                        error = %cancel_error,
                        original_error = %proof_error,
                        "failed to cancel two-factor attempt after proof preparation aborted"
                    );
                    return Err(cancel_error);
                }
                Err(proof_error)
            }
        }
    }

    async fn claim_prepared(&self, user_id: &str, proof: Self::PreparedProof) -> Result<bool> {
        if proof.user_id != user_id {
            self.lockout
                .cancel_attempt(&proof.lockout_identity, &proof.admission)
                .await?;
            return Ok(false);
        }
        let claimed = match proof.claim {
            PreparedTwoFactorClaim::Invalid => {
                self.lockout
                    .finalize_failed_attempt(&proof.lockout_identity, &proof.admission)
                    .await?;
                return Ok(false);
            }
            PreparedTwoFactorClaim::Totp { matched_step } => {
                self.store.claim_timestep(user_id, matched_step).await
            }
            PreparedTwoFactorClaim::Recovery {
                expected_ciphertext,
                next_ciphertext,
            } => {
                self.store
                    .swap_recovery_codes(user_id, &expected_ciphertext, next_ciphertext.as_deref())
                    .await
            }
        };
        let claimed = match claimed {
            Ok(claimed) => claimed,
            Err(claim_error) => {
                if let Err(cancel_error) = self
                    .lockout
                    .cancel_attempt(&proof.lockout_identity, &proof.admission)
                    .await
                {
                    tracing::error!(
                        error = %cancel_error,
                        original_error = %claim_error,
                        "failed to cancel two-factor attempt after proof claim aborted"
                    );
                    return Err(cancel_error);
                }
                return Err(claim_error);
            }
        };
        if !claimed {
            self.lockout
                .cancel_attempt(&proof.lockout_identity, &proof.admission)
                .await?;
            return Ok(false);
        }
        if claimed {
            self.lockout
                .reset_admitted_attempts(&proof.lockout_identity, &proof.admission)
                .await?;
        }
        Ok(claimed)
    }

    async fn cancel_prepared(&self, user_id: &str, proof: Self::PreparedProof) -> Result<()> {
        if proof.user_id != user_id {
            return Err(Error::Conflict {
                resource: "two-factor proof".to_owned(),
                message: "prepared proof belongs to another user".to_owned(),
            });
        }
        self.lockout
            .cancel_attempt(&proof.lockout_identity, &proof.admission)
            .await
    }
}

fn invalid_proof(operation: &str) -> Error {
    Error::InvalidInput {
        field: "proof".to_owned(),
        message: format!(
            "{operation} proof is neither a valid TOTP code nor an unused recovery code"
        ),
    }
}
