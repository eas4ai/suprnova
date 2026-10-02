//! Typed accessors for OAuth, WebAuthn, and device ceremonies.

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// Ceremony fields required by consume, peek, and compare-and-swap flows.
pub trait CeremonyFields: EntityBinding {
    /// Read the ceremony row identifier.
    fn read_ceremony_id(model: &Self::Model) -> String;
    /// Return the generated ceremony identifier column.
    fn ceremony_id_column() -> Self::Column;
    /// Read the namespaced ceremony kind.
    fn read_kind(model: &Self::Model) -> String;
    /// Return the generated kind column.
    fn kind_column() -> Self::Column;
    /// Return the generated kind column name.
    fn kind_column_name() -> &'static str;
    /// Read the unique selector.
    fn read_selector(model: &Self::Model) -> String;
    /// Return the generated selector column.
    fn selector_column() -> Self::Column;
    /// Return the generated selector column name.
    fn selector_column_name() -> &'static str;
    /// Read opaque serialized payload bytes without lossy decoding.
    fn read_payload(model: &Self::Model) -> Vec<u8>;
    /// Read the state used for conditional transitions.
    fn read_state(model: &Self::Model) -> String;
    /// Return the generated state column.
    fn state_column() -> Self::Column;
    /// Return the generated state column name.
    fn state_column_name() -> &'static str;
    /// Read ceremony expiry.
    fn read_expires_at(model: &Self::Model) -> DateTime<Utc>;
    /// Return the generated expiry column.
    fn expires_at_column() -> Self::Column;
    /// Read the optional consume timestamp.
    fn read_used_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Return the generated consume timestamp column.
    fn used_at_column() -> Self::Column;
    /// Write a next state during a conditional transition.
    fn write_state(model: &mut Self::ActiveModel, state: &str);
    /// Stamp or clear the consume timestamp.
    fn write_used_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);
    /// Set the ceremony identifier when constructing a new model.
    fn write_ceremony_id(model: &mut Self::ActiveModel, value: &str);
    /// Set the ceremony kind when constructing a new model.
    fn write_kind(model: &mut Self::ActiveModel, value: &str);
    /// Set the ceremony selector when constructing a new model.
    fn write_selector(model: &mut Self::ActiveModel, value: &str);
    /// Set the ceremony payload when constructing a new model.
    fn write_payload(model: &mut Self::ActiveModel, value: &[u8]);
    /// Set ceremony expiry when constructing a new model.
    fn write_expires_at(model: &mut Self::ActiveModel, value: DateTime<Utc>);
}

/// Compatibility name for generic code that refers to ceremony capabilities
/// as bindings.
pub use CeremonyFields as CeremonyBinding;
