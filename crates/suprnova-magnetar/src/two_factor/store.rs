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
    /// Set when the user proved possession of the new secret; 2FA is
    /// inactive until then.
    pub confirmed_at: Option<DateTime<Utc>>,
    /// The highest TOTP timestep that has ever matched, for replay
    /// rejection.
    pub last_used_timestep: Option<i64>,
}

/// Storage API for two-factor enrollments. Every state transition is a
/// conditional write whose affected-row count is the authority.
#[async_trait]
pub trait TwoFactorStore: Send + Sync {
    /// Read one enrollment.
    async fn find_enrollment(&self, user_id: &str) -> Result<Option<TwoFactorRow>>;
    /// Insert or overwrite an enrollment with a fresh secret and recovery
    /// blob, resetting `confirmed_at` and the replay stamp.
    async fn upsert_enrollment(
        &self,
        user_id: &str,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
    ) -> Result<()>;
    /// Stamp `confirmed_at`. Returns whether a row was stamped.
    async fn set_confirmed(&self, user_id: &str, at: DateTime<Utc>) -> Result<bool>;
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
    /// Replace the recovery blob unconditionally (proof-gated regeneration).
    async fn replace_recovery_codes(&self, user_id: &str, next: &[u8]) -> Result<()>;
    /// Delete the enrollment. Returns whether a row was removed, so hosts
    /// fire their disabled notification only on a true transition.
    async fn delete_enrollment(&self, user_id: &str) -> Result<bool>;
}
