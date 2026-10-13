//! Authentication epochs (HRD-007): the value every provider-backed sign-in
//! records and every later request compares, so a sign-in issued before a
//! password change stops working on its next request.
//!
//! # Why the framework checks on every request
//!
//! A password reset ends the user's other sessions at once
//! (`destroy_all_for_user`) and revokes their remember-me credentials. When
//! either store refuses, the reset reports the error, but the stolen session
//! or cookie would keep working until it expired: nothing compared it with
//! the password again. Laravel's `AuthenticateSession` makes that comparison
//! lazily with a stored password hash. Suprnova does both: the eager
//! revocation, and this comparison on every request that resolves the user,
//! so the lazy check covers a revocation that failed.
//!
//! # What an epoch is
//!
//! The epoch of a user is [`Authenticatable::auth_epoch`] when the model
//! returns one, and otherwise a 64-bit digest of the password hash
//! [`Authenticatable::get_auth_password`] returns ([`credential_epoch`]).
//! A user whose model exposes neither has no epoch, records none, and is
//! not checked.
//!
//! - **The digest is an epoch** because every password write stores a new
//!   hash: a new salt even for the same password, so the digest changes with
//!   every reset, change and rehash, and no table needs a new column.
//! - **The model's own counter wins** when it has one. Magnetar's
//!   `app_users.auth_epoch` also advances on "sign out everywhere" and on a
//!   first email proof, which leave the password hash unchanged; a counter
//!   that the application advances itself is the stronger signal.
//!
//! # Magnetar
//!
//! When the Magnetar engine's host sign-in authority is installed, the
//! default guard's sign-ins are already fenced by Magnetar's own epoch:
//! each one is issued a Magnetar session at the epoch read with the
//! credential, and the session middleware validates that session at the
//! start of every request. For that guard the framework records and
//! compares nothing more ([`magnetar_fences`]), so there is one source of
//! truth: a second comparison against another value (the digest) could only
//! disagree with Magnetar's and sign a valid session out.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::authenticatable::Authenticatable;
use super::guard::Auth;
use super::manager::AuthManager;
use super::provider::UserProvider;
use crate::container::App;
use crate::error::FrameworkError;
use crate::session::store::GuardSignInEpoch;

/// Domain separation for the password-hash digest, so it never equals a
/// SHA-256 the framework computes over the same bytes for another purpose.
const PASSWORD_HASH_EPOCH_DOMAIN: &[u8] = b"suprnova.auth-epoch.password-hash.v1\0";

/// The authentication epoch of `user`: [`Authenticatable::auth_epoch`] when
/// the model returns one, else a digest of the password hash, else `None`.
///
/// The digest is the first eight bytes of SHA-256 over the hash string,
/// read big-endian. Eight bytes make a collision between a hash and the
/// next one that replaces it a 2^-64 chance. The digest also gives nothing
/// away: a bcrypt or Argon2 hash carries its salt, which the digest does not
/// reveal, so it cannot be used to test a guessed password, and the session
/// that stores it is server-side or encrypted.
pub(crate) fn credential_epoch(user: &dyn Authenticatable) -> Option<u64> {
    user.auth_epoch()
        .or_else(|| user.get_auth_password().map(password_hash_epoch))
}

/// The digest epoch of the password hash `hash`. See [`credential_epoch`].
fn password_hash_epoch(hash: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(PASSWORD_HASH_EPOCH_DOMAIN);
    hasher.update(hash.as_bytes());
    let digest = hasher.finalize();
    let mut leading = [0_u8; 8];
    leading.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(leading)
}

/// Whether Magnetar fences the sign-ins of the guard `guard_name` with its
/// own epoch: the default guard, with the engine's host sign-in authority
/// installed. The framework then records and compares no epoch of its own
/// for that guard (see the module documentation).
pub(crate) fn magnetar_fences(guard_name: &str) -> bool {
    guard_name == Auth::default_guard_name()
        && crate::magnetar_integration::host_sign_in_authority().is_some()
}

/// The provider that resolves the users of the guard `guard_name` by name:
/// the provider its configuration names in the [`AuthManager`], or, for the
/// default guard, the globally bound [`UserProvider`] when the manager names
/// none (the order [`super::active_user_provider`] uses).
///
/// Remember-me verification resolves the provider this way, by the guard
/// name the cookie carries, and issuance binds a credential to an epoch
/// only for a guard name this lookup finds, so a credential is bound at
/// issue to exactly the epoch its verification reads back. `None` when no
/// provider resolves, such as for a `SessionGuard` built by hand under a
/// name the manager does not know: that guard records its own provider's
/// epoch in the session, and it cannot issue a remember credential (see
/// `remember::issue_for_guard`).
///
/// The session middleware also resolves the provider this way to record
/// the epoch of a sign-in that had no user at hand (`Auth::login_id`)
/// before it stores the session.
pub(crate) fn guard_provider(guard_name: &str) -> Option<Arc<dyn UserProvider>> {
    if let Some(manager) = App::get::<AuthManager>()
        && let Ok(provider) = manager.guard_provider(guard_name)
    {
        return Some(provider);
    }
    if guard_name == Auth::default_guard_name() {
        return App::make::<dyn UserProvider>();
    }
    None
}

/// The current authentication epoch of the user `user_id` of the guard
/// `guard_name`, read through [`guard_provider`]. `None` when no provider
/// resolves, when the provider does not know the user, or when the user
/// exposes no epoch.
///
/// # Errors
///
/// The provider's error. A caller fails closed on it: an epoch that could
/// not be read is never treated as "no epoch".
pub(crate) async fn guard_user_epoch(
    guard_name: &str,
    user_id: &str,
) -> Result<Option<u64>, FrameworkError> {
    let Some(provider) = guard_provider(guard_name) else {
        return Ok(None);
    };
    Ok(provider
        .retrieve_by_id(user_id)
        .await?
        .as_deref()
        .and_then(credential_epoch))
}

/// The outcome of [`check_session_sign_in`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionSignInCheck {
    /// The sign-in stands: its epoch is the user's current one, or it
    /// recorded none and is not checked.
    Current,
    /// The user's epoch moved since the sign-in recorded it: a password
    /// write (or, for a model with its own counter, a sign-out everywhere)
    /// happened after it. The caller ends the sign-in.
    Stale,
}

/// Compare the epoch the session recorded for the guard `guard_name`'s
/// sign-in as `user_id` with the current epoch of `user`, the user the
/// guard's provider just returned for that id.
///
/// - No epoch recorded: [`SessionSignInCheck::Current`]. A session signed in
///   before 4.0.0 recorded none, and so did a sign-in of a user who exposes
///   no epoch; neither is checked, as HRD-007 states. A 3.2.1 session is
///   checked from the user's next sign-in.
/// - [`GuardSignInEpoch::Unread`]: the sign-in had no user at hand
///   (`Auth::login_id`). The mark lives for the rest of the request that
///   made it: a resolution in that request records the user's current
///   epoch here and the sign-in stands, and otherwise the session
///   middleware records it before it stores the session. A mark found in a
///   later request is one the middleware could not resolve (no provider is
///   known by the guard name, such as for a `SessionGuard` built by hand,
///   or the provider did not know the user), and the guard's own provider
///   records it here.
/// - A recorded epoch: `Current` when the user's epoch is that value,
///   [`SessionSignInCheck::Stale`] otherwise, including when the user no
///   longer exposes one (the credential the sign-in was bound to is gone).
/// - A stored value that does not parse: `Stale`, so a damaged value never
///   switches the check off.
///
/// A guard that Magnetar fences is always `Current` here: Magnetar's session
/// validation is its check (see [`magnetar_fences`]).
pub(crate) fn check_session_sign_in(
    guard_name: &str,
    user_id: &str,
    user: &dyn Authenticatable,
) -> SessionSignInCheck {
    // The stored epoch is read first: a session without one (the common
    // case under Magnetar, whose sign-ins record none) answers without the
    // engine-installation lookup `magnetar_fences` takes.
    let Some(stored) =
        crate::session::middleware::persisted_guard_sign_in_epoch(guard_name, user_id)
    else {
        return SessionSignInCheck::Current;
    };
    if magnetar_fences(guard_name) {
        return SessionSignInCheck::Current;
    }
    match stored {
        GuardSignInEpoch::Unread => {
            crate::session::middleware::record_guard_sign_in_epoch(
                guard_name,
                user_id,
                credential_epoch(user),
            );
            SessionSignInCheck::Current
        }
        GuardSignInEpoch::Recorded(recorded) => {
            if credential_epoch(user) == Some(recorded) {
                SessionSignInCheck::Current
            } else {
                SessionSignInCheck::Stale
            }
        }
        GuardSignInEpoch::Unreadable => SessionSignInCheck::Stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    struct Model {
        password: Option<&'static str>,
        epoch: Option<u64>,
    }

    impl Authenticatable for Model {
        fn get_auth_identifier(&self) -> String {
            "7".to_owned()
        }
        fn get_auth_password(&self) -> Option<&str> {
            self.password
        }
        fn auth_epoch(&self) -> Option<u64> {
            self.epoch
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
    }

    #[test]
    fn hardening_credential_epoch_follows_the_password_hash() {
        let before = Model {
            password: Some("$2b$12$first-hash"),
            epoch: None,
        };
        let same = Model {
            password: Some("$2b$12$first-hash"),
            epoch: None,
        };
        let after = Model {
            password: Some("$2b$12$second-hash"),
            epoch: None,
        };
        assert!(credential_epoch(&before).is_some());
        assert_eq!(credential_epoch(&before), credential_epoch(&same));
        assert_ne!(credential_epoch(&before), credential_epoch(&after));
    }

    #[test]
    fn hardening_credential_epoch_prefers_the_models_counter() {
        let counted = Model {
            password: Some("$2b$12$any-hash"),
            epoch: Some(41),
        };
        assert_eq!(credential_epoch(&counted), Some(41));
    }

    #[test]
    fn hardening_credential_epoch_is_none_without_hash_or_counter() {
        let passwordless = Model {
            password: None,
            epoch: None,
        };
        assert_eq!(credential_epoch(&passwordless), None);
    }
}
