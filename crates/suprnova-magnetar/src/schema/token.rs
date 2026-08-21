//! Typed accessors for single-use tokens and token records.

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// Fields for the unified single-use token store.
pub trait TokenFields: EntityBinding {
    /// Read the token row identifier.
    fn read_token_id(model: &Self::Model) -> String;
    /// Return the generated token identifier column.
    fn token_id_column() -> Self::Column;
    /// Read the optional owning user identifier.
    fn read_user_id(model: &Self::Model) -> Option<String>;
    /// Return the generated owning-user column.
    fn user_id_column() -> Self::Column;
    /// Convert an opaque user ID into the binding's database value.
    fn user_id_value(value: &str) -> sea_orm::Value {
        value.to_owned().into()
    }
    /// Read the plugin-owned purpose namespace.
    fn read_purpose(model: &Self::Model) -> String;
    /// Return the generated purpose column.
    fn purpose_column() -> Self::Column;
    /// Return the generated purpose column name.
    fn purpose_column_name() -> &'static str;
    /// Read the stored digest, never plaintext.
    fn read_digest(model: &Self::Model) -> String;
    /// Return the generated digest column.
    fn digest_column() -> Self::Column;
    /// Return the generated digest column name.
    fn digest_column_name() -> &'static str;
    /// Read token expiry.
    fn read_expires_at(model: &Self::Model) -> DateTime<Utc>;
    /// Return the generated expiry column.
    fn expires_at_column() -> Self::Column;
    /// Read the optional one-time-use timestamp.
    fn read_used_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Return the generated use timestamp column.
    fn used_at_column() -> Self::Column;
    /// Return the generated use timestamp column name.
    fn used_at_column_name() -> &'static str;
    /// Stamp or clear the one-time-use timestamp.
    fn write_used_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);
    /// Set the token row identifier when constructing a new model.
    fn write_token_id(model: &mut Self::ActiveModel, value: &str);
    /// Set the owning user identifier when constructing a new model.
    fn write_user_id(model: &mut Self::ActiveModel, value: Option<&str>);
    /// Set the token purpose when constructing a new model.
    fn write_purpose(model: &mut Self::ActiveModel, value: &str);
    /// Set the token digest when constructing a new model.
    fn write_digest(model: &mut Self::ActiveModel, value: &str);
    /// Set token expiry when constructing a new model.
    fn write_expires_at(model: &mut Self::ActiveModel, value: DateTime<Utc>);
}

/// Fields for an immutable audit/token-record row.
pub trait TokenRecordFields: EntityBinding {
    /// Read the record identifier.
    fn read_record_id(model: &Self::Model) -> String;
    /// Read the associated token identifier.
    fn read_token_id(model: &Self::Model) -> String;
    /// Read the owning user identifier.
    fn read_user_id(model: &Self::Model) -> String;
    /// Read the plugin-owned purpose namespace.
    fn read_purpose(model: &Self::Model) -> String;
    /// Read the stored digest.
    fn read_digest(model: &Self::Model) -> String;
}
