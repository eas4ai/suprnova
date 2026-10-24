//! The `{model}` model factory.

use suprnova::Factory;

use crate::models::{model_module}::{model};

/// Builds `{model}` models for tests and seeders:
/// `{name}::new().count(10).create_many().await?` inserts ten of them.
pub struct {name};

impl Factory for {name} {
    type Model = {model};

    /// The attributes a new `{model}` starts with. Set each field the model
    /// needs, with a `suprnova::Faker` value where it should vary. A field
    /// left out keeps its `Default` value.
    fn definition() -> {model} {
        {model} {
            ..Default::default()
        }
    }
}
