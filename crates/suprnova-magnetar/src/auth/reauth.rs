//! Explicit, bounded reauthentication capabilities.

use chrono::{DateTime, Duration, Utc};

use crate::{Error, Result};

/// Maximum age of a password confirmation accepted for sensitive enrollment.
pub const REAUTH_WINDOW: Duration = Duration::hours(3);

/// A password-confirmation stamp presented for a sensitive operation.
///
/// A stamp is only useful after [`validate_reauth`] checks both its owner and
/// its age. It is not an alternate authentication path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReauthStamp {
    /// User that performed the password confirmation.
    pub owner_user_id: String,
    /// Time at which the password was confirmed.
    pub password_confirmed_at: DateTime<Utc>,
}

/// A validated owner-bound reauthentication capability.
///
/// This capability intentionally exposes no constructor. Future enrollment
/// flows must obtain it by calling [`validate_reauth`] for the exact owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReauthCapability {
    owner_user_id: String,
    password_confirmed_at: DateTime<Utc>,
}

impl ReauthCapability {
    /// Return the owner bound to this capability.
    #[must_use]
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    /// Return the password-confirmation time.
    #[must_use]
    pub const fn password_confirmed_at(&self) -> DateTime<Utc> {
        self.password_confirmed_at
    }
}

/// Validate an owner-bound password confirmation for a future enrollment.
///
/// The confirmation must belong to `owner_user_id`, must not be in the future,
/// and must be no older than three hours at `now`.
pub fn validate_reauth(
    owner_user_id: &str,
    stamp: ReauthStamp,
    now: DateTime<Utc>,
) -> Result<ReauthCapability> {
    if owner_user_id.is_empty() {
        return Err(invalid("owner_user_id", "must not be empty"));
    }
    if stamp.owner_user_id != owner_user_id {
        return Err(invalid(
            "owner_user_id",
            "does not match the authenticated owner",
        ));
    }
    if stamp.password_confirmed_at > now {
        return Err(invalid(
            "password_confirmed_at",
            "must not be in the future",
        ));
    }
    if now - stamp.password_confirmed_at > REAUTH_WINDOW {
        return Err(invalid(
            "password_confirmed_at",
            "must be no older than three hours",
        ));
    }
    Ok(ReauthCapability {
        owner_user_id: stamp.owner_user_id,
        password_confirmed_at: stamp.password_confirmed_at,
    })
}

fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.to_owned(),
    }
}
