//! User provider trait for retrieving authenticated users from storage
//!
//! The application must implement this trait and register it with the container
//! to enable `Auth::user()`.

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::OnceLock;

use super::authenticatable::Authenticatable;
use crate::error::FrameworkError;

/// Precomputed bcrypt hash of an arbitrary string at the framework's
/// OWASP-floor cost (12). Used as the fallback dummy hash when the
/// configured driver can't be resolved or fails to mint one - so
/// [`UserProvider::dummy_verify`] always does *some* representative work
/// and never errors out the auth flow.
const FALLBACK_DUMMY_HASH: &str = "$2b$12$WzkqK0YIMJW8a4hkOEX/cuFNNDU.lI5jvyiQekkLwnAi8sFxlnEv6";

/// Process-wide cache of the dummy hash, minted once from the configured
/// driver. Caching avoids re-running the (deliberately expensive) hash
/// primitive on every enumeration-equalising call; the verify against it
/// still runs the full cost per call.
static DUMMY_HASH: OnceLock<String> = OnceLock::new();

/// A throwaway hash in the *configured* algorithm, suitable for feeding to
/// [`crate::hashing::verify_async`] so the verify cost tracks a real
/// stored-password verify under any `HASH_DRIVER`. Minted once and cached.
///
/// Falls back to [`FALLBACK_DUMMY_HASH`] (bcrypt cost 12) if the configured
/// driver can't be resolved or hashing fails - equalisation degrades to the
/// previous fixed-cost behaviour rather than erroring.
fn dummy_hash() -> String {
    DUMMY_HASH
        .get_or_init(|| {
            crate::hashing::default_driver()
                .and_then(|driver| driver.hash("dummy_password_never_matches"))
                .unwrap_or_else(|_| FALLBACK_DUMMY_HASH.to_string())
        })
        .clone()
}

/// Trait for retrieving authenticated users from storage
///
/// The application must implement this trait and register it with the container
/// to enable `Auth::user()`.
///
/// # Example
///
/// ```rust,ignore
/// use suprnova::auth::{UserProvider, Authenticatable};
/// use suprnova::FrameworkError;
/// use async_trait::async_trait;
/// use std::sync::Arc;
///
/// pub struct DatabaseUserProvider;
///
/// #[async_trait]
/// impl UserProvider for DatabaseUserProvider {
///     async fn retrieve_by_id(&self, id: &str) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
///         let id: i64 = id.parse().map_err(|_| FrameworkError::bad_request("user id must be numeric"))?;
///         let user = User::query()
///             .filter(Column::Id.eq(id as i32))
///             .first()
///             .await?;
///         Ok(user.map(|u| Arc::new(u) as Arc<dyn Authenticatable>))
///     }
/// }
/// ```
#[async_trait]
pub trait UserProvider: Send + Sync + 'static {
    /// Retrieve a user by their unique identifier
    ///
    /// The `id` is the string stored in the session's `user_id` field.
    /// For apps with numeric primary keys, parse the string: `id.parse::<i64>()`.
    /// For Magnetar-backed apps, this is the raw Magnetar `UserId` string (e.g. `"usr_<base58>"`).
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError>;

    /// Retrieve a user by credentials (for custom authentication flows)
    ///
    /// Default implementation returns None (not supported).
    /// Override this if you need to authenticate by credentials other than ID.
    async fn retrieve_by_credentials(
        &self,
        _credentials: &serde_json::Value,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }

    /// Validate credentials against a user
    ///
    /// Default implementation returns false (not supported).
    /// Override this if you need password validation.
    ///
    /// It answers whether the password matches and writes nothing, as
    /// Laravel's `validateCredentials` does: `Auth::validate` and a guard's
    /// `validate` call it to check a password without signing anybody in.
    /// A rewrite of the stored hash belongs in
    /// [`rehash_password_if_required`](Self::rehash_password_if_required),
    /// which only a sign-in calls.
    async fn validate_credentials(
        &self,
        _user: &dyn Authenticatable,
        _credentials: &serde_json::Value,
    ) -> Result<bool, FrameworkError> {
        Ok(false)
    }

    /// Rewrite the user's stored password hash when it needs it, after the
    /// password in `credentials` was validated and before the user is signed
    /// in. Default: does nothing.
    ///
    /// The session guard calls it in `attempt` and `once`, and
    /// `Auth::logout_other_devices` after its password check, as Laravel's
    /// `SessionGuard` calls `rehashPasswordIfRequired`. The sign-in is the
    /// one moment the plaintext is at hand and proven, so a new hash can be
    /// minted from it. An error fails the sign-in: a provider that cannot
    /// store the hash it needs returns that error rather than signing in a
    /// user on a hash it meant to replace.
    ///
    /// `EloquentUserProvider` and `DatabaseUserProvider` rewrite a hash a
    /// Laravel application on the same database would refuse while
    /// [`LaravelDatabase::is_shared`](crate::LaravelDatabase::is_shared) is
    /// on (LDB-004).
    async fn rehash_password_if_required(
        &self,
        _user: &dyn Authenticatable,
        _credentials: &serde_json::Value,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Run a fixed-cost hash verification to absorb the timing signal
    /// `validate_credentials` would emit on a matched user. The
    /// [`StatefulGuard`](super::StatefulGuard) calls this on the
    /// `retrieve_by_credentials` MISS branch so the wall-clock of
    /// `attempt(...)` for an unknown identifier matches the wall-clock
    /// for a known identifier with the wrong password - closing the
    /// account-enumeration timing oracle that the natural
    /// short-circuit-on-miss flow would otherwise create.
    ///
    /// Returns `Ok(false)` once the dummy verify completes; the
    /// result is discarded by the caller. The default implementation
    /// drives [`crate::hashing::verify_async`] against a throwaway
    /// hash minted by the *configured* driver so providers using the
    /// framework's hashing surface get equalisation for free - and so
    /// the dummy cost tracks the real verify cost regardless of
    /// `HASH_DRIVER` (a bcrypt-cost-12 dummy under `HASH_DRIVER=argon2id`
    /// would re-open the enumeration oracle through the timing gap).
    /// Providers whose `validate_credentials` uses a different verifier
    /// (custom JWT, external IDP) should override this to emit a
    /// comparable-cost no-op against their own primitive.
    async fn dummy_verify(&self) -> Result<bool, FrameworkError> {
        // Verify a throwaway hash produced by the configured driver, so
        // the work matches what a real verify of a stored password would
        // cost. `verify_async`/`verify_with` dispatch on the *stored*
        // hash's algorithm, so as long as the dummy hash is in the
        // configured algorithm the dummy and real paths run the same
        // primitive at the same cost. Any password input is rejected.
        let dummy_hash = dummy_hash();
        let _ = crate::hashing::verify_async("dummy_password_never_matches", &dummy_hash).await;
        Ok(false)
    }

    /// Look up a user by email for the auth-flow facades. Default: not
    /// supported (token-only providers return None).
    ///
    /// [`crate::auth_flows::EmailVerification::resend`] takes only the id
    /// from the result: it binds its link to
    /// [`verification_email`](Self::verification_email), because the address
    /// a user is looked up by need not be the one its verification is for.
    async fn retrieve_by_email(
        &self,
        _email: &str,
    ) -> Result<Option<crate::auth::AuthFlowUser>, FrameworkError> {
        Ok(None)
    }

    /// Whether this provider supports the verified-account fallback used by
    /// [`crate::auth_flows::PasswordReset`] when no Magnetar engine is installed.
    ///
    /// The default is `false`: custom providers must opt in explicitly rather
    /// than accepting reset tokens they cannot safely persist.
    fn supports_password_reset(&self) -> bool {
        false
    }

    /// Look up a verified user that may receive a provider-backed password
    /// reset link.
    ///
    /// Implementations must perform the email lookup and verification check as
    /// one provider operation so unknown and unverified addresses have the same
    /// externally observable result. The default disables the fallback.
    async fn retrieve_verified_user_for_password_reset(
        &self,
        _email: &str,
    ) -> Result<Option<crate::auth::AuthFlowUser>, FrameworkError> {
        Ok(None)
    }

    /// Look up a user by id, returning the auth-flow carrier (email/name).
    /// Used by PasswordReset to address the change-notification mail.
    /// Default: not supported.
    async fn flow_user_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<crate::auth::AuthFlowUser>, FrameworkError> {
        Ok(None)
    }

    /// The address a user's email verification is for, looked up by id.
    ///
    /// [`crate::auth_flows::EmailVerification::verify`] compares it with
    /// the address a link was sent to, so a link never verifies an address
    /// it was not sent to, and
    /// [`EmailVerification::resend`](crate::auth_flows::EmailVerification::resend)
    /// binds the link it mints to it, so the two agree. Default: the email of
    /// [`flow_user_by_id`](Self::flow_user_by_id). Override it when the
    /// verification address is not that one.
    async fn verification_email(&self, id: &str) -> Result<Option<String>, FrameworkError> {
        Ok(self.flow_user_by_id(id).await?.map(|user| user.email))
    }

    /// Mark a user's email verified. Default: unsupported.
    async fn mark_email_verified(&self, _id: &str) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal(
            "this user provider does not support email verification",
        ))
    }

    /// Mark a user's email verified only while `email` is still its
    /// verification address, and report whether it did.
    ///
    /// [`crate::auth_flows::EmailVerification::verify`] reads the address
    /// through [`verification_email`](Self::verification_email), checks that
    /// the link was mailed to it, and then calls this. The address can
    /// change in between, and a write that ignored it would verify a mailbox
    /// the link never reached. So the check and the write belong in one
    /// storage operation: `EloquentUserProvider` rereads the user under a
    /// row lock inside a transaction.
    ///
    /// # The default leaves a window
    ///
    /// The default reads the address with
    /// [`verification_email`](Self::verification_email), compares it with
    /// `email`, and then calls
    /// [`mark_email_verified`](Self::mark_email_verified). Those are two
    /// separate storage operations. An address change that commits between
    /// them is marked verified without proof of the new mailbox: the
    /// comparison saw the old address, and `mark_email_verified` stamps the
    /// row whatever address it holds by then.
    ///
    /// A custom provider closes the window by overriding this method, not
    /// `mark_email_verified`, so that the check and the write are one storage
    /// operation. Two shapes do it: a conditional write, such as
    /// `UPDATE users SET email_verified_at = ? WHERE id = ? AND email = ?`,
    /// that reports whether it matched a row; or a reread of the user under
    /// a row lock (`SELECT ... FOR UPDATE`) in the same transaction as the
    /// write. Return `Ok(false)`, and write nothing, when the address is no
    /// longer `email`.
    async fn mark_email_verified_for(&self, id: &str, email: &str) -> Result<bool, FrameworkError> {
        if self.verification_email(id).await?.as_deref() != Some(email) {
            return Ok(false);
        }
        self.mark_email_verified(id).await?;
        Ok(true)
    }

    /// Set a user's password hash. Default: unsupported.
    async fn set_password(&self, _id: &str, _hashed: &str) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal(
            "this user provider does not support password reset",
        ))
    }

    /// Whether a user's email is verified. Default: unsupported.
    async fn is_email_verified(&self, _id: &str) -> Result<bool, FrameworkError> {
        Err(FrameworkError::internal(
            "this user provider does not support email verification",
        ))
    }

    /// Send the verification link to the user `id` names.
    ///
    /// [`crate::auth_flows::EmailVerification::resend`] holds only the id
    /// the provider returned, not the model, so it sends through this
    /// method. The default sends the framework's
    /// [`VerifyEmailNotification`](crate::auth_flows::VerifyEmailNotification)
    /// to [`verification_email`](Self::verification_email), the address
    /// `verify` checks the link against, greeting the name of
    /// [`flow_user_by_id`](Self::flow_user_by_id). `EloquentUserProvider`
    /// loads the model and calls its
    /// [`MustVerifyEmail::send_email_verification_notification`](crate::MustVerifyEmail::send_email_verification_notification),
    /// so a model that sends its own message is honoured on this path too.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider reports no verification address
    /// for `id`: a provider that cannot name the address cannot verify the
    /// link either, and dropping the message without a word would hide that.
    /// Also returns the error of the notification.
    async fn send_email_verification_notification(
        &self,
        id: &str,
        verification_link: &str,
    ) -> Result<(), FrameworkError> {
        let Some(address) = self.verification_email(id).await? else {
            return Err(FrameworkError::internal(
                "the user provider reports no verification address for this user; \
                 implement verification_email or flow_user_by_id, or override \
                 send_email_verification_notification",
            ));
        };
        let name = self.flow_user_by_id(id).await?.and_then(|user| user.name);
        crate::auth_flows::mail::send_verification_notification(&address, name, verification_link)
            .await
    }

    /// Send the password-reset link to the user `id` names.
    ///
    /// [`crate::auth_flows::PasswordReset::send_link`] holds only the id the
    /// provider returned, not the model, so its provider path sends through
    /// this method. The default sends the framework's
    /// [`PasswordResetMail`](crate::auth_flows::PasswordResetMail) to the
    /// address [`flow_user_by_id`](Self::flow_user_by_id) returns, the one
    /// the reset flow also sends the password-changed mail to.
    /// `EloquentUserProvider` loads the model and calls its
    /// [`CanResetPassword::send_password_reset_notification`](crate::CanResetPassword::send_password_reset_notification),
    /// as Laravel's broker calls `sendPasswordResetNotification` on the user.
    ///
    /// # Errors
    ///
    /// Returns an error when [`flow_user_by_id`](Self::flow_user_by_id)
    /// finds no user for `id`, so a reset-capable provider that cannot name
    /// the address does not drop the link without a word. Also returns the
    /// error of the mail.
    async fn send_password_reset_notification(
        &self,
        id: &str,
        reset_link: &str,
    ) -> Result<(), FrameworkError> {
        let Some(user) = self.flow_user_by_id(id).await? else {
            return Err(FrameworkError::internal(
                "the user provider reports no address for this user; implement \
                 flow_user_by_id or override send_password_reset_notification",
            ));
        };
        crate::auth_flows::mail::send_password_reset_mail(&user.email, user.name, reset_link).await
    }
}
