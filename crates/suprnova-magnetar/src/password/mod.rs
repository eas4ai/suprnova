//! Password-domain services: hashing policy and account lockout.
//!
//! Route surface for the password domain lives in [`crate::plugins`]; this
//! module owns the installable dual-format verifier and the lockout policy
//! service consumed by those plugins.

pub mod hash;
pub mod lockout;

pub use hash::{
    AttemptVerdict, CallProvenance, HashAlgorithm, HashParameters, HashWorkProfile,
    PasswordHashConfig, PasswordHashDriver, PasswordTarget, PasswordVerifier, RehashOutcome,
    StandardPasswordHashDriver, VerificationCall, configure_hash_work_limit, hash_work_waiting,
    run_hash_work,
};
pub use lockout::{
    AttemptAdmission, AttemptReservationToken, BackendErrorPolicy, FailedAttempt, LockoutConfig,
    LockoutService, LockoutStatus,
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

/// The longest email address, in characters, that Magnetar stores or
/// looks up.
///
/// RFC 5321 limits a forward path to 256 octets, two of them its angle
/// brackets, so no deliverable address is longer. The default schema keeps
/// addresses and email lockout identities in `varchar(255)` on MySQL and
/// MariaDB, which refuse a longer write. Sign-up refuses a longer address
/// as invalid input, and a sign-in with one answers as an unknown address
/// does, without a lookup.
pub const MAX_EMAIL_LENGTH: usize = 254;

/// Whether a normalized address is short enough for any account to hold.
pub(crate) fn email_fits(email: &str) -> bool {
    email.chars().count() <= MAX_EMAIL_LENGTH
}

/// The validation error for an address longer than [`MAX_EMAIL_LENGTH`].
pub(crate) fn email_too_long() -> crate::Error {
    crate::Error::InvalidInput {
        field: "email".to_owned(),
        message: format!("must be at most {MAX_EMAIL_LENGTH} characters"),
    }
}

/// Refuse a normalized sign-up address that is empty or longer than
/// [`MAX_EMAIL_LENGTH`], before it reaches any store.
#[cfg(any(
    feature = "password",
    feature = "magic-link",
    feature = "passkey",
    feature = "oauth"
))]
pub(crate) fn validate_sign_up_email(email: &str) -> crate::Result<()> {
    if email.is_empty() {
        return Err(crate::Error::InvalidInput {
            field: "email".to_owned(),
            message: "must not be empty".to_owned(),
        });
    }
    if !email_fits(email) {
        return Err(email_too_long());
    }
    Ok(())
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
