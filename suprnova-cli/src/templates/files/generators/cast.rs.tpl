//! The `{name}` attribute cast.

use suprnova::{Cast, FrameworkError};

/// Converts a model attribute between the value the model holds and the
/// value its column stores. Name it on a model field:
/// `#[model(casts = { field = crate::{module}::{name} })]`.
pub struct {name};

impl Cast for {name} {
    /// The type the model field holds.
    type Runtime = String;
    /// The type the column stores.
    type Storage = String;

    /// Convert the model's value into the value the column stores.
    fn to_storage(value: &String) -> Result<String, FrameworkError> {
        Ok(value.clone())
    }

    /// Convert the column's value into the value the model holds.
    fn from_storage(stored: &String) -> Result<String, FrameworkError> {
        Ok(stored.clone())
    }
}
