//! The `{name}` observer of `{model}`.

use suprnova::{FrameworkError, Observer, async_trait};

use crate::models::{model_module}::{model};

/// Runs code when a `{model}` is written. `#[suprnova::observer]` installs
/// it when the application boots; each method runs after the write it is
/// named for, and the methods left out register nothing. The attribute
/// names the model by its full path, since it reaches the model's events
/// through the module the model is in.
pub struct {name};

#[suprnova::observer(crate::models::{model_module}::{model})]
#[async_trait]
impl Observer<{model}> for {name} {
    /// A `{model}` was created.
    async fn created(&self, _{model_param}: &{model}) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// A `{model}` was updated.
    async fn updated(&self, _previous: &{model}, _current: &{model}) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// A `{model}` was deleted.
    async fn deleted(&self, _{model_param}: &{model}, _is_force: bool) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// A soft-deleted `{model}` was restored.
    async fn restored(&self, _{model_param}: &{model}) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// A `{model}` was force-deleted.
    async fn force_deleted(&self, _{model_param}: &{model}) -> Result<(), FrameworkError> {
        Ok(())
    }
}
