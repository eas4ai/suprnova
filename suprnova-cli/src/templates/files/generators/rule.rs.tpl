//! The `{name}` validation rule.

use suprnova::{Rule, ValidationMessage};

/// A custom validation rule. Use it in a `validate!` row
/// (`field => Required, {name};`) or call `{name}.passes(value)`.
pub struct {name};

impl Rule for {name} {
    /// Check one value. Return `Err` with the message a failing value
    /// shows, such as `Err("must be uppercase".to_string().into())`.
    fn passes(&self, _value: &str) -> Result<(), ValidationMessage> {
        Ok(())
    }
}
