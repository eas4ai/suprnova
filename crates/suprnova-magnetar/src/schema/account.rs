//! Typed accessors for linked OAuth/provider accounts.

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// Linked-account fields required by account linking and sign-in census code.
///
/// `(provider, provider_account_id)` must be protected by a driver-level
/// unique index (spec 01: "driver-level uniqueness on (provider key,
/// provider account id)", rejecting check-then-insert by name; mirrors the
/// `RememberRow::selector` convention of documenting the required index
/// on the field it governs). Authenticated
/// [`crate::storage::LinkedAccountStore::create`] and explicit
/// [`crate::storage::LinkedAccountInitializer::initialize`] both map the
/// driver's unique-constraint violation to [`crate::Error::Conflict`] rather
/// than enforcing uniqueness in application code.
pub trait LinkedAccountFields: EntityBinding {
    /// Read the linked-account identifier.
    fn read_account_id(model: &Self::Model) -> String;
    /// Return the generated linked-account identifier column.
    fn account_id_column() -> Self::Column;
    /// Set the linked-account identifier when constructing a new model.
    fn write_account_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the owning user identifier.
    fn read_user_id(model: &Self::Model) -> String;
    /// Return the generated owning-user column.
    fn user_id_column() -> Self::Column;
    /// Convert an opaque user ID into the binding's database value.
    fn user_id_value(value: &str) -> sea_orm::Value {
        value.to_owned().into()
    }
    /// Set the owning user identifier when constructing a new model.
    fn write_user_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the provider key.
    fn read_provider(model: &Self::Model) -> String;
    /// Return the generated provider column.
    fn provider_column() -> Self::Column;
    /// Set the provider key when constructing a new model.
    fn write_provider(model: &mut Self::ActiveModel, value: &str);
    /// Read the provider's account identifier.
    fn read_provider_account_id(model: &Self::Model) -> String;
    /// Return the generated provider-account-identifier column.
    fn provider_account_id_column() -> Self::Column;
    /// Set the provider's account identifier when constructing a new model.
    fn write_provider_account_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the optional access token ciphertext.
    fn read_access_token(model: &Self::Model) -> Option<String>;
    /// Read the optional refresh token ciphertext.
    fn read_refresh_token(model: &Self::Model) -> Option<String>;
    /// Read the optional provider-token expiry.
    fn read_expires_at(model: &Self::Model) -> Option<DateTime<Utc>>;
}

/// Compatibility name for generic code that refers to linked-account
/// capabilities as bindings.
pub use LinkedAccountFields as LinkedAccountBinding;
