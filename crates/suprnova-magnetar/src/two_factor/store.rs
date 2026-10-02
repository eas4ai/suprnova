//! Persistence boundary for two-factor enrollments.
//!
//! Mirrors the deployed `two_factor_credentials` row: an opaque string
//! `user_id` with deliberately no foreign-key requirement, ciphertext
//! secret and recovery blob (this module never sees plaintext), the
//! confirmed-at stamp that separates pending from active, and the
//! replay-protection timestep. Hosts implement this trait over their own
//! table, exactly like the session and remember-me stores.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::Result;
use crate::storage::CredentialActor;

/// One stored enrollment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TwoFactorRow {
    /// Opaque owning user identifier (no FK requirement, source's choice).
    pub user_id: String,
    /// Ciphertext TOTP secret ([`crate::crypto::CryptoPurpose::TwoFactorSecret`]).
    pub secret: Vec<u8>,
    /// Ciphertext newline-joined recovery codes
    /// ([`crate::crypto::CryptoPurpose::TwoFactorRecovery`]); `None` once
    /// the final code is consumed.
    pub recovery_codes: Option<Vec<u8>>,
    /// Authentication epoch of the actor that began this enrollment.
    pub enrollment_auth_epoch: u64,
    /// Opaque session that began this enrollment, when applicable.
    pub enrollment_session_id: Option<String>,
    /// Expiry snapshot of the actor that began this enrollment.
    pub enrollment_expires_at: Option<DateTime<Utc>>,
    /// Whether this pending enrollment is a proof-gated rotation.
    pub rotation_pending: bool,
    /// Set when the user proved possession of the new secret; 2FA is
    /// inactive until then.
    pub confirmed_at: Option<DateTime<Utc>>,
    /// The highest TOTP timestep that has ever matched, for replay
    /// rejection.
    pub last_used_timestep: Option<i64>,
}

/// A verified proof prepared for an atomic lifecycle mutation.
#[derive(Clone, PartialEq, Eq)]
pub enum TwoFactorProofClaim {
    /// Submitted proof did not verify. Store composites treat this as a
    /// non-winning claim after validating the actor fence.
    Invalid,
    /// Claim the TOTP timestep that matched the submitted proof.
    Totp {
        /// Matched timestep.
        matched_step: i64,
    },
    /// Claim the exact encrypted recovery-code set that was verified.
    Recovery {
        /// Expected ciphertext; deliberately redacted from debug output.
        expected_ciphertext: Vec<u8>,
    },
}

impl std::fmt::Debug for TwoFactorProofClaim {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => formatter.write_str("Invalid"),
            Self::Totp { matched_step } => formatter
                .debug_struct("Totp")
                .field("matched_step", matched_step)
                .finish(),
            Self::Recovery { .. } => formatter
                .debug_struct("Recovery")
                .field("expected_ciphertext", &"[REDACTED]")
                .finish(),
        }
    }
}

/// Storage API for two-factor enrollments. Every state transition is a
/// conditional write whose affected-row count is the authority.
#[async_trait]
pub trait TwoFactorStore: Send + Sync {
    /// Read one enrollment.
    async fn find_enrollment(&self, user_id: &str) -> Result<Option<TwoFactorRow>>;
    /// Start or restart an initial enrollment. Returns `false` when a
    /// confirmed enrollment or proof-gated pending rotation already exists.
    async fn begin_enrollment(
        &self,
        actor: &CredentialActor,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
    ) -> Result<bool>;
    /// Stamp `confirmed_at` and clear the pending-rotation marker.
    async fn set_confirmed(&self, actor: &CredentialActor, at: DateTime<Utc>) -> Result<bool>;
    /// Claim one matched timestep: set `last_used_timestep = matched_step`
    /// only when the stored value is null or lower. The claim and the
    /// success result are one atomic decision; the returned bool is the
    /// authority on whether this caller won.
    async fn claim_timestep(&self, user_id: &str, matched_step: i64) -> Result<bool>;
    /// Compare-and-swap the recovery blob: replace it only while it still
    /// equals `expected`. Returns whether this caller won.
    async fn swap_recovery_codes(
        &self,
        user_id: &str,
        expected: &[u8],
        next: Option<&[u8]>,
    ) -> Result<bool>;
    /// Atomically claim the old factor proof and create a pending rotation.
    async fn rotate_enrollment(
        &self,
        actor: &CredentialActor,
        claim: TwoFactorProofClaim,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
    ) -> Result<bool>;
    /// Atomically claim the old factor proof and replace all recovery codes.
    async fn regenerate_recovery_codes(
        &self,
        actor: &CredentialActor,
        claim: TwoFactorProofClaim,
        next: &[u8],
    ) -> Result<bool>;
    /// Delete the enrollment. Returns whether a row was removed, so hosts
    /// fire their disabled notification only on a true transition.
    async fn delete_enrollment(&self, actor: &CredentialActor) -> Result<bool>;
}
