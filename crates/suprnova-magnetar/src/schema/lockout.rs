//! Typed accessors for failed-login and lockout state.

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// Lockout fields required by abuse-limiting and account-unlock flows.
///
/// Lockout rows record failed sign-in attempts. The identity value returned
/// by [`LockoutFields::read_user_id`] is the attempted identity key (Magnetar
/// uses the normalized email), which deliberately may not correspond to any
/// stored user: attempts against unknown addresses are recorded with exactly
/// the same work as attempts against real accounts so lockout state cannot be
/// used as an account-enumeration oracle.
pub trait LockoutFields: EntityBinding {
    /// Read the lockout row identifier.
    fn read_lockout_id(model: &Self::Model) -> String;
    /// Set the lockout row identifier on a new row.
    fn write_lockout_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the attempted identity key.
    fn read_user_id(model: &Self::Model) -> String;
    /// Return the generated column containing the attempted identity key.
    fn user_id_column() -> Self::Column;
    /// Store the attempted identity key.
    fn write_user_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the failed-attempt timestamp.
    fn read_attempted_at(model: &Self::Model) -> DateTime<Utc>;
    /// Return the generated column containing the attempt timestamp.
    fn attempted_at_column() -> Self::Column;
    /// Store the failed-attempt timestamp.
    fn write_attempted_at(model: &mut Self::ActiveModel, value: DateTime<Utc>);
    /// Read the optional lock timestamp.
    fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Read the optional reason.
    fn read_reason(model: &Self::Model) -> Option<String>;
    /// Store the optional audit context, such as a client address.
    fn write_reason(model: &mut Self::ActiveModel, value: Option<&str>);
    /// Stamp or clear the lock timestamp.
    fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);
}
