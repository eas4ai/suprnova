//! Password-domain services: hashing policy and account lockout.
//!
//! Route surface for the password domain lives in [`crate::plugins`]; this
//! module owns the installable dual-format verifier and the lockout policy
//! service consumed by those plugins.

pub mod hash;
pub mod lockout;

pub use hash::{
    AttemptVerdict, CallProvenance, HashAlgorithm, HashParameters, HashWorkProfile,
    PasswordHashConfig, PasswordHashDriver, PasswordVerifier, RehashOutcome,
    StandardPasswordHashDriver, VerificationCall,
};
pub use lockout::{
    BackendErrorPolicy, FailedAttempt, LockoutConfig, LockoutService, LockoutStatus,
};

/// Normalize an email address for lookups, lockout keys, and abuse keys.
///
/// Normalization is deliberately minimal (trim plus ASCII-insensitive
/// lowercase) so it matches the deployed framework behavior; collision
/// handling for legacy mixed-case rows belongs to the migration domain.
#[must_use]
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Validate a candidate password against the ported torii policy:
/// non-empty, not whitespace-only, and between 8 and 128 bytes.
pub fn validate_password(password: &str) -> crate::Result<()> {
    let invalid = |message: &str| crate::Error::InvalidInput {
        field: "password".to_owned(),
        message: message.to_owned(),
    };
    if password.is_empty() {
        return Err(invalid("password is required"));
    }
    if password.trim().is_empty() {
        return Err(invalid("password cannot be only whitespace"));
    }
    if password.len() < 8 {
        return Err(invalid("password must be at least 8 characters long"));
    }
    if password.len() > 128 {
        return Err(invalid("password must be no more than 128 characters long"));
    }
    Ok(())
}
