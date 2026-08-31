//! Public sign-in outcomes shared by Magnetar authentication facades.

use super::{Session, User};
use crate::error::FrameworkError;

/// The result of a primary sign-in attempt.
///
/// A factor-required result carries the opaque selector needed to continue
/// through the installed Magnetar engine and does not bind a framework
/// session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
// Keep the documented public fields concrete so callers can destructure the
// authenticated user and session without an allocation-only API distinction.
#[allow(clippy::large_enum_variant)]
pub enum SignInOutcome {
    /// Authentication completed and the framework session was bound.
    Authenticated {
        /// The authenticated application user.
        user: User,
        /// The newly issued Magnetar session.
        session: Session,
    },
    /// The primary credential passed, but another factor is required.
    FactorRequired {
        /// Opaque selector accepted by the installed engine's factor ceremony.
        challenge_selector: String,
    },
}

impl SignInOutcome {
    pub(crate) fn into_legacy_tuple(
        self,
        factor_required_message: &'static str,
    ) -> Result<(User, Session), FrameworkError> {
        match self {
            Self::Authenticated { user, session } => Ok((user, session)),
            Self::FactorRequired { .. } => Err(FrameworkError::Domain {
                message: factor_required_message.to_owned(),
                status_code: 401,
            }),
        }
    }
}
