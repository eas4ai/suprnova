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

use crate::auth::FactorVerifier;
use crate::crypto::{CryptoPurpose, Encryptor};
use crate::password::{LockoutService, normalize_email};
use crate::storage::UserStore;
use crate::{Error, Result};

pub use store::{TwoFactorRow, TwoFactorStore};

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
    /// confirmed enrollment already exists — a session-hijacked attacker
    /// must not pivot from "I have a session" to "I own 2FA"; rotation
    /// goes through [`TwoFactorService::re_enroll`] with proof.
    pub async fn enroll(&self, user_id: &str) -> Result<EnrollmentResponse> {
        if self.is_enabled(user_id).await? {
            return Err(Error::Conflict {
                resource: "two-factor enrollment".to_owned(),
                message: "2FA is already enabled; rotation requires proof via re_enroll".to_owned(),
            });
        }
        self.write_new_enrollment(user_id).await
    }

    /// Rotate the secret of a confirmed enrollment. Requires a current
    /// TOTP code or an unused recovery code as proof of possession, and is
    /// refused while the account is locked out.
    pub async fn re_enroll(&self, user_id: &str, proof: &str) -> Result<EnrollmentResponse> {
        if !self.is_enabled(user_id).await? {
            return Err(Error::InvalidInput {
                field: "enrollment".to_owned(),
                message: "no confirmed 2FA enrollment to rotate; enroll first".to_owned(),
            });
        }
        let identity = self.lockout_identity(user_id).await?;
        self.require_unlocked(&identity).await?;
        if !self.accept_proof(user_id, proof).await? {
            let _ = self
                .lockout
                .record_failed_attempt(&identity, Some("two-factor re-enroll"))
                .await;
            return Err(invalid_proof("re-enrollment"));
        }
        self.lockout.reset_attempts(&identity).await?;
        self.write_new_enrollment(user_id).await
    }

    /// Confirm a pending enrollment with a live code; 2FA is inactive
    /// until this succeeds.
    pub async fn confirm(&self, user_id: &str, code: &str) -> Result<()> {
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
        self.lockout.reset_attempts(&identity).await?;
        if !self.store.set_confirmed(user_id, Utc::now()).await? {
            return Err(Error::Internal {
                message: "two-factor enrollment vanished mid-confirm".to_owned(),
            });
        }
        Ok(())
    }

    /// Silent matched-step verification: no lockout accounting. The claim
    /// and the success result are one atomic decision — the conditional
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
    /// unlocked account — without proof, a hijacked session could destroy
    /// the legitimate user's recovery path.
    pub async fn regenerate_recovery_codes(
        &self,
        user_id: &str,
        proof: &str,
    ) -> Result<Vec<String>> {
        if !self.is_enabled(user_id).await? {
            return Err(Error::InvalidInput {
                field: "enrollment".to_owned(),
                message: "no confirmed 2FA enrollment; cannot regenerate recovery codes".to_owned(),
            });
        }
        let identity = self.lockout_identity(user_id).await?;
        self.require_unlocked(&identity).await?;
        if !self.accept_proof(user_id, proof).await? {
            let _ = self
                .lockout
                .record_failed_attempt(&identity, Some("two-factor recovery-rotate"))
                .await;
            return Err(invalid_proof("recovery-code regeneration"));
        }
        self.lockout.reset_attempts(&identity).await?;
        let codes = recovery::generate(recovery::RECOVERY_CODE_COUNT);
        let ciphertext = self.encryptor.encrypt(
            CryptoPurpose::TwoFactorRecovery,
            codes.join("\n").as_bytes(),
        )?;
        self.store
            .replace_recovery_codes(user_id, &ciphertext)
            .await?;
        Ok(codes)
    }

    /// Disable 2FA. Idempotent; returns whether a row was actually
    /// removed so hosts fire their disabled notification only on a true
    /// transition.
    pub async fn disable(&self, user_id: &str) -> Result<bool> {
        self.store.delete_enrollment(user_id).await
    }

    /// Whether an active (confirmed) enrollment exists.
    pub async fn is_enabled(&self, user_id: &str) -> Result<bool> {
        Ok(self
            .store
            .find_enrollment(user_id)
            .await?
            .is_some_and(|row| row.confirmed_at.is_some()))
    }

    async fn write_new_enrollment(&self, user_id: &str) -> Result<EnrollmentResponse> {
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
        self.store
            .upsert_enrollment(user_id, &secret_ciphertext, Some(&recovery_ciphertext))
            .await?;
        Ok(EnrollmentResponse {
            otpauth_url: provisioned.otpauth_url,
            qr_code_svg: provisioned.qr_code_svg,
            recovery_codes,
        })
    }

    /// One bad proof counts as one failed attempt: TOTP first, then a
    /// recovery code, both silent.
    async fn accept_proof(&self, user_id: &str, proof: &str) -> Result<bool> {
        if self.verify(user_id, proof).await? {
            return Ok(true);
        }
        self.consume_recovery_code(user_id, proof).await
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
/// sign-in, and challenge proof accepts a TOTP code or a recovery code
/// with one canonical lockout record per failure.
#[async_trait]
impl FactorVerifier for TwoFactorService {
    async fn has_confirmed_enrollment(&self, user_id: &str) -> Result<bool> {
        self.is_enabled(user_id).await
    }

    async fn verify_code(&self, user_id: &str, code: &str) -> Result<bool> {
        let identity = self.lockout_identity(user_id).await?;
        self.require_unlocked(&identity).await?;
        let accepted = self.accept_proof(user_id, code).await?;
        if accepted {
            self.lockout.reset_attempts(&identity).await?;
        } else {
            let _ = self
                .lockout
                .record_failed_attempt(&identity, Some("two-factor challenge"))
                .await;
        }
        Ok(accepted)
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
