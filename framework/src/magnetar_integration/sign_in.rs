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

/// The result of a password registration.
///
/// An address that already belongs to an account yields [`Self::Accepted`],
/// which carries nothing about that account. Returning the existing account
/// would hand it to whoever typed its address: an app that signs the
/// registered user in would sign the requester in as the owner, and an app
/// that echoes the user would reveal who has an account.
///
/// To keep it private whether an address is registered, answer both
/// variants the same way - for example "check your email", or "you can now
/// sign in" - and let the user sign in with their password afterwards.
/// Signing a [`Self::Created`] account in at once is safe, because its
/// password is the one just submitted, but that answer differs from the one
/// [`Self::Accepted`] can give, so it shows that an address was free.
///
/// Both outcomes hash the password and read an account back, so they take
/// nearly the same time; only a new address also writes a row.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use = "a registration must be answered the same way for both variants"]
pub enum Registration {
    /// A new account now holds the address, with the submitted password.
    Created(User),
    /// The address already belonged to an account. Nothing changed, and
    /// nothing about that account is returned.
    Accepted,
}

impl Registration {
    /// The new account, or `None` when the address was already registered.
    pub fn created(self) -> Option<User> {
        match self {
            Self::Created(user) => Some(user),
            Self::Accepted => None,
        }
    }
}
