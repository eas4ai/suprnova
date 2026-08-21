//! Typed accessors for the application session entity.

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// Session fields required by session and revocation stores.
pub trait SessionFields: EntityBinding {
    /// Read the session identifier.
    fn read_session_id(model: &Self::Model) -> String;
    /// Return the generated session identifier column.
    fn session_id_column() -> Self::Column;
    /// Read the owning user identifier.
    fn read_user_id(model: &Self::Model) -> String;
    /// Return the generated owning-user column.
    fn user_id_column() -> Self::Column;
    /// Convert an opaque user ID into the binding's database value.
    fn user_id_value(value: &str) -> sea_orm::Value {
        value.to_owned().into()
    }
    /// Read the stored session-token digest.
    fn read_token_digest(model: &Self::Model) -> String;
    /// Read the expiry timestamp.
    fn read_expires_at(model: &Self::Model) -> DateTime<Utc>;
    /// Read the optional revocation timestamp.
    fn read_revoked_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Return the generated revocation timestamp column.
    fn revoked_at_column() -> Self::Column;
    /// Stamp or clear the revocation timestamp.
    fn write_revoked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);
}
