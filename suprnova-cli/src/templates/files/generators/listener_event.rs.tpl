//! The `{name}` listener of `{event}`.

use suprnova::{FrameworkError, Listener, async_trait};

use crate::events::{event_module}::{event};

/// Handles `{event}`. Register it in `src/bootstrap.rs`:
/// `EventFacade::listen::<{event}, _>(Arc::new({name})).await;`.
pub struct {name};

#[async_trait]
impl Listener<{event}> for {name} {
    /// Handle the event. An `Err` stops a fail-fast dispatch.
    async fn handle(&self, _event: &{event}) -> Result<(), FrameworkError> {
        Ok(())
    }
}
