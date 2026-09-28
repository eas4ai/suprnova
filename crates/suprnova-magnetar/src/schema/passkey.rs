//! Typed accessors for application passkey credentials.

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// Passkey fields required by registration and assertion flows.
///
/// The credential identifier column stores the deployed base64-standard
/// encoding; the public-key column stores the deployed `data_json` envelope
/// (outer JSON carrying the base64 of the serialized webauthn credential).
pub trait PasskeyFields: EntityBinding {
    /// Read the passkey row identifier.
    fn read_passkey_id(model: &Self::Model) -> String;
    /// Return the generated passkey identifier column.
    fn passkey_id_column() -> Self::Column;
    /// Set the passkey row identifier on a new row.
    fn write_passkey_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the owning user identifier.
    fn read_user_id(model: &Self::Model) -> String;
    /// Return the generated owning-user column.
    fn user_id_column() -> Self::Column;
    /// Convert an opaque user ID into the binding's database value.
    fn user_id_value(value: &str) -> sea_orm::Value {
        value.to_owned().into()
    }
    /// Set the owning user identifier on a new row.
    fn write_user_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the base64-encoded WebAuthn credential identifier.
    fn read_credential_id(model: &Self::Model) -> String;
    /// Return the generated credential identifier column.
    fn credential_id_column() -> Self::Column;
    /// Store the base64-encoded WebAuthn credential identifier.
    fn write_credential_id(model: &mut Self::ActiveModel, value: &str);
    /// Read the serialized public-key envelope (`data_json`).
    fn read_public_key(model: &Self::Model) -> String;
    /// Store the serialized public-key envelope (`data_json`).
    fn write_public_key(model: &mut Self::ActiveModel, value: &str);
    /// Read the authenticator signature counter.
    fn read_sign_count(model: &Self::Model) -> i64;
    /// Read the optional serialized transport list.
    fn read_transports(model: &Self::Model) -> Option<String>;
    /// Read the creation timestamp.
    fn read_created_at(model: &Self::Model) -> DateTime<Utc>;
}
