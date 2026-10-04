//! Persistence boundary for two-factor enrollments.
//!
//! Mirrors the deployed `two_factor_credentials` row: an opaque string
//! `user_id` with deliberately no foreign-key requirement, ciphertext
//! secret and recovery blob (this module never sees plaintext), the
//! confirmed-at stamp that separates pending from active, and the
//! replay-protection timestep. A proven rotation waits beside the confirmed
//! secret in the pending columns, so the confirmed secret keeps gating
//! sign-in until the new one is confirmed. Hosts implement this trait over
//! their own table, exactly like the session and remember-me stores.

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
    /// Once confirmed, this is the secret that gates sign-in, also while a
    /// rotation waits in [`Self::pending_secret`].
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
    /// Whether a proof-gated rotation waits in [`Self::pending_secret`].
    pub rotation_pending: bool,
    /// Ciphertext secret of a proven rotation waiting for its confirmation.
    /// It replaces [`Self::secret`] only when
    /// [`TwoFactorStore::confirm_rotation`] proves a code from it; until
    /// then the confirmed secret keeps gating sign-in.
    pub pending_secret: Option<Vec<u8>>,
    /// Ciphertext recovery codes minted with [`Self::pending_secret`]; they
    /// replace [`Self::recovery_codes`] with it.
    pub pending_recovery_codes: Option<Vec<u8>>,
    /// Set when the user proved possession of the secret; 2FA is inactive
    /// until then. A rotation never clears it.
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
    /// Confirm exactly the enrollment a code was checked against, once.
    ///
    /// Stamp `confirmed_at`, clear the pending-rotation marker and claim
    /// `matched_step` as `last_used_timestep`, in one conditional write that
    /// holds only while the row still stores `expected_secret`, is still
    /// unconfirmed, and has not used that timestep. A concurrent enrollment
    /// that replaced the secret after the check therefore stays unconfirmed:
    /// the code proved possession of the old secret, not of the new one. A
    /// second confirmation of the same enrollment, or a later reuse of its
    /// code, finds nothing to change. Returns whether this caller confirmed.
    async fn set_confirmed(
        &self,
        actor: &CredentialActor,
        expected_secret: &[u8],
        matched_step: i64,
        at: DateTime<Utc>,
    ) -> Result<bool>;
    /// Promote exactly the pending rotation a code was checked against, once.
    ///
    /// Its secret and recovery codes replace the confirmed ones, the pending
    /// columns clear, `confirmed_at` becomes `at`, and `matched_step` is
    /// claimed as `last_used_timestep` of the new secret, in one conditional
    /// write that holds only while the row is confirmed and its pending
    /// secret is still `expected_pending_secret`. A later rotation that
    /// replaced it after the check therefore stays pending, and a second
    /// promotion finds nothing to change. Returns whether this caller
    /// promoted.
    async fn confirm_rotation(
        &self,
        actor: &CredentialActor,
        expected_pending_secret: &[u8],
        matched_step: i64,
        at: DateTime<Utc>,
    ) -> Result<bool>;
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
    /// Atomically claim the old factor proof and store a pending rotation.
    ///
    /// The rotation waits in the pending columns of a confirmed enrollment,
    /// which keeps its secret, recovery codes and confirmation, and so keeps
    /// gating sign-in, until [`Self::confirm_rotation`] promotes the new
    /// secret. The actor snapshot is the rotating actor's. A later rotation
    /// replaces a pending one.
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
    /// Delete the enrollment, with any rotation waiting in it. Returns
    /// whether a row was removed, so hosts fire their disabled notification
    /// only on a true transition.
    async fn delete_enrollment(&self, actor: &CredentialActor) -> Result<bool>;
}
