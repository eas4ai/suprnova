//! The `{name}` global scope of `{model}`.

use suprnova::{Builder, GlobalScope};

use crate::models::{model_module}::{model};

/// Constrains every `{model}::query()`. Register it at boot:
/// `suprnova::ScopeRegistry::register::<{model}, _>({name});`.
pub struct {name};

impl GlobalScope<{model}> for {name} {
    /// Add the scope's constraints to the query.
    fn apply(&self, query: Builder<{model}>) -> Builder<{model}> {
        query
    }
}
