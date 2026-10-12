//! Typed accessors for the application user entity.

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// The reserved representation for a passwordless value in a NOT-NULL column.
///
/// A binding with a nullable password column should map `None` to `NULL`
/// instead. A NOT-NULL binding may map `None` to this empty value, but its
/// [`UserFields::read_password_hash`] implementation MUST map the value back
/// to `None`. This sentinel is never a password hash and MUST NOT be passed to
/// a verifier.
pub const NOT_NULL_PASSWORD_EMPTY_SENTINEL: &str = "";

/// User fields consumed by generic Magnetar services.
pub trait UserFields: EntityBinding {
    /// Read the application-defined user identifier as an opaque string.
    fn read_user_id(model: &Self::Model) -> String;
    /// Return the generated column containing the user identifier.
    fn user_id_column() -> Self::Column;
    /// Convert an opaque user ID into the binding's database value.
    fn user_id_value(value: &str) -> sea_orm::Value {
        value.to_owned().into()
    }
    /// Set the application-defined user identifier on a new row.
    ///
    /// Generic stores call this only at user creation, with an opaque
    /// numeric-string identifier; bindings with integer keys may parse it.
    fn write_user_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the normalized email address.
    fn read_email(model: &Self::Model) -> String;
    /// Return the generated column containing the normalized email.
    fn email_column() -> Self::Column;
    /// Store the normalized email address.
    fn write_email(model: &mut Self::ActiveModel, value: &str);
    /// Read a password hash, mapping passwordless representations to `None`.
    fn read_password_hash(model: &Self::Model) -> Option<String>;
    /// Return the generated column containing the password hash.
    fn password_hash_column() -> Self::Column;
    /// Store a password hash or the binding's documented passwordless value.
    fn write_password_hash(model: &mut Self::ActiveModel, value: Option<&str>);
    /// Read the nullable lock timestamp, when the app stores it on users.
    fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Return the generated column containing the nullable lock timestamp.
    fn locked_at_column() -> Self::Column;
    /// Set or clear the nullable user lock timestamp.
    fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);
}

/// Optional user columns supported by bindings that persist these fields.
pub trait UserOptionalFields: EntityBinding {
    /// Read the optional display name.
    fn read_name(model: &Self::Model) -> Option<String>;
    /// Read the nullable email-verification timestamp.
    fn read_email_verified_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Set or clear the nullable email-verification timestamp.
    fn write_email_verified_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);
    /// Read the optional remember-me token.
    fn read_remember_token(model: &Self::Model) -> Option<String>;
    /// Set or clear the optional remember-me token.
    fn write_remember_token(model: &mut Self::ActiveModel, value: Option<&str>);
}

/// Optional optimistic-concurrency capability for global JWT revocation.
pub trait SessionEpoch: EntityBinding {
    /// Read the current authentication epoch.
    fn auth_epoch(model: &Self::Model) -> u64;
    /// Return the generated epoch column.
    fn auth_epoch_column() -> Self::Column;
    /// Convert an authentication epoch into the binding's database value.
    fn auth_epoch_value(value: u64) -> crate::Result<sea_orm::Value> {
        Ok(value.into())
    }
    /// Set an epoch value on an active model.
    fn write_auth_epoch(model: &mut Self::ActiveModel, value: u64);
}

/// Return only a verifier-safe hash from a user binding.
///
/// This defensive boundary filters the documented empty sentinel even if an
/// application binding accidentally returns it instead of mapping it to
/// `None`. Callers that verify passwords MUST use this helper rather than
/// handing the raw storage representation to a verifier.
pub fn password_hash_for_verifier<B: UserFields>(model: &B::Model) -> Option<String> {
    B::read_password_hash(model).filter(|hash| !hash.is_empty())
}

/// Compatibility name for generic code that refers to the user descriptor
/// capability as a binding.
pub use UserFields as UserBinding;
