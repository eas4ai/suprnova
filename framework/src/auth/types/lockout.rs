//! Computed account lockout status.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The current brute-force lockout state for an email address.
///
/// This is a computed status rather than a persistence model. The policy layer
/// determines the attempt window and populates these fields for controllers and
/// middleware to inspect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LockoutStatus {
    /// The email address whose lockout status was computed.
    pub email: String,
    /// The number of failed attempts in the current lockout window.
    pub failed_attempts: u32,
    /// Whether the account is currently locked.
    pub is_locked: bool,
    /// When the lockout expires, if the account is locked.
    pub locked_until: Option<DateTime<Utc>>,
}

impl LockoutStatus {
    /// Returns whole seconds until lockout expiry.
    ///
    /// Returns `None` when no expiry is recorded and returns `Some(0)` for an
    /// expiry that has already passed.
    #[must_use]
    pub fn retry_after_seconds(&self) -> Option<i64> {
        self.locked_until.map(|until| {
            let seconds = (until - Utc::now()).num_seconds();
            seconds.max(0)
        })
    }
}
